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

/// Default analytical depth — kept deliberately *deeper* than the
/// typical engine-play depth so the retrospective is a stronger
/// reference than the bot the student is playing against. At depth
/// 10 we observed opening-move verdicts that flipped at depth 12
/// (e.g. 1.e4 e5 2.Nf3 reads as an inaccuracy at d=10 but emerges
/// as best at d=12). The CLI `--retrospective-depth` flag overrides
/// when the user wants a different trade-off, and the desktop hard-
/// codes this value (UI exposure deferred).
pub const RETROSPECTIVE_DEPTH: u32 = 12;

/// Safety caps so a pathological position (notably MultiPV around a
/// found mate — see search.rs) can't pin the auto-firing
/// retrospective for minutes. The wall-clock cap is the user-visible
/// guarantee; the node cap is a backstop.
const RETROSPECTIVE_NODE_CAP: u64 = 100_000_000;
const RETROSPECTIVE_TIME_MS: u64 = 10_000;

/// Configuration for a single retrospective pass.
pub struct RetrospectiveConfig {
    pub max_depth: u32,
    pub max_time_ms: Option<u64>,
    /// When true, a `Best` verdict still runs the full term-level
    /// narration instead of short-circuiting after the one-line
    /// headline. Useful when the student wants to understand *why*
    /// their move was the best, not just *that* it was.
    pub explain_best: bool,
    /// Number of search threads. The CLI's `--threads N` flag (default
    /// 1) flows in here. Single-thread is the deliberate default for
    /// the teaching tool: Lazy SMP introduces enough per-run score
    /// variance to flip the same move between verdicts (Best one run,
    /// Good the next, Best again after a takeback) — directly
    /// undermining the "play the same move, get the same verdict"
    /// contract the retrospective exists to provide. Raise this only
    /// for benchmarking.
    pub threads: usize,
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
    // Honour the user's explicit time budget if set; otherwise apply
    // the default safety cap. Either way, also apply the node-cap
    // backstop.
    let max_time = cfg
        .max_time_ms
        .map(Duration::from_millis)
        .or(Some(Duration::from_millis(RETROSPECTIVE_TIME_MS)));
    let params = SearchParams {
        max_depth: cfg.max_depth,
        max_nodes: Some(RETROSPECTIVE_NODE_CAP),
        max_time,
        multi_pv: RETROSPECTIVE_MULTI_PV,
        game_history,
        force_include: vec![user_mv],
        verbose_progress: false,
        threads: cfg.threads.max(1),
        // Retrospective must judge the user's move against true best
        // play — never apply the opponent's mid-game eval mask here.
        eval_mask: chess_tutor_engine::opponent::EvalMask::EMPTY,
    };
    let analyses = analyze_position(engine, pre_move_pos, params);
    let opts = NarrationOptions {
        explain_best: cfg.explain_best,
    };
    let text = format_retrospective(pre_move_pos, &analyses, user_mv, &opts);
    out.write_all(text.as_bytes())
}
