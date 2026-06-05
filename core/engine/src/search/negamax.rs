//! [`Search::negamax`]: the alpha-beta node. Entry guards, TT probe,
//! static eval, razoring/RFP, calls into the null-move and ProbCut
//! pre-loop phases (`pre_loop`) and the move loop (`move_loop`), then the
//! terminal / TT-save tail.

use super::*;
use crate::eval::evaluate_with_pawn_cache;
use crate::movepick::ContHistKeys;
use crate::position::Position;
use crate::types::{Bound, Depth, Move, PieceType, Square, Value};

impl<'a> Search<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn negamax(
        &mut self,
        pos: &mut Position,
        mut alpha: Value,
        mut beta: Value,
        depth: i32,
        ply: usize,
        is_root: bool,
        is_pv: bool,
        prev: Option<(PieceType, Square)>,
        cut_node: bool,
    ) -> Value {
        debug_assert!(!(is_pv && cut_node), "PvNode && cutNode is an SF invariant");
        // Hard cap on recursion depth. Check extensions don't decrement
        // depth, so a position rich in forcing checks (e.g. infiltrated
        // queen + knight + rook) can drive `ply` past normal bounds.
        // Without this bail, indexing `pv_length[ply]` and the child's
        // reciprocal read `pv_length[ply + 1]` go out of bounds.
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

        if depth <= 0 {
            return self.qsearch(pos, alpha, beta, ply, depth);
        }

        self.nodes += 1;
        let ply_bucket = ply.min(MAX_PLY - 1);
        self.nodes_per_ply[ply_bucket] += 1;
        if ply as u32 > self.seldepth {
            self.seldepth = ply as u32;
        }

        if !is_root {
            if pos.halfmove_clock() >= 100 || self.is_repetition(pos) {
                return self.draw_value(depth, pos);
            }

            // Mate distance pruning.
            alpha = alpha.max(Value::mated_in(ply as i32));
            beta = beta.min(Value::mate_in(ply as i32 + 1));
            if alpha >= beta {
                return alpha;
            }
        }

        // --- TT probe ---
        let key = pos.key();
        let probe = self.tt.probe(key);
        let tt_hit = probe.hit;
        // SF11 search.cpp:699-700 — running exponential TT-hit average,
        // read by the LMR relaxer (`r--` when hits are common) and the
        // capture-LMR enable gate (`< 375·…` when hits are rare).
        self.tt_hit_average = (TT_HIT_AVERAGE_WINDOW - 1) * self.tt_hit_average
            / TT_HIT_AVERAGE_WINDOW
            + TT_HIT_AVERAGE_RESOLUTION * tt_hit as i64;
        let tt_move = if tt_hit { probe.data.mv } else { Move::NONE };
        let tt_value = if tt_hit {
            value_from_tt(probe.data.value, ply as i32)
        } else {
            Value::NONE
        };
        let tt_depth = if tt_hit { probe.data.depth.0 } else { -999 };
        let tt_bound = if tt_hit {
            probe.data.bound
        } else {
            Bound::None
        };

        // Sticky PV flag (SF11 search.cpp:697). Once a position has
        // been touched on the PV at any point in this or a prior
        // iteration, it stays flagged in the TT — even when re-visited
        // through a non-PV path later. Sticky semantics: union the
        // call's PvNode-ness with whatever the TT already remembers.
        //
        // Currently consumed only by the `probe.save` site below, which
        // writes the union back so the flag survives across iterative-
        // deepening rounds. SF11's other consumer is the LMR reduction
        // at search.cpp:1137 (`r -= 2`); see the LMR block below for
        // why we don't fire that yet.
        let tt_pv = is_pv || (tt_hit && probe.data.is_pv);

        if !is_pv && tt_hit && tt_depth >= depth && tt_value != Value::NONE {
            let usable = match tt_bound {
                Bound::Exact => true,
                Bound::Lower => tt_value >= beta,
                Bound::Upper => tt_value <= alpha,
                Bound::None => false,
            };
            if usable {
                return tt_value;
            }
        }

        let in_check = pos.in_check();

        // --- Initialize the grandchild frame's `stat_score` to zero ---
        //
        // SF11 search.cpp:683-686. statScore is "shared between
        // grandchildren" — siblings at ply+2 inherit whichever
        // statScore the previous sibling wrote during its LMR loop.
        // Only the *first* grandchild starts fresh, which is what this
        // reset guarantees. At root we look two layers further out
        // (`ss+4`) so the same property holds for the deeper subtree.
        let reset_offset = if is_root { 4 } else { 2 };
        self.stack[STACK_SENTINEL + ply + reset_offset].stat_score = 0;

        // --- Zero this ply's `move_count` ---
        //
        // SF11 search.cpp:638. Children read `(ss-1)->moveCount` to
        // widen their CMP gate when we're on our first quiet (= the
        // TT move). Until we get into the move loop and increment,
        // it must read as 0 (no quiet iterated yet) so the CMP
        // widening does *not* fire on a stale sibling-search value.
        self.stack[STACK_SENTINEL + ply].move_count = 0;

        // Did the parent reach this node via a null move? Governs both
        // the after-null eval refinement below and the NMP "don't null
        // twice" gate (SF11 `(ss-1)->currentMove == MOVE_NULL`).
        let parent_was_null = self.stack[STACK_SENTINEL + ply - 1].was_null;

        // --- Static eval (SF11 search.cpp:808-820, Steps 6/C2) ---
        //
        // `raw_static_eval` is the evaluator's untinted output — what
        // we persist to the TT so that later searches (possibly with
        // a different root side-to-move) can re-apply the right
        // contempt sign. `static_eval` is the contempt-adjusted form
        // used for this search's pruning decisions.
        //
        // On a TT miss (no stored eval) and out of check, SF refines
        // the fresh static eval two ways, both kept in *raw*
        // (contempt-free) space so the persisted value stays clean:
        //   - parent played a real move: bias by `-(parent statScore)
        //     / 512` — a strongly-scored parent quiet suggests the eval
        //     is about to climb, so nudge it (SF11 search.cpp:812-814).
        //   - parent played the null move: don't re-evaluate; negate the
        //     parent's raw static eval and add two tempi (search.cpp:817).
        let raw_static_eval = if in_check {
            Value::NONE
        } else if tt_hit && probe.data.eval != Value::NONE {
            probe.data.eval
        } else if parent_was_null {
            let parent_raw = self.stack[STACK_SENTINEL + ply - 1].raw_static_eval;
            Value(-parent_raw.0 + 2 * crate::eval::TEMPO.0)
        } else {
            let parent_stat = self.stack[STACK_SENTINEL + ply - 1].stat_score;
            let bonus = -parent_stat / 512;
            Value(
                evaluate_with_pawn_cache(pos, self.pawn_cache, self.eval_mask, self.eg_skill).0
                    + bonus,
            )
        };
        let static_eval = self.apply_contempt(raw_static_eval, pos);

        // Persist this ply's static eval for the `improving` lookup
        // performed by descendants — they read `stack[ply-2]` and
        // `stack[ply-4]`. The raw form feeds a null-move child's
        // after-null refinement above (and qsearch's Q4).
        self.stack[STACK_SENTINEL + ply].static_eval = static_eval;
        self.stack[STACK_SENTINEL + ply].raw_static_eval = raw_static_eval;

        // SF11 search.cpp:804-806 — the refined `eval`. `static_eval` is
        // the raw (contempt-adjusted) evaluation that gets persisted and
        // drives `improving`; `eval` is the value the early-pruning gates
        // (RFP, NMP, razoring) actually test. When the TT holds a tighter
        // estimate in the bound's direction, it replaces the static eval:
        // a LOWER/EXACT entry whose value exceeds the static eval, or an
        // UPPER/EXACT entry whose value is below it. In check both are
        // NONE and this is a no-op.
        let mut eval = static_eval;
        if static_eval != Value::NONE && tt_hit && tt_value != Value::NONE {
            let use_tt = if tt_value > eval {
                matches!(tt_bound, Bound::Lower | Bound::Exact)
            } else {
                matches!(tt_bound, Bound::Upper | Bound::Exact)
            };
            if use_tt {
                eval = tt_value;
            }
        }

        // --- Step 7. Razoring (SF11 search.cpp:822-826, ~1 Elo) ---
        // At `depth < 2`, if even the refined eval is a full
        // `RAZOR_MARGIN` below alpha, this node almost surely can't
        // raise alpha — skip the move loop and settle it in quiescence.
        // `!is_root` mirrors SF's `!rootNode` (the root needs PV
        // handling qsearch can't provide). `eval == NONE` (in check)
        // is a no-op since `NONE` is a large sentinel, but we gate it
        // explicitly for clarity.
        if !is_root && depth < 2 && eval != Value::NONE && eval.0 <= alpha.0 - RAZOR_MARGIN {
            return self.qsearch(pos, alpha, beta, ply, 0);
        }

        // `improving`: true when this ply's static eval is trending up
        // versus the same side-to-move's eval two plies ago. Mirrors
        // Stockfish 11 (search.cpp:828): if the 2-back frame was in
        // check (NONE), fall back to comparing against 4 back; if both
        // are NONE (early in the search), default to `true`. In check,
        // the concept doesn't apply — set false.
        let s_back2 = self.stack[STACK_SENTINEL + ply - 2].static_eval;
        let s_back4 = self.stack[STACK_SENTINEL + ply - 4].static_eval;
        let improving = if in_check {
            false
        } else if s_back2 == Value::NONE {
            s_back4 == Value::NONE || static_eval.0 >= s_back4.0
        } else {
            static_eval.0 >= s_back2.0
        };

        // --- Reverse futility pruning (SF11 "child-node futility") ---
        // Stockfish 11 search.cpp:831-836 (claimed ~50 Elo). If our
        // static eval is already so far above beta that a
        // `futility_margin(depth)` worth of plausible swing wouldn't
        // bring it below, return early and skip the entire subtree.
        // The static eval is fully computed (unlike lazy eval, which
        // returned an *approximate* eval and was rolled back for
        // changing best-move decisions); the heuristic is purely
        // "subtree below this honest eval probably can't matter".
        //
        // `!is_pv` covers root (root is always called with is_pv=true).
        // `static_eval < KNOWN_WIN` mirrors SF's "do not return
        // unproven wins" — beyond +10000 cp we're claiming a forced
        // win that the search hasn't actually verified.
        // SF11 search.cpp:831-836 tests the refined `eval` and returns it.
        if !is_pv
            && !in_check
            && depth < 6
            && eval != Value::NONE
            && eval.0 < Value::KNOWN_WIN.0
            && eval.0 - futility_margin(depth, improving) >= beta.0
        {
            return eval;
        }

        // Null-move pruning with verification — see [`try_null_move`].
        if let Some(v) = self.try_null_move(
            pos,
            beta,
            depth,
            ply,
            in_check,
            is_pv,
            cut_node,
            prev,
            eval,
            static_eval,
            improving,
            parent_was_null,
        ) {
            return v;
        }

        // Build the four cont-hist keys identifying the parent
        // sub-tables at offsets 1, 2, 4, 6 plies ago. Hoisted above
        // ProbCut because ProbCut's child negamax recursion reads
        // them via the per-ply stack writes below.
        let cont_keys: ContHistKeys = [
            cont_key_at(&self.stack, ply, 1),
            cont_key_at(&self.stack, ply, 2),
            cont_key_at(&self.stack, ply, 4),
            cont_key_at(&self.stack, ply, 6),
        ];

        // ProbCut pre-loop pruning phase — see [`try_probcut`].
        if let Some(v) = self.try_probcut(
            pos,
            beta,
            depth,
            ply,
            in_check,
            is_pv,
            cut_node,
            static_eval,
            improving,
            tt_move,
        ) {
            return v;
        }

        // Iterate the legal moves with the full SF11 ordering/pruning
        // stack — see [`negamax_moves`].
        let (best_score, best_move, raised_alpha, move_count) = match self.negamax_moves(
            pos,
            alpha,
            beta,
            depth,
            ply,
            is_root,
            is_pv,
            cut_node,
            prev,
            in_check,
            static_eval,
            tt_move,
            tt_pv,
            improving,
            cont_keys,
        ) {
            MovesOutcome::Aborted => return Value::ZERO,
            MovesOutcome::Done {
                best_score,
                best_move,
                raised_alpha,
                move_count,
            } => (best_score, best_move, raised_alpha, move_count),
        };

        if move_count == 0 {
            return if in_check {
                Value::mated_in(ply as i32)
            } else {
                Value::DRAW
            };
        }

        // Skip TT save at the root for secondary PV slots — otherwise
        // the search for pv_idx > 0 would clobber the root's best-move
        // entry with a deliberately-second-best pick, polluting future
        // probes. The primary slot still writes normally.
        let skip_tt_save = is_root && self.pv_idx > 0;

        let bound = if best_score >= beta {
            Bound::Lower
        } else if is_pv && raised_alpha {
            Bound::Exact
        } else {
            Bound::Upper
        };
        if skip_tt_save {
            return best_score;
        }
        probe.save(
            key,
            value_to_tt(best_score, ply as i32),
            tt_pv,
            bound,
            Depth(depth),
            best_move,
            raw_static_eval,
        );

        best_score
    }
}
