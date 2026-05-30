//! Score units, point-of-view, and formatting — the agent's defence
//! against the two CLI confusions documented in the teaching-positions
//! post-mortems (PLAN-cli.md):
//!
//! 1. **POV.** The engine reports side-to-move-signed scores; chess.com
//!    reports white-POV. Mixing them flips the sign on every other
//!    output. We default every CLI surface to **white-POV** and label
//!    the orientation explicitly on every value.
//! 2. **Scale.** The engine's internal centipawn is on the SF11 scale
//!    where `PAWN_EG = 213`. Dividing by 100 (the chess.com / lichess /
//!    UCI convention) makes every value look ~2× more decisive than it
//!    is. We convert to *conventional* centipawns (pawn = 100) for
//!    every human-facing pawn rendering, identical to what
//!    [`crate::analysis::win_chances`] does internally.
//!
//! Mate scores stay as `#N` / `-#N`; the conventional-cp conversion only
//! applies to plain values.

use chess_tutor_engine::analysis::win_chances;
use chess_tutor_engine::types::{Color, Value};

/// How to render a score that was computed from the side-to-move's
/// POV. `WhitePov` re-signs so positive = better for white (the CLI
/// default; matches chess.com / lichess / UCI). `Stm` leaves the
/// signing untouched (engine-internal convention).
#[derive(Clone, Copy, Debug)]
pub enum Orientation {
    WhitePov,
    Stm,
}

impl Orientation {
    /// `--stm` on the CLI → `Stm`; absent → `WhitePov` (the default).
    pub fn from_stm_flag(stm_flag: bool) -> Self {
        if stm_flag {
            Orientation::Stm
        } else {
            Orientation::WhitePov
        }
    }

    /// Re-sign a side-to-move-POV [`Value`] to match this orientation.
    pub fn apply(self, stm_value: Value, stm: Color) -> Value {
        match self {
            Orientation::WhitePov => to_white_pov(stm_value, stm),
            Orientation::Stm => stm_value,
        }
    }

    /// Short label for the score line: `"white-POV"` or `"side-to-move"`.
    pub fn label(self) -> &'static str {
        match self {
            Orientation::WhitePov => "white-POV",
            Orientation::Stm => "side-to-move",
        }
    }
}

/// Convert an internal engine [`Value`] to *conventional* centipawns
/// (the scale where a pawn ≈ 100). Mirrors `win_chances`' internal
/// conversion so headline scores, win%, and any sigmoid all speak the
/// same language.
pub fn engine_cp_to_conventional_cp(v: Value) -> f64 {
    v.0 as f64 * 100.0 / Value::PAWN_EG.0 as f64
}

/// Same conversion in pawns (the chess.com / UCI bar reading).
pub fn engine_cp_to_pawns(v: Value) -> f64 {
    v.0 as f64 / Value::PAWN_EG.0 as f64
}

/// Re-sign a side-to-move-POV engine score so it reads from white's
/// perspective: positive ⇒ better for white. Mate scores survive
/// untouched modulo the sign flip.
pub fn to_white_pov(stm_value: Value, stm: Color) -> Value {
    if stm == Color::White {
        stm_value
    } else {
        Value(-stm_value.0)
    }
}

/// "Is this a mate / mated score worth rendering as `#N`?"
pub fn is_mate_score(v: Value) -> bool {
    v.0.abs() >= Value::MATE_IN_MAX_PLY.0
}

/// `#N` / `-#N` for mate scores; `+0.85` / `-1.20` for ordinary ones,
/// always rendered as conventional pawns. The caller is responsible
/// for pre-orienting `v` (use [`to_white_pov`] for white-POV output).
pub fn format_pawns(v: Value) -> String {
    if is_mate_score(v) {
        let plies = Value::MATE.0 - v.0.abs();
        let moves = (plies + 1) / 2;
        if v.0 >= 0 {
            format!("#{}", moves)
        } else {
            format!("-#{}", moves)
        }
    } else {
        format!("{:+.2}", engine_cp_to_pawns(v))
    }
}

/// Conventional centipawns as an int (`+85` / `-120`) — the UCI /
/// chess.com / lichess scale where a pawn = 100 cp. Used by JSON and
/// by anything that wants pawns-with-finer-precision; the text
/// headline shows pawns and engine-cp directly, since the
/// conventional-cp number is just `pawns × 100`. Mate scores fall
/// back to the same `#N` notation as [`format_pawns`].
pub fn format_conventional_cp(v: Value) -> String {
    if is_mate_score(v) {
        format_pawns(v)
    } else {
        format!("{:+}", engine_cp_to_conventional_cp(v).round() as i32)
    }
}

/// Raw engine-internal centipawns (`+372` / `-150`) — what the engine
/// computes natively (PawnEG = 213 in our scale, mg pawn = 128, all
/// other piece values scaled accordingly). This is the number that
/// shows up in [`crate::engine`] / [`crate::search`] / profiler
/// output, so the CLI surfaces it labelled `engine-cp` next to pawns
/// so the displayed numbers connect to the source.
pub fn format_engine_cp(v: Value) -> String {
    if is_mate_score(v) {
        format_pawns(v)
    } else {
        format!("{:+}", v.0)
    }
}

/// Compute the headline display fields for a position score. Returns
/// `(pawns_white_pov, conv_cp_white_pov, win_pct_white)`. Pawns and
/// conv-cp express the same number two ways (matching chess.com /
/// UCI); engine-cp is intentionally absent because it requires the
/// side-to-move-POV signing and is rendered separately by the caller.
///
/// `win_pct_white` is in `0..=100`, white's chance of converting
/// (clamped to 1/99 for mate scores so the format is unambiguous).
pub fn headline_triple(white_pov: Value) -> (String, String, u8) {
    let pawns = format_pawns(white_pov);
    let cp = format_conventional_cp(white_pov);
    let signed = win_chances(white_pov);
    let pct = ((signed + 1.0) * 50.0).round().clamp(1.0, 99.0) as u8;
    (pawns, cp, pct)
}

#[cfg(test)]
#[path = "units_tests.rs"]
mod tests;
