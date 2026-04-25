//! Threats evaluation: bonuses for pieces we attack, penalties for our
//! pieces attacked by the enemy, and piece-specific threat motifs like
//! knight/slider forks on the enemy queen.
//!
//! Mirrors `Evaluation::threats<Us>()` in the Stockfish reference. Reads
//! the per-colour attack tables populated by [`super::initialize`] and
//! [`super::pieces::evaluate`]. All numerical parameters are factual
//! data from `evaluate.cpp`.

use super::Evaluator;
use crate::attacks::knight_attacks;
use crate::bitboard::{RANK_3, RANK_6};
use crate::magics::{bishop_attacks, rook_attacks};
use crate::types::{Color, Direction, PieceType, Score};

// =========================================================================
// Weight tables
// =========================================================================
//
// Per-target-piece-type bonuses. Indexed by `PieceType::index()` (1..=6)
// with slot 0 unused; slot 6 (king) unused because kings can't be
// "attacked" as targets in the way pawn/minor/rook are.

const THREAT_BY_MINOR: [Score; 7] = [
    Score::ZERO,
    Score::new(6, 32),
    Score::new(59, 41),
    Score::new(79, 56),
    Score::new(90, 119),
    Score::new(79, 161),
    Score::ZERO,
];
const THREAT_BY_ROOK: [Score; 7] = [
    Score::ZERO,
    Score::new(3, 44),
    Score::new(38, 71),
    Score::new(38, 61),
    Score::new(0, 38),
    Score::new(51, 38),
    Score::ZERO,
];

const HANGING: Score = Score::new(69, 36);
const KNIGHT_ON_QUEEN: Score = Score::new(16, 12);
const RESTRICTED_PIECE: Score = Score::new(7, 7);
const SLIDER_ON_QUEEN: Score = Score::new(59, 18);
const THREAT_BY_KING: Score = Score::new(24, 89);
const THREAT_BY_PAWN_PUSH: Score = Score::new(48, 39);
const THREAT_BY_SAFE_PAWN: Score = Score::new(173, 94);

// =========================================================================
// Breakdown
// =========================================================================

/// Per-sub-term decomposition of the Stockfish-11 threats score. Each
/// named pattern carries its own contribution so the teaching layer
/// can attribute swings to the exact concept. The sum equals the
/// aggregate threats score this colour contributes — see
/// [`total`](ThreatsBreakdown::total).
///
/// Field names mirror Stockfish's internal terminology so a reader
/// cross-referencing the engine and the reference can line them up
/// 1:1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThreatsBreakdown {
    /// Minor-piece (knight or bishop) attacks on defended or weak
    /// non-pawn enemies, weighted by target piece type.
    pub by_minor: Score,
    /// Rook attacks on weak enemies, weighted by target piece type.
    pub by_rook: Score,
    /// Flat bonus when our king attacks a weak enemy piece.
    pub by_king: Score,
    /// Per-hanging-piece bonus. Hanging = weak AND undefended, or
    /// non-pawn enemies we double-attack.
    pub hanging: Score,
    /// Per-square bonus counting enemy squares they attack that we
    /// also cover but they don't strongly protect — squares where
    /// we've contested their piece activity.
    pub restricted: Score,
    /// Per-target bonus for our safe pawns (pawns on squares not
    /// attacked, or that we also attack) threatening enemy non-pawn
    /// pieces.
    pub by_safe_pawn: Score,
    /// Per-target bonus for one-ply pawn pushes that land safely and
    /// would threaten an enemy non-pawn piece after the push.
    pub by_pawn_push: Score,
    /// Bonus for each square a knight could jump to (safe, in our
    /// mobility area) from which it would attack the enemy queen.
    pub knight_on_queen: Score,
    /// Bonus for bishop or rook attacks aimed at the enemy queen,
    /// restricted to squares we doubly-defend.
    pub slider_on_queen: Score,
}

impl ThreatsBreakdown {
    /// An all-zero breakdown.
    pub const fn zero() -> ThreatsBreakdown {
        ThreatsBreakdown {
            by_minor: Score::ZERO,
            by_rook: Score::ZERO,
            by_king: Score::ZERO,
            hanging: Score::ZERO,
            restricted: Score::ZERO,
            by_safe_pawn: Score::ZERO,
            by_pawn_push: Score::ZERO,
            knight_on_queen: Score::ZERO,
            slider_on_queen: Score::ZERO,
        }
    }

    /// Sum of every sub-term. Equal to the aggregate threats score
    /// this colour contributes (what the pre-split single `Score`
    /// return value held).
    pub fn total(&self) -> Score {
        self.by_minor
            + self.by_rook
            + self.by_king
            + self.hanging
            + self.restricted
            + self.by_safe_pawn
            + self.by_pawn_push
            + self.knight_on_queen
            + self.slider_on_queen
    }
}

// =========================================================================
// Public entry point
// =========================================================================

pub(crate) fn evaluate(e: &Evaluator<'_>, us: Color) -> ThreatsBreakdown {
    let them = !us;
    let us_idx = us.index();
    let them_idx = them.index();
    let pos = e.pos;

    let non_pawn_enemies = pos.pieces_by_color(them) & !pos.pieces(PieceType::Pawn);

    // Enemy-defended squares considered strong: defended by a pawn, or
    // attacked twice by them while we single-cover (or less).
    let strongly_protected = e.attacked_by[them_idx][PieceType::Pawn.index()]
        | (e.attacked_by_2[them_idx] & !e.attacked_by_2[us_idx]);

    // Non-pawn enemies standing on strongly-protected squares. Still
    // eligible for our minor-piece threat bonus because a knight or
    // bishop attack contests them.
    let defended = non_pawn_enemies & strongly_protected;

    // Enemies that are not strongly protected and are attacked by any
    // of our pieces — targets of our threats.
    let weak = pos.pieces_by_color(them) & !strongly_protected & e.attacked_by_all[us_idx];

    let mut breakdown = ThreatsBreakdown::zero();

    if (defended | weak).any() {
        // Minor-piece threats: both defended and weak enemy pieces.
        let by_minor = (defended | weak)
            & (e.attacked_by[us_idx][PieceType::Knight.index()]
                | e.attacked_by[us_idx][PieceType::Bishop.index()]);
        for sq in by_minor {
            if let Some(p) = pos.piece_on(sq) {
                breakdown.by_minor += THREAT_BY_MINOR[p.kind().index()];
            }
        }

        // Rook threats: weak enemies only. A rook going after a
        // protected piece would typically trade down material.
        let by_rook = weak & e.attacked_by[us_idx][PieceType::Rook.index()];
        for sq in by_rook {
            if let Some(p) = pos.piece_on(sq) {
                breakdown.by_rook += THREAT_BY_ROOK[p.kind().index()];
            }
        }

        // King threats — our king forks a weak enemy.
        if (weak & e.attacked_by[us_idx][PieceType::King.index()]).any() {
            breakdown.by_king += THREAT_BY_KING;
        }

        // Hanging pieces: weak enemies that are also undefended, or
        // non-pawn enemies we double-attack.
        let hanging_set =
            !e.attacked_by_all[them_idx] | (non_pawn_enemies & e.attacked_by_2[us_idx]);
        breakdown.hanging += HANGING * (weak & hanging_set).popcount() as i32;
    }

    // Restricted piece: enemy squares they attack that we also attack
    // but they don't strongly protect. Limits their piece activity.
    let restricted = e.attacked_by_all[them_idx] & !strongly_protected & e.attacked_by_all[us_idx];
    breakdown.restricted += RESTRICTED_PIECE * restricted.popcount() as i32;

    // Squares "relatively safe" for our pieces: either not attacked by
    // the enemy, or we attack them too.
    let safe = !e.attacked_by_all[them_idx] | e.attacked_by_all[us_idx];

    // Safe-pawn threats: our pawns on safe squares hitting non-pawn
    // enemies.
    let safe_pawns = pos.pieces_of(us, PieceType::Pawn) & safe;
    let safe_pawn_attacks = safe_pawns.pawn_attacks(us) & non_pawn_enemies;
    breakdown.by_safe_pawn += THREAT_BY_SAFE_PAWN * safe_pawn_attacks.popcount() as i32;

    // Pawn-push threats: squares our pawns can reach in one move
    // (counting double pushes from the starting rank) that are safe,
    // and the enemy non-pawn pieces those potential advances would
    // attack.
    let up = Direction::pawn_push(us);
    let our_pawns = pos.pieces_of(us, PieceType::Pawn);
    let empty = !pos.occupied();
    let third_rank = match us {
        Color::White => RANK_3,
        Color::Black => RANK_6,
    };
    let mut push_targets = our_pawns.shift(up) & empty;
    push_targets |= (push_targets & third_rank).shift(up) & empty;
    push_targets &= !e.attacked_by[them_idx][PieceType::Pawn.index()] & safe;
    let push_attacks = push_targets.pawn_attacks(us) & non_pawn_enemies;
    breakdown.by_pawn_push += THREAT_BY_PAWN_PUSH * push_attacks.popcount() as i32;

    // Piece-on-enemy-queen motifs. Only fire with a single enemy queen —
    // in multi-queen positions the reference skips this term.
    if pos.count(them, PieceType::Queen) == 1 {
        let their_queen_sq = pos.pieces_of(them, PieceType::Queen).lsb();
        let safe_for_queen_threats = e.mobility_area[us_idx] & !strongly_protected;

        // Knight fork / attack against their queen.
        let knight_on_queen =
            e.attacked_by[us_idx][PieceType::Knight.index()] & knight_attacks(their_queen_sq);
        breakdown.knight_on_queen +=
            KNIGHT_ON_QUEEN * (knight_on_queen & safe_for_queen_threats).popcount() as i32;

        // Slider on queen: bishop or rook attacks that land on squares
        // reaching the queen, double-defended for us.
        let occ = pos.occupied();
        let slider_on_queen = (e.attacked_by[us_idx][PieceType::Bishop.index()]
            & bishop_attacks(their_queen_sq, occ))
            | (e.attacked_by[us_idx][PieceType::Rook.index()] & rook_attacks(their_queen_sq, occ));
        breakdown.slider_on_queen += SLIDER_ON_QUEEN
            * (slider_on_queen & safe_for_queen_threats & e.attacked_by_2[us_idx]).popcount()
                as i32;
    }

    breakdown
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::position::Position;

    fn threats_breakdown(fen: &str, us: Color) -> ThreatsBreakdown {
        let pos = Position::from_fen(fen).unwrap();
        let mut e = Evaluator::new(&pos);
        e.initialize(Color::White);
        e.initialize(Color::Black);
        super::super::pieces::evaluate(&mut e, Color::White);
        super::super::pieces::evaluate(&mut e, Color::Black);
        evaluate(&e, us)
    }

    #[test]
    fn startpos_threats_are_symmetric() {
        let w = threats_breakdown(
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            Color::White,
        );
        let b = threats_breakdown(
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            Color::Black,
        );
        assert_eq!(w, b);
    }

    #[test]
    fn knight_forking_rook_yields_threat_bonus() {
        // White knight on c6 attacks black rook on a7. Black rook is
        // not defended, so this is a hanging-rook threat. The
        // positive contribution should land on by_minor
        // (THREAT_BY_MINOR[ROOK]) and hanging — verify both fields
        // are strictly positive and the aggregate follows.
        let p = Position::from_fen("4k3/r7/2N5/8/8/8/8/4K3 w - - 0 1").unwrap();
        let mut e = Evaluator::new(&p);
        e.initialize(Color::White);
        e.initialize(Color::Black);
        super::super::pieces::evaluate(&mut e, Color::White);
        super::super::pieces::evaluate(&mut e, Color::Black);
        let w = evaluate(&e, Color::White);
        assert!(
            w.by_minor.mg().0 > 0,
            "by_minor should fire for a minor attacking a rook, got {:?}",
            w.by_minor,
        );
        assert!(
            w.hanging.mg().0 > 0,
            "hanging should fire for the undefended rook, got {:?}",
            w.hanging,
        );
        assert!(w.total().mg().0 > 0);
    }

    #[test]
    fn restricted_piece_counts_contested_squares() {
        // Black knight on e4 attacks a handful of squares; the subset
        // of those that our king on e1 also attacks (and that black
        // doesn't strongly protect) should produce a positive
        // `restricted` sub-term.
        let p = Position::from_fen("4k3/8/8/8/4n3/8/8/4K3 w - - 0 1").unwrap();
        let mut e = Evaluator::new(&p);
        e.initialize(Color::White);
        e.initialize(Color::Black);
        super::super::pieces::evaluate(&mut e, Color::White);
        super::super::pieces::evaluate(&mut e, Color::Black);
        let w = evaluate(&e, Color::White);
        assert!(
            w.restricted.mg().0 > 0,
            "restricted sub-term should fire when we contest enemy attack squares, got {:?}",
            w.restricted,
        );
    }

    #[test]
    fn breakdown_total_equals_sum_of_subterms() {
        // Sanity check: ThreatsBreakdown::total() is the raw sum of
        // all 9 Score fields. Pick a non-trivial position so multiple
        // sub-terms fire and the sum is an interesting number.
        let b = threats_breakdown("4k3/r7/2N5/8/8/8/8/4K3 w - - 0 1", Color::White);
        let expected = b.by_minor
            + b.by_rook
            + b.by_king
            + b.hanging
            + b.restricted
            + b.by_safe_pawn
            + b.by_pawn_push
            + b.knight_on_queen
            + b.slider_on_queen;
        assert_eq!(b.total(), expected);
    }
}
