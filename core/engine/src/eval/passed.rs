//! Passed-pawn scoring. The passed-pawn detection lives in `pawns.rs`
//! (it needs only pawn structure); this module layers on the
//! attack-aware bonuses that depend on the full evaluator scratchpad —
//! king proximity to the block square, whether the block square is
//! defended, and whether the promotion path is contested.
//!
//! Mirrors `Evaluation::passed<Us>()` in the reference. Weight tables
//! carry over verbatim.
//!
//! The per-sub-term [`PassedBreakdown`] exposed alongside the aggregate
//! score is the teaching hook: rank bonus, king-proximity endgame
//! adjustment, free-advance bonus, and file-based stopper penalty each
//! land in their own field so the analysis pipeline can attribute a
//! passed-pawn swing to the specific reason it moved. Per-passer detail
//! (which pawn moved up a rank, which lost its free-advance clearance)
//! is deferred — the narrow breakdown mirrors the other Phase-0 splits.

use super::Evaluator;
use crate::attacks::square_distance;
use crate::bitboard::{forward_file_bb, passed_pawn_span};
use crate::types::{Color, Direction, PieceType, Rank, Score, Square};

// =========================================================================
// Sub-term breakdown
// =========================================================================

/// Per-sub-term passed-pawn breakdown. The four fields sum to the
/// aggregate passed-pawn score this colour contributes — see
/// [`total`](PassedBreakdown::total).
///
/// The split mirrors the teaching concepts a classical coach would
/// name:
/// - **Rank bonus**: raw "this pawn is advanced" reward from the
///   per-relative-rank table.
/// - **King proximity**: endgame adjustment favouring positions where
///   *their* king is far from the block square and *ours* is close.
/// - **Free advance**: scales by how clear the promotion path is —
///   vacated block square, undefended stopper ray, supporting major
///   piece behind.
/// - **Stopper penalty**: queenside-folded file penalty; rook-file
///   passers edge out central ones.
///
/// Sub-terms absorb the candidate-passer halving per-component (via
/// [`Score`]'s componentwise division). Integer truncation means the
/// sum of halved sub-terms can differ by up to ~1 cp per passer from
/// the reference's "halve the aggregate bonus" formulation — within
/// the noise of the classical weight tuning.
///
/// Mirrors the Phase-0 [`crate::pawns::PawnsBreakdown`] /
/// [`super::PiecesBreakdown`] / [`super::MobilityBreakdown`] /
/// [`super::ThreatsBreakdown`] / [`super::KingBreakdown`] pattern.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PassedBreakdown {
    /// `PASSED_RANK[r_rel]` contributions, summed across every passer
    /// (candidate-halved when applicable).
    pub rank_bonus: Score,
    /// Endgame-only king-proximity adjustments — their king ahead of
    /// the block square favours us, ours ahead favours them.
    pub king_proximity: Score,
    /// Clear-path bonus proportional to `5*r_rel - 13` with a
    /// per-tier multiplier (35 / 20 / 9 / 0) and a +5 bump when we
    /// defend the block square or have a major piece behind.
    pub free_advance: Score,
    /// Per-passer file penalty `PASSED_FILE * fold_to_queenside(file)`.
    /// Stored as a negative [`Score`] so `total()` sums to the
    /// aggregate without sign flips at the call site.
    pub stopper_penalty: Score,
}

impl PassedBreakdown {
    /// An all-zero breakdown.
    pub const fn zero() -> PassedBreakdown {
        PassedBreakdown {
            rank_bonus: Score::ZERO,
            king_proximity: Score::ZERO,
            free_advance: Score::ZERO,
            stopper_penalty: Score::ZERO,
        }
    }

    /// Sum of every sub-term. Equal to the aggregate passed-pawn
    /// score this colour contributes.
    pub fn total(&self) -> Score {
        self.rank_bonus + self.king_proximity + self.free_advance + self.stopper_penalty
    }
}

// =========================================================================
// Weight tables
// =========================================================================

/// Per-relative-rank passed-pawn bonus. Indexed by the rank from the
/// pawn's own perspective; ranks 0 and 7 never host a passed pawn and
/// those slots are zero placeholders.
const PASSED_RANK: [Score; 8] = [
    Score::ZERO,
    Score::new(10, 28),
    Score::new(17, 33),
    Score::new(15, 41),
    Score::new(62, 72),
    Score::new(168, 177),
    Score::new(276, 260),
    Score::ZERO,
];

/// Per-passer file penalty. Indexed by queenside-folded file (0..=3).
const PASSED_FILE: Score = Score::new(11, 8);

// =========================================================================
// Public entry point
// =========================================================================

pub(crate) fn evaluate(e: &Evaluator<'_>, us: Color) -> PassedBreakdown {
    let them = !us;
    let us_idx = us.index();
    let them_idx = them.index();
    let pos = e.pos;
    let up = Direction::pawn_push(us);
    let our_king = pos.king_square(us);
    let their_king = pos.king_square(them);

    let mut breakdown = PassedBreakdown::zero();

    for s in e.pawns.passed_pawns[us_idx] {
        let r_rel = s.rank().from_perspective(us).index() as i32;

        let mut rank_component = PASSED_RANK[r_rel as usize];
        let mut kp_component = Score::ZERO;
        let mut fa_component = Score::ZERO;

        // Only accrue advance / king-proximity bonuses for pawns past
        // the 3rd relative rank — nothing further advanced than that
        // really threatens to promote.
        if r_rel > Rank::R3.index() as i32 {
            let w = 5 * r_rel - 13;
            let block_sq = s + up;

            // King-proximity eg adjustment: more weight to the enemy
            // king being close to the block square (they need to stop
            // it), less weight to ours being close (we need to
            // escort it).
            let our_kp = king_proximity(our_king, block_sq);
            let their_kp = king_proximity(their_king, block_sq);
            kp_component += Score::new(0, ((their_kp * 19) / 4 - our_kp * 2) * w);

            // Second-push proximity penalty, unless already on the 7th
            // rank (where the second push is the queening move itself).
            if r_rel != Rank::R7.index() as i32 {
                let second = block_sq + up;
                kp_component -= Score::new(0, king_proximity(our_king, second) * w);
            }

            // Free-advance bonus. When the block square is empty, scale
            // the bonus by the danger the promotion path carries.
            if pos.piece_on(block_sq).is_none() {
                let squares_to_queen = forward_file_bb(us, s);
                let mut unsafe_squares = passed_pawn_span(us, s);

                // Enemy rook/queen behind our pawn along the file acts
                // as a supporting attacker on every square ahead, so in
                // the absence of such a piece we can restrict the
                // "unsafe" set to squares the enemy actually attacks.
                let behind = forward_file_bb(them, s)
                    & (pos.pieces(PieceType::Rook) | pos.pieces(PieceType::Queen));

                if (pos.pieces_by_color(them) & behind).is_empty() {
                    unsafe_squares &= e.attacked_by_all[them_idx];
                }

                // Tier the bonus by how clear the path is.
                let k = if unsafe_squares.is_empty() {
                    35
                } else if (unsafe_squares & squares_to_queen).is_empty() {
                    20
                } else if !unsafe_squares.contains(block_sq) {
                    9
                } else {
                    0
                };

                // Bonus bump when we either have a rook/queen behind
                // the passer supporting it, or we defend the block
                // square directly.
                let our_defender_behind = (pos.pieces_by_color(us) & behind).any();
                let block_defended = e.attacked_by_all[us_idx].contains(block_sq);
                let k = if our_defender_behind || block_defended {
                    k + 5
                } else {
                    k
                };

                fa_component += Score::new(k * w, k * w);
            }
        }

        // Candidate-passer scaling: halve when the square directly in
        // front is already occupied by a pawn, or a pawn standing on
        // that square wouldn't itself be passed (so the passer needs
        // more than one push to really promote). Applied per sub-term
        // so the breakdown still sums cleanly via [`Score`]'s
        // componentwise division.
        let next = s + up;
        let next_blocked = pos.pieces(PieceType::Pawn).contains(next);
        if next_blocked || !pos.pawn_passed(us, next) {
            rank_component = rank_component / 2;
            kp_component = kp_component / 2;
            fa_component = fa_component / 2;
        }

        breakdown.rank_bonus += rank_component;
        breakdown.king_proximity += kp_component;
        breakdown.free_advance += fa_component;
        breakdown.stopper_penalty -= PASSED_FILE * s.file().fold_to_queenside().index() as i32;
    }

    breakdown
}

fn king_proximity(king_sq: Square, target: Square) -> i32 {
    (square_distance(king_sq, target) as i32).min(5)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::position::Position;

    fn passed_breakdown(fen: &str, us: Color) -> PassedBreakdown {
        let pos = Position::from_fen(fen).unwrap();
        let mut e = Evaluator::new(&pos);
        e.initialize(Color::White);
        e.initialize(Color::Black);
        super::super::pieces::evaluate(&mut e, Color::White);
        super::super::pieces::evaluate(&mut e, Color::Black);
        evaluate(&e, us)
    }

    #[test]
    fn advanced_passer_scores_better_than_early_passer() {
        // Same pawn placement but different rank: a passer on the 7th
        // rank should score higher than one on the 4th.
        let low = "4k3/8/8/8/3P4/8/8/4K3 w - - 0 1";
        let high = "4k3/3P4/8/8/8/8/8/4K3 w - - 0 1";
        let a = passed_breakdown(low, Color::White).total();
        let b = passed_breakdown(high, Color::White).total();
        assert!(
            b.mg().0 > a.mg().0,
            "pawn on the 7th should score more mg than a pawn on the 4th ({} vs {})",
            b.mg().0,
            a.mg().0,
        );
    }

    #[test]
    fn no_passers_yields_zero_score() {
        // Starting position — no passed pawns, so the passed-pawn term
        // contributes zero across every sub-term.
        let b = passed_breakdown(
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            Color::White,
        );
        assert_eq!(b.total(), Score::ZERO);
        assert_eq!(b.rank_bonus, Score::ZERO);
        assert_eq!(b.king_proximity, Score::ZERO);
        assert_eq!(b.free_advance, Score::ZERO);
        assert_eq!(b.stopper_penalty, Score::ZERO);
    }

    #[test]
    fn central_passer_costs_less_than_rook_file_passer() {
        // PASSED_FILE penalty grows with queenside-fold index. An
        // a-file or h-file passer (fold index 0) pays zero file
        // penalty; a d-file passer pays the full S(33, 24) = 3 * file
        // penalty. So the central passer scores less, not more.
        let rook_file = "4k3/8/8/P7/8/8/8/4K3 w - - 0 1";
        let central = "4k3/8/8/3P4/8/8/8/4K3 w - - 0 1";
        let r = passed_breakdown(rook_file, Color::White).total();
        let c = passed_breakdown(central, Color::White).total();
        assert!(
            r.mg().0 > c.mg().0,
            "a-file passer should score higher than d-file due to PassedFile penalty ({} vs {})",
            r.mg().0,
            c.mg().0,
        );
    }

    #[test]
    fn breakdown_total_equals_sum_of_subterms() {
        // .total() must equal field-sum — the main evaluator relies on
        // this identity to recover the pre-split aggregate score.
        let fens = [
            // No passers — everything zero.
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            // A central passer on the 4th.
            "4k3/8/8/8/3P4/8/8/4K3 w - - 0 1",
            // An advanced passer.
            "4k3/3P4/8/8/8/8/8/4K3 w - - 0 1",
            // A rook-file passer — no file penalty, different k-tier.
            "4k3/8/8/P7/8/8/8/4K3 w - - 0 1",
        ];
        for fen in fens {
            let pb = passed_breakdown(fen, Color::White);
            let summed = pb.rank_bonus + pb.king_proximity + pb.free_advance + pb.stopper_penalty;
            assert_eq!(pb.total(), summed, "total() must equal field-sum for {fen}",);
        }
    }

    #[test]
    fn stopper_penalty_is_non_positive_and_equals_file_fold() {
        // The stopper penalty is the only sub-term that's
        // unconditionally non-positive — every passer subtracts
        // `PASSED_FILE * fold_to_queenside(file)` regardless of rank.
        // Rook-file passers (fold 0) pay zero; central passers pay
        // more.
        let rook_file = "4k3/8/8/P7/8/8/8/4K3 w - - 0 1";
        let central = "4k3/8/8/3P4/8/8/8/4K3 w - - 0 1";
        let r = passed_breakdown(rook_file, Color::White);
        let c = passed_breakdown(central, Color::White);
        assert_eq!(r.stopper_penalty, Score::ZERO);
        assert!(c.stopper_penalty.mg().0 < 0);
        assert!(c.stopper_penalty.eg().0 < 0);
    }
}
