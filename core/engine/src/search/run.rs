//! Iterative deepening: [`Search::run`] (the MultiPV driver),
//! `run_forced_slots` (force-include passes), and `aspiration_search`.

use super::*;
use crate::engine::{SearchLine, SearchParams};
use crate::position::Position;
use crate::types::{Color, Move, Value};
use std::time::Instant;

impl<'a> Search<'a> {
    /// Run a search under `params` and return up to `params.multi_pv`
    /// ranked principal variations with per-line traces. Returns an
    /// empty vector when the root position has no legal moves
    /// (checkmate or stalemate) — callers can surface this as a
    /// terminal result.
    pub(crate) fn run(&mut self, pos: &mut Position, params: &SearchParams) -> Vec<SearchLine> {
        self.tt.new_search();
        self.nodes = 0;
        for slot in self.nodes_per_ply.iter_mut() {
            *slot = 0;
        }
        self.seldepth = 0;
        // The stop flag is shared and may have been left `true` by a
        // previous search (the caller is responsible for handing us a
        // fresh one when they want a new run). Don't reset here.
        self.start_time = Instant::now();
        self.stop_time = params.max_time.map(|d| self.start_time + d);
        self.max_nodes = params.max_nodes;
        self.eval_mask = params.eval_mask;
        self.qsearch_cap = params
            .qsearch_max_plies
            .map(|q| q as i32)
            .unwrap_or(QSEARCH_UNBOUNDED);
        self.eg_skill = params.endgame_skill;
        self.tt_hit_average = TT_HIT_AVERAGE_INIT;
        self.nmp_min_ply = 0;
        self.nmp_color = Color::White;
        self.next_stop_check = STOP_CHECK_INTERVAL;
        self.verbose_progress = params.verbose_progress;
        self.verbose_next_tick = VERBOSE_TICK_INTERVAL;
        self.root_stm = pos.side_to_move();
        // Seed repetition tracking with the caller-supplied game
        // history, then the root. `is_repetition` fires on any match in
        // `path_keys[..len - 1]`, so a move inside the search that
        // returns to a position already reached in the real game is
        // scored as a draw — which is the point: the engine must not
        // recommend a repetition when it's winning.
        self.path_keys.clear();
        self.path_keys.extend_from_slice(&params.game_history);
        self.path_keys.push(pos.key());
        for k in self.killers.iter_mut() {
            *k = [Move::NONE; 2];
        }

        // Build the root move list up front. Legal-move generation at
        // the root gives us the full candidate set; MultiPV iterates
        // through it slot-by-slot.
        self.root_moves.clear();
        let mut root_legal = crate::movegen::MoveList::new();
        crate::movegen::generate_legal_moves(pos, &mut root_legal);
        for &mv in &root_legal {
            self.root_moves.push(RootMove {
                mv,
                score: -Value::INFINITE,
                pv: vec![mv],
                prev_score: Value::ZERO,
            });
        }

        if self.root_moves.is_empty() {
            return Vec::new();
        }

        // Clamp MultiPV to the number of legal moves — asking for 5
        // lines in a position with only 3 legal replies just returns 3.
        self.multi_pv = params.multi_pv.clamp(1, self.root_moves.len());

        let max_depth = params.max_depth.max(1);
        let mut completed_depth: u32 = 0;

        'ids: for depth in 1..=max_depth {
            if self.verbose_progress {
                eprintln!(
                    "[search] depth {depth} starting ({} nodes so far, {} ms elapsed)",
                    self.nodes,
                    self.start_time.elapsed().as_millis(),
                );
            }
            for pv_idx in 0..self.multi_pv {
                self.pv_idx = pv_idx;
                let prev = self.root_moves[pv_idx].prev_score;
                let _score = self.aspiration_search(pos, depth as i32, prev);

                if self.is_aborted() {
                    break 'ids;
                }

                // Stable-sort the tail [pv_idx..] by score desc, so the
                // newly-discovered best unclaimed move lands at pv_idx.
                // Moves at indices < pv_idx were claimed by earlier
                // slots and stay fixed.
                self.root_moves[pv_idx..].sort_by_key(|rm| std::cmp::Reverse(rm.score));
            }

            // Promote current scores to prev_score for the next
            // iteration's aspiration-window seed.
            for rm in self.root_moves.iter_mut() {
                rm.prev_score = rm.score;
            }

            completed_depth = depth;

            if self.verbose_progress {
                let best = &self.root_moves[0];
                eprintln!(
                    "[search] depth {depth} complete: best={}-{} score={} nodes={} elapsed={} ms",
                    best.mv.from().to_algebraic(),
                    best.mv.to().to_algebraic(),
                    best.score.0,
                    self.nodes,
                    self.start_time.elapsed().as_millis(),
                );
            }

            // NOTE: we deliberately do NOT stop iterative deepening when
            // a mate appears in the leader. SF11 only short-circuits on
            // mate under an explicit `go mate X` limit (search.cpp:521-525,
            // gated on `Limits.mate`); in a normal depth-budget search it
            // keeps iterating to the depth limit. That continuation is
            // load-bearing for *mate-distance* correctness: a single
            // depth-limited search (especially under MultiPV, where sibling
            // PV slots pollute the TT) frequently first surfaces a *longer*
            // mate than the optimum. Letting iterative deepening run to the
            // full depth lets mate-distance pruning converge the leader onto
            // the shortest mate, and makes the reported distance both
            // depth-deterministic and MultiPV-invariant. An earlier
            // `break`-on-mate here was the root cause of the eval-bar
            // "mate-in-N jumps around" pathology (fixed 2026-06-01; the
            // aspiration delta long suspected for it was a red herring).
            // The continuation is cheap (mate pruning collapses the tree
            // once the leader is a mate) and bounded by the analytical
            // node/time caps; the parity bench is unaffected (its positions
            // hold no forced mate at the bench depths).
        }

        // Final ordering: each PV slot's search ran with its own
        // aspiration window and a slightly-different TT state, so the
        // per-slot scores aren't guaranteed to be strictly monotonic
        // across slots (a well-known MultiPV quirk). Stable-sort the
        // first `multi_pv` slots by score descending so the output
        // reflects what a teaching UI expects: "best move first, then
        // next-best, etc.". Slots past `multi_pv` stay untouched.
        self.root_moves[..self.multi_pv].sort_by_key(|rm| std::cmp::Reverse(rm.score));

        // force_include pass: after natural MultiPV has found its
        // top-k, run a dedicated single-move IDS for each forced move
        // that isn't already in the top-k. Each forced slot reuses the
        // same aspiration_search + negamax path — we just pin the
        // forced move into position `multi_pv` and temporarily truncate
        // the tail so `allowed_root` resolves to that one move only.
        // The slot's output lands at `root_moves[multi_pv]` and
        // `self.multi_pv` increments by one per successful forced slot.
        if !params.force_include.is_empty() && !self.is_aborted() {
            self.run_forced_slots(pos, &params.force_include, max_depth);

            // Final re-sort so forced moves interleave with the natural
            // top-k by score, keeping "best move first" even when a
            // forced move happens to be the strongest alternative.
            self.root_moves[..self.multi_pv].sort_by_key(|rm| std::cmp::Reverse(rm.score));
        }

        // Build the output vector. For each PV slot, walk the line and
        // capture per-ply traces, then compute the settled-ply index
        // from the white-POV score trajectory. Slots that never got
        // scored (we aborted before any iteration finished) are dropped.
        let root_stm = pos.side_to_move();
        let mut out = Vec::with_capacity(self.multi_pv);
        for rm in self.root_moves.iter().take(self.multi_pv) {
            if rm.score == -Value::INFINITE {
                continue;
            }
            let ply_traces = self.trace_along_pv(pos, &rm.pv);
            let settled_ply = compute_settled_ply(&ply_traces, root_stm);
            out.push(SearchLine {
                pv: rm.pv.clone(),
                score: rm.score,
                depth: completed_depth,
                ply_traces,
                settled_ply,
            });
        }
        out
    }

    /// Run a dedicated single-move iterative-deepening pass for every
    /// move in `forced` that isn't already in `root_moves[..multi_pv]`.
    /// Each successful forced slot grows `self.multi_pv` by one.
    ///
    /// Mechanics: we swap the forced move into `root_moves[multi_pv]`,
    /// temporarily split off the rest of `root_moves` past that slot
    /// (so `allowed_root` inside `negamax` resolves to just the forced
    /// move), run the full IDS, then restore the split-off tail.
    pub(super) fn run_forced_slots(&mut self, pos: &mut Position, forced: &[Move], max_depth: u32) {
        // Deduplicate forced list + skip anything already in the top-k
        // with a real score. `Move::NONE` is a sentinel used by callers
        // that pass an uninitialized slot — silently ignore it.
        let mut already_covered: std::collections::HashSet<Move> = self.root_moves[..self.multi_pv]
            .iter()
            .filter(|rm| rm.score != -Value::INFINITE)
            .map(|rm| rm.mv)
            .collect();

        for &forced_mv in forced {
            if self.is_aborted() {
                break;
            }
            if forced_mv == Move::NONE {
                continue;
            }
            if !already_covered.insert(forced_mv) {
                continue; // duplicate in the forced list, or already in top-k
            }
            // Illegal moves are silently dropped.
            let Some(idx) = self.root_moves.iter().position(|rm| rm.mv == forced_mv) else {
                continue;
            };

            let new_slot = self.multi_pv;
            if idx != new_slot {
                self.root_moves.swap(idx, new_slot);
            }

            // Reset per-slot scratch so the fresh IDS doesn't inherit
            // stale aspiration seeds or leftover -INFINITE scores.
            self.root_moves[new_slot].prev_score = Value::ZERO;
            self.root_moves[new_slot].score = -Value::INFINITE;
            self.root_moves[new_slot].pv = vec![forced_mv];

            // Truncate the tail so `allowed_root[..pv_idx..]` becomes
            // exactly `[forced_mv]` for this slot's search. Save the
            // removed portion for restoration.
            let saved_tail = self.root_moves.split_off(new_slot + 1);

            self.pv_idx = new_slot;
            for depth in 1..=max_depth {
                let prev = self.root_moves[new_slot].prev_score;
                let _ = self.aspiration_search(pos, depth as i32, prev);
                if self.is_aborted() {
                    break;
                }
                self.root_moves[new_slot].prev_score = self.root_moves[new_slot].score;

                // Run the forced slot to the full depth even when it's a
                // mate — same reasoning as the main IDS loop above. A
                // forced move that mates should report the *shortest* mate
                // its line reaches; breaking on the first mate found would
                // freeze in a too-long distance and re-introduce the
                // MultiPV mate-distance pathology this slot feeds into.
            }

            // Restore the tail so subsequent forced slots see the full
            // legal-move list again.
            self.root_moves.extend(saved_tail);

            // Include this slot in the output. Only grow on a real
            // score — if aborted before any depth completed, drop the
            // slot rather than emitting an -INFINITE line.
            if self.root_moves[new_slot].score != -Value::INFINITE {
                self.multi_pv += 1;
            }
        }
    }

    // ------------------------------------------------------------------
    // Iterative deepening + aspiration
    // ------------------------------------------------------------------

    pub(super) fn aspiration_search(
        &mut self,
        pos: &mut Position,
        depth: i32,
        prev_score: Value,
    ) -> Value {
        let mut alpha = -Value::INFINITE;
        let mut beta = Value::INFINITE;
        let mut delta = ASPIRATION_DELTA;

        if depth >= 4 {
            alpha = Value((prev_score.0 - delta).max(-Value::INFINITE.0));
            beta = Value((prev_score.0 + delta).min(Value::INFINITE.0));
        }

        // SF11 search.cpp:450, 453, 485, 492 — consecutive fail-highs
        // accumulate `failed_high_cnt`, and each re-search runs at
        // `max(1, rootDepth - failed_high_cnt)` instead of full depth.
        // Reset to 0 on every fail-low (and at the start of every new
        // iterative-deepening depth, since we re-enter with cnt=0).
        // The fail-high re-search is what gets cheaper; the search
        // returns a slightly shallower PV when the chain ends on a
        // reduced-depth iteration. The reduction only applies to
        // fail-highs because fail-low chains converge naturally as
        // alpha tracks the actually-returned score.
        //
        // We deliberately keep our existing `ASPIRATION_DELTA = 17`
        // initial and `delta *= 2` growth rather than SF11's
        // `21 + |prev|/256` initial and `delta + delta/4 + 5` growth.
        // SF11's tuning regressed FEN 26 d=13 ~3× (138k → 447k) on
        // our codebase — the wider initial window costs us more in
        // alpha-beta inefficiency than it saves in avoided re-searches.
        // Aggressive 2× growth + depth-reduction is the right local
        // optimum.
        let mut failed_high_cnt: i32 = 0;
        loop {
            let adjusted_depth = (depth - failed_high_cnt).max(1);
            if self.verbose_progress {
                eprintln!(
                    "[search] aspiration depth={depth} adj_depth={adjusted_depth} window=[{}, {}] delta={delta}",
                    alpha.0, beta.0,
                );
            }

            // Root is a PV node, and SF's invariant is `!(PvNode &&
            // cutNode)` — so we always enter with `cut_node = false`.
            let score = self.negamax(pos, alpha, beta, adjusted_depth, 0, true, true, None, false);

            if self.is_aborted() {
                return score;
            }

            if self.verbose_progress {
                let outcome = if score <= alpha {
                    "FAIL-LOW"
                } else if score >= beta {
                    "FAIL-HIGH"
                } else {
                    "OK"
                };
                eprintln!(
                    "[search] aspiration depth={depth} result={outcome} score={}",
                    score.0,
                );
            }

            if score <= alpha {
                beta = Value((alpha.0 + beta.0) / 2);
                alpha = Value((score.0 - delta).max(-Value::INFINITE.0));
                failed_high_cnt = 0;
            } else if score >= beta {
                beta = Value((score.0 + delta).min(Value::INFINITE.0));
                failed_high_cnt += 1;
            } else {
                return score;
            }

            delta = (delta * 2).max(delta + 1);

            debug_assert!(alpha >= -Value::INFINITE && beta <= Value::INFINITE);
        }
    }
}
