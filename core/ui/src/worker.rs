//! Background search worker.
//!
//! Receives [`WorkerJob`]s from the session, drives the engine, and
//! sends [`WorkerResult`]s back. After each send the worker calls
//! the renderer-supplied [`crate::session::RepaintFn`] to nudge the
//! UI's event loop — egui's `request_repaint` for desktop, a native
//! run-loop post for mobile, etc.

use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use chess_tutor_engine::analysis::{analyze_position, MoveAnalysis};
use chess_tutor_engine::engine::{Engine, SearchLine, SearchParams};
use chess_tutor_engine::noise::{self, NoisePick};
use chess_tutor_engine::opponent::NoiseProfile;
use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::Move;
use crate::session::RepaintFn;

/// Best + true-second-best + the force-included user line. The second
/// line powers the `only_good_move` signal on the verdict claim (and
/// thus chess.com's "Great" / "Brilliant" tiers in the translator); the
/// cards otherwise consume only `analyses[0]` + the user line, so PV3+
/// bought nothing. See PLAN-teaching-translation-layer.md §"Analysis
/// config".
const RETROSPECTIVE_MULTI_PV: usize = 2;
/// Safety caps for analytical searches that auto-fire (retrospective).
/// Without these, pathological positions — notably MultiPV around a
/// found mate — can pin the worker thread for minutes, locking the GUI
/// mid-game. The wall-clock cap is the user-visible guarantee
/// ("retrospective takes max N seconds"); the node cap is a backstop in
/// case the time check is starved by scheduling.
const ANALYSIS_NODE_CAP: u64 = 100_000_000;
const ANALYSIS_TIME_MS: u64 = 10_000;

pub(crate) enum WorkerJob {
    NewGame,
    Search {
        pos: Box<Position>,
        params: SearchParams,
        gen: u64,
        /// Bot noise profile to apply *after* the search returns. The
        /// engine search itself doesn't read this — `params.multi_pv`
        /// is set wide enough by the caller to surface candidates.
        noise: NoiseProfile,
        seed: u64,
        ply: u64,
    },
    Retrospective {
        pre_move_pos: Box<Position>,
        user_move: Move,
        depth: u32,
        game_history: Vec<u64>,
        gen: u64,
        target_index: usize,
    },
    /// Run a search and reply with the raw analyses + timing. The
    /// dispatcher ([`crate::session::Session::run_analysis`]) blocks
    /// on the response. Used by the CLI's REPL `search` / `analyze`
    /// commands. (The GUI Hint pop-over no longer runs a search at all —
    /// it surfaces a static `build_coaching_view` snapshot — so the old
    /// fire-and-forget `Analyze` job is gone.)
    AnalyzeSync {
        pos: Box<Position>,
        params: SearchParams,
    },
}

pub(crate) enum WorkerResult {
    Search {
        gen: u64,
        /// The move the bot will play, or `None` for terminal
        /// positions (no legal replies).
        mv: Option<Move>,
        /// Search-line context for the chosen move. Present for normal
        /// / softmax / blunder picks; `None` for wild picks because the
        /// engine didn't search the wild move specifically. The GUI's
        /// per-move score/depth display reads this — wild moves end up
        /// without an engine_info badge.
        line: Option<SearchLine>,
        /// Diagnostic info for the move list / debug log when noise
        /// drove the bot off `lines[0]`. `None` on the off-profile /
        /// engine-best hot path.
        noise_pick: Option<NoisePickInfo>,
        elapsed: Duration,
        /// Total nodes searched by the play engine for this turn. Used
        /// by the CLI to print "N nodes · X Mnps" alongside the move;
        /// the GUI ignores it.
        nodes: u64,
        /// Mega-nodes per second (`engine.last_nps() / 1e6`) for the
        /// CLI's perf surface.
        nps_m: f64,
    },
    Retrospective {
        gen: u64,
        target_index: usize,
        user_move: Move,
        analyses: Vec<MoveAnalysis>,
        elapsed: Duration,
        nodes: u64,
        nps_m: f64,
    },
    /// Blocking analysis for headless callers — CLI's REPL `search` /
    /// `analyze` commands. No stale-detection; the caller blocks via
    /// [`crate::session::Session::run_analysis`] until the matching
    /// result arrives.
    AnalyzeSync {
        analyses: Vec<MoveAnalysis>,
        elapsed: Duration,
        nodes: u64,
        nps_m: f64,
    },
}

#[derive(Clone, Debug)]
pub enum NoisePickInfo {
    /// Variety branch fired — sampled `pick_idx` from the ranked lines
    /// per the `avg_move_rank` dial.
    Variety {
        pick_idx: usize,
        num_lines: usize,
    },
    /// Blunder branch fired — picked a line that loses material inside
    /// the configured band. `pick_idx` is always `>= 1`.
    Blunder {
        pick_idx: usize,
        num_lines: usize,
        delta_from_top_cp: i32,
    },
    /// Miss branch fired — a material-winning move was available and
    /// the bot deliberately declined it, playing the best non-winning
    /// line. `engine_top` is the winning move that was passed up.
    Miss {
        pick_idx: usize,
        num_lines: usize,
        engine_top: Move,
    },
}

pub(crate) fn worker_loop(rx: Receiver<WorkerJob>, tx: Sender<WorkerResult>, repaint: RepaintFn) {
    // Two engines live in the worker:
    //
    // - `engine` — the play engine. Searches for the bot's move and
    //   accumulates TT / history learning across moves the way SF
    //   does. Persisting state across moves is what makes the bot
    //   stronger over the course of a game.
    //
    // - `analysis_engine` — dedicated to retrospective / hint /
    //   analyze. Its state is cleared via `new_game()` before every
    //   job so the analytical answer for a given position is
    //   bit-identical regardless of session history. **This is
    //   load-bearing for the teaching contract**: same position, same
    //   verdict — across takebacks, across days, across reinstalls.
    //   The prior pattern was `engine.clone()` for each analytical
    //   call, which captured whatever state the play engine had
    //   accumulated and silently produced different verdicts for the
    //   same move depending on what the user had done previously.
    let mut engine = Engine::default();
    let mut analysis_engine = Engine::default();
    while let Ok(job) = rx.recv() {
        match job {
            WorkerJob::NewGame => {
                engine.new_game();
                analysis_engine.new_game();
            }
            WorkerJob::Search { mut pos, params, gen, noise, seed, ply } => {
                let started = Instant::now();
                let lines = engine.search(&mut pos, params);
                let elapsed = started.elapsed();
                let pick = noise::pick(&noise, seed, ply, &pos, &lines);
                let (mv, line, noise_pick) = match pick {
                    NoisePick::Line(idx) => {
                        let line = lines.get(idx).cloned();
                        let mv = line.as_ref().and_then(|l| l.pv.first().copied());
                        let info = if idx == 0 || lines.is_empty() {
                            None
                        } else {
                            Some(NoisePickInfo::Variety {
                                pick_idx: idx,
                                num_lines: lines.len(),
                            })
                        };
                        (mv, line, info)
                    }
                    NoisePick::Blunder(idx) => {
                        let line = lines.get(idx).cloned();
                        let mv = line.as_ref().and_then(|l| l.pv.first().copied());
                        let info = lines.get(idx).map(|l| NoisePickInfo::Blunder {
                            pick_idx: idx,
                            num_lines: lines.len(),
                            delta_from_top_cp: l.score.0 - lines[0].score.0,
                        });
                        (mv, line, info)
                    }
                    NoisePick::Miss(idx) => {
                        // Declined a material-winning move (#1) and
                        // played the best non-winning line at `idx`.
                        let line = lines.get(idx).cloned();
                        let mv = line.as_ref().and_then(|l| l.pv.first().copied());
                        let info = lines
                            .first()
                            .and_then(|top| top.pv.first().copied())
                            .map(|engine_top| NoisePickInfo::Miss {
                                pick_idx: idx,
                                num_lines: lines.len(),
                                engine_top,
                            });
                        (mv, line, info)
                    }
                };
                let nodes = engine.last_nodes();
                let nps_m = engine.last_nps() / 1.0e6;
                let _ = tx.send(WorkerResult::Search {
                    gen,
                    mv,
                    line,
                    noise_pick,
                    elapsed,
                    nodes,
                    nps_m,
                });
                repaint();
            }
            WorkerJob::Retrospective {
                mut pre_move_pos,
                user_move,
                depth,
                game_history,
                gen,
                target_index,
            } => {
                // Clear the analysis engine's TT / history before every
                // retrospective so the result depends only on the
                // position + params, not on session history. (See the
                // worker_loop preamble for the full reasoning — this
                // closes the takeback verdict-flip bug.)
                analysis_engine.new_game();
                let params = SearchParams {
                    max_depth: depth,
                    max_nodes: Some(ANALYSIS_NODE_CAP),
                    max_time: Some(Duration::from_millis(ANALYSIS_TIME_MS)),
                    multi_pv: RETROSPECTIVE_MULTI_PV,
                    game_history,
                    force_include: vec![user_move],
                    verbose_progress: false,
                    // Retrospective is single-threaded for full
                    // determinism. Lazy SMP introduces enough per-run
                    // score variance to flip the same move between
                    // verdicts (e.g. e4 reading as "Best" one run and
                    // "Good" the next, then "Best" again after a
                    // takeback) — a major teaching-tool disconnect
                    // for a student trying to learn what "best" means.
                    // Single-thread gives bit-identical retrospectives
                    // across runs and across takebacks. Cost at the
                    // desktop's default depth=10 is ~60ms vs
                    // multi-thread — well within "feels instant".
                    threads: 1,
                    // Retrospective is analytical — always unbiased
                    // eval, regardless of any mid-game bot mask.
                    eval_mask: chess_tutor_engine::opponent::EvalMask::EMPTY,
                    qsearch_max_plies: None,
                };
                let started = Instant::now();
                let analyses = analyze_position(&mut analysis_engine, &mut pre_move_pos, params);
                let elapsed = started.elapsed();
                let nodes = analysis_engine.last_nodes();
                let nps_m = analysis_engine.last_nps() / 1.0e6;
                let _ = tx.send(WorkerResult::Retrospective {
                    gen,
                    target_index,
                    user_move,
                    analyses,
                    elapsed,
                    nodes,
                    nps_m,
                });
                repaint();
            }
            WorkerJob::AnalyzeSync { mut pos, params } => {
                // Reset to keep the answer deterministic, same rule as
                // the other analytical paths.
                analysis_engine.new_game();
                let started = Instant::now();
                let analyses = analyze_position(&mut analysis_engine, &mut pos, params);
                let elapsed = started.elapsed();
                let nodes = analysis_engine.last_nodes();
                let nps_m = analysis_engine.last_nps() / 1.0e6;
                let _ = tx.send(WorkerResult::AnalyzeSync {
                    analyses,
                    elapsed,
                    nodes,
                    nps_m,
                });
                repaint();
            }
        }
    }
}
