//! `qsearch`: quiescence search — stand-pat, captures (and checks near
//! the top), with the recapture-only tail that bounds capture chains.

use super::*;
use crate::eval::evaluate_with_pawn_cache;
use crate::movepick::{ContHistKeys, MovePicker};
use crate::position::Position;
use crate::types::{Bound, Depth, Move, Square, Value};

impl<'a> Search<'a> {
    pub(super) fn qsearch(
        &mut self,
        pos: &mut Position,
        mut alpha: Value,
        beta: Value,
        ply: usize,
        depth: i32,
    ) -> Value {
        if ply >= MAX_PLY {
            self.pv_length[ply] = 0;
            return if pos.in_check() {
                Value::DRAW
            } else {
                self.search_eval(pos)
            };
        }

        self.pv_length[ply] = 0;

        if self.check_should_stop() {
            return Value::ZERO;
        }

        self.nodes += 1;
        let ply_bucket = ply.min(MAX_PLY - 1);
        self.nodes_per_ply[ply_bucket] += 1;
        if ply as u32 > self.seldepth {
            self.seldepth = ply as u32;
        }

        let in_check = pos.in_check();

        let key = pos.key();
        let probe = self.tt.probe(key);
        let tt_hit = probe.hit;
        let tt_move = if tt_hit { probe.data.mv } else { Move::NONE };
        let tt_value = if tt_hit {
            value_from_tt(probe.data.value, ply as i32)
        } else {
            Value::NONE
        };

        if tt_hit && tt_value != Value::NONE {
            let usable = match probe.data.bound {
                Bound::Exact => true,
                Bound::Lower => tt_value >= beta,
                Bound::Upper => tt_value <= alpha,
                Bound::None => false,
            };
            if usable {
                return tt_value;
            }
        }

        // --- Stand-pat / static eval (SF11 search.cpp:1408-1446) ---
        //
        // `raw_stand_pat` is the contempt-free static eval (SF's
        // `ss->staticEval`); we persist it on the stack so a null-move
        // child reached straight from quiescence can take the after-null
        // refinement (Q4). On a TT hit we read the stored eval (falling
        // back to a fresh evaluation if the entry had none); on a miss
        // we either negate the parent's raw eval and add two tempi when
        // the parent played the null move (Q4), or evaluate fresh.
        let parent_was_null = self.stack[STACK_SENTINEL + ply - 1].was_null;
        let raw_stand_pat = if in_check {
            Value::NONE
        } else if tt_hit {
            if probe.data.eval != Value::NONE {
                probe.data.eval
            } else {
                evaluate_with_pawn_cache(pos, self.pawn_cache, self.eval_mask, self.eg_skill)
            }
        } else if parent_was_null {
            let parent_raw = self.stack[STACK_SENTINEL + ply - 1].raw_static_eval;
            Value(-parent_raw.0 + 2 * crate::eval::TEMPO.0)
        } else {
            evaluate_with_pawn_cache(pos, self.pawn_cache, self.eval_mask, self.eg_skill)
        };
        self.stack[STACK_SENTINEL + ply].raw_static_eval = raw_stand_pat;

        let mut stand_pat = self.apply_contempt(raw_stand_pat, pos);

        // Q3 (SF11 search.cpp:1422-1425): on a TT hit, a stored value
        // tighter than the static eval in the bound's direction is a
        // better stand-pat estimate. Mirrors the negamax `eval`
        // refinement (S4). Only out of check, where the stand-pat is a
        // real value.
        if !in_check && tt_hit && tt_value != Value::NONE {
            let use_tt = if tt_value > stand_pat {
                matches!(probe.data.bound, Bound::Lower | Bound::Exact)
            } else {
                matches!(probe.data.bound, Bound::Upper | Bound::Exact)
            };
            if use_tt {
                stand_pat = tt_value;
            }
        }

        let mut best_score;
        if !in_check {
            best_score = stand_pat;
            if best_score >= beta {
                return best_score;
            }
            if best_score > alpha {
                alpha = best_score;
            }
        } else {
            best_score = -Value::INFINITE;
        }

        // Tactical-horizon cap (play-engine weak-bot lever). qsearch enters
        // at `depth == 0` and recurses with `depth - 1`, so `-depth` counts
        // the capture plies resolved so far. Once that reaches the bot's
        // `qsearch_cap`, stop resolving captures and return the stand-pat
        // (static) eval — the bot is "blind" past this horizon. `cap == 0`
        // resolves no captures at all (hangs pieces). Never applied in
        // check: a forced position must still find its evasions.
        // [`QSEARCH_UNBOUNDED`] makes `-depth <= -cap` unreachable, so the
        // full-strength / analytical path is unaffected.
        if !in_check && depth <= -self.qsearch_cap {
            return best_score;
        }

        // SF11 qsearch picker depth (search.cpp:1391): always
        // [`Depth::QS_CHECKS`] when in check (we still want to look at
        // evasions); otherwise the *current* recursion depth, which
        // decreases by 1 each recursive qsearch call. At
        // [`Depth::QS_RECAPTURES`] (= -5) the picker switches to
        // recapture-only mode — only moves landing on the
        // [`recapture_square`] are tried. This is the SF11 mechanism
        // that bounds qsearch chains in capture-rich endgames; without
        // it, long alternating-capture sequences explode the deep ply
        // tail (FEN 19 d=20, FEN 41 d=14).
        let qs_picker_depth = if in_check {
            Depth::QS_CHECKS
        } else {
            Depth(depth)
        };

        // Recapture square = the to-square of the move that brought
        // us to this position (SF11 search.cpp:1459, `to_sq((ss-1)->
        // currentMove)`). The picker only consults it once depth
        // descends to [`Depth::QS_RECAPTURES`]. For null-move parents
        // this reads the sentinel square (A1 in our encoding); the
        // recapture filter at -5 almost never matches captures
        // landing there, so the corner case is benign.
        let parent_to = self.stack[STACK_SENTINEL + ply - 1].to_idx;
        let recapture_square = Some(Square::from_index(parent_to));

        // Cont-hist keys for qsearch: only the 1-ply-ago slot affects
        // evasion ordering (Stockfish's `score<EVASIONS>` reads
        // `continuationHistory[0]`). Pass the full set so the picker
        // logic stays uniform; unused slots are no-ops.
        let cont_keys: ContHistKeys = [
            cont_key_at(&self.stack, ply, 1),
            cont_key_at(&self.stack, ply, 2),
            cont_key_at(&self.stack, ply, 4),
            cont_key_at(&self.stack, ply, 6),
        ];
        let mut picker =
            MovePicker::new_qs(pos, tt_move, qs_picker_depth, recapture_square, cont_keys);
        let mut move_count = 0usize;

        loop {
            let mv = picker.next_move(
                pos,
                Some(self.history),
                Some(self.cont_history),
                Some(self.capture_history),
                false,
            );
            if mv == Move::NONE {
                break;
            }

            if !in_check && !pos.see_ge(mv, Value::ZERO) {
                continue;
            }

            // Perception filter: an exchange you can't see doesn't get
            // resolved — the stand-pat already banked above is exactly
            // the blind read (no never-empty machinery needed here;
            // out of check, "no qsearch moves" just means stand pat).
            // This is the defense half of the lever: the opponent's
            // subtle recapture (knight-move, cross-board) goes unseen
            // inside the bot's own lines, so it walks into it. Never
            // applied in check — evasions are forced.
            if !in_check && self.perception.is_some() && !self.move_is_seen(pos, mv, ply) {
                continue;
            }

            // Capture moved piece before do_move so we can record it in
            // the per-ply stack for descendants' cont-hist lookups.
            let moved_piece = pos.moved_piece(mv);

            let state = pos.do_move(mv);
            self.tt.prefetch(pos.key());
            let us_was = !pos.side_to_move();
            let our_king = pos.king_square(us_was);
            let still_attacked =
                (pos.attackers_to(our_king, pos.occupied()) & pos.pieces_by_color(!us_was)).any();
            if still_attacked {
                pos.undo_move(mv, state);
                continue;
            }

            move_count += 1;
            // Update parent's stack frame before recursing so the
            // child's cont-hist lookups find this move's sub-table.
            let is_capture =
                state.captured.is_some() || mv.kind() == crate::types::MoveKind::EnPassant;
            self.stack[STACK_SENTINEL + ply].moved_piece_idx = moved_piece.index() as u8;
            self.stack[STACK_SENTINEL + ply].to_idx = mv.to().index() as u8;
            self.stack[STACK_SENTINEL + ply].in_check = in_check;
            self.stack[STACK_SENTINEL + ply].was_capture = is_capture;
            self.stack[STACK_SENTINEL + ply].was_null = false;

            self.path_keys.push(pos.key());
            // SF11 search.cpp:1522 — recursive qsearch decrements
            // `depth` by 1. Once `depth <= DEPTH_QS_RECAPTURES (-5)`
            // the picker switches to recapture-only mode, bounding
            // long alternating-capture chains.
            let score = -self.qsearch(pos, -beta, -alpha, ply + 1, depth - 1);
            self.path_keys.pop();
            pos.undo_move(mv, state);

            if self.is_aborted() {
                return Value::ZERO;
            }

            if score > best_score {
                best_score = score;
                if score > alpha {
                    alpha = score;
                    self.update_pv(ply, mv);
                    if score >= beta {
                        return best_score;
                    }
                }
            }
        }

        if in_check && move_count == 0 {
            return Value::mated_in(ply as i32);
        }

        best_score
    }
}
