//! Centipawn → win-probability sigmoid (lila's `win_chances`).
//!
//! Hand-transliterated from lichess-puzzler's `generator/util.py` /
//! `tagger/zugzwang.py` (`reference/lichess-puzzler/`, AGPL-3.0 — never
//! shipped, never modified). The fitted multiplier comes from
//! lichess/lila PR #11148. Per the idea/expression dichotomy (see
//! `CLAUDE.md`), a fitted numerical constant is data, not copyrightable
//! expression; this is independently authored Rust, not copied source.
//!
//! ## Why a sigmoid instead of raw centipawns
//!
//! The cp → "how winning is this" relationship is non-linear: steep near
//! equality (a one-pawn swing matters enormously at 0.0) and flat at the
//! extremes (a one-pawn swing barely matters at +9). Judging "was this a
//! blunder / a sound sacrifice / a missed tactic" in raw cp over-reacts
//! when winning and under-reacts near equality. Win-probability deltas
//! mean the same thing everywhere on the board, and "winning chances
//! 70% → 45%" is far more legible to a 1200 student than "−180 cp".
//!
//! ## Scale normalization (the load-bearing gotcha)
//!
//! lila's constant assumes the *conventional* centipawn scale where a
//! pawn = 100. Our internal [`Value`] is on the classical-evaluator
//! scale where a pawn ≈ [`Value::PAWN_EG`] = 213 (and the midgame pawn
//! is 128). Feeding raw internal cp to the sigmoid would make every
//! position look ~2× more decisive than it is. So we convert to
//! conventional cp first — `internal * 100 / PAWN_EG` — the same
//! convention Stockfish's UCI output uses and that `traps::logic`
//! already speaks in.
//!
//! The multiplier was fitted by lila against NNUE evals; our eval is the
//! SF11 *classical* one. The constant is ported as-is and is good enough
//! for the threshold uses it has today (gating which teaching cards show,
//! sacrifice soundness). Refitting against our classical eval is a
//! documented follow-up, not part of this port.

use crate::types::Value;

/// lila PR #11148's fitted multiplier, applied to *conventional*
/// centipawns (pawn = 100).
const MULTIPLIER: f64 = -0.00368208;

/// Conventional-centipawn value of one pawn on our internal scale. The
/// endgame pawn value is the conversion divisor Stockfish's UCI uses
/// (`v * 100 / PawnValueEg`).
const INTERNAL_PAWN_CP: f64 = Value::PAWN_EG.0 as f64;

/// Win probability for the side the score is reported from, in
/// `[-1.0, 1.0]` (`+1` = certainly winning, `0` = dead equal, `-1` =
/// certainly losing). A mate score (either sign) saturates to `±1`.
///
/// `score` is an engine-internal [`Value`] (pawn ≈ 213), **not**
/// conventional centipawns — the conversion happens inside. Pass a score
/// already oriented to the point of view you care about (e.g.
/// [`crate::analysis::MoveAnalysis::score`] is from the root side to
/// move's POV).
pub fn win_chances(score: Value) -> f64 {
    // Mate (or ±infinity): no sigmoid, the game is decided.
    if score.0.abs() >= Value::MATE_IN_MAX_PLY.0 {
        return if score.0 > 0 { 1.0 } else { -1.0 };
    }
    let cp = score.0 as f64 * 100.0 / INTERNAL_PAWN_CP;
    2.0 / (1.0 + (MULTIPLIER * cp).exp()) - 1.0
}

#[cfg(test)]
#[path = "win_chances_tests.rs"]
mod tests;
