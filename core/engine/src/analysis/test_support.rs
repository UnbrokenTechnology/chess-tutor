//! Test helpers shared across the `analysis::*` test modules. Private
//! to the crate; compiled only under `#[cfg(test)]`.

#![cfg(test)]

use super::MoveAnalysis;
use crate::eval::EvalTrace;
use crate::types::{Move, Value};

/// Build a bare [`MoveAnalysis`] with only `pv` and `settled_ply`
/// filled in. Every outcome that walks the user's PV reads only those
/// two fields; the rest can be zeroed.
pub(super) fn ma_with_pv(pv: Vec<Move>, settled_ply: Option<usize>) -> MoveAnalysis {
    MoveAnalysis {
        mv: pv.first().copied().unwrap_or(Move::NONE),
        score: Value::ZERO,
        depth: 1,
        pv,
        ply_traces: Vec::new(),
        settled_ply,
        pre_move_trace: EvalTrace::zero(),
        term_deltas: Vec::new(),
    }
}
