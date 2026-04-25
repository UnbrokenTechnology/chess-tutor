//! King-safety aggregator: combines the pawn-shelter score with a
//! kingDanger accumulator fed by enemy attack coverage, safe checks, pinned
//! defenders, and flank pressure, then applies pawnless-flank and
//! flank-attack penalties.
//!
//! Mirrors `Evaluation::king<Us>()` in the Stockfish reference. Every
//! numerical weight here is factual data carried over verbatim; the
//! shape of the accumulator and the Score conversion formula are too.
//!
//! The per-sub-term [`KingBreakdown`] exposed alongside the aggregate
//! score is the teaching hook: shelter, danger, pawnless flank, and
//! flank-attack pressure each land in their own field so the analysis
//! pipeline can attribute a king-safety swing to the specific concept
//! that moved. `danger` stays atomic — the quadratic blend of ~10 raw
//! signals (safe checks, attacker count × weight, weak-ring squares,
//! blockers, unsafe checks, mobility diff, shelter discount, knight
//! defender, enemy-queen absence, base constant) is irreducible for
//! teaching purposes; finer splits would surface noise.

use super::Evaluator;
use crate::attacks::knight_attacks;
use crate::bitboard::{Bitboard, KING_FLANK, RANK_1, RANK_2, RANK_3, RANK_6, RANK_7, RANK_8};
use crate::magics::{bishop_attacks, rook_attacks};
use crate::pawns;
use crate::types::{Color, PieceType, Score};

// =========================================================================
// Sub-term breakdown
// =========================================================================

/// Per-sub-term king-safety breakdown. The four fields sum to the
/// aggregate king score this colour contributes — see
/// [`total`](KingBreakdown::total).
///
/// Signs are baked in: `shelter` is the raw shelter/storm bonus (often
/// positive); `danger`, `pawnless_flank`, and `flank_attacks` are
/// already-negated penalty contributions (so their direct sum is the
/// subtraction the aggregator applies). This keeps `total()` a simple
/// field-sum without any sign flips at the call site.
///
/// Mirrors the Phase-0 [`crate::pawns::PawnsBreakdown`] /
/// [`super::PiecesBreakdown`] / [`super::MobilityBreakdown`] /
/// [`super::ThreatsBreakdown`] pattern: the sub-terms live here, the
/// top-level evaluator reads `.total()`, and the teaching pipeline
/// surfaces the individual fields as named
/// [`crate::analysis::TermId`] variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KingBreakdown {
    /// Pawn shelter / storm bonus — the starting value the reference's
    /// aggregator seeds the running score with.
    pub shelter: Score,
    /// Quadratic-in-mg, linear-in-eg penalty converted from the
    /// `king_danger` accumulator. Zero unless `king_danger > 100`.
    /// Stored as a negative [`Score`] so `total()` adds rather than
    /// subtracts.
    pub danger: Score,
    /// Fixed `S(17, 95)` penalty when no pawn of either colour stands
    /// on the king's flank (so no blockade material is available).
    /// Stored negated.
    pub pawnless_flank: Score,
    /// Flank-attack pressure penalty — per-square linear mg term
    /// scaling with the count of enemy attacks on the king's camp
    /// flank. Stored negated.
    pub flank_attacks: Score,
}

impl KingBreakdown {
    /// An all-zero breakdown.
    pub const fn zero() -> KingBreakdown {
        KingBreakdown {
            shelter: Score::ZERO,
            danger: Score::ZERO,
            pawnless_flank: Score::ZERO,
            flank_attacks: Score::ZERO,
        }
    }

    /// Sum of every sub-term. Equal to the aggregate king score this
    /// colour contributes (what the pre-split `king: [Score; 2]` field
    /// held).
    pub fn total(&self) -> Score {
        self.shelter + self.danger + self.pawnless_flank + self.flank_attacks
    }
}

// =========================================================================
// Weight constants
// =========================================================================

const ROOK_SAFE_CHECK: i32 = 1080;
const QUEEN_SAFE_CHECK: i32 = 780;
const BISHOP_SAFE_CHECK: i32 = 635;
const KNIGHT_SAFE_CHECK: i32 = 790;

const PAWNLESS_FLANK: Score = Score::new(17, 95);
const FLANK_ATTACKS: Score = Score::new(8, 0);

// =========================================================================
// Public entry point
// =========================================================================

/// Evaluate king safety for `us`. The returned [`KingBreakdown`]
/// decomposes into shelter, danger, pawnless-flank, and flank-attacks
/// sub-terms; call [`KingBreakdown::total`] for the aggregate [`Score`]
/// that the pre-split `evaluate()` returned.
pub(crate) fn evaluate(e: &Evaluator<'_>, us: Color) -> KingBreakdown {
    let them = !us;
    let us_idx = us.index();
    let them_idx = them.index();
    let pos = e.pos;
    let king_sq = pos.king_square(us);

    // Seed with pawn-shelter / pawn-storm evaluation. Same function the
    // reference calls via `pe->king_safety<Us>(pos)`.
    let shelter = pawns::king_safety(pos, us);

    let attacked_by_all_them = e.attacked_by_all[them_idx];
    let attacked_by_all_us = e.attacked_by_all[us_idx];
    let attacked_by_2_us = e.attacked_by_2[us_idx];
    let attacked_by_2_them = e.attacked_by_2[them_idx];
    let attacked_by_us_king = e.attacked_by[us_idx][PieceType::King.index()];
    let attacked_by_us_queen = e.attacked_by[us_idx][PieceType::Queen.index()];
    let attacked_by_us_knight = e.attacked_by[us_idx][PieceType::Knight.index()];
    let attacked_by_them_rook = e.attacked_by[them_idx][PieceType::Rook.index()];
    let attacked_by_them_queen = e.attacked_by[them_idx][PieceType::Queen.index()];
    let attacked_by_them_bishop = e.attacked_by[them_idx][PieceType::Bishop.index()];
    let attacked_by_them_knight = e.attacked_by[them_idx][PieceType::Knight.index()];

    // Squares the enemy attacks that are defended at most once on our
    // side — and only by our queen or king, never by a minor/rook (so
    // those defences are fragile).
    let weak = attacked_by_all_them
        & !attacked_by_2_us
        & (!attacked_by_all_us | attacked_by_us_king | attacked_by_us_queen);

    // Squares from which an enemy piece could give a "safe" check: not
    // blocked by their own piece, and either we don't attack the square
    // at all or the square is in our weak-defence set the enemy already
    // double-covers.
    let mut safe = !pos.pieces_by_color(them);
    safe &= !attacked_by_all_us | (weak & attacked_by_2_them);

    // Potential check rays from our king square, computed with our own
    // queen treated as transparent (so the reference counts a check that
    // our queen would ordinarily block as still a "check" for danger
    // purposes). This makes the check detector conservative, which is
    // what we want for a safety heuristic.
    let king_occupancy = pos.occupied() ^ pos.pieces_of(us, PieceType::Queen);
    let rook_lines = rook_attacks(king_sq, king_occupancy);
    let bishop_lines = bishop_attacks(king_sq, king_occupancy);

    let mut king_danger: i32 = 0;
    let mut unsafe_checks = Bitboard::EMPTY;

    // Rook checks.
    let rook_checks = rook_lines & safe & attacked_by_them_rook;
    if rook_checks.any() {
        king_danger += ROOK_SAFE_CHECK;
    } else {
        unsafe_checks |= rook_lines & attacked_by_them_rook;
    }

    // Queen checks. Counted only from squares where the enemy can't give
    // a rook check instead — rook checks are more dangerous, so when
    // both are available we credit just the rook.
    let queen_checks = (rook_lines | bishop_lines)
        & attacked_by_them_queen
        & safe
        & !attacked_by_us_queen
        & !rook_checks;
    if queen_checks.any() {
        king_danger += QUEEN_SAFE_CHECK;
    }

    // Bishop checks. Filtered out where the enemy could give a queen
    // check instead (queens strictly dominate bishops on the same square).
    let bishop_checks = bishop_lines & attacked_by_them_bishop & safe & !queen_checks;
    if bishop_checks.any() {
        king_danger += BISHOP_SAFE_CHECK;
    } else {
        unsafe_checks |= bishop_lines & attacked_by_them_bishop;
    }

    // Knight checks. A square checks our king iff a knight on that
    // square attacks the king, which is the same set as knight_attacks
    // from the king square by symmetry.
    let knight_checks = knight_attacks(king_sq) & attacked_by_them_knight;
    if (knight_checks & safe).any() {
        king_danger += KNIGHT_SAFE_CHECK;
    } else {
        unsafe_checks |= knight_checks;
    }

    // Flank terms. "Camp" is our half of the board; `KingFlank[file]`
    // picks the 3-4 files centred on the king's file. The intersection
    // gives the region where flank-side attack pressure builds.
    let camp = match us {
        Color::White => Bitboard::ALL ^ RANK_6 ^ RANK_7 ^ RANK_8,
        Color::Black => Bitboard::ALL ^ RANK_1 ^ RANK_2 ^ RANK_3,
    };
    let flank = KING_FLANK[king_sq.file().index()];
    let flank_attacked = attacked_by_all_them & flank & camp;
    let flank_attacked_twice = flank_attacked & attacked_by_2_them;
    let flank_defended = attacked_by_all_us & flank & camp;

    let king_flank_attack =
        flank_attacked.popcount() as i32 + flank_attacked_twice.popcount() as i32;
    let king_flank_defense = flank_defended.popcount() as i32;

    // Aggregate kingDanger from all signals. Coefficients are factual
    // weights from the reference.
    let mg_mobility_diff = (e.mobility[them_idx].total() - e.mobility[us_idx].total())
        .mg()
        .0;
    let mg_shelter = shelter.mg().0;
    let enemy_has_no_queen = pos.count(them, PieceType::Queen) == 0;
    let our_king_has_knight_defender = (attacked_by_us_knight & attacked_by_us_king).any();

    king_danger += e.king_attackers_count[them_idx] * e.king_attackers_weight[them_idx]
        + 185 * (e.king_ring[us_idx] & weak).popcount() as i32
        + 148 * unsafe_checks.popcount() as i32
        + 98 * pos.blockers_for_king(us).popcount() as i32
        + 69 * e.king_attacks_count[them_idx]
        + 3 * king_flank_attack * king_flank_attack / 8
        + mg_mobility_diff
        - 873 * (enemy_has_no_queen as i32)
        - 100 * (our_king_has_knight_defender as i32)
        - 6 * mg_shelter / 8
        - 4 * king_flank_defense
        + 37;

    // Convert the accumulated kingDanger units into a Score penalty.
    // Quadratic in the mg component (attack escalates), linear in the
    // eg (material-down endgames care less about king attacks). Stored
    // negated so `total()` reduces to a plain field-sum.
    let danger = if king_danger > 100 {
        -Score::new(king_danger * king_danger / 4096, king_danger / 16)
    } else {
        Score::ZERO
    };

    // Pawnless-flank penalty — the flank has no pawn of either colour.
    let pawnless_flank = if (pos.pieces(PieceType::Pawn) & flank).is_empty() {
        -PAWNLESS_FLANK
    } else {
        Score::ZERO
    };

    // Flank-attack penalty scales linearly with the weighted attack
    // count (already built with a double-attack bonus).
    let flank_attacks = -(FLANK_ATTACKS * king_flank_attack);

    KingBreakdown {
        shelter,
        danger,
        pawnless_flank,
        flank_attacks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::position::Position;

    /// Build an Evaluator, run initialize + pieces so the attack tables
    /// are populated, then call king::evaluate. This mirrors what the
    /// real evaluate() pipeline does and is the only sensible way to
    /// exercise king() directly.
    fn king_breakdown(fen: &str, us: Color) -> KingBreakdown {
        let pos = Position::from_fen(fen).unwrap();
        let mut e = Evaluator::new(&pos);
        e.initialize(Color::White);
        e.initialize(Color::Black);
        super::super::pieces::evaluate(&mut e, Color::White);
        super::super::pieces::evaluate(&mut e, Color::Black);
        evaluate(&e, us)
    }

    #[test]
    fn startpos_king_safety_is_mirrored_between_colours() {
        // In the starting position the two kings have identical shelter
        // and zero incoming attackers, so the per-colour king score
        // must match exactly.
        let pos = Position::startpos();
        let mut e = Evaluator::new(&pos);
        e.initialize(Color::White);
        e.initialize(Color::Black);
        super::super::pieces::evaluate(&mut e, Color::White);
        super::super::pieces::evaluate(&mut e, Color::Black);
        let w = evaluate(&e, Color::White);
        let b = evaluate(&e, Color::Black);
        assert_eq!(w, b, "startpos king-safety should be perfectly symmetric");
    }

    #[test]
    fn exposed_king_scores_worse_than_sheltered_king() {
        // Two positions identical except for the king's pawn cover.
        // Sheltered: intact f2/g2/h2 in front of a g1 king.
        // Exposed: those pawns pushed forward off the shelter file.
        let sheltered = "4k3/8/8/8/8/8/5PPP/6K1 w - - 0 1";
        let exposed = "4k3/8/8/8/8/5P1P/6P1/6K1 w - - 0 1";
        let s = king_breakdown(sheltered, Color::White).total();
        let x = king_breakdown(exposed, Color::White).total();
        assert!(
            s.mg().0 > x.mg().0,
            "sheltered king mg ({}) should beat exposed mg ({})",
            s.mg().0,
            x.mg().0,
        );
    }

    #[test]
    fn king_safety_penalises_enemy_attacker_presence() {
        // Same white king shelter (f2/g2/h2) in both positions. The
        // second position adds a black rook on g3 staring at the king
        // flank. That should worsen white's king-safety score.
        let calm = "4k3/8/8/8/8/8/5PPP/6K1 w - - 0 1";
        let rook_near_king = "4k3/8/8/8/8/6r1/5PPP/6K1 w - - 0 1";
        let a = king_breakdown(calm, Color::White).total();
        let b = king_breakdown(rook_near_king, Color::White).total();
        assert!(
            a.mg().0 > b.mg().0,
            "rook near king should worsen king-safety mg ({} -> {})",
            a.mg().0,
            b.mg().0,
        );
    }

    #[test]
    fn breakdown_total_equals_sum_of_subterms() {
        // .total() must equal field-sum — the main evaluator relies on
        // this identity to recover the pre-split aggregate score.
        let fens = [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "4k3/8/8/8/8/8/5PPP/6K1 w - - 0 1",
            "4k3/8/8/8/8/6r1/5PPP/6K1 w - - 0 1",
            // Pawnless flank position — the white king's h-file flank
            // has no pawns, so the pawnless-flank sub-term should fire.
            "4k3/8/8/8/8/8/PPPPPPPP/6K1 w - - 0 1",
        ];
        for fen in fens {
            let kb = king_breakdown(fen, Color::White);
            let summed = kb.shelter + kb.danger + kb.pawnless_flank + kb.flank_attacks;
            assert_eq!(kb.total(), summed, "total() must equal field-sum for {fen}",);
        }
    }

    #[test]
    fn pawnless_flank_fires_when_flank_has_no_pawns() {
        // White king on h1 — the flank for file H covers files f, g,
        // h. In the first FEN f2/g2/h2 all hold white pawns so the
        // penalty is suppressed; in the second FEN every pawn lives on
        // the queenside (a2-d2) so the flank contains no pawn of
        // either colour and `pawnless_flank` fires negative.
        let with_pawns = "4k3/8/8/8/8/8/5PPP/7K w - - 0 1";
        let without = "4k3/8/8/8/8/8/PPPP4/7K w - - 0 1";
        let a = king_breakdown(with_pawns, Color::White);
        let b = king_breakdown(without, Color::White);
        assert_eq!(a.pawnless_flank, Score::ZERO);
        assert!(b.pawnless_flank.mg().0 < 0);
        assert!(b.pawnless_flank.eg().0 < 0);
    }
}
