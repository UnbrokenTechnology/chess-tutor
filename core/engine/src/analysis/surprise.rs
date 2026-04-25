//! Shallow-vs-deep [`SurpriseKind`] detector.
//!
//! **Scope limitation**: the shallow signal is derived from
//! `ply_traces[0]`, which is only populated for moves inside the
//! MultiPV top-k (or forced into the output via
//! [`crate::engine::SearchParams::force_include`]). Truly unexpected
//! "tempting but bad" moves buried below rank N can't be flagged this
//! way — that's what the future cheap-pass evaluator (Phase 2) is for.

use super::MoveAnalysis;
use crate::types::Color;

/// Classification for a move whose *shallow* static-eval impression
/// disagrees substantially with the *deep* search score.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurpriseKind {
    /// Shallow eval said "this is fine/good for us" but the deep
    /// score comes back materially worse. Typical example: a capture
    /// that hangs a piece to a follow-up the shallow eval's single
    /// ply doesn't see.
    LooksGoodButBad,
    /// Shallow eval said "this is bad for us" but the deep score
    /// comes back substantially better. Typical example: a sacrifice
    /// whose compensation only emerges a few plies in.
    LooksBadButGood,
}

/// Minimum gap between shallow and deep (engine-cp) to call a move
/// a surprise. Below this threshold, the two evals "agree" and no
/// tag is emitted. Calibrated alongside [`super::MoveVerdict`]'s
/// thresholds — a surprise is roughly "at least a mistake's worth
/// of swing."
const SURPRISE_DELTA_CP: i32 = 150;

/// Detect a shallow-vs-deep surprise for `ma` given the root
/// side-to-move. Returns `None` when the move's shallow and deep
/// scores agree within [`SURPRISE_DELTA_CP`], or when `ply_traces`
/// is empty (pre-move baseline only).
///
/// Units: both scores are converted to white-POV tempo-free (via
/// [`crate::eval::EvalTrace::white_pov_value`]) and then re-signed
/// to the root side-to-move's POV, so the comparison is
/// apples-to-apples with `ma.score`.
pub fn detect_surprise(ma: &MoveAnalysis, root_stm: Color) -> Option<SurpriseKind> {
    let first_trace = ma.ply_traces.first()?;
    // ply_traces[0] is evaluated at the opponent's position (after
    // our move), so `white_pov_value` takes the opponent's colour.
    let shallow_white = first_trace.white_pov_value(!root_stm).0;
    let shallow = match root_stm {
        Color::White => shallow_white,
        Color::Black => -shallow_white,
    };
    let deep = ma.score.0;

    let delta = deep - shallow;
    if delta <= -SURPRISE_DELTA_CP {
        Some(SurpriseKind::LooksGoodButBad)
    } else if delta >= SURPRISE_DELTA_CP {
        Some(SurpriseKind::LooksBadButGood)
    } else {
        None
    }
}

impl MoveAnalysis {
    /// Convenience wrapper over [`detect_surprise`]. `root_stm` is
    /// the side to move at the position this analysis was produced
    /// from.
    pub fn surprise(&self, root_stm: Color) -> Option<SurpriseKind> {
        detect_surprise(self, root_stm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::{EvalTrace, TEMPO};
    use crate::types::{Move, Value};

    /// Build a MoveAnalysis with a hand-crafted shallow
    /// (ply_traces[0]) and deep (score) pair, so we can test
    /// detect_surprise in isolation without needing the search to
    /// reproduce a specific shape.
    ///
    /// `shallow_root_pov` is the white-POV shallow eval we want
    /// `ply_traces[0].white_pov_value(!root_stm)` to yield. We stash
    /// it into `final_value` with the tempo added and the sign
    /// flipped appropriately — `white_pov_value` strips tempo and
    /// re-flips.
    fn ma_with_shallow_and_deep(root_stm: Color, shallow_root_pov: i32, deep: i32) -> MoveAnalysis {
        let opp = !root_stm;
        let want_white_pov = match root_stm {
            Color::White => shallow_root_pov,
            Color::Black => -shallow_root_pov,
        };
        let stm_unsigned = match opp {
            Color::White => want_white_pov,
            Color::Black => -want_white_pov,
        };
        let mut trace = EvalTrace::zero();
        trace.tempo = TEMPO;
        trace.final_value = Value(stm_unsigned + TEMPO.0);
        MoveAnalysis {
            mv: Move::NONE,
            score: Value(deep),
            depth: 1,
            pv: vec![Move::NONE],
            ply_traces: vec![trace],
            settled_ply: None,
            pre_move_trace: EvalTrace::zero(),
            term_deltas: Vec::new(),
        }
    }

    #[test]
    fn detect_surprise_white_looks_good_but_bad() {
        let ma = ma_with_shallow_and_deep(Color::White, 50, -200);
        assert_eq!(
            detect_surprise(&ma, Color::White),
            Some(SurpriseKind::LooksGoodButBad)
        );
    }

    #[test]
    fn detect_surprise_white_looks_bad_but_good() {
        let ma = ma_with_shallow_and_deep(Color::White, -300, 100);
        assert_eq!(
            detect_surprise(&ma, Color::White),
            Some(SurpriseKind::LooksBadButGood)
        );
    }

    #[test]
    fn detect_surprise_black_looks_good_but_bad() {
        let ma = ma_with_shallow_and_deep(Color::Black, 50, -200);
        assert_eq!(
            detect_surprise(&ma, Color::Black),
            Some(SurpriseKind::LooksGoodButBad)
        );
    }

    #[test]
    fn detect_surprise_no_tag_when_shallow_and_deep_agree() {
        let ma = ma_with_shallow_and_deep(Color::White, 40, 60);
        assert_eq!(detect_surprise(&ma, Color::White), None);
    }

    #[test]
    fn detect_surprise_no_tag_at_threshold_boundary() {
        let ma_small_loss = ma_with_shallow_and_deep(Color::White, 0, -SURPRISE_DELTA_CP + 1);
        assert_eq!(detect_surprise(&ma_small_loss, Color::White), None);
    }

    #[test]
    fn detect_surprise_returns_none_on_empty_ply_traces() {
        let ma = MoveAnalysis {
            mv: Move::NONE,
            score: Value(-500),
            depth: 0,
            pv: Vec::new(),
            ply_traces: Vec::new(),
            settled_ply: None,
            pre_move_trace: EvalTrace::zero(),
            term_deltas: Vec::new(),
        };
        assert_eq!(detect_surprise(&ma, Color::White), None);
    }

    #[test]
    fn surprise_via_move_analysis_method_delegates() {
        let ma = ma_with_shallow_and_deep(Color::White, 50, -300);
        assert_eq!(
            ma.surprise(Color::White),
            Some(SurpriseKind::LooksGoodButBad)
        );
    }
}
