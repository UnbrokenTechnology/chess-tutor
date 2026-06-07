//! Material-settled detection ([`stm_after_ply`],
//! `compute_material_settled`) and the small TT/stack value helpers
//! (`value_to_tt`, `value_from_tt`, `cont_key_at`).

use super::*;
use crate::position::Position;
use crate::types::{Move, MoveKind, Value};

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

/// Length of the run of consecutive non-forcing plies that closes the
/// material-resolution window in [`compute_material_settled`]. 3
/// bridges the longest quiet gap inside a single tactic we care about:
/// a fork is quiet-move → quiet-flee → capture, a 2-quiet-ply gap.
/// Deflection→fork chain links are mostly checks/captures and don't
/// open a gap at all.
pub const MATERIAL_QUIET_RUN: usize = 3;

/// Compute the **material-settled** ply for a PV: the ply index of the
/// last *forcing event* (capture, promotion, or check) before the
/// first run of [`MATERIAL_QUIET_RUN`] consecutive non-forcing plies.
/// `Some(0)` for a line whose first quiet run starts immediately —
/// "settles at once, banks nothing." `None` only for an empty PV.
///
/// This is the cap the material classifiers walk to (`noise.rs`
/// miss/blunder pools, `analysis::compute_material_outcome`): material
/// up to this ply is *forced* — what the line actually banks — while
/// captures past it are speculative deep-line trading that must not
/// count toward "this move wins/loses material."
///
/// Design notes (settled-audit, 2026-06-06 — see PLAN-perception.md):
///
/// - **Events, not eval deltas.** The previous implementation walked
///   the per-ply eval traces *backward* for the last 2-ply swing
///   ≥ 25 cp. On deep PVs the horizon tail always carries such a
///   swing, so ~90 % of lines "settled" at the leaf (at d12: 100 % of
///   lines had settled-cap material ≡ whole-PV material), and quiet
///   opening moves classified as material losses off speculative
///   12-ply gambit lines. Captures/promotions/checks are discrete
///   facts; positional wobble can't trigger them.
/// - **First resolution, not last shift.** Every consumer wants "when
///   does this move's claim resolve"; a payoff several quiet plies
///   later is a plan, not banked material. The original backward walk
///   existed to avoid stopping at a quiet move *inside* a tactic
///   (a skewer's quiet first move); the quiet-run length handles that
///   without inheriting the tail noise.
/// - **Checks count as forcing** even when they capture nothing: they
///   keep a combination's window open (check → block → fork) exactly
///   as a human reads "still forcing."
/// - **Eval reads do NOT belong here.** The eval-swing consumer
///   (`initiative_outcome`) reads the *leaf* trace — a stable-eval
///   read-point, a different question; the sacrifice-compensation
///   climax has its own forcing-tail walk in `core/teaching`. One
///   number cannot serve all three (that conflation produced the old
///   25-cp design).
pub(crate) fn compute_material_settled(root: &Position, pv: &[Move]) -> Option<usize> {
    if pv.is_empty() {
        return None;
    }
    let mut scratch = root.clone();
    let mut last_event = 0usize;
    let mut quiet_run = 0usize;
    for (ply, &mv) in pv.iter().enumerate() {
        let is_capture = match mv.kind() {
            MoveKind::Castling => false,
            MoveKind::EnPassant => true,
            _ => scratch.piece_on(mv.to()).is_some(),
        };
        let forcing =
            is_capture || mv.kind() == MoveKind::Promotion || scratch.gives_check(mv);
        if forcing {
            last_event = ply;
            quiet_run = 0;
        } else {
            quiet_run += 1;
            if quiet_run >= MATERIAL_QUIET_RUN {
                break;
            }
        }
        scratch.do_move(mv);
    }
    Some(last_event)
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
