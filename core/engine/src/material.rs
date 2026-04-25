//! Material-level evaluation: game phase, imbalance, and scale factor.
//!
//! A position's material configuration contributes three things to the
//! eval that the main function in `evaluate.rs` needs before it scores
//! anything piece-specific:
//!
//! 1. **Game phase** — a number in `0..=128` interpolating between
//!    `PHASE_ENDGAME` and `PHASE_MIDGAME`, based on total non-pawn material
//!    on the board. Used to weight the two halves of every `Score` into a
//!    single `Value`.
//!
//! 2. **Imbalance** — a second-degree polynomial over the piece counts of
//!    both sides. Captures effects like "bishop pair is worth more than
//!    two separate bishops" and "a rook's value sags if you have too many
//!    of them because they get in each other's way." The coefficients are
//!    the factual data from Stockfish 11's `material.cpp`, used under the
//!    idea/expression split.
//!
//! 3. **Scale factor per color** — reduces the endgame half of the score
//!    in drawish configurations (e.g., one side has no pawns and only a
//!    small material edge). This catches trivial draws like KBK and
//!    scales down "close" endgames that'll likely end in a draw anyway.
//!
//! Endgame specializations plug into `MaterialEval.endgame_value`. The
//! KXK driver (mate against a lone king) is live via [`crate::endgame`];
//! other patterns (KBNK, KPK bitbase, drawish rook endings) are still
//! deferred.

use crate::endgame;
use crate::position::Position;
use crate::types::{Color, Phase, PieceType, ScaleFactor, Score, Value};

// =========================================================================
// Imbalance polynomial coefficients
// =========================================================================
//
// Indexed by `[pt1][pt2]` with 0 ≤ pt2 ≤ pt1. Only the lower triangle is
// populated; the upper triangle stays zero.
//
// The index mapping: 0 = "bishop-pair virtual piece" (0 or 1, reflecting
// whether we have at least two bishops), 1 = pawn, 2 = knight, 3 = bishop,
// 4 = rook, 5 = queen. Kings don't participate.
//
// These numerical weights are the classical-eval parameters from
// Stockfish 11. They're factual data — independently authored code here,
// numbers carried over.

const QUADRATIC_OURS: [[i32; 6]; 6] = [
    //  pair  pawn  knight  bishop   rook  queen
    [1438, 0, 0, 0, 0, 0],          // bishop pair
    [40, 38, 0, 0, 0, 0],           // pawn
    [32, 255, -62, 0, 0, 0],        // knight
    [0, 104, 4, 0, 0, 0],           // bishop
    [-26, -2, 47, 105, -208, 0],    // rook
    [-189, 24, 117, 133, -134, -6], // queen
];

const QUADRATIC_THEIRS: [[i32; 6]; 6] = [
    //  pair  pawn  knight  bishop   rook  queen
    [0, 0, 0, 0, 0, 0],          // bishop pair
    [36, 0, 0, 0, 0, 0],         // pawn
    [9, 63, 0, 0, 0, 0],         // knight
    [59, 65, 42, 0, 0, 0],       // bishop
    [46, 39, 24, -24, 0, 0],     // rook
    [97, 100, -42, 137, 268, 0], // queen
];

// =========================================================================
// Output
// =========================================================================

/// Material-level evaluation summary. Produced once per position and then
/// consumed by the main evaluator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MaterialEval {
    /// Polynomial imbalance contribution, as a `Score`. Same value in both
    /// phases: the reference uses `make_score(v, v)`. White is positive,
    /// black negative.
    pub imbalance: Score,

    /// Interpolation factor between endgame (0) and middlegame (128).
    pub game_phase: Phase,

    /// Per-color scale factor applied to the endgame half of the score.
    /// Normal is 64; values less than 64 signal drawish material.
    pub scale_factor: [ScaleFactor; 2],

    /// Specialized endgame evaluator, if any. Always `None` until we port
    /// `endgame.cpp`. When `Some`, the caller should trust this number as
    /// the full evaluation and skip the standard terms.
    pub endgame_value: Option<Value>,
}

// =========================================================================
// Public entry point
// =========================================================================

/// Compute the material-level evaluation for `pos`.
///
/// No caching yet — this runs every call. Stockfish caches by material key
/// (a Zobrist-like hash over piece counts) in an 8192-entry table; we'll
/// add it when profiling warrants.
pub fn evaluate(pos: &Position) -> MaterialEval {
    let mut scale_factor = [ScaleFactor::NORMAL; 2];

    // --- Game phase ---------------------------------------------------
    let game_phase = compute_game_phase(pos);

    // --- Drawish-material scale factor --------------------------------
    //
    // If a side has no pawns and only a bishop's worth of material lead
    // (or less), pretend its endgame is worse than its raw score would
    // suggest. Thresholds below are the reference's.
    apply_drawish_factor(pos, &mut scale_factor);

    // --- Imbalance ----------------------------------------------------
    let piece_count = build_piece_count_table(pos);
    let imbalance_value =
        (imbalance(&piece_count, Color::White) - imbalance(&piece_count, Color::Black)) / 16;
    let imbalance = Score::new(imbalance_value, imbalance_value);

    MaterialEval {
        imbalance,
        game_phase,
        scale_factor,
        endgame_value: endgame::probe(pos),
    }
}

// =========================================================================
// Game phase
// =========================================================================

fn compute_game_phase(pos: &Position) -> Phase {
    let npm = pos.non_pawn_material(Color::White).0 + pos.non_pawn_material(Color::Black).0;
    let clamped = npm.clamp(Value::ENDGAME_LIMIT.0, Value::MIDGAME_LIMIT.0);
    let span = Value::MIDGAME_LIMIT.0 - Value::ENDGAME_LIMIT.0;
    Phase((clamped - Value::ENDGAME_LIMIT.0) * Phase::MIDGAME.0 / span)
}

// =========================================================================
// Drawish-material scaling
// =========================================================================

fn apply_drawish_factor(pos: &Position, scale: &mut [ScaleFactor; 2]) {
    let npm_w = pos.non_pawn_material(Color::White);
    let npm_b = pos.non_pawn_material(Color::Black);

    // White has no pawns and at most a bishop's worth of material lead.
    if pos.count(Color::White, PieceType::Pawn) == 0 && npm_w.0 - npm_b.0 <= Value::BISHOP_MG.0 {
        scale[Color::White.index()] = if npm_w.0 < Value::ROOK_MG.0 {
            ScaleFactor::DRAW
        } else if npm_b.0 <= Value::BISHOP_MG.0 {
            ScaleFactor(4)
        } else {
            ScaleFactor(14)
        };
    }

    // Symmetric for black.
    if pos.count(Color::Black, PieceType::Pawn) == 0 && npm_b.0 - npm_w.0 <= Value::BISHOP_MG.0 {
        scale[Color::Black.index()] = if npm_b.0 < Value::ROOK_MG.0 {
            ScaleFactor::DRAW
        } else if npm_w.0 <= Value::BISHOP_MG.0 {
            ScaleFactor(4)
        } else {
            ScaleFactor(14)
        };
    }
}

// =========================================================================
// Imbalance polynomial
// =========================================================================

/// Per-color piece counts in the layout the polynomial expects: index 0 is
/// the "virtual bishop pair" flag (1 if this color has ≥ 2 bishops).
fn build_piece_count_table(pos: &Position) -> [[i32; 6]; 2] {
    let mut counts = [[0i32; 6]; 2];
    for color in Color::both() {
        let c = color.index();
        let bishops = pos.count(color, PieceType::Bishop) as i32;
        counts[c][0] = if bishops > 1 { 1 } else { 0 };
        counts[c][1] = pos.count(color, PieceType::Pawn) as i32;
        counts[c][2] = pos.count(color, PieceType::Knight) as i32;
        counts[c][3] = bishops;
        counts[c][4] = pos.count(color, PieceType::Rook) as i32;
        counts[c][5] = pos.count(color, PieceType::Queen) as i32;
    }
    counts
}

/// Polynomial imbalance evaluation for one side. The total imbalance in
/// the position is `imbalance(white) − imbalance(black)`, then divided by
/// 16 (Stockfish's normalising constant).
fn imbalance(counts: &[[i32; 6]; 2], us: Color) -> i32 {
    let us_idx = us.index();
    let them_idx = (!us).index();
    let mut bonus = 0i32;

    // Sum over "our" piece types we hold any of, weighting by a quadratic
    // in (our counts, their counts) below pt1.
    for pt1 in 0..6 {
        let n_us = counts[us_idx][pt1];
        if n_us == 0 {
            continue;
        }
        let mut v = 0i32;
        for pt2 in 0..=pt1 {
            v += QUADRATIC_OURS[pt1][pt2] * counts[us_idx][pt2]
                + QUADRATIC_THEIRS[pt1][pt2] * counts[them_idx][pt2];
        }
        bonus += n_us * v;
    }

    bonus
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Game phase --------------------------------------------------

    #[test]
    fn startpos_is_full_middlegame() {
        let p = Position::startpos();
        let e = evaluate(&p);
        assert_eq!(e.game_phase, Phase::MIDGAME);
    }

    #[test]
    fn king_only_is_full_endgame() {
        let p = Position::from_fen("4k3/8/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        let e = evaluate(&p);
        assert_eq!(e.game_phase, Phase::ENDGAME);
    }

    #[test]
    fn queen_endgame_interpolates() {
        // White has a queen (2538 mg) and black only the king. Total npm
        // = 2538, below EndgameLimit = 3915, so phase clamps to ENDGAME.
        let p = Position::from_fen("4k3/8/8/8/8/8/8/3QK3 w - - 0 1").unwrap();
        let e = evaluate(&p);
        assert_eq!(e.game_phase, Phase::ENDGAME);
    }

    #[test]
    fn phase_is_between_endgame_and_midgame_in_the_middle() {
        // Both sides keep a rook and a knight — npm_w + npm_b
        // = 2*(1276 + 781) = 4114. That's just above EndgameLimit, so
        // phase should be a small positive number, not zero.
        let p = Position::from_fen("r3kn2/8/8/8/8/8/8/R3KN2 w - - 0 1").unwrap();
        let e = evaluate(&p);
        assert!(e.game_phase > Phase::ENDGAME);
        assert!(e.game_phase < Phase::MIDGAME);
    }

    // ---- Imbalance ---------------------------------------------------

    #[test]
    fn startpos_imbalance_is_zero() {
        // Material is symmetric, so white's imbalance polynomial equals
        // black's; the difference is zero.
        let p = Position::startpos();
        let e = evaluate(&p);
        assert_eq!(e.imbalance, Score::ZERO);
    }

    #[test]
    fn bishop_pair_advantage_shows_up_in_imbalance() {
        // White has a bishop pair (two bishops), black has none. White
        // should score positively even with identical pawn structure.
        let p = Position::from_fen("4k3/pppppppp/8/8/8/8/PPPPPPPP/B1B1K3 w - - 0 1").unwrap();
        let e = evaluate(&p);
        assert!(
            e.imbalance.mg().0 > 0,
            "white should have a positive imbalance with a bishop pair edge"
        );
    }

    #[test]
    fn imbalance_mg_and_eg_are_equal() {
        // The reference stores the same value in both phases:
        // `make_score(v, v)`. Check for a random-ish position.
        let p =
            Position::from_fen("r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3")
                .unwrap();
        let e = evaluate(&p);
        assert_eq!(e.imbalance.mg(), e.imbalance.eg());
    }

    #[test]
    fn imbalance_sign_flips_if_sides_swap() {
        // Take a position, mirror materials (white ↔ black), the imbalance
        // should negate. The cleanest mirror: both side-to-move-variant
        // FENs of a symmetric position is the wrong test; instead compare
        // two positions that are mirror images.
        let a = Position::from_fen("4k3/pppppppp/8/8/8/8/PPPPPPPP/B1B1K3 w - - 0 1").unwrap();
        let b = Position::from_fen("b1b1k3/pppppppp/8/8/8/8/PPPPPPPP/4K3 w - - 0 1").unwrap();
        let ea = evaluate(&a);
        let eb = evaluate(&b);
        assert_eq!(ea.imbalance.mg().0, -eb.imbalance.mg().0);
    }

    // ---- Scale factor ------------------------------------------------

    #[test]
    fn kbk_is_drawn_for_the_weaker_side() {
        // White has only a bishop. Black has only a king. White can't
        // mate with a lone bishop. White's scale factor should be DRAW.
        let p = Position::from_fen("4k3/8/8/8/8/8/8/B3K3 w - - 0 1").unwrap();
        let e = evaluate(&p);
        assert_eq!(e.scale_factor[Color::White.index()], ScaleFactor::DRAW);
    }

    #[test]
    fn krk_is_not_drawn_for_the_stronger_side() {
        // White rook vs lone king: this is a mate. White's scale factor
        // should stay at NORMAL (a rook is a rook's worth of material).
        let p = Position::from_fen("4k3/8/8/8/8/8/8/R3K3 w - - 0 1").unwrap();
        let e = evaluate(&p);
        // White has no pawns AND at most a bishop lead? No — white has
        // more than a bishop's material (rook > bishop), so the drawish
        // branch doesn't fire.
        assert_eq!(e.scale_factor[Color::White.index()], ScaleFactor::NORMAL);
    }

    #[test]
    fn startpos_scale_factor_is_normal_both_sides() {
        let p = Position::startpos();
        let e = evaluate(&p);
        assert_eq!(e.scale_factor, [ScaleFactor::NORMAL; 2]);
    }

    // ---- Endgame hook -----------------------------------------------

    #[test]
    fn endgame_value_is_none_in_starting_position() {
        let p = Position::startpos();
        let e = evaluate(&p);
        assert!(e.endgame_value.is_none());
    }

    #[test]
    fn endgame_value_is_some_in_kxk_pattern() {
        // Strong: K + Q on white; weak: lone black king.
        let p = Position::from_fen("7k/8/5KQ1/8/8/8/8/8 w - - 0 1").unwrap();
        let e = evaluate(&p);
        assert!(
            e.endgame_value.is_some(),
            "KXK pattern must populate endgame_value"
        );
    }

    // ---- Incremental non_pawn_material accessor --------------------

    #[test]
    fn startpos_non_pawn_material_matches_expected() {
        // Each side: 2 knights + 2 bishops + 2 rooks + 1 queen
        //          = 2*781 + 2*825 + 2*1276 + 2538 = 8302.
        let p = Position::startpos();
        assert_eq!(p.non_pawn_material(Color::White).0, 8302);
        assert_eq!(p.non_pawn_material(Color::Black).0, 8302);
    }

    #[test]
    fn non_pawn_material_ignores_pawns_and_kings() {
        // Position with only kings: both colors should report 0.
        let p = Position::from_fen("4k3/8/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        assert_eq!(p.non_pawn_material(Color::White), Value::ZERO);
        assert_eq!(p.non_pawn_material(Color::Black), Value::ZERO);
    }
}
