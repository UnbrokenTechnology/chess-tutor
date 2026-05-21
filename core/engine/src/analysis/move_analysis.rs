//! [`MoveAnalysis`] — a search line wrapped with its
//! pre-move-vs-ply-1 trace diff. The main orchestrator
//! [`analyze_position`] produces a `Vec<MoveAnalysis>` by calling
//! the engine search and attributing every returned line.

use super::{compute_term_deltas, TermDelta};
use crate::engine::{Engine, SearchLine, SearchParams};
use crate::eval::{evaluate_with_trace, EvalTrace};
use crate::position::Position;
use crate::types::{Move, Value};

/// A single root move, its search output, and the term-delta
/// attribution between the root baseline and the position
/// immediately after the user's move.
#[derive(Clone, Debug)]
pub struct MoveAnalysis {
    pub mv: Move,
    pub score: Value,
    /// Iterative-deepening depth (plies) the producing search reached
    /// for this line. Same semantics as
    /// [`crate::engine::SearchLine::depth`].
    pub depth: u32,
    pub pv: Vec<Move>,
    pub ply_traces: Vec<EvalTrace>,
    pub settled_ply: Option<usize>,
    pub pre_move_trace: EvalTrace,
    /// Static eval of the root position (before the user's move),
    /// from root side-to-move's POV — same scale as `score`. Shared
    /// across all `MoveAnalysis` instances from the same
    /// [`analyze_position`] call. Used by [`classify_move`] to
    /// distinguish "missed a stronger move" from "actually worsened
    /// the position."
    pub pre_score: Value,
    /// Per-term deltas from `pre_move_trace` to the ply-1 trace (the
    /// position immediately after the user's move). Sorted by
    /// `|delta_tapered|` descending.
    pub term_deltas: Vec<TermDelta>,
}

/// Run a search on `pos` and wrap every returned line as a
/// [`MoveAnalysis`]. The root's [`evaluate_with_trace`] is computed
/// once up front and shared across every returned analysis as
/// `pre_move_trace`. Returns empty when the position is terminal.
///
/// The caller controls the number of ranked moves via
/// `params.multi_pv`; for a full "analyze every legal move" report,
/// pass `multi_pv = legal_move_count` (see the Phase-2 note in
/// HANDOFF.md's design brief about this doubling as a cheap-pass
/// substitute).
pub fn analyze_position(
    engine: &mut Engine,
    pos: &mut Position,
    params: SearchParams,
) -> Vec<MoveAnalysis> {
    let (pre_score, pre_move_trace) = evaluate_with_trace(pos);
    let lines = engine.search(pos, params);
    lines
        .into_iter()
        .map(|line| analysis_from_line(line, pre_move_trace, pre_score))
        .collect()
}

fn analysis_from_line(
    line: SearchLine,
    pre_move_trace: EvalTrace,
    pre_score: Value,
) -> MoveAnalysis {
    let SearchLine {
        pv,
        score,
        depth,
        ply_traces,
        settled_ply,
    } = line;

    let mv = pv.first().copied().unwrap_or(Move::NONE);

    // Every term diffs against the same ply-1 (post-user-move) trace
    // — the honest "what changed on the board" snapshot. Tactical
    // futures live in `score`, `pv`, and `MaterialOutcome`, not in
    // leaf-trace artifacts. If `ply_traces` is empty (terminal root,
    // shouldn't reach here in practice), fall back to pre_move_trace
    // so all deltas are zero.
    let ply1_trace: &EvalTrace = ply_traces.first().unwrap_or(&pre_move_trace);
    let term_deltas = compute_term_deltas(&pre_move_trace, ply1_trace);

    MoveAnalysis {
        mv,
        score,
        depth,
        pv,
        ply_traces,
        settled_ply,
        pre_move_trace,
        pre_score,
        term_deltas,
    }
}

#[cfg(test)]
mod tests {
    use super::super::TermId;
    use super::*;

    #[test]
    fn analyze_position_returns_nonempty_for_startpos() {
        let mut pos = Position::startpos();
        let mut engine = Engine::default();
        let analyses = analyze_position(
            &mut engine,
            &mut pos,
            SearchParams {
                max_depth: 4,
                multi_pv: 2,
                ..SearchParams::default()
            },
        );
        assert!(!analyses.is_empty());
        assert!(analyses.len() <= 2);
        for a in &analyses {
            assert_eq!(a.term_deltas.len(), TermId::ALL.len());
            assert!(a.pv.first().copied() == Some(a.mv));
            let any_nonzero = a.term_deltas.iter().any(|d| d.delta_tapered != 0);
            assert!(
                any_nonzero,
                "every term was zero — unlikely for a searched root move"
            );
        }
    }

}
