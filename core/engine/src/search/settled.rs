//! Settled-ply detection ([`stm_after_ply`], `compute_settled_ply`) and
//! the small TT/stack value helpers (`value_to_tt`, `value_from_tt`,
//! `cont_key_at`).

use super::*;
use crate::eval::EvalTrace;
use crate::types::Value;

/// Side-to-move at the position reached after playing `ply + 1` moves
/// from a root where `root_stm` was to move. Exposed publicly so the
/// teaching-analysis pipeline (CLI debug renderer, future `MoveAnalysis`
/// assembly) can compute the same alternation without re-deriving it.
///
/// `ply` is a 0-indexed position in a PV: ply 0 is the position reached
/// after the first PV move has been played (so stm has flipped once).
pub fn stm_after_ply(root_stm: crate::types::Color, ply: usize) -> crate::types::Color {
    if ply % 2 == 0 {
        !root_stm
    } else {
        root_stm
    }
}

/// Compute the settled-ply index for a PV's per-ply trace sequence.
///
/// Walks backward from the end of the PV looking for the latest
/// index `i` (≥ 2) where the white-POV score differs from the score
/// **two plies earlier** by at least [`SETTLED_THRESHOLD_CP`]. When
/// such an `i` exists *and* the PV has at least one more ply, we
/// return `i + 1` — the position right after the last shift has
/// fully resolved. When the PV ends mid-shift (the unstable `i` is
/// the leaf), we return `i` itself, since there's no post-resolution
/// trace to land on. When the PV is uniformly quiet, we return 0.
///
/// **Why 2 plies, not 1**: every move temporarily shifts the eval in
/// the mover's favor — their choice is committed but the opponent
/// hasn't responded. Adjacent plies have opposite side-to-move and
/// show the "sawtooth" of these unanswered commitments, routinely
/// 100–300 cp even in quiet positions. Same-side-to-move plies (2
/// apart) represent complete exchanges, so the delta between them
/// reflects what really changed — material swings, positional gains,
/// etc. — not the artificial side-to-move asymmetry.
///
/// **Why land on `i + 1`**: with the 2-ply rule the largest
/// same-side jump often lands on the peak of a mid-exchange position
/// (e.g. white plays Bxe6, ply `i`'s trace shows white temporarily
/// up a bishop, but black's recapture on ply `i + 1` is already
/// part of the PV and restores parity). Consumers that walk the PV
/// up to the settled ply want the *resolved* position, not the
/// peak.
///
/// `root_stm` is the side to move at the PV's root; the helper walks
/// the alternation to pick the right sign for each ply's white-POV
/// normalization.
pub(crate) fn compute_settled_ply(traces: &[EvalTrace], root_stm: crate::types::Color) -> Option<usize> {
    if traces.is_empty() {
        return None;
    }
    if traces.len() == 1 {
        return Some(0);
    }

    let white_pov: Vec<i32> = traces
        .iter()
        .enumerate()
        .map(|(i, t)| t.white_pov_value(stm_after_ply(root_stm, i)).0)
        .collect();

    for i in (2..white_pov.len()).rev() {
        let delta = (white_pov[i] - white_pov[i - 2]).abs();
        if delta >= SETTLED_THRESHOLD_CP {
            // Prefer the post-resolution ply when one exists.
            return if i + 1 < white_pov.len() {
                Some(i + 1)
            } else {
                Some(i)
            };
        }
    }
    Some(0)
}

// =========================================================================
// Tuning helpers
// =========================================================================

pub(crate) fn value_to_tt(v: Value, ply: i32) -> Value {
    if v.0 >= Value::MATE.0 - Value::MAX_PLY {
        Value(v.0 + ply)
    } else if v.0 <= -Value::MATE.0 + Value::MAX_PLY {
        Value(v.0 - ply)
    } else {
        v
    }
}

pub(crate) fn value_from_tt(v: Value, ply: i32) -> Value {
    if v == Value::NONE {
        return Value::NONE;
    }
    if v.0 >= Value::MATE.0 - Value::MAX_PLY {
        Value(v.0 - ply)
    } else if v.0 <= -Value::MATE.0 + Value::MAX_PLY {
        Value(v.0 + ply)
    } else {
        v
    }
}

/// Resolve `stack[ply - offset]` into the cont-hist key tuple
/// `(in_check, was_capture, moved_piece_idx, to_idx)`. The 7-frame
/// sentinel padding makes offset reads up to 6 plies safe even at
/// ply 0 — they return the all-zero sentinel which the cont-hist
/// store treats as "no parent move".
pub(crate) fn cont_key_at(stack: &[StackEntry], ply: usize, offset: usize) -> (bool, bool, u8, u8) {
    let idx = STACK_SENTINEL + ply - offset;
    let e = &stack[idx];
    (e.in_check, e.was_capture, e.moved_piece_idx, e.to_idx)
}
