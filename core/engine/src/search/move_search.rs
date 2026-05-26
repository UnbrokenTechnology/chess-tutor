//! `search_made_move`: search one already-made move at the right
//! depth/window (LMR reduced search, full-depth re-search with its
//! continuation-history feedback, and the PV re-search).

use super::*;
use crate::movepick::ContHistKeys;
use crate::position::{Position, StateInfo};
use crate::types::{Move, PieceType, Square, Value};

impl<'a> Search<'a> {
    /// Search one already-made move at the right depth/window and return
    /// its score (from this node's POV, i.e. the negated child value).
    /// Encapsulates SF11 search.cpp:1117-1217: the LMR reduced
    /// zero-window search, the full-depth zero-window re-search (with the
    /// continuation-history feedback that follows it), and the PV
    /// re-search for PV nodes. `pos` is already past `do_move` for `mv`;
    /// the caller does `undo_move` after this returns.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn search_made_move(
        &mut self,
        pos: &mut Position,
        mv: Move,
        moved_piece: crate::types::Piece,
        child_prev: Option<(PieceType, Square)>,
        state: &StateInfo,
        alpha: Value,
        beta: Value,
        depth: i32,
        new_depth: i32,
        ply: usize,
        move_count: usize,
        best_score: Value,
        is_root: bool,
        is_pv: bool,
        cut_node: bool,
        in_check: bool,
        gives_check: bool,
        is_cap_or_promo: bool,
        move_count_pruning: bool,
        improving: bool,
        static_eval: Value,
        tt_move: Move,
        tt_pv: bool,
        cont_keys: ContHistKeys,
    ) -> Value {
        let mut score: Value;
        let mut full_depth = true;
        let did_lmr;

        // --- LMR: zero-window reduced-depth search on late moves ---
        // SF11 search.cpp:1117-1217. Faithful reduction plus the full
        // relaxer/adjuster stack. The move-count gate matches SF: at a
        // non-root node LMR begins on the 2nd move (`move_count > 1`),
        // and at the root it begins later (`> 2`, or `> 3` once a move
        // has already failed to beat alpha). Captures and promotions
        // are eligible too, under SF's 4-condition gate below.
        //
        // (Root divergence: SF additionally guards root LMR with
        // `best_move_count(move) == 0` to protect a previously-best
        // root move from reduction. We omit that guard — the root gate
        // already excludes the first 2-3 moves and the full-depth
        // re-search recovers any fail-high, so the effect is negligible
        // and root nodes are a tiny fraction of the tree.)
        let lmr_move_gate =
            1 + (is_root as usize) + ((is_root && best_score < alpha) as usize);
        // SF11 search.cpp:1120-1124 — captures/promotions are eligible
        // for LMR only when one of these holds; otherwise they are
        // searched at full depth. (`captured_eg` is the EG value of the
        // captured piece, 0 for a non-capturing promotion.)
        let captured_eg = state
            .captured
            .map(|p| Value::eg_of_piece(p.kind()).0)
            .unwrap_or(0);
        let lmr_eligible = !is_cap_or_promo
            || move_count_pruning
            || static_eval.0 + captured_eg <= alpha.0
            || cut_node
            || self.tt_hit_average
                < 375 * TT_HIT_AVERAGE_RESOLUTION * TT_HIT_AVERAGE_WINDOW / 1024;
        if depth >= LMR_MIN_DEPTH
            && move_count > lmr_move_gate
            && lmr_eligible
            && !in_check
            && !gives_check
        {
            let mut r = lmr_reduction(depth, move_count, improving);

            // SF11 search.cpp:1129 — decrease reduction when the
            // running TT-hit average is high (transposition-rich
            // region; the ordering is trustworthy, reduce less).
            if self.tt_hit_average
                > 500 * TT_HIT_AVERAGE_RESOLUTION * TT_HIT_AVERAGE_WINDOW / 1024
            {
                r -= 1;
            }

            // (SF11 search.cpp:1133 breadcrumb `r++` is multi-thread
            // only; single-threaded it never fires, so it is omitted.)

            // SF11 search.cpp:1137 — decrease reduction for nodes that
            // are, or have been, on the PV.
            if tt_pv {
                r -= 2;
            }

            // SF11 search.cpp:1141 — decrease reduction when the
            // opponent's previous move count was high.
            if self.stack[STACK_SENTINEL + ply - 1].move_count > 14 {
                r -= 1;
            }

            // (SF11 search.cpp:1145 `singularLMR → r -= 2` omitted: no
            // singular extensions yet, so the flag is always false.)

            // SF11 search.cpp:1148-1191 — the ttCapture/cutNode/escape/
            // statScore adjusters apply to quiet moves only; captures
            // and promotions instead get a flat late-move bump.
            if !is_cap_or_promo {
                // SF11 search.cpp:1151 — increase reduction if the
                // ttMove is a capture/promotion (a tactical alternative
                // exists; reduce this quiet more).
                let tt_capture = tt_move != Move::NONE
                    && (pos.is_capture(tt_move)
                        || tt_move.kind() == crate::types::MoveKind::Promotion);
                if tt_capture {
                    r += 1;
                }

                if cut_node {
                    // SF11 search.cpp:1155 — increase reduction at cut
                    // nodes; the parent expects a fail-high.
                    r += 2;
                } else if mv.kind() == crate::types::MoveKind::Normal
                    && !pos.see_ge(Move::normal(mv.to(), mv.from()), Value::ZERO)
                {
                    // SF11 search.cpp:1161 — decrease reduction for
                    // moves that escape a capture: a (now-quiet) reverse
                    // move from `to` back to `from` losing material
                    // means the from-square was unsafe for this piece,
                    // so moving away was useful. Castling is excluded by
                    // the NORMAL gate.
                    r -= 2;
                }

                // SF11 statScore: blend main + cont-history into a
                // single quality estimate for the move we're about to
                // search, compare against the parent's statScore to
                // nudge `r` in {-1, 0, +1}, then gravity-scale by
                // `statScore / 16384`.
                let us = !pos.side_to_move(); // side-to-move *before* do_move
                let mvp_idx = moved_piece.index() as u8;
                let mvt_idx = mv.to().index() as u8;
                let main_h = self.history.get(us, mv.from(), mv.to()) as i32;
                // SF11 reads contHist[0], [1], [3] = 1-, 2-, 4-plies-ago.
                // Our `cont_keys` packs those at indices [0], [1], [2].
                let ch0 = self
                    .cont_history
                    .sub_for_key(cont_keys[0])[mvp_idx as usize][mvt_idx as usize]
                    as i32;
                let ch1 = self
                    .cont_history
                    .sub_for_key(cont_keys[1])[mvp_idx as usize][mvt_idx as usize]
                    as i32;
                let ch3 = self
                    .cont_history
                    .sub_for_key(cont_keys[2])[mvp_idx as usize][mvt_idx as usize]
                    as i32;

                let mut stat_score = main_h + ch0 + ch1 + ch3 - 4926;
                // The flat `-4926` offset can pull an "all-good"
                // move slightly negative; clip those false
                // negatives so the gravity scaling doesn't reduce
                // a move whose sub-components all say "fine".
                if stat_score < 0 && ch0 >= 0 && ch1 >= 0 && main_h >= 0 {
                    stat_score = 0;
                }
                self.stack[STACK_SENTINEL + ply].stat_score = stat_score;

                let parent_stat = self.stack[STACK_SENTINEL + ply - 1].stat_score;
                if stat_score >= -102 && parent_stat < -114 {
                    r -= 1;
                } else if parent_stat >= -116 && stat_score < -154 {
                    r += 1;
                }
                r -= stat_score / 16384;
            } else if depth < 8 && move_count > 2 {
                // SF11 search.cpp:1190 — increase reduction for late
                // captures/promotions at low depth.
                r += 1;
            }

            // SF clamps the resulting depth to `[1, new_depth]`
            // — a negative `r` would otherwise extend, which LMR
            // is not supposed to do.
            let reduced = (new_depth - r).clamp(1, new_depth);

            let reduced_score = -self.negamax(
                pos,
                Value(-alpha.0 - 1),
                -alpha,
                reduced,
                ply + 1,
                false,
                false,
                child_prev,
                // LMR's reduced search treats the child as a cut
                // node (we expect it to fail low quickly).
                true,
            );
            // SF11 search.cpp:1197 — `doFullDepthSearch = (value >
            // alpha && d != newDepth)`. The `d != newDepth` guard
            // matters: when the relaxers drive `r <= 0` the clamp gives
            // `reduced == new_depth`, so the reduced search WAS a
            // full-depth search; re-running it would be pure waste. In
            // that case the reduced value is the move's value.
            full_depth = reduced_score > alpha && reduced != new_depth;
            if !full_depth {
                score = reduced_score;
            } else {
                score = Value::NONE;
            }
            did_lmr = true;
        } else {
            score = Value::NONE;
            did_lmr = false;
        }

        if full_depth && !(is_pv && move_count == 1) {
            score = -self.negamax(
                pos,
                Value(-alpha.0 - 1),
                -alpha,
                new_depth,
                ply + 1,
                false,
                false,
                child_prev,
                !cut_node,
            );

            // SF11 search.cpp:1207-1216 — after an LMR move is
            // re-searched at full depth, feed the result back into
            // continuation history: positive bonus if it beat alpha,
            // negative otherwise, with a +¼ kicker for the first
            // killer. Quiet moves only (`!captureOrPromotion`).
            if did_lmr && !is_cap_or_promo {
                let mut bonus = if score > alpha {
                    stat_bonus(new_depth)
                } else {
                    -stat_bonus(new_depth)
                };
                if mv == self.killers[ply][0] {
                    bonus += bonus / 4;
                }
                update_cont_histories(
                    self.cont_history,
                    &cont_keys,
                    moved_piece.index() as u8,
                    mv.to().index() as u8,
                    bonus,
                );
            }
        }

        if is_pv && (move_count == 1 || (score > alpha && (is_root || score < beta))) {
            score = -self.negamax(
                pos,
                -beta,
                -alpha,
                new_depth,
                ply + 1,
                false,
                true,
                child_prev,
                // PV-search children are themselves PV (so never
                // cut nodes).
                false,
            );
        }

        score
    }
}
