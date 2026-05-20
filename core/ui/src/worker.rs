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
use chess_tutor_engine::movegen::legal_moves_vec;
use chess_tutor_engine::noise::{self, NoisePick};
use chess_tutor_engine::opponent::NoiseProfile;
use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::{Move, Value};
use chess_tutor_narration::{format_retrospective, NarrationOptions};

use crate::session::RepaintFn;

const RETROSPECTIVE_MULTI_PV: usize = 3;
/// Safety caps for analytical searches that auto-fire (retrospective,
/// hint panel). Without these, pathological positions — notably
/// MultiPV around a found mate — can pin the worker thread for
/// minutes, locking the GUI mid-game. The wall-clock cap is the
/// user-visible guarantee ("retrospective takes max N seconds"); the
/// node cap is a backstop in case the time check is starved by
/// scheduling.
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
    Analyze {
        pos: Box<Position>,
        depth: u32,
        multi_pv: usize,
        game_history: Vec<u64>,
        for_key: u64,
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
        text: String,
    },
    Analyze {
        for_key: u64,
        analyses: Vec<MoveAnalysis>,
    },
}

#[derive(Clone, Debug)]
pub enum NoisePickInfo {
    /// Softmax branch fired — sampled `pick_idx` from the top-K.
    Softmax {
        pick_idx: usize,
        num_lines: usize,
        delta_from_top_cp: i32,
    },
    /// Blunder branch fired — picked a deliberately worse line.
    /// `pick_idx` is always `>= 1`; either an in-band line or one
    /// from the closest-on-each-side fallback pool.
    Blunder {
        pick_idx: usize,
        num_lines: usize,
        delta_from_top_cp: i32,
    },
    /// Blunder roll fired but no plausible alternative was available
    /// within the tolerance cap — bot played best instead. Logged so
    /// the user can see when the configured rate is being
    /// under-delivered.
    BlunderSkipped {
        closest_above_loss_cp: i32,
    },
    /// Wild branch fired — bot played `mv`; the engine's preferred
    /// move was `engine_top`. The two may coincidentally match.
    Wild {
        engine_top: Move,
        engine_top_score: Value,
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
                // Wild branch needs the legal-move list — generated
                // here so the worker stays self-contained.
                let legal = legal_moves_vec(&mut pos);
                let started = Instant::now();
                let lines = engine.search(&mut pos, params);
                let elapsed = started.elapsed();
                let pick = noise::pick(&noise, seed, ply, &lines, &legal);
                let (mv, line, noise_pick) = match pick {
                    NoisePick::Line(idx) => {
                        let line = lines.get(idx).cloned();
                        let mv = line.as_ref().and_then(|l| l.pv.first().copied());
                        let info = if idx == 0 || lines.is_empty() {
                            None
                        } else {
                            Some(NoisePickInfo::Softmax {
                                pick_idx: idx,
                                num_lines: lines.len(),
                                delta_from_top_cp: lines[idx].score.0 - lines[0].score.0,
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
                    NoisePick::BlunderSkipped { closest_above_loss_cp } => {
                        // Roll fired but no plausible alternative
                        // was available. Play #1, report the skip
                        // so the user sees their configured rate is
                        // being slightly under-delivered.
                        let line = lines.first().cloned();
                        let mv = line.as_ref().and_then(|l| l.pv.first().copied());
                        let info = Some(NoisePickInfo::BlunderSkipped {
                            closest_above_loss_cp,
                        });
                        (mv, line, info)
                    }
                    NoisePick::Wild(wild_mv) => {
                        let info = lines.first().and_then(|top| {
                            top.pv.first().map(|&top_mv| NoisePickInfo::Wild {
                                engine_top: top_mv,
                                engine_top_score: top.score,
                            })
                        });
                        (Some(wild_mv), None, info)
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
                };
                let analyses = analyze_position(&mut analysis_engine, &mut pre_move_pos, params);
                let text = format_retrospective(
                    &pre_move_pos,
                    &analyses,
                    user_move,
                    &NarrationOptions::default(),
                );
                let _ = tx.send(WorkerResult::Retrospective {
                    gen,
                    target_index,
                    text,
                });
                repaint();
            }
            WorkerJob::Analyze {
                mut pos,
                depth,
                multi_pv,
                game_history,
                for_key,
            } => {
                // Same reset-before-use pattern as Retrospective —
                // hint / analyze answer should be deterministic for
                // the position the user is asking about.
                analysis_engine.new_game();
                let params = SearchParams {
                    max_depth: depth,
                    max_nodes: Some(ANALYSIS_NODE_CAP),
                    max_time: Some(Duration::from_millis(ANALYSIS_TIME_MS)),
                    multi_pv,
                    game_history,
                    force_include: Vec::new(),
                    verbose_progress: false,
                    // Hint / analyze: single-threaded for the same
                    // determinism reason as the retrospective. The user
                    // is exploring "what would the engine think about
                    // X" — same question twice should give the same
                    // answer.
                    threads: 1,
                    // Hint panel is analytical — unbiased eval.
                    eval_mask: chess_tutor_engine::opponent::EvalMask::EMPTY,
                };
                let analyses = analyze_position(&mut analysis_engine, &mut pos, params);
                let _ = tx.send(WorkerResult::Analyze { for_key, analyses });
                repaint();
            }
        }
    }
}
