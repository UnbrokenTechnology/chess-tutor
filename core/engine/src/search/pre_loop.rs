//! Pre-loop pruning phases of `negamax`: `try_null_move` and
//! `try_probcut`. Each returns `Some(value)` to prune (or on abort) and
//! `None` to let the search proceed.

use super::*;
use crate::movepick::MovePicker;
use crate::position::Position;
use crate::types::{Depth, Move, PieceType, Square, Value};

impl<'a> Search<'a> {
    /// Null-move pruning with verification (SF11 search.cpp:838-885), a
    /// pre-loop pruning phase. Returns `Some(value)` when the node can be
    /// pruned (a passing move still fails high) or the search was aborted
    /// mid-phase; `None` when the gate doesn't fire and the search should
    /// proceed. Extracted verbatim from `negamax`; mutates
    /// `nmp_min_ply` / `nmp_color` only transiently around the
    /// verification search.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn try_null_move(
        &mut self,
        pos: &mut Position,
        beta: Value,
        depth: i32,
        ply: usize,
        in_check: bool,
        is_pv: bool,
        cut_node: bool,
        prev: Option<(PieceType, Square)>,
        eval: Value,
        static_eval: Value,
        improving: bool,
        parent_was_null: bool,
    ) -> Option<Value> {
        // --- Step 9. Null-move pruning with verification (SF11
        // search.cpp:838-885, ~40 Elo) ---
        //
        // Gate (search.cpp:839-847): the refined `eval` must clear beta
        // *and* not undershoot the raw static eval, the raw static eval
        // must clear a depth-scaled floor (loosened 30 cp when
        // improving — a rising eval is trusted more), the parent must
        // not itself have played a null move, its last quiet must not
        // have scored hugely (`statScore < 23397`), the side to move
        // must hold non-pawn material (zugzwang guard), and NMP must
        // not be suspended for us by an active verification search.
        let nmp_eval_floor = beta.0 - 32 * depth + 292 - if improving { 30 } else { 0 };
        let parent_stat_score = self.stack[STACK_SENTINEL + ply - 1].stat_score;
        let us = pos.side_to_move();
        if !is_pv
            && !in_check
            && !parent_was_null
            && depth >= NULL_MIN_DEPTH
            && eval != Value::NONE
            && eval >= beta
            && eval >= static_eval
            && static_eval.0 >= nmp_eval_floor
            && parent_stat_score < 23397
            && pos.non_pawn_material(us).0 > 0
            && (ply >= self.nmp_min_ply || us != self.nmp_color)
        {
            // SF11 search.cpp:852 — dynamic reduction from depth and how
            // far the eval clears beta. `eval >= beta` is guaranteed by
            // the gate, so the margin term is already non-negative.
            let r = (854 + 68 * depth) / 258 + ((eval.0 - beta.0) / 192).min(3);
            // Faithful to SF: `depth - R` (no clamp). When it lands at
            // or below zero the child dives straight into quiescence,
            // which is what makes the qsearch after-null refinement (Q4)
            // reachable.
            let reduced = depth - r;

            // Mark this ply as the null move: cont-hist "1 ply ago"
            // resolves to the sentinel slot (moved_piece_idx == 0 reads
            // zero, mirroring SF's `continuationHistory[0][0][NO_PIECE]
            // [0]`), and `was_null` tells the child to skip NMP and take
            // the after-null eval path.
            self.stack[STACK_SENTINEL + ply].moved_piece_idx = 0;
            self.stack[STACK_SENTINEL + ply].to_idx = 0;
            self.stack[STACK_SENTINEL + ply].was_capture = false;
            self.stack[STACK_SENTINEL + ply].in_check = false;
            self.stack[STACK_SENTINEL + ply].captured_piece_kind = None;
            self.stack[STACK_SENTINEL + ply].was_null = true;

            let saved = pos.do_null_move();
            self.tt.prefetch(pos.key());
            self.path_keys.push(pos.key());
            let null_score = -self.negamax(
                pos,
                -beta,
                Value(-beta.0 + 1),
                reduced,
                ply + 1,
                false,
                false,
                None,
                !cut_node,
            );
            self.path_keys.pop();
            pos.undo_null_move(saved);

            if self.is_aborted() {
                return Some(Value::ZERO);
            }

            if null_score >= beta {
                // Don't return unproven mate scores (search.cpp:866-867).
                let clamped = if null_score.0 >= Value::MATE.0 - Value::MAX_PLY {
                    beta
                } else {
                    null_score
                };

                // Trust the cutoff directly when a verification is
                // already running (no recursive verification) or when
                // the depth is shallow and beta isn't in mate territory.
                // Otherwise re-search at `depth - R` with NMP suspended
                // for us until `ply` passes `nmp_min_ply`, and only
                // accept the cutoff if the verification also fails high.
                if self.nmp_min_ply != 0
                    || (beta.0.abs() < Value::KNOWN_WIN.0 && depth < NMP_VERIFY_MIN_DEPTH)
                {
                    return Some(clamped);
                }

                // `depth - reduced` can be non-positive at shallow,
                // mate-territory verifications; saturate the resulting
                // ply offset at zero (SF uses signed plies, where a
                // negative floor is simply always-satisfied).
                let verify_offset = (3 * reduced / 4).max(0) as usize;
                self.nmp_min_ply = ply + verify_offset;
                self.nmp_color = us;
                let v = self.negamax(
                    pos,
                    Value(beta.0 - 1),
                    beta,
                    reduced,
                    ply,
                    false,
                    false,
                    prev,
                    false,
                );
                self.nmp_min_ply = 0;

                if self.is_aborted() {
                    return Some(Value::ZERO);
                }
                if v >= beta {
                    return Some(clamped);
                }
            }
        }

        None
    }

    // ------------------------------------------------------------------
    // ProbCut
    // ------------------------------------------------------------------

    /// ProbCut pre-loop pruning phase (SF11 search.cpp:888-929). Returns
    /// `Some(value)` when the node can be pruned (a "good enough" capture
    /// refutes the parent move) or the search was aborted mid-phase;
    /// `None` when the gate doesn't fire and the main move loop should
    /// proceed. Extracted verbatim from `negamax`; see the inline comment
    /// for the heuristic.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn try_probcut(
        &mut self,
        pos: &mut Position,
        beta: Value,
        depth: i32,
        ply: usize,
        in_check: bool,
        is_pv: bool,
        cut_node: bool,
        static_eval: Value,
        improving: bool,
        tt_move: Move,
    ) -> Option<Value> {
        // --- ProbCut (SF11 search.cpp:888-929, claimed ~10 Elo) ---
        //
        // If a "good enough" capture (SEE clearing a raised-beta
        // margin) returns a reduced search value above raisedBeta,
        // we can prune the whole node — the parent move that led
        // here is refuted by a capture we wouldn't even need to
        // search deeply.
        //
        // Two-phase verification: first a zero-window qsearch (cheap
        // — bails on stand-pat / quick recapture); only if that
        // holds do we run a `depth - 4` regular search. The qsearch
        // gate kills most candidates before the expensive recurse.
        //
        // Capture budget = `2 + 2 * cut_node`: at cut nodes (where a
        // fail-high is expected) we try up to 4 captures; elsewhere
        // only 2. This formula was the load-bearing piece our prior
        // (2026-05-12) ProbCut attempt lacked — no flat budget
        // worked across position types.
        if !is_pv
            && !in_check
            && depth >= 5
            && static_eval != Value::NONE
            && beta.0.abs() < Value::MATE.0 - Value::MAX_PLY
        {
            let raised_beta = (beta.0 + 189 - 45 * improving as i32).min(Value::INFINITE.0);
            let raised_beta_v = Value(raised_beta);
            let see_threshold = Value(raised_beta - static_eval.0);
            let budget = 2 + 2 * cut_node as i32;
            let mut probcut_count: i32 = 0;

            // TT move only enters via the picker if it's a capture
            // /promotion — otherwise the picker's first emission
            // would be a quiet move that we'd then skip.
            let tt_is_cap_or_promo = tt_move != Move::NONE
                && (pos.is_capture(tt_move)
                    || tt_move.kind() == crate::types::MoveKind::Promotion);
            let pc_tt = if tt_is_cap_or_promo {
                tt_move
            } else {
                Move::NONE
            };
            let mut pc_picker = MovePicker::new_qs(
                pos,
                pc_tt,
                Depth::QS_NO_CHECKS,
                None,
                crate::movepick::NO_CONT_HIST,
            );

            let mut pc_value = Value::NONE;
            while probcut_count < budget {
                let mv = pc_picker.next_move(
                    pos,
                    Some(self.history),
                    Some(self.cont_history),
                    Some(self.capture_history),
                    true, // qsearch picker emits captures only anyway
                );
                if mv == Move::NONE {
                    break;
                }

                // Captures/promotions only, and only those whose SEE
                // clears the raised-beta margin. Bad captures and
                // moves not clearing the threshold are skipped — we
                // keep iterating until we exhaust the picker or hit
                // the budget.
                let is_cap_or_promo =
                    pos.is_capture(mv) || mv.kind() == crate::types::MoveKind::Promotion;
                if !is_cap_or_promo {
                    continue;
                }
                if !pos.see_ge(mv, see_threshold) {
                    continue;
                }

                let moved_piece = pos.moved_piece(mv);
                let state = pos.do_move(mv);
                self.tt.prefetch(pos.key());
                let us_was = !pos.side_to_move();
                let our_king = pos.king_square(us_was);
                let still_attacked = (pos.attackers_to(our_king, pos.occupied())
                    & pos.pieces_by_color(!us_was))
                    .any();
                if still_attacked {
                    pos.undo_move(mv, state);
                    continue;
                }

                probcut_count += 1;

                // Record on the per-ply stack so the recursive
                // search reads the right parent (in_check,
                // was_capture, moved_piece, to) for cont-hist
                // lookups.
                self.stack[STACK_SENTINEL + ply].moved_piece_idx = moved_piece.index() as u8;
                self.stack[STACK_SENTINEL + ply].to_idx = mv.to().index() as u8;
                self.stack[STACK_SENTINEL + ply].in_check = in_check;
                self.stack[STACK_SENTINEL + ply].was_capture = true;
                self.stack[STACK_SENTINEL + ply].captured_piece_kind =
                    state.captured.map(|p| p.kind());
                self.stack[STACK_SENTINEL + ply].was_null = false;

                self.path_keys.push(pos.key());
                let child_prev = Some((moved_piece.kind(), mv.to()));

                // Phase 1: zero-window qsearch — cheap rejection.
                // SF11 search.cpp:918 calls qsearch with the default
                // depth (DEPTH_ZERO = QS_CHECKS), so the probcut
                // qsearch starts at the same "include checks" depth
                // as a fresh entry from negamax.
                let mut value = -self.qsearch(
                    pos,
                    Value(-raised_beta),
                    Value(-raised_beta + 1),
                    ply + 1,
                    Depth::QS_CHECKS.0,
                );

                // Phase 2: regular search at depth-4 if qsearch held.
                if value >= raised_beta_v {
                    value = -self.negamax(
                        pos,
                        Value(-raised_beta),
                        Value(-raised_beta + 1),
                        depth - 4,
                        ply + 1,
                        false,
                        false,
                        child_prev,
                        !cut_node,
                    );
                }

                self.path_keys.pop();
                pos.undo_move(mv, state);

                if self.is_aborted() {
                    return Some(Value::ZERO);
                }

                if value >= raised_beta_v {
                    pc_value = value;
                    break;
                }
            }

            if pc_value != Value::NONE {
                return Some(pc_value);
            }
        }

        None
    }
}
