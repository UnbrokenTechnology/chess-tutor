//! Move-loop helpers: the `cmp_cont_negative` / `quiet_futility_inner`
//! inner Step-13 tests and `update_all_stats`, plus the standalone
//! reduction / pruning / extension / history-update functions and SF11's
//! reduction table.

use super::*;
use crate::movepick::{
    cont_history_update, ContHistKeys, ContHistStore, BUTTERFLY_HISTORY_BOUND, CAPTURE_HISTORY_BOUND, CONT_HISTORY_BOUND,
};
use crate::position::Position;
use crate::types::{Color, Move, PieceType, Square, Value};

/// Stockfish 11's `stat_bonus` (search.cpp:86): the depth-dependent
/// bonus applied on β-cutoff to history / continuation-history
/// counters for the cutting move (and to losers tried before it,
/// negated). Bound at ±[`CONT_HISTORY_BOUND`] which the table-update
/// gravity-formula tolerates.
pub(crate) fn stat_bonus(depth: i32) -> i32 {
    if depth > 15 {
        -8
    } else {
        let raw = 19 * depth * depth + 155 * depth - 132;
        raw.clamp(-CONT_HISTORY_BOUND, CONT_HISTORY_BOUND)
    }
}

/// SF11's `Reductions[]` table (`search.cpp` `Search::init`):
/// `Reductions[i] = int((24.8 + ln(threads)/2) * ln(i))` for `i >= 1`
/// (SF11 search.cpp:197). Single-threaded → the `ln(threads)/2` term is
/// `ln(1)/2 = 0`, so the coefficient is exactly **24.8**. (A prior port
/// used `23.4`, which appears nowhere in SF11 and systematically
/// under-reduced; corrected 2026-05-26 as part of the faithful LMR
/// bundle.) Initialised lazily on first access (one-time `ln()` cost).
/// Sized for `MAX_MOVES = 256` (SF11's constant); index 0 stays `0`
/// per SF (default-initialised).
pub(crate) static SF11_REDUCTIONS: std::sync::LazyLock<[i32; 256]> = std::sync::LazyLock::new(|| {
    let mut arr = [0i32; 256];
    for (i, slot) in arr.iter_mut().enumerate().skip(1) {
        *slot = (24.8 * (i as f64).ln()) as i32;
    }
    arr
});

/// SF11's late-move reduction (`search.cpp` `Search::reduction`):
/// `r = Reductions[d] * Reductions[mn]; (r + 511) / 1024 + (!i && r > 1007)`.
/// Used both by actual LMR application and by Lever 2b's `lmrDepth`
/// gate, so the two stay consistent. Replaced our earlier
/// `log₂·log₂/2` formula on 2026-05-14 after the divergence caused
/// FEN 19 to regress 290× under verbatim SF11 thresholds (see HANDOFF
/// "Why FEN 19 ran away under raw Lever 2b").
pub(crate) fn lmr_reduction(depth: i32, move_count: usize, improving: bool) -> i32 {
    let d = depth.clamp(0, (SF11_REDUCTIONS.len() - 1) as i32) as usize;
    let mc = move_count.min(SF11_REDUCTIONS.len() - 1);
    let r = SF11_REDUCTIONS[d] * SF11_REDUCTIONS[mc];
    (r + 511) / 1024 + (!improving && r > 1007) as i32
}

pub(crate) fn late_move_prune(depth: i32, move_count: usize, improving: bool) -> bool {
    // Stockfish's `futility_move_count(improving, depth) =
    // (5 + d^2) * (1 + improving) / 2 - 1` — when improving, the
    // count threshold is roughly doubled, so fewer moves get pruned.
    let base = (5 + depth * depth) as usize;
    let threshold = base * (1 + improving as usize) / 2 - 1;
    move_count > threshold
}

/// Stockfish 11's `futility_margin` (search.cpp:69-71). Margin shrinks
/// by one depth-step's worth (217 cp) when the static eval is
/// improving, letting reverse-futility pruning take a slightly tighter
/// bet. Per-move forward futility is now lmrDepth-based (SF11
/// search.cpp:1016-1024) and uses its own margin (`235 + 172 *
/// lmrDepth`) inline; this function is only the reverse-futility
/// (parent-level) check ("we're already past beta, skip the subtree").
pub(crate) fn futility_margin(depth: i32, improving: bool) -> i32 {
    217 * (depth - improving as i32)
}

/// SF11 extension chain (search.cpp:1072-1090). Returns the `+ply`
/// extension this move earns at the current node.
///
/// Four predicates, mutually-exclusive `else if` for the
/// first three; castling is a separate `if` at the bottom
/// that overrides any prior result. Each fires `+1 ply`.
/// Singular extensions belong here too in SF's full
/// structure but aren't ported yet (see HANDOFF).
///
/// CHECK EXTENSION: previously blanket — every check got
/// +1 ply. SF's gate is tighter: only extend when the
/// check is either a discovery (moving piece was a
/// blocker for the enemy king, so its departure unblocks
/// a slider check) OR has SEE >= 0 (the checking piece
/// won't simply lose material to a recapture). The
/// filtered-out moves are SEE-negative sac-checks that
/// were noise.
///
/// ISOLATED-ADDITION CAVEAT: A/B isolation (same session)
/// showed each of the other three extensions (passed-pawn,
/// last-captures, castling) is net-negative in isolation
/// on top of check-gating, sometimes catastrophically so
/// (last-captures alone on pawn-race endgames runs >20 min
/// because every capture drops NPM below 2*ROOK_MG and
/// extends the whole subtree). But all four *together*
/// are net-positive at depth 13 (9× vs blanket-check
/// baseline) and depth 14 (6×). The interaction matters:
/// when only one extension fires per node, its over-
/// extension on pathological positions isn't crowded out
/// by competing extensions firing elsewhere. Don't try to
/// simplify by removing one — the per-extension results
/// are misleading.
#[allow(clippy::too_many_arguments)]
pub(crate) fn compute_extension(
    pos: &Position,
    mv: Move,
    us_at_node: Color,
    gives_check: bool,
    from_was_enemy_blocker: bool,
    see_nonneg: bool,
    is_first_killer: bool,
    is_advanced_pawn_push: bool,
    last_captures_node_eligible: bool,
) -> i32 {
    let mut extension: i32 = 0;
    if gives_check && (from_was_enemy_blocker || see_nonneg) {
        extension = 1;
    } else if is_first_killer
        && is_advanced_pawn_push
        && pos.pawn_passed(us_at_node, mv.to())
    {
        // Passed-pawn extension: ply's killer is an advanced
        // passed-pawn push. Killers are the ply-stable
        // refutation moves; if the move that already worked
        // is itself a race-changing pawn push, +1 ply is
        // worth confirming the race.
        extension = 1;
    } else if last_captures_node_eligible {
        // Last-captures: parent's move was a heavy capture
        // and we're in thin material (≤ 2 rooks). Every move
        // at this node gets +1 to find concrete endgame
        // technique. Node-level (computed once outside the
        // loop), not move-level.
        extension = 1;
    }
    // Castling override: SF's bottom-of-chain `if` (line 1089)
    // — castling is a one-shot structural move that re-shapes
    // king safety; an extra ply of verification is worth it.
    if mv.kind() == crate::types::MoveKind::Castling {
        extension = 1;
    }
    extension
}

/// Stockfish 11's `update_continuation_histories`: bumps the
/// `[piece][to]` slot of each parent table referenced by `keys` by
/// `bonus`. Parents for which no real move was played (sentinel slot 0)
/// are skipped — their tables are reserved for "no move" and stay at
/// zero so cont-hist scoring of those plies returns 0.
pub(crate) fn update_cont_histories(
    cont: &mut ContHistStore,
    keys: &ContHistKeys,
    moved_piece_idx: u8,
    to_idx: u8,
    bonus: i32,
) {
    debug_assert!(bonus.abs() <= CONT_HISTORY_BOUND);
    for &(ic, wc, p, t) in keys {
        if p == 0 {
            // Sentinel: no real parent move at this ply offset.
            continue;
        }
        let sub = cont.tables[ic as usize][wc as usize].sub_mut(p as usize, t as usize);
        cont_history_update(
            &mut sub[moved_piece_idx as usize][to_idx as usize],
            bonus,
        );
    }
}

pub(crate) fn history_bonus(depth: i32) -> i32 {
    let raw = depth * depth + 2 * depth - 2;
    raw.clamp(0, BUTTERFLY_HISTORY_BOUND)
}

/// CMP-only continuation-history read. Stockfish 11 fills the
/// `NO_PIECE` sentinel row of every contHistory table with -1
/// (`CounterMovePruneThreshold - 1`) so the CMP gate fires
/// uniformly at frames whose parent was a null move or the
/// pre-search padding. We keep the sentinel row at zero in our
/// tables (every other cont-hist read site treats sentinel as "no
/// signal, contribute 0"); the override here is local to CMP so
/// other read sites — move ordering, statScore, etc. — are
/// unaffected.
pub(crate) fn cmp_cont_hist_read(
    store: &crate::movepick::ContHistStore,
    key: (bool, bool, u8, u8),
    moved_piece_idx: usize,
    to_idx: usize,
) -> i32 {
    if key.2 == 0 {
        -1
    } else {
        store.sub_for_key(key)[moved_piece_idx][to_idx] as i32
    }
}

impl<'a> Search<'a> {
    /// CMP inner test (SF11 search.cpp:1008-1014): the move's 1- and
    /// 2-plies-ago continuation-history scores are both negative at a
    /// shallow `lmrDepth`. The outer Step-13 gate stays at the call
    /// site; this is the trailing `&&` operand, so it runs only when
    /// the gate already passed. See [`cmp_cont_hist_read`] for the
    /// sentinel handling.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn cmp_cont_negative(
        &self,
        depth: i32,
        move_count: usize,
        improving: bool,
        ply: usize,
        moved_piece: crate::types::Piece,
        mv: Move,
        cont_keys: ContHistKeys,
    ) -> bool {
        let lmr_r = lmr_reduction(depth, move_count, improving);
        // SF11 search.cpp:1008 — `lmrDepth = max(newDepth -
        // reduction, 0)`, where `newDepth = depth - 1` at
        // this point (extensions are computed *after*
        // pruning step 13). Using `depth - 1` directly
        // keeps the gate independent of whichever
        // extension we later assign.
        let lmr_d = ((depth - 1) - lmr_r).max(0);
        let parent_stat = self.stack[STACK_SENTINEL + ply - 1].stat_score;
        let parent_mc = self.stack[STACK_SENTINEL + ply - 1].move_count;
        let widen = (parent_stat > 0 || parent_mc == 1) as i32;
        if lmr_d >= 4 + widen {
            false
        } else {
            let mvp = moved_piece.index();
            let mvt = mv.to().index();
            let ch0 = cmp_cont_hist_read(self.cont_history, cont_keys[0], mvp, mvt);
            let ch1 = cmp_cont_hist_read(self.cont_history, cont_keys[1], mvp, mvt);
            ch0 < 0 && ch1 < 0
        }
    }

    /// Quiet futility inner test (SF11 search.cpp:1016-1024, "Lever 2b").
    /// The outer Step-13 gate stays at the call site; this computes the
    /// `lmrDepth`-based margin check plus the negative-composite-history
    /// gate. `stm` is reproduced from `us_at_node` to match the post-
    /// `do_move` side-to-move read without making the move.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn quiet_futility_inner(
        &self,
        depth: i32,
        move_count: usize,
        improving: bool,
        static_eval: Value,
        alpha: Value,
        us_at_node: Color,
        moved_piece: crate::types::Piece,
        mv: Move,
        cont_keys: ContHistKeys,
    ) -> bool {
        let lmr_r = lmr_reduction(depth, move_count, improving);
        // newDepth = depth - 1 here; extensions are computed
        // *after* Step 13 in SF11, so the gate is keyed on
        // pre-extension depth (search.cpp:994, 1008).
        let lmr_d = ((depth - 1) - lmr_r).max(0);
        if lmr_d >= 6 || static_eval.0 + 235 + 172 * lmr_d > alpha.0 {
            false
        } else {
            // Post-`do_move` this read was `pos.side_to_move()`
            // (the opponent); pre-move we reproduce that exact
            // value as `!us_at_node` to stay node-neutral.
            let stm = !us_at_node;
            let mvp_idx = moved_piece.index();
            let mvt_idx = mv.to().index();
            let main_h = self.history.get(stm, mv.from(), mv.to()) as i32;
            let ch0 = self
                .cont_history
                .sub_for_key(cont_keys[0])[mvp_idx][mvt_idx]
                as i32;
            let ch1 = self
                .cont_history
                .sub_for_key(cont_keys[1])[mvp_idx][mvt_idx]
                as i32;
            let ch3 = self
                .cont_history
                .sub_for_key(cont_keys[2])[mvp_idx][mvt_idx]
                as i32;
            (main_h + ch0 + ch1 + ch3) < 25000
        }
    }

    /// Stockfish's `update_all_stats` (search.cpp:1255-1301), run on a
    /// β-cutoff. Bumps the cutoff move's history (killer/counter-move +
    /// butterfly + continuation for quiets, capture-history for
    /// captures), decrements every quiet tried before it, decrements
    /// every losing capture tried (regardless of the cutoff move's
    /// kind), and zeroes this ply's `statScore` (search.cpp:1288). The
    /// `if score >= beta` gate and the `break` stay at the call site.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn update_all_stats(
        &mut self,
        pos: &Position,
        mv: Move,
        moved_piece: crate::types::Piece,
        is_capture: bool,
        depth: i32,
        ply: usize,
        prev: Option<(PieceType, Square)>,
        quiets_tried: &crate::movegen::MoveList,
        captures_tried: &crate::movegen::MoveList,
        cont_keys: ContHistKeys,
    ) {
        // Stockfish's `update_all_stats`:
        //   bonus1 = stat_bonus(depth + 1) — used for
        //     the cutoff capture's bump and the
        //     decrement of every losing capture tried.
        //   For quiets we keep our existing
        //     `history_bonus`/`stat_bonus(depth)` mix.
        let bonus1 = stat_bonus(depth + 1).clamp(
            -CAPTURE_HISTORY_BOUND,
            CAPTURE_HISTORY_BOUND,
        );

        if !is_capture {
            if self.killers[ply][0] != mv {
                self.killers[ply][1] = self.killers[ply][0];
                self.killers[ply][0] = mv;
            }
            if let Some((pt, sq)) = prev {
                self.counter_moves.set(pt, sq, mv);
            }
            let bonus = history_bonus(depth);
            let us = pos.side_to_move();
            self.history.update(us, mv.from(), mv.to(), bonus);
            for q in quiets_tried {
                self.history.update(us, q.from(), q.to(), -bonus);
            }

            // Continuation history: bump our move's
            // slot in each parent table at offsets
            // {1, 2, 4, 6} ply ago, and decrement the
            // same slot for every quiet tried before
            // the cutoff. Mirrors Stockfish's
            // `update_continuation_histories(...)`
            // applied via `update_quiet_stats`.
            let cont_bonus = stat_bonus(depth);
            let mv_piece_idx = moved_piece.index() as u8;
            let mv_to_idx = mv.to().index() as u8;
            update_cont_histories(
                self.cont_history,
                &cont_keys,
                mv_piece_idx,
                mv_to_idx,
                cont_bonus,
            );
            for q in quiets_tried {
                let q_piece = pos.moved_piece(*q);
                update_cont_histories(
                    self.cont_history,
                    &cont_keys,
                    q_piece.index() as u8,
                    q.to().index() as u8,
                    -cont_bonus,
                );
            }
        } else {
            // Cutoff move was a capture: bump its
            // capture-history slot. `pos.piece_on(to)`
            // reads the captured piece because the
            // search has already undone the move; for
            // en passant the to-square is empty so the
            // captured-pt slot collapses to 0 — matches
            // Stockfish's `piece_on(to_sq(bestMove))`.
            let captured_pt = pos
                .piece_on(mv.to())
                .map(|p| p.kind().index() as u8)
                .unwrap_or(0);
            self.capture_history.update(
                moved_piece.index() as u8,
                mv.to().index() as u8,
                captured_pt,
                bonus1,
            );
        }

        // Decrement every losing capture's slot,
        // regardless of whether the cutoff move was a
        // capture or a quiet. Mirrors Stockfish's
        // unconditional capture-loser decrement.
        for cap in captures_tried {
            let cap_piece = pos.moved_piece(*cap);
            let cap_captured_pt = pos
                .piece_on(cap.to())
                .map(|p| p.kind().index() as u8)
                .unwrap_or(0);
            self.capture_history.update(
                cap_piece.index() as u8,
                cap.to().index() as u8,
                cap_captured_pt,
                -bonus1,
            );
        }

        // SF11 search.cpp:1288 zeros this ply's
        // statScore on a fail-high so that, if this
        // frame is reached again via a sibling at a
        // higher ply, the LMR parent-comparison reads
        // a clean baseline rather than the cutoff
        // move's (possibly very large) value.
        self.stack[STACK_SENTINEL + ply].stat_score = 0;
    }
}
