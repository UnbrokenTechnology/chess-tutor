//! Engine-worker dispatch and result handling, plus the isolated
//! analytical search path ([`Session::run_analysis`]).

use super::*;

use chess_tutor_engine::engine::SearchParams;
use chess_tutor_engine::movegen::legal_moves_vec;
use chess_tutor_engine::position::Position;
use chess_tutor_engine::san;
use chess_tutor_engine::types::{Color, Move};

use crate::learning_mode::{gating_config_for, intervention_required, PendingIntervention};
use crate::worker::{NoisePickInfo, WorkerJob, WorkerResult};

/// Emit a one-line stderr entry describing a noise-driven pick.
/// Extracted from [`Session::handle_worker_result`] so the same
/// log line still fires when `log_to_stderr` is on without
/// inlining the match over every variant in the hot path.
pub(crate) fn log_noise_pick_to_stderr(info: &NoisePickInfo, pos: &Position, _mv: Move) {
    match info {
        NoisePickInfo::Variety {
            pick_idx,
            num_lines,
        } => {
            eprintln!("noise: variety played #{} of {}", pick_idx + 1, num_lines);
        }
        NoisePickInfo::Blunder {
            pick_idx,
            num_lines,
            delta_from_top_cp,
        } => {
            eprintln!(
                "noise: blunder picked #{} of {} ({:+} cp from #1)",
                pick_idx + 1,
                num_lines,
                delta_from_top_cp,
            );
        }
        NoisePickInfo::Miss {
            pick_idx,
            num_lines,
            engine_top,
        } => {
            eprintln!(
                "noise: miss — bot declined material-winning {} and played #{} of {}.",
                san::format(pos, *engine_top),
                pick_idx + 1,
                num_lines,
            );
        }
    }
}

impl Session {
    /// Queue an analytical retrospective for an already-applied engine
    /// move at `target_index` (book / wild / search — all of them).
    ///
    /// This mirrors [`Session::apply_user_move`]'s retrospective job: it
    /// roots the search at the *pre-move* position and force-includes the
    /// move the engine actually chose, at `ANALYTICAL_DEPTH` with
    /// `eval_mask: EMPTY` and multi-PV — i.e. the unbiased analytical
    /// config, independent of the bot's play depth / mask / noise. The
    /// result is graded **as if a human played it** (no "book" / "wild"
    /// labels ever surface — that's a load-bearing non-goal), and it fills
    /// the eval/verdict gap for opening-book and wild moves that carry no
    /// `engine_info`. Only fires when `auto_retrospective` is on, matching
    /// the user-move path.
    pub(crate) fn queue_engine_move_retrospective(&mut self, target_index: usize) {
        if !self.auto_retrospective {
            return;
        }
        let Some(entry) = self.history.get(target_index) else {
            return;
        };
        let engine_move = entry.mv;
        let pre_move_pos = self.pre_move_position(target_index);
        // History keys *before* the pre-move position. `position_keys`
        // currently ends with the post-move key (the move is already
        // applied), so drop it and let `game_history_for_search` drop the
        // pre-move position's own key.
        let pre_move_history = if self.position_keys.is_empty() {
            Vec::new()
        } else {
            game_history_for_search(&self.position_keys[..self.position_keys.len() - 1])
        };
        let _ = self.worker_tx.send(WorkerJob::Retrospective {
            pre_move_pos: Box::new(pre_move_pos),
            user_move: engine_move,
            depth: self.retrospective_depth,
            game_history: pre_move_history,
            gen: self.gen,
            target_index,
        });
    }

    pub(crate) fn maybe_queue_engine_search(&mut self) {
        // Loop because in self-play (EngineMode::Both) consecutive book
        // moves can fire synchronously — after each one it's *still*
        // the engine's turn, so we keep playing book moves until we hit
        // an out-of-book position and queue an actual search (or the
        // game ends). For user-vs-engine flows the loop iterates at
        // most once: after one engine ply it's the user's turn and the
        // top-of-loop guard returns.
        loop {
            if self.engine_thinking {
                return;
            }
            // Hold the engine reply while we're either (a) showing an
            // intervention prompt to the user or (b) waiting for the
            // classifier to decide whether one's needed. The
            // intervention-response events and the Retrospective
            // worker arrival path are responsible for re-calling this
            // method once the wait clears.
            if self.pending_intervention.is_some() || self.awaiting_intervention_decision {
                return;
            }
            if !self
                .engine_plays
                .is_engine_turn(self.position.side_to_move())
            {
                return;
            }
            let mut scratch = self.position.clone();
            if legal_moves_vec(&mut scratch).is_empty() {
                return;
            }
            // Book first: walk allowed openings for any whose stored
            // move-prefix still matches the moves played so far; if any
            // match, play the deterministically-picked next move
            // synchronously and skip the worker round-trip entirely.
            let history_moves: Vec<Move> = self.history.iter().map(|e| e.mv).collect();
            let book_pick = self
                .book_cursor
                .as_ref()
                .and_then(|c| c.peek(&history_moves));
            if let Some(book_pick) = book_pick {
                if self.log_to_stderr {
                    let san_str = san::format(&self.position, book_pick.mv);
                    if let Some(entry) = chess_tutor_engine::openings::entry(book_pick.opening_id) {
                        eprintln!(
                            "book: engine plays {} ({} {})",
                            san_str, entry.eco, entry.name
                        );
                    } else {
                        eprintln!("book: engine plays {}", san_str);
                    }
                }
                // A successful book pick clears the "we've announced
                // out-of-book" flag — the user may have taken back to
                // an in-book position, and if they later deviate again
                // we want the announcement to print fresh.
                self.book_out_announced = false;
                self.apply_move(book_pick.mv);
                // Book moves carry no `engine_info` and were never graded;
                // analyse them retrospectively just like any other move so
                // the eval bar + verdict don't gap out on book openings.
                self.queue_engine_move_retrospective(self.history.len() - 1);
                continue;
            }
            // No book match on this position. Announce once per
            // out-of-book streak — *don't* drop the cursor itself,
            // because a takeback might bring us back into book
            // territory and we need peek to keep working on the next
            // bot turn.
            if self.book_cursor.is_some() && !self.book_out_announced {
                if self.log_to_stderr {
                    eprintln!("out of book — engine now plays from search.");
                }
                self.book_out_announced = true;
            }
            return self.dispatch_engine_search();
        }
    }

    /// Dispatch a [`WorkerJob::Search`] for the current position. The
    /// caller is responsible for checking `engine_thinking` /
    /// `is_engine_turn` first — extracted from
    /// [`Self::maybe_queue_engine_search`] only so the book-pick loop
    /// can fall through to "queue a real search and exit".
    pub(crate) fn dispatch_engine_search(&mut self) {
        let params = SearchParams {
            max_depth: self.depth,
            max_nodes: Some(ENGINE_TURN_NODE_CAP),
            max_time: None,
            // Bot noise widens this beyond 1 when the opponent profile
            // wants alternatives to sample from; off-profile keeps the
            // engine's single-PV fast path.
            multi_pv: self.opponent.noise.effective_multi_pv(),
            game_history: game_history_for_search(&self.position_keys),
            force_include: Vec::new(),
            verbose_progress: false,
            // Engine moves: single-threaded. We're targeting iOS where
            // single-core utilisation is much friendlier to the
            // thermal/battery envelope, and at depth 10 startpos the
            // single-thread search finishes in ~40 ms — perceptually
            // instant. Multi-thread is kept available through the CLI
            // `--threads N` flag for bench / dev work.
            threads: 1,
            // Play engine move — apply the opponent's mid-game eval
            // mask so the bot plays as if blind to the masked
            // categories, and its tactical-vision (qsearch) horizon.
            eval_mask: self.opponent.eval_mask,
            qsearch_max_plies: self.opponent.qsearch_max_plies,
            // ...and its endgame-book skill tier (botches endgames it
            // doesn't yet "know", and queens instead of underpromoting).
            endgame_skill: self.opponent.endgame_skill,
            // ...and its move-visibility filter (geometric blind spots:
            // backward moves, knight punishes, screened rays). The
            // attention locus is the user's move that triggered this
            // engine turn.
            perception: self
                .opponent
                .perception_params(self.history.last().map(|e| e.mv.to())),
        };
        self.engine_thinking = true;
        let _ = self.worker_tx.send(WorkerJob::Search {
            pos: Box::new(self.position.clone()),
            params,
            gen: self.gen,
            noise: self.opponent.noise.clone(),
            seed: self.opponent.seed,
            ply: self.position_keys.len() as u64,
        });
    }

    pub fn poll_worker(&mut self) {
        while let Ok(result) = self.worker_rx.try_recv() {
            self.handle_worker_result(result);
        }
    }

    pub(crate) fn handle_worker_result(&mut self, result: WorkerResult) {
        match result {
            WorkerResult::Search {
                gen,
                mv,
                line,
                noise_pick,
                elapsed,
                nodes,
                nps_m,
            } => {
                if gen != self.gen {
                    return;
                }
                self.engine_thinking = false;
                let Some(mv) = mv else {
                    return;
                };
                if self.log_to_stderr {
                    if let Some(info) = &noise_pick {
                        log_noise_pick_to_stderr(info, &self.position, mv);
                    }
                }
                let root_stm = self.position.side_to_move();
                self.apply_move(mv);
                let engine_move_index = self.history.len() - 1;
                // Wild picks have no SearchLine (no search for that
                // exact move); the per-move score badge stays empty.
                if let Some(line) = line {
                    let white_pov = if root_stm == Color::White {
                        line.score
                    } else {
                        -line.score
                    };
                    if let Some(entry) = self.history.last_mut() {
                        entry.engine_info = Some(EngineInfo {
                            score_white_pov: white_pov,
                            depth: line.depth,
                            elapsed,
                            nodes,
                            nps_m,
                        });
                    }
                }
                if let Some(entry) = self.history.last_mut() {
                    entry.noise_pick = noise_pick;
                }
                // Analyse the engine's move retrospectively (unbiased
                // config) so it gets an honest verdict + eval-bar score,
                // exactly like a user move. The `engine_info` above is the
                // play-time score (play depth, possibly masked / a noise
                // pick) and is only a transient eval-bar placeholder; the
                // retrospective analysis is the source of truth.
                self.queue_engine_move_retrospective(engine_move_index);
                // Engine just moved — any open Hint pop-over was for the
                // prior position, so close it.
                self.close_hint();
                // Self-play (EngineMode::Both) needs us to queue the
                // *next* engine move after each completes; without this
                // the bot freezes after move 1. For EngineMode::Side
                // and EngineMode::None this is a no-op — the post-move
                // side-to-move isn't the engine, so the guard returns
                // immediately.
                self.maybe_queue_engine_search();
                // The move is now (usually) back with the user — auto-
                // open the Hint pop-over if they've opted into auto-coach.
                // Guarded so it never fires mid-self-play or while the
                // next search is still queued.
                self.maybe_auto_coach();
            }
            WorkerResult::Retrospective {
                gen,
                target_index,
                user_move,
                analyses,
                elapsed,
                nodes,
                nps_m,
            } => {
                if gen != self.gen {
                    return;
                }
                // Snapshot pre-move position before mutating the entry
                // — we need it for the classifier below and the
                // immutable borrow can't coexist with the later
                // `history.get_mut`.
                let pre_pos = (target_index <= self.history.len())
                    .then(|| self.pre_move_position(target_index));
                if let Some(entry) = self.history.get_mut(target_index) {
                    entry.retrospective = Some(RetrospectiveResult {
                        user_move,
                        analyses: analyses.clone(),
                        elapsed,
                        nodes,
                        nps_m,
                    });
                }
                // If we held the engine reply waiting for the
                // classifier to decide, decide now. The retrospective
                // we just received must be for the *latest* user move
                // — anything else is a stale arrival and we ignore it
                // for intervention purposes (the gen-check above
                // already filtered most of those).
                if self.awaiting_intervention_decision && target_index + 1 == self.history.len() {
                    self.awaiting_intervention_decision = false;
                    let prior_move = self.prior_move_for(target_index);
                    let assessment = pre_pos.as_ref().map(|pp| {
                        chess_tutor_engine::analysis::classify_user_move(
                            pp,
                            &analyses,
                            user_move,
                            &gating_config_for(self.learning.mistake_handling),
                            prior_move,
                        )
                    });
                    if let Some(assessment) = assessment {
                        if intervention_required(&assessment, &self.learning) {
                            self.pending_intervention = Some(PendingIntervention {
                                at_history_index: target_index,
                                original_move: user_move,
                                assessment,
                                concept_revealed: false,
                            });
                        }
                    }
                    self.maybe_queue_engine_search();
                }
            }
            WorkerResult::AnalyzeSync { .. } => {
                // Synchronous analyses are consumed inline by
                // [`Self::run_analysis`], not via the regular event
                // stream. Any AnalyzeSync result that reaches here is
                // a stale arrival — drop it.
            }
        }
    }

    /// True between a [`Self::maybe_queue_engine_search`] that
    /// dispatched a worker job and the matching [`WorkerResult`]
    /// arriving. CLI / headless callers use this to decide whether
    /// to block on [`Self::wait_for_worker`] or prompt the user.
    pub fn is_engine_thinking(&self) -> bool {
        self.engine_thinking
    }

    /// Block until the next worker result arrives, process it, then
    /// drain any further results. Companion to [`Self::poll_worker`]
    /// for synchronous callers (CLI). Returns immediately if the
    /// worker channel is disconnected.
    pub fn wait_for_worker(&mut self) {
        if let Ok(result) = self.worker_rx.recv() {
            self.handle_worker_result(result);
        }
        self.poll_worker();
    }

    /// Run an analysis on `pos` with `params`, blocking until the
    /// worker returns. The CLI's REPL `search` and `analyze` commands
    /// use this so they don't need a private engine — the same
    /// analytical worker that powers retrospective and hint paths
    /// handles them too. Other worker results encountered while
    /// waiting are processed normally (engine moves applied to
    /// history, etc.). Returns an empty [`AnalysisOutcome`] if the
    /// worker channel is disconnected.
    pub fn run_analysis(&mut self, pos: Position, params: SearchParams) -> AnalysisOutcome {
        let _ = self.worker_tx.send(WorkerJob::AnalyzeSync {
            pos: Box::new(pos),
            params,
        });
        loop {
            match self.worker_rx.recv() {
                Ok(WorkerResult::AnalyzeSync {
                    analyses,
                    elapsed,
                    nodes,
                    nps_m,
                }) => {
                    return AnalysisOutcome {
                        analyses,
                        elapsed,
                        nodes,
                        nps_m,
                    };
                }
                Ok(other) => self.handle_worker_result(other),
                Err(_) => return AnalysisOutcome::default(),
            }
        }
    }
}
