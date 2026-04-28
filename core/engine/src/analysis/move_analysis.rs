//! [`MoveAnalysis`] — a search line wrapped with its
//! pre-move-vs-settled-ply trace diff. The main orchestrator
//! [`analyze_position`] produces a `Vec<MoveAnalysis>` by calling
//! the engine search and attributing every returned line.

use super::{compute_term_deltas, TermDelta};
use crate::engine::{Engine, SearchLine, SearchParams};
use crate::eval::{evaluate_with_trace, EvalTrace};
use crate::position::Position;
use crate::types::{Move, Value};

/// A single root move, its search output, and the term-delta
/// attribution between the root baseline and the settled ply along
/// the move's principal variation.
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
    /// Per-term deltas from `pre_move_trace` to the trace at
    /// `settled_ply` (falling back to the leaf if `settled_ply` is
    /// `None` and `ply_traces` is non-empty). Sorted by
    /// `|delta_tapered|` descending.
    pub term_deltas: Vec<TermDelta>,
}

impl MoveAnalysis {
    /// Return the trace used to compute `term_deltas` — the
    /// settled-ply trace when available, else the leaf, else the root
    /// baseline.
    pub fn diff_trace(&self) -> &EvalTrace {
        if let Some(idx) = self.settled_ply {
            if let Some(t) = self.ply_traces.get(idx) {
                return t;
            }
        }
        if let Some(t) = self.ply_traces.last() {
            return t;
        }
        &self.pre_move_trace
    }
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

    // Two diff baselines:
    // - ply1: the position immediately after the user's move (used by
    //   State-timing terms — threats, king safety, mobility, etc.).
    // - settled: the settled-ply trace, used by Outcome-timing terms
    //   (Material, Imbalance — the line's net trade).
    // If `ply_traces` is empty (terminal root, shouldn't reach here
    // in practice), fall back to pre_move_trace so all deltas are
    // zero.
    let ply1_trace: &EvalTrace = ply_traces.first().unwrap_or(&pre_move_trace);
    let settled_trace: &EvalTrace = if let Some(idx) = settled_ply {
        ply_traces.get(idx).unwrap_or(&pre_move_trace)
    } else {
        ply_traces.last().unwrap_or(&pre_move_trace)
    };
    let term_deltas = compute_term_deltas(&pre_move_trace, ply1_trace, settled_trace);

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
    use crate::types::Score;

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

    #[test]
    fn analyze_position_uses_settled_ply_when_available() {
        let mut pre = EvalTrace::zero();
        let mut settled = EvalTrace::zero();
        let mut leaf = EvalTrace::zero();
        // Use `imbalance` as the discriminator — it's a single Score
        // field on the trace, simpler than the post-split material
        // breakdown for this "did diff_trace pick the right slot?"
        // assertion.
        pre.imbalance = Score::new(0, 0);
        settled.imbalance = Score::new(50, 50); // diff target
        leaf.imbalance = Score::new(999, 999); // should NOT be used

        let ma = MoveAnalysis {
            mv: Move::NONE,
            score: Value::ZERO,
            depth: 1,
            pv: vec![Move::NONE],
            ply_traces: vec![settled, leaf],
            settled_ply: Some(0),
            pre_move_trace: pre,
            pre_score: Value::ZERO,
            term_deltas: Vec::new(),
        };
        assert_eq!(ma.diff_trace().imbalance, Score::new(50, 50));
    }

    #[test]
    fn move_analysis_diff_trace_falls_back_to_leaf_when_settled_missing() {
        let pre = EvalTrace::zero();
        let mut leaf = EvalTrace::zero();
        leaf.imbalance = Score::new(7, 7);

        let ma = MoveAnalysis {
            mv: Move::NONE,
            score: Value::ZERO,
            depth: 1,
            pv: vec![Move::NONE],
            ply_traces: vec![leaf],
            settled_ply: None,
            pre_move_trace: pre,
            pre_score: Value::ZERO,
            term_deltas: Vec::new(),
        };
        assert_eq!(ma.diff_trace().imbalance, Score::new(7, 7));
    }

    #[test]
    fn move_analysis_diff_trace_falls_back_to_pre_when_no_plies() {
        let mut pre = EvalTrace::zero();
        pre.imbalance = Score::new(3, 3);
        let ma = MoveAnalysis {
            mv: Move::NONE,
            score: Value::ZERO,
            depth: 1,
            pv: Vec::new(),
            ply_traces: Vec::new(),
            settled_ply: None,
            pre_move_trace: pre,
            pre_score: Value::ZERO,
            term_deltas: Vec::new(),
        };
        assert_eq!(ma.diff_trace().imbalance, Score::new(3, 3));
    }
}
