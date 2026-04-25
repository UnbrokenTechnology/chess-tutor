//! Pawn-structure evaluation: scores and bitboard artifacts that `evaluate.rs`
//! consumes when piecing together the full position evaluation.
//!
//! What this module produces (per colour) via [`evaluate`]:
//!
//! - A `Score` that captures everything pawns-only can see: bonuses for
//!   connected/phalanx pawns, and penalties for isolated, backward, doubled,
//!   and weak-lever pawns.
//! - A `pawn_attacks` bitboard — every square attacked by one of this colour's
//!   pawns.
//! - A `pawn_attacks_span` bitboard — pawn_attacks extended with the attack
//!   span of every non-backward, non-blocked pawn, projected all the way to
//!   promotion. Used by the main evaluator to identify stable outposts and
//!   to score space.
//! - A `passed_pawns` bitboard — pawns whose promotion is no longer
//!   preventable by opposing pawns alone. The per-passer score lives in the
//!   main evaluator because it needs king and piece-attack information we
//!   don't have here.
//!
//! King safety — pawn-shelter strength in front of the king, and the pawn
//! storm coming from the opposite side — is a separate [`king_safety`]
//! function in this module. It's kept alongside pawn evaluation because
//! both are pure pawn/king geometry; in the reference the two are members of
//! the same `Pawns::Entry`.
//!
//! Numerical weight tables (`CONNECTED`, `SHELTER_STRENGTH`,
//! `UNBLOCKED_STORM`, etc.) are the factual parameters from Stockfish 11's
//! `pawns.cpp`, used under the idea/expression split. All code and
//! identifiers are independently authored.
//!
//! Caching is deliberately not implemented yet. The reference caches the
//! full entry in a 131 072-slot hash table keyed by `pawn_key`; we compute
//! on demand. `Position::pawn_key()` is already plumbed through so a future
//! cache is a drop-in addition.

use crate::attacks::{king_attacks, pawn_attacks_from, square_distance};
use crate::bitboard::{
    adjacent_files_bb, file_bb, forward_file_bb, forward_ranks_bb, passed_pawn_span,
    pawn_attack_span, rank_bb, Bitboard,
};
use crate::position::Position;
use crate::types::{CastlingRights, Color, Direction, File, PieceType, Rank, Score, Square};

// =========================================================================
// Weight tables
// =========================================================================
//
// Factual numerical parameters from Stockfish 11's `pawns.cpp`. Names and
// layout are independently chosen; values are carried over verbatim.

/// Penalty for a backward pawn.
const BACKWARD: Score = Score::new(9, 24);

/// Penalty when an enemy storm pawn is blocked directly by our shelter pawn
/// on the third rank — applied in place of the normal storm term.
const BLOCKED_STORM: Score = Score::new(82, 82);

/// Penalty for a doubled pawn that has no support from its own side.
const DOUBLED: Score = Score::new(11, 56);

/// Penalty for an isolated pawn.
const ISOLATED: Score = Score::new(5, 15);

/// Additional penalty for a pawn attacked by more than one enemy pawn with
/// no friendly support to defend it.
const WEAK_LEVER: Score = Score::new(0, 56);

/// Additional penalty for isolated / backward pawns that are also
/// unopposed (no enemy pawn on the same file ahead) — those are prime
/// attack targets.
const WEAK_UNOPPOSED: Score = Score::new(13, 27);

/// Per-relative-rank bonus for a connected pawn (one with a same-colour
/// neighbour on an adjacent file, either in phalanx on the same rank or
/// supporting it from directly behind). Indexed by [`Rank::index()`] from
/// the pawn's own perspective. Ranks 1 and 8 never host pawns, so those
/// slots are zero placeholders.
const CONNECTED: [i32; 8] = [0, 7, 8, 12, 29, 48, 86, 0];

/// Shelter strength for the king's own pawns, indexed
/// `[distance_to_queenside_half][our_frontmost_pawn_rank_from_our_pov]`.
///
/// The file index runs 0..=3: 0 = a/h, 1 = b/g, 2 = c/f, 3 = d/e (the file
/// is always folded to the queenside half for symmetry). The rank index is
/// the relative rank of our frontmost pawn on that file, or 0 when we have
/// no shelter pawn on that file.
const SHELTER_STRENGTH: [[i32; 8]; 4] = [
    [-6, 81, 93, 58, 39, 18, 25, 0],
    [-43, 61, 35, -49, -29, -11, -63, 0],
    [-10, 75, 23, -2, 32, 3, -45, 0],
    [-39, -13, -29, -52, -48, -67, -166, 0],
];

/// Storm danger from an unblocked enemy pawn advancing toward our king,
/// indexed `[distance_to_queenside_half][their_frontmost_pawn_rank_from_our_pov]`.
/// The rank index is zero when the enemy has no pawn on that file within the
/// half of the board closer to our king.
const UNBLOCKED_STORM: [[i32; 8]; 4] = [
    [85, -289, -166, 97, 50, 45, 50, 0],
    [46, -25, 122, 45, 37, -10, 20, 0],
    [-6, 51, 168, 34, -2, -22, -14, 0],
    [-15, -11, 101, 4, 11, -15, -29, 0],
];

/// Base shelter/storm bonus added before any per-file contributions.
const SHELTER_BASE: Score = Score::new(5, 5);

/// Per-square mg penalty accrued for each king-to-nearest-own-pawn step in
/// the endgame. Reference: `- make_score(0, 16 * minPawnDist)`.
const KING_TO_NEAREST_PAWN_PENALTY_EG: i32 = 16;

// =========================================================================
// Public output shape
// =========================================================================

/// Per-colour breakdown of the pawn-structure score into its named
/// sub-terms. The teaching layer consumes this directly — each field maps
/// to a chess concept a student can read about (isolated pawn, connected
/// pawn, etc.). Values are cumulative across all of this colour's pawns;
/// bonuses are positive, penalties are negative.
///
/// The sum of all fields equals the aggregate pawn-structure score this
/// colour contributes — see [`total`](PawnsBreakdown::total).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PawnsBreakdown {
    /// Bonus for pawns that have a same-colour neighbour on an adjacent
    /// file — either in phalanx (same rank) or supporting from directly
    /// behind. Scaled by rank and by whether the pawn is opposed.
    pub connected: Score,
    /// Penalty for a pawn with no same-colour neighbour on either
    /// adjacent file.
    pub isolated: Score,
    /// Penalty for a pawn that cannot advance safely: no same-colour
    /// neighbour that could defend the push square, and the push square
    /// is either blocked or under enemy-pawn lever attack.
    pub backward: Score,
    /// Penalty for a pawn stacked behind a same-colour pawn on the same
    /// file, with no friendly support from an adjacent file.
    pub doubled: Score,
    /// Extra penalty applied to isolated or backward pawns that are also
    /// unopposed (no enemy pawn blocks the file ahead of them) — a prime
    /// attack target.
    pub weak_unopposed: Score,
    /// Penalty for a pawn attacked by more than one enemy pawn with no
    /// same-colour pawn supporting it from behind.
    pub weak_lever: Score,
}

impl PawnsBreakdown {
    /// An all-zero breakdown.
    pub const fn zero() -> PawnsBreakdown {
        PawnsBreakdown {
            connected: Score::ZERO,
            isolated: Score::ZERO,
            backward: Score::ZERO,
            doubled: Score::ZERO,
            weak_unopposed: Score::ZERO,
            weak_lever: Score::ZERO,
        }
    }

    /// Sum of every sub-term. Equal to the aggregate pawn-structure score
    /// this colour contributes before king safety.
    pub fn total(&self) -> Score {
        self.connected
            + self.isolated
            + self.backward
            + self.doubled
            + self.weak_unopposed
            + self.weak_lever
    }
}

/// The artifacts of pawn-structure evaluation that other evaluators need.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PawnsEval {
    /// Pawn-only score per colour. White is index 0, black index 1.
    /// Always equal to `breakdowns[c].total()`; kept as a cached field so
    /// hot callers don't re-sum the sub-terms on every access.
    pub scores: [Score; 2],
    /// Granular per-sub-term pawn-structure scores per colour. The
    /// teaching layer reads these to attribute score changes to specific
    /// chess concepts.
    pub breakdowns: [PawnsBreakdown; 2],
    /// Bitboard of passed pawns per colour. These are detected here and
    /// scored later when the full attack picture is available.
    pub passed_pawns: [Bitboard; 2],
    /// Every square attacked by any pawn of the given colour.
    pub pawn_attacks: [Bitboard; 2],
    /// Pawn attacks extended with the `pawn_attack_span` of every
    /// non-backward, non-blocked pawn — the set of squares this colour's
    /// pawns might plausibly attack if we keep pushing.
    pub pawn_attacks_span: [Bitboard; 2],
}

impl PawnsEval {
    /// Signed pawn score from white's perspective: `scores[white] − scores[black]`.
    pub fn score(&self) -> Score {
        self.scores[Color::White.index()] - self.scores[Color::Black.index()]
    }
}

// =========================================================================
// Public entry point
// =========================================================================

/// Evaluate the pawn structure of both colours on this position.
pub fn evaluate(pos: &Position) -> PawnsEval {
    let mut eval = PawnsEval {
        scores: [Score::ZERO; 2],
        breakdowns: [PawnsBreakdown::zero(); 2],
        passed_pawns: [Bitboard::EMPTY; 2],
        pawn_attacks: [Bitboard::EMPTY; 2],
        pawn_attacks_span: [Bitboard::EMPTY; 2],
    };
    for &us in &Color::both() {
        evaluate_color(pos, us, &mut eval);
    }
    eval
}

// =========================================================================
// Per-colour evaluation
// =========================================================================

fn evaluate_color(pos: &Position, us: Color, eval: &mut PawnsEval) {
    let them = !us;
    let up = Direction::pawn_push(us);

    let our_pawns = pos.pieces_of(us, PieceType::Pawn);
    let their_pawns = pos.pieces_of(them, PieceType::Pawn);
    let their_double_attacks = their_pawns.pawn_double_attacks(them);

    let our_attacks = our_pawns.pawn_attacks(us);
    eval.pawn_attacks[us.index()] = our_attacks;

    // Attack-span starts equal to the raw attack set and accumulates
    // contributions from every non-backward, non-blocked pawn below.
    let mut attacks_span = our_attacks;
    let mut breakdown = PawnsBreakdown::zero();
    let mut passed = Bitboard::EMPTY;

    for s in our_pawns {
        let r_rel = s.rank().from_perspective(us).index() as i32;

        let opposed = their_pawns & forward_file_bb(us, s);
        let blocked = their_pawns & (s + up);
        let stoppers = their_pawns & passed_pawn_span(us, s);
        let lever = their_pawns & pawn_attacks_from(us, s);
        let lever_push = their_pawns & pawn_attacks_from(us, s + up);
        let doubled = our_pawns & (s - up);
        let neighbours = our_pawns & adjacent_files_bb(s);
        let phalanx = neighbours & rank_bb(s.rank());
        let support = neighbours & rank_bb((s - up).rank());

        // Backward: no same-colour neighbour that could defend the push
        // target or any square beyond it, *and* the pawn is either blocked
        // or the push square is under enemy pawn lever attack.
        let has_advancing_neighbour = (neighbours & forward_ranks_bb(them, s + up)).any();
        let is_backward = !has_advancing_neighbour && (lever_push | blocked).any();

        if !is_backward && blocked.is_empty() {
            attacks_span |= pawn_attack_span(us, s);
        }

        // Passed: no stoppers other than levers / leverPush we outnumber,
        // or a single blocker we could lever past from rank 5+.
        let is_passed = {
            let no_stoppers_beyond_lever = (stoppers ^ lever).is_empty();
            let only_lever_push_stoppers =
                (stoppers ^ lever_push).is_empty() && phalanx.popcount() >= lever_push.popcount();
            let lone_blocker_can_be_levered = stoppers == blocked
                && r_rel >= Rank::R5.index() as i32
                && (support.shift(up) & !(their_pawns | their_double_attacks)).any();
            no_stoppers_beyond_lever || only_lever_push_stoppers || lone_blocker_can_be_levered
        };

        if is_passed {
            passed = passed | s;
        }

        // Connected / phalanx bonus.
        if (support | phalanx).any() {
            let phalanx_bonus = if phalanx.any() { 1 } else { 0 };
            let opposed_penalty = if opposed.any() { 1 } else { 0 };
            let v = CONNECTED[r_rel as usize] * (2 + phalanx_bonus - opposed_penalty)
                + 21 * support.popcount() as i32;
            let mg = v;
            let eg = v * (r_rel - 2) / 4;
            breakdown.connected += Score::new(mg, eg);
        } else if neighbours.is_empty() {
            breakdown.isolated -= ISOLATED;
            if opposed.is_empty() {
                breakdown.weak_unopposed -= WEAK_UNOPPOSED;
            }
        } else if is_backward {
            breakdown.backward -= BACKWARD;
            if opposed.is_empty() {
                breakdown.weak_unopposed -= WEAK_UNOPPOSED;
            }
        }

        if support.is_empty() {
            if doubled.any() {
                breakdown.doubled -= DOUBLED;
            }
            if lever.more_than_one() {
                breakdown.weak_lever -= WEAK_LEVER;
            }
        }
    }

    eval.pawn_attacks_span[us.index()] = attacks_span;
    eval.passed_pawns[us.index()] = passed;
    eval.scores[us.index()] = breakdown.total();
    eval.breakdowns[us.index()] = breakdown;
}

// =========================================================================
// King safety
// =========================================================================

/// Pawn-shelter and pawn-storm evaluation for one colour's king, plus an
/// endgame penalty proportional to king-to-nearest-own-pawn distance.
///
/// When the king still has castling rights, the returned score is the
/// maximum (by mg value) of the evaluation at its current square and at
/// each legal castling destination — the evaluator assumes the side will
/// pick the best shelter available.
pub fn king_safety(pos: &Position, us: Color) -> Score {
    let king_sq = pos.king_square(us);
    let mut best = evaluate_shelter(pos, us, king_sq);

    let our_rights = pos.castling_rights() & CastlingRights::for_color(us);
    if our_rights.intersects(CastlingRights::KING_SIDE) {
        let target = Square::G1.from_perspective(us);
        let candidate = evaluate_shelter(pos, us, target);
        if candidate.mg().0 > best.mg().0 {
            best = candidate;
        }
    }
    if our_rights.intersects(CastlingRights::QUEEN_SIDE) {
        let target = Square::C1.from_perspective(us);
        let candidate = evaluate_shelter(pos, us, target);
        if candidate.mg().0 > best.mg().0 {
            best = candidate;
        }
    }

    // Endgame: bring the king close to our nearest pawn. The reference
    // uses 8 as the "no pawns" fallback (larger than any chebyshev
    // distance); we mirror that by returning 0 in that case so the
    // endgame penalty vanishes when there's no pawn to approach.
    let our_pawns = pos.pieces_of(us, PieceType::Pawn);
    let min_dist = nearest_own_pawn_distance(king_sq, our_pawns);

    best - Score::new(0, KING_TO_NEAREST_PAWN_PENALTY_EG * min_dist)
}

/// Evaluate pawn shelter + storm as if our king stood on `king_sq`. The
/// square may differ from the actual king square when we're speculatively
/// comparing castling options.
fn evaluate_shelter(pos: &Position, us: Color, king_sq: Square) -> Score {
    let them = !us;

    // Only pawns on our side of the king contribute to shelter / storm.
    // "Our side" = squares not strictly in front of our king from our POV.
    let relevant = pos.pieces(PieceType::Pawn) & !forward_ranks_bb(them, king_sq);
    let our_pawns = relevant & pos.pieces_by_color(us);
    let their_pawns = relevant & pos.pieces_by_color(them);

    let mut bonus = SHELTER_BASE;

    // Evaluate across the king's file and its two neighbours. Clamp the
    // center file to the b..g range so the three-file sweep never falls
    // off the edge of the board.
    let center_idx = (king_sq.file().index()).clamp(File::B.index(), File::G.index());
    for offset in -1i32..=1 {
        let f_idx = (center_idx as i32 + offset) as usize;
        let file = File::from_index(f_idx as u8).expect("clamped in b..g; ±1 stays in a..h");

        let our_on_file = our_pawns & file_bb(file);
        let our_rank = if our_on_file.any() {
            our_on_file
                .frontmost(them)
                .rank()
                .from_perspective(us)
                .index() as i32
        } else {
            0
        };

        let their_on_file = their_pawns & file_bb(file);
        let their_rank = if their_on_file.any() {
            their_on_file
                .frontmost(them)
                .rank()
                .from_perspective(us)
                .index() as i32
        } else {
            0
        };

        let folded = file.fold_to_queenside().index();
        bonus += Score::new(SHELTER_STRENGTH[folded][our_rank as usize], 0);

        if our_rank > 0 && our_rank == their_rank - 1 {
            // Their pawn is immediately in front of ours — blocked storm.
            // The very-early case (their pawn on relative rank 3) draws
            // the heaviest penalty. `Rank::R3.index()` is 2 in 0-indexed
            // terms, which matches the reference's `RANK_3` enum value.
            if their_rank == Rank::R3.index() as i32 {
                bonus -= BLOCKED_STORM;
            }
        } else {
            bonus -= Score::new(UNBLOCKED_STORM[folded][their_rank as usize], 0);
        }
    }

    bonus
}

/// Chebyshev distance from `king_sq` to the nearest own pawn, for use as
/// the endgame king-activity term. Returns 0 when the side has no pawns
/// (making the term vanish), and short-circuits to 1 when any pawn is
/// inside the king's attack radius.
fn nearest_own_pawn_distance(king_sq: Square, our_pawns: Bitboard) -> i32 {
    if our_pawns.is_empty() {
        return 0;
    }
    if (our_pawns & king_attacks(king_sq)).any() {
        return 1;
    }
    let mut min_dist = 8i32;
    for pawn_sq in our_pawns {
        let d = square_distance(king_sq, pawn_sq) as i32;
        if d < min_dist {
            min_dist = d;
        }
    }
    min_dist
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Starting position ------------------------------------------

    #[test]
    fn startpos_pawn_scores_are_mirrored() {
        // Starting position is perfectly symmetric in pawn structure, so
        // white's and black's pawn scores must be equal. The signed
        // aggregate score is therefore zero.
        let p = Position::startpos();
        let e = evaluate(&p);
        assert_eq!(e.scores[0], e.scores[1]);
        assert_eq!(e.score(), Score::ZERO);
    }

    #[test]
    fn startpos_has_no_passed_pawns() {
        let p = Position::startpos();
        let e = evaluate(&p);
        assert!(e.passed_pawns[0].is_empty());
        assert!(e.passed_pawns[1].is_empty());
    }

    #[test]
    fn startpos_pawn_attacks_cover_ranks_3_and_6() {
        let p = Position::startpos();
        let e = evaluate(&p);
        // White pawns on rank 2 attack every square on rank 3.
        let rank3 = crate::bitboard::RANK_3;
        assert_eq!(e.pawn_attacks[Color::White.index()], rank3);
        // Black pawns on rank 7 attack every square on rank 6.
        let rank6 = crate::bitboard::RANK_6;
        assert_eq!(e.pawn_attacks[Color::Black.index()], rank6);
    }

    // ---- Passed pawn detection --------------------------------------

    #[test]
    fn isolated_advanced_pawn_is_passed() {
        // White pawn on d7 with no black pawn in front or on adjacent
        // files: a textbook passed pawn.
        let p = Position::from_fen("4k3/3P4/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        let e = evaluate(&p);
        assert!(e.passed_pawns[Color::White.index()].contains(Square::D7));
    }

    #[test]
    fn pawn_with_stopper_is_not_passed() {
        // White d4, black d5 directly blocks it. Not passed.
        let p = Position::from_fen("4k3/8/8/3p4/3P4/8/8/4K3 w - - 0 1").unwrap();
        let e = evaluate(&p);
        assert!(e.passed_pawns[Color::White.index()].is_empty());
    }

    #[test]
    fn pawn_with_unlevered_adjacent_stopper_ahead_is_not_passed() {
        // White e4 with a black pawn on f6 — the f6 pawn defends f5 and
        // would attack e5 if we push. It's a stopper the e4 pawn cannot
        // capture, and we don't outnumber it on the phalanx, so this is
        // not a passed pawn. (Contrast with e4 + black-d5, which Stockfish
        // *does* consider passed because e4xd5 clears the path.)
        let p = Position::from_fen("4k3/8/5p2/8/4P3/8/8/4K3 w - - 0 1").unwrap();
        let e = evaluate(&p);
        assert!(e.passed_pawns[Color::White.index()].is_empty());
    }

    // ---- Structural penalties ---------------------------------------

    #[test]
    fn doubled_pawns_cost_more_than_undoubled() {
        // Identical otherwise, but white has two pawns stacked on the
        // e-file (e2 and e3). The doubled pawn penalty applies to the
        // back pawn which has no support on adjacent files.
        let doubled = Position::from_fen("4k3/8/8/8/8/4P3/4P3/4K3 w - - 0 1").unwrap();
        let singled = Position::from_fen("4k3/8/8/8/8/8/4P3/4K3 w - - 0 1").unwrap();
        let d = evaluate(&doubled);
        let s = evaluate(&singled);
        assert!(
            d.scores[Color::White.index()].mg().0 < s.scores[Color::White.index()].mg().0,
            "doubled pawn should score worse than a single pawn"
        );
    }

    #[test]
    fn isolated_pawn_costs_more_than_connected_pair() {
        // Isolated: a single pawn on d4. Connected pair: c4 and d4. The
        // isolated case should score lower than the connected case.
        let isolated = Position::from_fen("4k3/8/8/8/3P4/8/8/4K3 w - - 0 1").unwrap();
        let connected = Position::from_fen("4k3/8/8/8/2PP4/8/8/4K3 w - - 0 1").unwrap();
        let i = evaluate(&isolated);
        let c = evaluate(&connected);
        assert!(
            i.scores[Color::White.index()].mg().0
                < c.scores[Color::White.index()].mg().0
                    / i32::max(connected.count(Color::White, PieceType::Pawn) as i32, 1),
            "isolated pawn should be worse than one pawn within a connected pair"
        );
    }

    // ---- Attack-span ------------------------------------------------

    #[test]
    fn attacks_span_extends_to_promotion_for_healthy_pawn() {
        // A lone white pawn on e4 with no obstructions — attack span
        // covers d5..d8, f5..f8 (plus the immediate d5/f5 pawn attacks).
        let p = Position::from_fen("4k3/8/8/8/4P3/8/8/4K3 w - - 0 1").unwrap();
        let e = evaluate(&p);
        let span = e.pawn_attacks_span[Color::White.index()];
        for sq in &[
            Square::D5,
            Square::D6,
            Square::D7,
            Square::D8,
            Square::F5,
            Square::F6,
            Square::F7,
            Square::F8,
        ] {
            assert!(span.contains(*sq), "span should contain {:?}", sq);
        }
    }

    // ---- King safety ------------------------------------------------

    #[test]
    fn intact_white_shelter_scores_better_than_exposed_king() {
        // Kinged on g1 with the f2/g2/h2 trio intact vs. the same king but
        // all three shelter pawns pushed one rank forward (weaker shelter).
        let intact = Position::from_fen("4k3/8/8/8/8/8/5PPP/6K1 w - - 0 1").unwrap();
        let pushed = Position::from_fen("4k3/8/8/8/8/5PPP/8/6K1 w - - 0 1").unwrap();
        let a = king_safety(&intact, Color::White);
        let b = king_safety(&pushed, Color::White);
        assert!(
            a.mg().0 > b.mg().0,
            "intact f2/g2/h2 shelter ({}) should beat f3/g3/h3 ({})",
            a.mg().0,
            b.mg().0,
        );
    }

    #[test]
    fn king_safety_is_equal_for_mirrored_positions() {
        // A position and its colour-flipped mirror produce the same score
        // for each side's own king.
        let white_fen = "4k3/8/8/8/8/8/5PPP/6K1 w - - 0 1";
        let black_fen = "6k1/5ppp/8/8/8/8/8/4K3 w - - 0 1";
        let w = Position::from_fen(white_fen).unwrap();
        let b = Position::from_fen(black_fen).unwrap();
        assert_eq!(
            king_safety(&w, Color::White).mg(),
            king_safety(&b, Color::Black).mg(),
            "mirrored king safety should agree"
        );
    }

    #[test]
    fn king_far_from_pawns_gets_endgame_penalty() {
        // King on a1, only pawn on h7 — maximum king-pawn distance. The
        // eg component should be strictly more negative than a variant
        // where the king sits next to the pawn.
        let far = Position::from_fen("4k3/7P/8/8/8/8/8/K7 w - - 0 1").unwrap();
        let near = Position::from_fen("4k3/7P/6K1/8/8/8/8/8 w - - 0 1").unwrap();
        let a = king_safety(&far, Color::White);
        let b = king_safety(&near, Color::White);
        assert!(
            a.eg().0 < b.eg().0,
            "distant king should score worse in the endgame half"
        );
    }

    // ---- Determinism ------------------------------------------------

    #[test]
    fn evaluate_is_pure() {
        // Calling evaluate twice on the same position must yield identical
        // results. Guards against accidental reliance on hidden state.
        let p = Position::from_fen(
            "r1bqkb1r/ppp2ppp/2np1n2/4p3/2B1P3/2N2N2/PPPP1PPP/R1BQK2R w KQkq - 0 5",
        )
        .unwrap();
        let a = evaluate(&p);
        let b = evaluate(&p);
        assert_eq!(a, b);
    }

    // ---- Spot check: symmetric pawn arrangement -----------------

    #[test]
    fn symmetric_pawns_produce_zero_signed_score() {
        // Same pawn structure for both colours (vertically mirrored) =>
        // signed pawn score is zero.
        let p = Position::from_fen("4k3/1p1p1p1p/8/8/8/8/1P1P1P1P/4K3 w - - 0 1").unwrap();
        let e = evaluate(&p);
        assert_eq!(e.score(), Score::ZERO);
        assert_eq!(e.scores[0], e.scores[1]);
        // And the passed-pawn sets are also mirrored.
        assert_eq!(e.passed_pawns[0].popcount(), e.passed_pawns[1].popcount());
    }

    // ---- PawnsBreakdown granular attribution ------------------------

    fn white_breakdown(fen: &str) -> PawnsBreakdown {
        let p = Position::from_fen(fen).unwrap();
        evaluate(&p).breakdowns[Color::White.index()]
    }

    #[test]
    fn breakdown_total_sums_every_sub_term() {
        // total() must equal the sum of every field. A future refactor
        // that adds a field but forgets to update total() would drift
        // silently — this test catches that.
        let b = white_breakdown("4k3/1p1p1p1p/8/8/8/8/1P1P1P1P/4K3 w - - 0 1");
        let manual =
            b.connected + b.isolated + b.backward + b.doubled + b.weak_unopposed + b.weak_lever;
        assert_eq!(b.total(), manual);
    }

    #[test]
    fn breakdown_total_equals_scores_field() {
        // scores[c] is a cached sum of the per-colour breakdown — the two
        // must be identical by construction.
        let p = Position::from_fen("4k3/1p1p1p1p/8/8/8/8/1P1P1P1P/4K3 w - - 0 1").unwrap();
        let e = evaluate(&p);
        for &c in &Color::both() {
            assert_eq!(
                e.scores[c.index()],
                e.breakdowns[c.index()].total(),
                "scores and breakdown.total() must agree for {:?}",
                c
            );
        }
    }

    #[test]
    fn isolated_pawn_lands_on_isolated_and_weak_unopposed_fields() {
        // Lone white pawn on d4 — isolated (no c/e neighbours) and
        // unopposed (no black pawn on d-file ahead). Connected must stay
        // at zero; backward / doubled must stay at zero.
        let b = white_breakdown("4k3/8/8/8/3P4/8/8/4K3 w - - 0 1");
        assert_eq!(b.isolated, Score::ZERO - ISOLATED);
        assert_eq!(b.weak_unopposed, Score::ZERO - WEAK_UNOPPOSED);
        assert_eq!(b.connected, Score::ZERO);
        assert_eq!(b.backward, Score::ZERO);
        assert_eq!(b.doubled, Score::ZERO);
        assert_eq!(b.weak_lever, Score::ZERO);
    }

    #[test]
    fn connected_pair_lands_on_connected_field() {
        // Phalanx c4-d4 — both pawns have a same-rank neighbour. The
        // connected field accumulates the rank-scaled bonus; isolated /
        // backward / doubled all stay at zero.
        let b = white_breakdown("4k3/8/8/8/2PP4/8/8/4K3 w - - 0 1");
        assert!(
            b.connected.mg().0 > 0,
            "phalanx should award a positive connected bonus, got {:?}",
            b.connected
        );
        assert_eq!(b.isolated, Score::ZERO);
        assert_eq!(b.backward, Score::ZERO);
        assert_eq!(b.doubled, Score::ZERO);
    }

    #[test]
    fn doubled_pawn_lands_on_doubled_field() {
        // e2 / e3 stacked — the front pawn is "doubled" in Stockfish
        // terms (it has a same-colour pawn directly behind it) and has
        // no support from adjacent files. Doubled field picks up one
        // -DOUBLED penalty; isolated fires on both pawns (no neighbours).
        let b = white_breakdown("4k3/8/8/8/8/4P3/4P3/4K3 w - - 0 1");
        assert_eq!(
            b.doubled,
            Score::ZERO - DOUBLED,
            "exactly one doubled penalty on the stacked pair"
        );
    }

    #[test]
    fn backward_pawn_lands_on_backward_field() {
        // White pawn on b2, black pawn directly in front on b3 (blocks
        // the push), white neighbour on a3 so b2 is not isolated. b2's
        // only neighbour sits on rank 3 — not strictly behind the push
        // square b3 — so "no advancing neighbour" holds and b2 meets the
        // backward predicate. The a3 pawn itself is not backward; it
        // contributes a connected-bonus via b2's support, but that lands
        // in a separate field we don't assert here.
        let b = white_breakdown("4k3/8/8/8/8/Pp6/1P6/4K3 w - - 0 1");
        assert_eq!(
            b.backward,
            Score::ZERO - BACKWARD,
            "b2 blocked by b3 with no advancing a-file neighbour should be backward"
        );
        assert_eq!(b.weak_unopposed, Score::ZERO, "b2 is opposed by b3");
        assert_eq!(b.doubled, Score::ZERO);
        assert_eq!(b.weak_lever, Score::ZERO);
    }

    // ---- Mirror symmetry of the breakdown ---------------------------

    #[test]
    fn mirrored_positions_produce_mirrored_breakdowns() {
        // Colour-flipped mirror positions produce equal per-colour
        // breakdowns for the relevant side.
        let white = Position::from_fen("4k3/8/8/8/3P4/8/8/4K3 w - - 0 1").unwrap();
        let black = Position::from_fen("4k3/8/8/3p4/8/8/8/4K3 w - - 0 1").unwrap();
        let w = evaluate(&white).breakdowns[Color::White.index()];
        let b = evaluate(&black).breakdowns[Color::Black.index()];
        assert_eq!(w.isolated, b.isolated);
        assert_eq!(w.weak_unopposed, b.weak_unopposed);
        assert_eq!(w.total(), b.total());
    }
}
