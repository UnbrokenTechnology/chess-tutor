//! CLI orchestration for the per-move retrospective: invoke
//! `analyze_position` on the pre-move position with the user's move
//! forced into the output, then call the shared
//! [`chess_tutor_narration`] crate to format the report and write it
//! to stdout.
//!
//! All prose-rendering logic lives in `chess-tutor-narration` so it
//! can be shared with the GUI surfaces; this module owns only the
//! search-side knobs (`multi_pv`, depth/time budgets, history
//! plumbing) and the stdout sink.

use std::io::{self, Write};
use std::time::Duration;

use chess_tutor_engine::analysis::analyze_position;
use chess_tutor_engine::engine::{Engine, SearchParams};
use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::Move;
use chess_tutor_narration::{format_retrospective, NarrationOptions};

/// How many alternatives to pull from the search when running
/// retrospective. Kept small (top 2 alternatives + the forced user
/// move) so the pause is tolerable on every human move.
const RETROSPECTIVE_MULTI_PV: usize = 3;

/// Configuration for a single retrospective pass.
pub struct RetrospectiveConfig {
    pub max_depth: u32,
    pub max_time_ms: Option<u64>,
    /// When true, a `Best` verdict still runs the full term-level
    /// narration instead of short-circuiting after the one-line
    /// headline. Useful when the student wants to understand *why*
    /// their move was the best, not just *that* it was.
    pub explain_best: bool,
}

/// Analyze `pre_move_pos` with the user's move forced into the
/// output, classify the move, and write a short teaching paragraph
/// to `out`.
///
/// `game_history` is the zobrist-key history up to and including
/// the pre-move position (the search treats the last element as
/// the root and the preceding ones as prior positions for
/// repetition detection).
pub fn run_and_render(
    out: &mut io::StdoutLock<'_>,
    pre_move_pos: &mut Position,
    engine: &mut Engine,
    cfg: &RetrospectiveConfig,
    game_history: Vec<u64>,
    user_mv: Move,
) -> io::Result<()> {
    let params = SearchParams {
        max_depth: cfg.max_depth,
        max_nodes: None,
        max_time: cfg.max_time_ms.map(Duration::from_millis),
        multi_pv: RETROSPECTIVE_MULTI_PV,
        game_history,
        force_include: vec![user_mv],
        verbose_progress: false,
        // Retrospectives are teaching output — keep them
        // deterministic so the same position always produces the
        // same narration.
        threads: 1,
    };
    let analyses = analyze_position(engine, pre_move_pos, params);
    let opts = NarrationOptions {
        explain_best: cfg.explain_best,
    };
    let text = format_retrospective(pre_move_pos, &analyses, user_mv, &opts);
    out.write_all(text.as_bytes())
}
