//! `negamax_moves`: the move loop of `negamax` — move ordering, the
//! Step-13 prunes, the extension chain, per-move search, and the
//! beta-cutoff stats. Returns a [`MovesOutcome`] for the caller's
//! terminal-position check and TT save.

use super::*;
use crate::movepick::{
    ContHistKeys, MovePicker,
};
use crate::position::Position;
use crate::types::{Depth, Move, PieceType, Square, Value};

impl<'a> Search<'a> {
    /// The move-loop body of `negamax`: iterate the legal moves (root
    /// MultiPV filter, Step-13 prunes, extension chain, LMR/PVS search via
    /// [`Search::search_made_move`], and the β-cutoff stats via
    /// [`Search::update_all_stats`]) and return the loop outcome. The
    /// caller (`negamax`) handles the terminal-position check and TT save.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn negamax_moves(
        &mut self,
        pos: &mut Position,
        mut alpha: Value,
        beta: Value,
        depth: i32,
        ply: usize,
        is_root: bool,
        is_pv: bool,
        cut_node: bool,
        prev: Option<(PieceType, Square)>,
        in_check: bool,
        static_eval: Value,
        tt_move: Move,
        tt_pv: bool,
        improving: bool,
        cont_keys: ContHistKeys,
        // Perception never-empty fallback rerun: when `Some`, only this
        // move is searched and the visibility filter is bypassed for it
        // (the caller picked it as the highest-visibility pruned move
        // after the normal pass came back empty).
        visibility_override: Option<Move>,
    ) -> MovesOutcome {
        // --- Main move loop ---
        let counter_move = match prev {
            Some((pt, sq)) => self.counter_moves.get(pt, sq),
            None => Move::NONE,
        };
        let mut picker = MovePicker::new_main(
            pos,
            tt_move,
            Depth(depth),
            self.killers[ply],
            counter_move,
            cont_keys,
        );

        // At the root, we want the picker's ordering (TT move, then
        // captures, then killers/history) but only among moves the
        // current PV slot still owns — earlier slots claimed their top
        // moves in previous iterations and those stay fixed. Collect
        // the set of in-bounds root moves for O(n) membership checks.
        let allowed_root: Option<Vec<Move>> = if is_root {
            Some(
                self.root_moves[self.pv_idx..]
                    .iter()
                    .map(|rm| rm.mv)
                    .collect(),
            )
        } else {
            None
        };

        let mut best_score = -Value::INFINITE;
        let mut best_move = Move::NONE;
        let mut move_count = 0usize;
        // Highest-visibility move the perception filter pruned at this
        // node — the never-empty fallback candidate (see
        // [`MovesOutcome::Done::unseen_fallback`]).
        let mut best_unseen: Option<(Move, f64)> = None;
        // SF11 `moveCountPruning` (search.cpp:629/956/1002). Lifted out
        // of the depth-gated shallow-prune box so it can fire at any
        // depth — once `move_count` hits the LMP threshold, the picker
        // skips remaining quiets for the rest of this node. Gated by
        // [`MOVE_COUNT_PRUNING_UNIVERSAL`] so we can A/B the change.
        let mut move_count_pruning: bool = false;
        // Stack-allocated rather than Vec — at ~30–50 quiets per node and
        // millions of nodes per search, the prior `Vec::new() + push` form
        // reallocated through capacities 4→8→16→32→64 every frame. Per-frame
        // cost is 512 bytes (Move is u16); MAX_PLY recursion stays under
        // budget.
        let mut quiets_tried = crate::movegen::MoveList::new();
        // Captures tried before the cutoff — used for the
        // capture-history `-bonus1` decrement on β-cutoff (regardless
        // of whether the cutoff move itself was a capture).
        let mut captures_tried = crate::movegen::MoveList::new();
        let mut raised_alpha = false;

        // --- Per-node precomputes for the SF11 extension chain
        // (search.cpp:1072-1090) ---
        //
        // 1. Enemy king's blockers, used by the check extension to
        //    distinguish discovery checks from direct ones. A move
        //    from any square in this bitboard that lands on a check
        //    is a discovery check (the discoverer is already aimed
        //    at the king and the moving piece unblocks the line);
        //    those we extend unconditionally. Direct non-discovery
        //    checks fall back to a SEE filter to drop SEE-negative
        //    sac-checks that the search refutes trivially.
        //
        // 2. Last-captures node-eligibility. The extension fires
        //    when the parent's move was a heavy capture (≥ minor in
        //    endgame value) AND the position is now in thin material
        //    (≤ 2 rooks of non-pawn material). It widens *every*
        //    move at the current node by 1 ply, so it's a node-level
        //    precompute, not per-move. SF reads the captured piece
        //    via `pos.captured_piece()` (the child sees the parent's
        //    move via StateInfo); we read the parent's
        //    `captured_piece_kind` directly off the stack.
        //
        // Both are invariant across the move loop's iterations (we
        // undo each move), so a single compute up front is fine.
        let us_at_node = pos.side_to_move();
        let enemy_blockers = pos.blockers_for_king(!us_at_node);
        let parent_captured = self.stack[STACK_SENTINEL + ply - 1].captured_piece_kind;
        let parent_was_heavy_capture =
            matches!(parent_captured, Some(pt) if Value::eg_of_piece(pt).0 > Value::PAWN_EG.0);
        let last_captures_node_eligible =
            parent_was_heavy_capture && pos.non_pawn_material_total().0 <= 2 * Value::ROOK_MG.0;

        loop {
            let mv = picker.next_move(
                pos,
                Some(self.history),
                Some(self.cont_history),
                Some(self.capture_history),
                move_count_pruning,
            );
            if mv == Move::NONE {
                break;
            }

            // Root MultiPV filter: skip moves claimed by earlier PV
            // slots — they're fixed at positions [0..pv_idx].
            if let Some(allowed) = &allowed_root {
                if !allowed.contains(&mv) {
                    continue;
                }
            }

            // Capture the moved piece (before do_move clears the
            // from-square): kind goes into `prev` for the child's
            // counter-move lookup; the colored Piece's index goes into
            // the per-ply stack so descendants' cont-hist lookups can
            // find this move's sub-table.
            let moved_piece = pos.moved_piece(mv);
            let moved_pt = moved_piece.kind();

            // Pre-move snapshots for the extension chain. SEE reads
            // `piece_on(from)`, so it must run before `do_move`. The
            // discovery test is a single bitboard intersection
            // against the cached `enemy_blockers` snapshot.
            // `is_advanced_pawn_push` and `is_first_killer` are
            // static facts about the move; cheaper to capture once
            // here than re-derive in the extension chain.
            let from_was_enemy_blocker = (enemy_blockers & mv.from()).any();
            let see_nonneg = pos.see_ge(mv, Value::ZERO);
            let is_advanced_pawn_push = moved_pt == crate::types::PieceType::Pawn
                && (mv.to().from_perspective(us_at_node).rank() as u8)
                    >= (crate::types::Rank::R6 as u8);
            let is_first_killer = mv == self.killers[ply][0];

            // Legality (B1): test before making the move, so the
            // Step-13 prunes below can reject a move without a
            // `do_move`/`undo_move` round-trip. `pos.legal` is
            // oracle-tested against the make/unmake filter it replaces.
            if !pos.legal(mv) {
                continue;
            }

            // Perception filter ("a move you didn't see is never in
            // your tree") — after legality so the fallback candidate is
            // guaranteed playable, before `move_count += 1` so an
            // unseen move never counts as searched. Never applied in
            // check (evasions are forced — same rule as the qsearch
            // cap). The override rerun pins the loop to the fallback
            // move instead.
            if let Some(forced) = visibility_override {
                if mv != forced {
                    continue;
                }
            } else if self.perception.is_some()
                && !in_check
                && !self.move_is_seen(pos, mv, ply)
            {
                let v = self.move_visibility(pos, mv, ply);
                if best_unseen.is_none_or(|(_, bv)| v > bv) {
                    best_unseen = Some((mv, v));
                }
                continue;
            }

            move_count += 1;
            // Publish our running move_count onto the stack so the
            // child we're about to recurse into can read it via
            // `(ss-1)->move_count` for its CMP gate. SF11 search.cpp:979.
            self.stack[STACK_SENTINEL + ply].move_count = move_count as u32;

            // --- Pre-move move classification (B1) ---
            // Derived before `do_move` so the Step-13 prunes can fire
            // without making the move. `is_capture` equals the old
            // `state.captured.is_some() || ep`; `gives_check` is the
            // oracle-tested no-make predicate (== post-move `in_check`).
            let is_capture = pos.is_capture(mv);
            // SF11 `captureOrPromotion` — captures (incl. en passant) plus
            // every promotion. The quiet-LMR adjuster block and the
            // post-re-search cont-history feedback gate on its negation.
            let is_cap_or_promo =
                is_capture || mv.kind() == crate::types::MoveKind::Promotion;
            let gives_check = pos.gives_check(mv);
            // SF11 Step-13 outer gate `pos.non_pawn_material(us) > 0`,
            // evaluated for the position *after* the move: a promotion
            // adds a non-pawn piece, so a pure-pawn side gains material.
            // The `|| Promotion` term reproduces that post-move truth
            // from the pre-move board, preserving node-for-node behaviour.
            let npm_us_after_positive = pos.non_pawn_material(us_at_node).0 > 0
                || mv.kind() == crate::types::MoveKind::Promotion;

            // SF11 `moveCountPruning` update (search.cpp:1002): once
            // tripped, the *next* picker call skips quiet generation.
            if MOVE_COUNT_PRUNING_UNIVERSAL
                && !is_root
                && best_score > Value::MATED_IN_MAX_PLY
                && npm_us_after_positive
            {
                move_count_pruning = late_move_prune(depth, move_count, improving);
            }
            if is_root && self.verbose_progress {
                eprintln!(
                    "[search]   depth {depth} slot {} move #{move_count}: {}-{} ({} nodes, {} ms)",
                    self.pv_idx,
                    mv.from().to_algebraic(),
                    mv.to().to_algebraic(),
                    self.nodes,
                    self.start_time.elapsed().as_millis(),
                );
            }

            // --- Counter-move-based pruning (SF11 search.cpp:1010-1014, ~20 Elo) ---
            //
            // Drop quiet moves whose 1-ply-ago and 2-plies-ago
            // cont-history scores are both negative
            // (CounterMovePruneThreshold = 0 in SF11). At shallow
            // `lmr_d` only — beyond that the search is deep enough
            // to recover from a false positive.
            //
            // The depth threshold widens by 1 ply when the parent
            // context suggests the gate is safe: either the
            // parent's last quiet scored well (`statScore > 0`) or
            // the parent is on its first quiet (typically the TT
            // move). Both conditions correlate with "parent picked
            // a strong move, so a sibling that looks bad here is
            // probably actually bad." This widening is the
            // load-bearing piece our prior 2026-05-12 attempt
            // (gated on flat `lmr_d < 4`) was missing — without it,
            // CMP fires uniformly across position types and
            // catches good moves with noisy cont-hist as collateral.
            //
            // Sentinel handling: at a frame whose parent was the
            // root or a null-move ancestor, SF11 fills the
            // `NO_PIECE` row of every contHistory table with -1 so
            // the gate fires uniformly. We mimic that read-side
            // via [`cmp_cont_hist_read`] rather than mutating the
            // tables.
            let cmp_prune = !is_root
                && !in_check
                && !is_capture
                && !gives_check
                && best_score > Value::MATED_IN_MAX_PLY
                && npm_us_after_positive
                && self.cmp_cont_negative(depth, move_count, improving, ply, moved_piece, mv, cont_keys);
            if cmp_prune {
                // CMP-pruned moves are never searched, so (per SF's
                // `quietsSearched`, line 1300) they don't join the
                // bonus-decrement list. No move was made (B1 prunes
                // before `do_move`).
                continue;
            }

            // Quiet futility pruning (SF11 search.cpp:1016-1024, "Lever 2b").
            //
            // Gate is `lmrDepth < 6`, not raw `depth <= 7` — when chained
            // extensions keep raw `depth` high at deep ply, LMR still
            // pushes lmrDepth toward 0, and SF11's gate fires where the
            // old raw-depth gate didn't. This is the load-bearing
            // mechanism that prevents the deep-ply quiet tail in
            // chained-extension endgames (FENs 20 / 26 / 40 in the bench).
            //
            // History-sum gate matches SF11 verbatim: only futility-
            // prune when this quiet has a negative composite history
            // signal (main + cont[0,1,3] < 25000). Without the gate,
            // SF11's experience is that futility cuts good moves that
            // happen to land below `eval + margin` for noisy positional
            // reasons. Universal LMP (Lever 1, wired into the
            // [`MovePicker`]) handles move-count pruning independently;
            // no LMP check here.
            let do_futility_prune = !is_root
                && !in_check
                && !is_capture
                && !gives_check
                && best_score > Value::MATED_IN_MAX_PLY
                // SF11 Step 13 gate (search.cpp:998): side-to-move at
                // this frame must have non-pawn material. After
                // `do_move` the side-to-move is the *opponent*, so the
                // side that just moved (== `us` in SF's sense) is the
                // negated side. Pure-pawn endgames skip Step 13.
                && npm_us_after_positive
                && self.quiet_futility_inner(
                    depth, move_count, improving, static_eval, alpha, us_at_node, moved_piece, mv,
                    cont_keys,
                );
            if do_futility_prune {
                quiets_tried.push(mv);
                continue;
            }

            // SEE pruning on losing captures at shallow depth. The
            // `best_score > MATED_IN_MAX_PLY` gate mirrors SF11's step
            // 13 outer condition (search.cpp:998) and is **load-bearing
            // for correctness**: without it, the first move at an
            // in-check node with one capture-evasion can be pruned
            // before being searched, leaving `best_score = -INFINITE`.
            // That sentinel then propagates through `value_to_tt` (which
            // stores INFINITE+ply, exceeding MATE), gets read back as
            // INFINITE on subsequent probes, and feeds a self-sustaining
            // INFINITE chain — visible as an aspiration loop that can't
            // exit because beta saturates at INFINITE. (The other two
            // step-13 prunes already carry this gate; SEE was the
            // outlier.) `pos.non_pawn_material(us_at_node) > 0` is
            // SF11's third outer gate — pure-pawn endgames also skip
            // shallow-prune for soundness.
            if !is_root
                && is_capture
                && depth <= 6
                && !gives_check
                && best_score > Value::MATED_IN_MAX_PLY
                && npm_us_after_positive
            {
                // B1: SEE-prune the losing capture before it is made.
                let margin = Value(-200 * depth);
                if !pos.see_ge(mv, margin) {
                    continue;
                }
            }

            // Extension chain (SF11 search.cpp:1072-1090) — see
            // [`compute_extension`] for the per-predicate rationale and
            // the isolated-addition caveat (don't remove any one of the
            // four; they're net-positive only together).
            let extension = compute_extension(
                pos,
                mv,
                us_at_node,
                gives_check,
                from_was_enemy_blocker,
                see_nonneg,
                is_first_killer,
                is_advanced_pawn_push,
                last_captures_node_eligible,
            );
            let new_depth = depth - 1 + extension;

            // B1: only now — for the moves that survived legality and
            // every Step-13 prune — do we actually make the move.
            let state = pos.do_move(mv);
            self.tt.prefetch(pos.key());

            self.path_keys.push(pos.key());

            // Record this move into the parent's stack frame so the
            // child's cont-hist lookups at "1 ply ago" find this move's
            // sub-table. Mirrors Stockfish's `ss->continuationHistory =
            // &thisThread->continuationHistory[inCheck][captureOrPromotion]
            // [movedPiece][to_sq(move)]` write.
            self.stack[STACK_SENTINEL + ply].moved_piece_idx = moved_piece.index() as u8;
            self.stack[STACK_SENTINEL + ply].to_idx = mv.to().index() as u8;
            self.stack[STACK_SENTINEL + ply].in_check = in_check;
            self.stack[STACK_SENTINEL + ply].was_capture = is_capture;
            self.stack[STACK_SENTINEL + ply].captured_piece_kind =
                state.captured.map(|p| p.kind());
            self.stack[STACK_SENTINEL + ply].was_null = false;

            let child_prev = Some((moved_pt, mv.to()));

            // Search the made move at the right depth/window: the LMR
            // reduced zero-window search, the full-depth re-search (with
            // its continuation-history feedback), and the PV re-search —
            // see [`search_made_move`].
            let score = self.search_made_move(
                pos,
                mv,
                moved_piece,
                child_prev,
                &state,
                alpha,
                beta,
                depth,
                new_depth,
                ply,
                move_count,
                best_score,
                is_root,
                is_pv,
                cut_node,
                in_check,
                gives_check,
                is_cap_or_promo,
                move_count_pruning,
                improving,
                static_eval,
                tt_move,
                tt_pv,
                cont_keys,
            );

            self.path_keys.pop();
            pos.undo_move(mv, state);

            if self.is_aborted() {
                return MovesOutcome::Aborted;
            }

            // Root bookkeeping: before the alpha/best_score update
            // below, write this move's authoritative score + PV back to
            // its slot in `root_moves`. Stockfish's rule: only store a
            // useful score when this move is either the first examined
            // (so it's our current best by default) or strictly
            // improves on the pre-update alpha. Other moves are tagged
            // with `-INFINITE` so the post-slot stable-sort pushes them
            // below the survivors.
            if is_root {
                let idx = self
                    .root_moves
                    .iter()
                    .position(|rm| rm.mv == mv)
                    .expect("root move picked must exist in root_moves");
                if move_count == 1 || score > alpha {
                    self.root_moves[idx].score = score;
                    let child_len = self.pv_length[ply + 1];
                    let mut pv_out = Vec::with_capacity(1 + child_len);
                    pv_out.push(mv);
                    for i in 0..child_len {
                        pv_out.push(self.pv[(ply + 1) * MAX_PLY + i]);
                    }
                    self.root_moves[idx].pv = pv_out;
                } else {
                    self.root_moves[idx].score = -Value::INFINITE;
                }
            }

            if score > best_score {
                best_score = score;
                best_move = mv;

                if score > alpha {
                    alpha = score;
                    raised_alpha = true;
                    self.update_pv(ply, mv);

                    if score >= beta {
                        // β-cutoff history updates — see [`update_all_stats`].
                        self.update_all_stats(
                            pos,
                            mv,
                            moved_piece,
                            is_capture,
                            depth,
                            ply,
                            prev,
                            &quiets_tried,
                            &captures_tried,
                            cont_keys,
                        );
                        break;
                    }
                }
            }

            if !is_capture {
                quiets_tried.push(mv);
            } else {
                captures_tried.push(mv);
            }
        }

        MovesOutcome::Done {
            best_score,
            best_move,
            raised_alpha,
            move_count,
            unseen_fallback: best_unseen.map(|(m, _)| m),
        }
    }
}
