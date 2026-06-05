//! `Search` per-search state: the [`Search`] struct, its constructor and
//! accessors, the small per-node helper methods (PV update, contempt, draw
//! scoring, repetition, stop checks, PV trace), and the [`StackEntry`],
//! [`RootMove`], and [`MovesOutcome`] supporting types.

use super::*;
use crate::endgame::EndgameSkill;
use crate::engine::WorkerState;
use crate::eval::{evaluate_with_pawn_cache, evaluate_with_trace, EvalTrace};
use crate::movepick::{ButterflyHistory, CaptureHistory, ContHistStore, CounterMoveTable};
use crate::opponent::EvalMask;
use crate::pawns;
use crate::position::{Position, StateInfo};
use crate::tt::TranspositionTable;
use crate::types::{Color, Move, Value};
use std::sync::atomic::Ordering;
use std::time::Instant;

/// One entry of the per-ply search stack — captures the move played
/// from this ply (so the child at ply+1 can read it as "1-ply-ago"),
/// the static eval at this ply (used for the `improving` flag), and
/// whether the position-pre-move was in check / the move was a
/// capture (used to pick the right [`ContHistStore`] sub-arena).
#[derive(Copy, Clone, Debug)]
pub(crate) struct StackEntry {
    /// `Piece::index()` of the piece moved at this ply, or 0 for the
    /// "no move" sentinel (root and pre-search padding).
    pub moved_piece_idx: u8,
    /// `Square::index()` of the destination, or 0 in the sentinel.
    pub to_idx: u8,
    /// Was the position in check *before* this ply's move was played?
    pub in_check: bool,
    /// Was this ply's move a capture (incl. en-passant / promotion to
    /// piece on a non-empty square)?
    pub was_capture: bool,
    /// Static evaluation at this ply, before any move was played.
    /// `Value::NONE` when the searcher was in check or for sentinels.
    pub static_eval: Value,
    /// The *contempt-free* static evaluation at this ply — what we
    /// persist to the TT, and what the after-null eval refinement
    /// (SF11 search.cpp:817/1429) negates: a child reached via a null
    /// move derives its eval as `-(ss-1)->staticEval + 2·Tempo`. Read
    /// from this raw value so the result stays contempt-free.
    /// `Value::NONE` when in check or for sentinels.
    pub raw_static_eval: Value,
    /// Was this ply's move the *null move* (SF11 `(ss-1)->currentMove
    /// == MOVE_NULL`)? Set true only in the null-move-pruning block;
    /// reset false at every real-move recursion site. Children read
    /// `stack[ply-1].was_null` to gate NMP (don't null twice in a row)
    /// and to select the after-null eval refinement.
    pub was_null: bool,
    /// Stockfish's per-ply `statScore`: blended main + cont-history
    /// score for the *most recent quiet move iterated at this ply*.
    /// Read by children for LMR comparison (`(ss-1)->statScore`) and
    /// by the parent itself for NMP gating. Carries through siblings:
    /// only the first grandchild iterates with statScore=0 (the
    /// `(ss+2)/(ss+4)` reset at node entry); later grandchildren
    /// inherit whichever value the previous sibling left behind.
    pub stat_score: i32,
    /// 1-indexed count of the move currently being iterated at this
    /// ply, or 0 before the first legal child has been picked up.
    /// Read by the child's CMP gate (`(ss-1)->moveCount == 1`) to
    /// widen the LMR-depth threshold when the parent is on its top
    /// quiet (typically the TT move, which carries strong signal).
    pub move_count: u32,
    /// Kind of the piece this ply's move captured, if any. Read by
    /// the child node's "last captures" extension (SF11 search.cpp:1084):
    /// SF accesses `pos.captured_piece()` from the child's `StateInfo`,
    /// which after the parent's `do_move` reflects the move that
    /// brought the child here. We propagate it explicitly through the
    /// stack so the child doesn't have to re-derive it.
    pub captured_piece_kind: Option<crate::types::PieceType>,
}

impl StackEntry {
    pub const SENTINEL: StackEntry = StackEntry {
        moved_piece_idx: 0,
        to_idx: 0,
        in_check: false,
        was_capture: false,
        static_eval: Value::NONE,
        raw_static_eval: Value::NONE,
        was_null: false,
        stat_score: 0,
        move_count: 0,
        captured_piece_kind: None,
    };
}

// =========================================================================
// Root move tracking
// =========================================================================

/// One candidate root move with its most-recent score and principal
/// variation. [`Search::root_moves`] holds one of these per legal move at
/// the root; the MultiPV loop sorts this vector (stably) after each PV
/// slot finishes so `root_moves[i]` holds the i-th best move.
#[derive(Clone, Debug)]
pub(crate) struct RootMove {
    /// The root move itself — the first move of `pv`.
    pub(crate) mv: Move,
    /// Score from the root side-to-move's point of view. Equal to
    /// `-Value::INFINITE` before the first iteration scores it.
    pub(crate) score: Value,
    /// Principal variation starting with `mv`. Captured after each
    /// slot's root-level search completes.
    pub(crate) pv: Vec<Move>,
    /// Score from the previous completed iterative-deepening iteration
    /// — used as the aspiration-window seed for the next iteration.
    pub(crate) prev_score: Value,
}

/// Result of [`Search::negamax_moves`] — the move-loop body lifted out of
/// `negamax`. `Aborted` means the shared stop flag fired mid-loop and the
/// caller must return `Value::ZERO`; `Done` carries the loop's outcome for
/// the caller's terminal-position check and TT save.
pub(crate) enum MovesOutcome {
    Aborted,
    Done {
        best_score: Value,
        best_move: Move,
        raised_alpha: bool,
        move_count: usize,
    },
}

// =========================================================================
// Per-search state
// =========================================================================

/// Per-search scratchpad: killers, PV table, node counter, stop
/// machinery, repetition path. One `Search` is constructed per
/// `Engine::search` call and thrown away. The TT and history reference
/// shared engine state.
pub(crate) struct Search<'a> {
    pub(crate) tt: &'a TranspositionTable,
    pub(crate) history: &'a mut ButterflyHistory,
    pub(crate) counter_moves: &'a mut CounterMoveTable,
    pub(crate) cont_history: &'a mut ContHistStore,
    pub(crate) capture_history: &'a mut CaptureHistory,
    pub(crate) pawn_cache: &'a mut pawns::Table,

    /// Per-ply search stack with 7 leading sentinel frames so that
    /// `stack[STACK_SENTINEL + ply - i]` for `i ∈ {1, 2, 4, 6}` is
    /// always in-bounds even at ply 0. Sized `MAX_PLY +
    /// STACK_SENTINEL + 1` and allocated once per `Search::new`.
    pub(crate) stack: Vec<StackEntry>,

    /// Killer moves per ply: two slots, `killers[ply][0]` is the latest
    /// fail-high quiet found at that ply.
    pub(crate) killers: Vec<[Move; 2]>,

    /// Flat PV storage: `MAX_PLY` slots per ply, addressed as
    /// `pv[ply * MAX_PLY + idx]`. Paired with `pv_length` per ply.
    pub(crate) pv: Vec<Move>,
    pub(crate) pv_length: Vec<usize>,

    /// Path of position keys from root to current node. Used for
    /// repetition detection inside the search tree.
    pub(crate) path_keys: Vec<u64>,

    /// Every legal move at the root position with its most-recent score
    /// and PV. Stable-sorted by score descending (in the `[pv_idx..]`
    /// range) after each PV slot's search completes.
    pub(crate) root_moves: Vec<RootMove>,
    /// Current PV slot being searched. The root move loop only considers
    /// `root_moves[pv_idx..]` so earlier slots stay fixed.
    pub(crate) pv_idx: usize,
    /// Effective MultiPV count for this search — clamped to the number
    /// of legal root moves when the caller requests more lines than are
    /// available.
    pub(crate) multi_pv: usize,

    pub(crate) nodes: u64,
    /// Per-ply node histogram (TEMPORARY: perf investigation). Indexed
    /// by recursion depth from root (`ply`). Index 0 = root; deeper
    /// indices = nodes visited at that distance from root, including
    /// qsearch and extension-stretched leaves. Sized `MAX_PLY` so the
    /// extension-stretched tail can be observed; ply >= MAX_PLY is
    /// clamped into the last bucket. Reset to zero at every `run()`
    /// start. Exposed via [`Search::nodes_per_ply`].
    pub(crate) nodes_per_ply: Vec<u64>,
    /// Maximum `ply` reached during the most recent `run()` (TEMPORARY:
    /// perf investigation). Mirrors SF's `selDepth` — distinguishes
    /// horizon-stretching (`seldepth >> nominal_depth`) from wide
    /// branching. Reset at every `run()` start.
    pub(crate) seldepth: u32,
    pub(crate) max_nodes: Option<u64>,
    pub(crate) start_time: Instant,
    pub(crate) stop_time: Option<Instant>,
    pub(crate) next_stop_check: u64,
    /// Shared stop flag. In single-thread mode only this thread writes
    /// to it (when its own limits fire); in multi-thread mode the
    /// main thread sets it once iterative deepening finishes so the
    /// helper threads see it and bail. Read via [`should_stop`]
    /// which folds in the local node/time limits too.
    pub(crate) stop_flag: StopFlag,

    /// When `true`, write iterative-deepening and root-move progress
    /// to stderr. Mirrors [`SearchParams::verbose_progress`]; set from
    /// `run()`.
    pub(crate) verbose_progress: bool,

    /// Node count at which the next verbose "still alive" heartbeat
    /// should print. Only used when [`verbose_progress`] is `true`.
    pub(crate) verbose_next_tick: u64,

    /// Side-to-move at the root. Captured at the start of
    /// [`Search::run`] so contempt can be applied asymmetrically
    /// (root prefers playing on; opponent is nudged toward drawing).
    pub(crate) root_stm: Color,

    /// Evaluation-category mask the bot is "blind" to for this
    /// search. [`EvalMask::EMPTY`] is the hot path; populated only
    /// for play-engine searches whose [`crate::engine::SearchParams::
    /// eval_mask`] was set by an [`crate::opponent::OpponentProfile`].
    /// Captured from `params` at `run()` start; passed to every
    /// `evaluate_with_pawn_cache` call inside the search.
    pub(crate) eval_mask: EvalMask,

    /// Quiescence-search horizon cap, in plies of capture resolution.
    /// [`QSEARCH_UNBOUNDED`] (the default) means qsearch resolves
    /// captures normally (full tactical vision). A small value limits how
    /// many capture plies qsearch sees before falling back to the static
    /// eval — the "tactical horizon" lever for believable weak bots: a
    /// cap of `0` makes the bot tactically blind (it can't see that its
    /// queen gets recaptured, so it hangs pieces like a sub-600 human).
    /// Play-engine-only, exactly like [`eval_mask`](Self::eval_mask): the
    /// analytical engine must keep full qsearch so teaching feedback
    /// judges against true best play. Captured from `params` at `run()`.
    pub(crate) qsearch_cap: i32,

    /// Endgame-book knowledge tier the bot may use for this search.
    /// [`EndgameSkill::Full`] (the default) consults every specialist;
    /// lower tiers withhold the harder ones so a weak bot misplays
    /// endgames (no king-driving gradient, botched KBNK) like a human of
    /// that level. Play-engine-only, exactly like [`eval_mask`](Self::
    /// eval_mask) and [`qsearch_cap`](Self::qsearch_cap): analytical
    /// searches keep `Full` so teaching judges true best play. Captured
    /// from `params` at `run()` start; passed to every
    /// `evaluate_with_pawn_cache` call inside the search.
    pub(crate) eg_skill: EndgameSkill,

    /// SF11's `Thread::ttHitAverage` (search.cpp:699-700): a running
    /// exponential average of TT-hit success, in units of
    /// `TT_HIT_AVERAGE_RESOLUTION`. Updated once per `negamax` node
    /// after the probe; read by the LMR relaxer/capture-gate. Reset to
    /// half-window at every `run()`.
    pub(crate) tt_hit_average: i64,

    /// SF11's `Thread::nmpMinPly` (search.cpp:876). While a null-move
    /// *verification* search is active, NMP is disabled for the
    /// verifying side ([`nmp_color`]) until `ply` reaches this value —
    /// this forbids recursive verification. `0` means no verification
    /// is active (NMP allowed everywhere, since `ply >= 0` always
    /// holds). Reset to `0` at every `run()`.
    pub(crate) nmp_min_ply: usize,
    /// SF11's `Thread::nmpColor` (search.cpp:877). The side for which
    /// NMP is suspended during an active verification search. Only
    /// consulted when [`nmp_min_ply`] is non-zero.
    pub(crate) nmp_color: Color,
}

impl<'a> Search<'a> {
    pub(crate) fn new(
        tt: &'a TranspositionTable,
        worker: &'a mut WorkerState,
        stop_flag: StopFlag,
    ) -> Search<'a> {
        // Destructure into disjoint &mut field borrows so the rest of
        // the search code can keep its existing per-table call sites.
        let WorkerState {
            history,
            counter_moves,
            cont_history,
            capture_history,
            pawn_cache,
        } = worker;
        Search {
            tt,
            history,
            counter_moves,
            cont_history,
            capture_history,
            pawn_cache,
            stack: vec![StackEntry::SENTINEL; MAX_PLY + STACK_SENTINEL + STACK_LOOKAHEAD],
            // `pv_length` is sized `MAX_PLY + 1` so the parent's
            // `update_pv` read of `pv_length[ply + 1]` is in bounds when
            // the child bailed at `ply == MAX_PLY`. The child still
            // writes `pv_length[ply] = 0` on the bail path, so the
            // parent sees a correctly-empty child PV.
            killers: vec![[Move::NONE; 2]; MAX_PLY],
            pv: vec![Move::NONE; MAX_PLY * MAX_PLY],
            pv_length: vec![0; MAX_PLY + 1],
            path_keys: Vec::with_capacity(MAX_PLY),
            root_moves: Vec::new(),
            pv_idx: 0,
            multi_pv: 1,
            nodes: 0,
            nodes_per_ply: vec![0; MAX_PLY],
            seldepth: 0,
            max_nodes: None,
            start_time: Instant::now(),
            stop_time: None,
            next_stop_check: STOP_CHECK_INTERVAL,
            stop_flag,
            verbose_progress: false,
            verbose_next_tick: 0,
            root_stm: Color::White,
            eval_mask: EvalMask::EMPTY,
            qsearch_cap: QSEARCH_UNBOUNDED,
            eg_skill: EndgameSkill::Full,
            tt_hit_average: TT_HIT_AVERAGE_INIT,
            nmp_min_ply: 0,
            nmp_color: Color::White,
        }
    }

    /// Total nodes visited by this `Search` so far. Read by
    /// [`crate::engine::Engine::search`] after [`run`](Self::run) returns
    /// to surface the count alongside the search lines.
    pub(crate) fn node_count(&self) -> u64 {
        self.nodes
    }

    /// Maximum `ply` reached during the most recent `run()` (selective
    /// depth). TEMPORARY perf-investigation accessor.
    pub(crate) fn seldepth(&self) -> u32 {
        self.seldepth
    }

    /// Per-ply node histogram from the most recent `run()`. Index = ply
    /// from root. TEMPORARY perf-investigation accessor.
    pub(crate) fn nodes_per_ply(&self) -> &[u64] {
        &self.nodes_per_ply
    }

    pub(super) fn update_pv(&mut self, ply: usize, mv: Move) {
        self.pv[ply * MAX_PLY] = mv;
        let child_len = self.pv_length[ply + 1];
        for i in 0..child_len {
            self.pv[ply * MAX_PLY + 1 + i] = self.pv[(ply + 1) * MAX_PLY + i];
        }
        self.pv_length[ply] = 1 + child_len;
    }

    /// Side-to-move-asymmetric contempt offset. Returns the adjustment
    /// to add to a raw score (from `curr_stm`'s POV) so that, after
    /// propagation back to the root via negamax negations, every non-
    /// drawing evaluation is shifted by `-CONTEMPT_CP` from the root's
    /// POV — a small preference for playing on.
    pub(super) fn contempt_for_pov(&self, curr_stm: Color) -> i32 {
        if curr_stm == self.root_stm {
            -CONTEMPT_CP
        } else {
            CONTEMPT_CP
        }
    }

    /// Contempt-adjusted static evaluation. The raw `evaluate_with_pawn_cache(pos)`
    /// returns a position score from the side-to-move's POV; we add
    /// the appropriate contempt so non-drawing lines are preferred
    /// over drawing ones at the root. Leaf evaluations used in
    /// pruning decisions should go through this, but **not** values
    /// written into the TT — see `probe.save(..., raw_eval)` below.
    pub(super) fn search_eval(&mut self, pos: &Position) -> Value {
        let raw = evaluate_with_pawn_cache(pos, self.pawn_cache, self.eval_mask, self.eg_skill);
        Value(raw.0 + self.contempt_for_pov(pos.side_to_move()))
    }

    /// Apply contempt to a raw eval pulled from the TT. Mirrors
    /// [`search_eval`] for already-computed values.
    pub(super) fn apply_contempt(&self, raw: Value, pos: &Position) -> Value {
        if raw == Value::NONE {
            raw
        } else {
            Value(raw.0 + self.contempt_for_pov(pos.side_to_move()))
        }
    }

    /// Score returned for draw-by-repetition / 50-move-rule. Combines
    /// contempt with a node-counter-based ±1 jitter (above a minimum
    /// depth) so distinct search paths to a draw return distinct
    /// values — alpha-beta pruning then has real differences to cut
    /// on, instead of an everything-zero subtree.
    pub(super) fn draw_value(&self, depth: i32, pos: &Position) -> Value {
        let contempt = self.contempt_for_pov(pos.side_to_move());
        let jitter = if depth < DRAW_JITTER_MIN_DEPTH {
            0
        } else {
            2 * (self.nodes & 1) as i32 - 1
        };
        Value(contempt + jitter)
    }

    pub(super) fn is_repetition(&self, pos: &Position) -> bool {
        let key = pos.key();
        let len = self.path_keys.len();
        if len < 2 {
            return false;
        }
        // Chess repetition can only occur across reversible moves —
        // any pawn move or capture resets the halfmove clock *and*
        // makes it impossible for positions prior to that reset to
        // repeat. Scan only the last `halfmove_clock()` entries of
        // the path instead of the whole vector. This is a perf fix
        // for pathological late-endgame searches where the full
        // `game_history` is long (100+ plies) and most of those
        // keys are forever unreachable — but more importantly, it
        // shrinks the "every subtree leaf returns draw" zone that
        // otherwise kills alpha-beta pruning in near-draw positions.
        //
        // `len - 1` is the current position's own key (we compare
        // against all earlier keys, not it). We look back at most
        // `halfmove_clock` further entries from there.
        let hmc = pos.halfmove_clock() as usize;
        let start = (len - 1).saturating_sub(hmc);
        for k in &self.path_keys[start..len - 1] {
            if *k == key {
                return true;
            }
        }
        false
    }

    /// Cheap read of the shared stop flag. Used by code paths that
    /// just need to bail an in-progress recursion without redoing the
    /// node-cadence / time-deadline / heartbeat checks.
    pub(super) fn is_aborted(&self) -> bool {
        self.stop_flag.load(Ordering::Relaxed)
    }

    pub(super) fn check_should_stop(&mut self) -> bool {
        // Shared stop-flag observation: another thread (or this thread's
        // own earlier limit-hit) may have set it. Read it on every
        // call so a helper thread sees the main thread's stop signal
        // promptly.
        if self.stop_flag.load(Ordering::Relaxed) {
            return true;
        }
        if self.verbose_progress && self.nodes >= self.verbose_next_tick {
            eprintln!(
                "[search]     tick: {} nodes, {} ms elapsed",
                self.nodes,
                self.start_time.elapsed().as_millis(),
            );
            self.verbose_next_tick = self.nodes + VERBOSE_TICK_INTERVAL;
        }
        if self.nodes >= self.next_stop_check {
            self.next_stop_check = self.nodes + STOP_CHECK_INTERVAL;
            if let Some(n) = self.max_nodes {
                if self.nodes >= n {
                    self.stop_flag.store(true, Ordering::Relaxed);
                    return true;
                }
            }
            if let Some(deadline) = self.stop_time {
                if Instant::now() >= deadline {
                    self.stop_flag.store(true, Ordering::Relaxed);
                    return true;
                }
            }
        }
        false
    }

    /// Walk the principal variation and capture an [`EvalTrace`] at each
    /// ply. Returns one trace per move in `pv`: `traces[i]` is the
    /// evaluation of the position reached after playing `pv[0..=i]`. The
    /// leaf trace (the evaluation at the end of the PV) is the last
    /// element. `pos` is mutated during the walk via `do_move` / undone
    /// on the way out, so callers get the original position back.
    ///
    /// Used by the teaching-analysis pipeline's settled-ply detection:
    /// the score trajectory along a PV tells the UI where the evaluation
    /// stops meaningfully changing, which is the "aha moment" to
    /// attribute term deltas to.
    pub(super) fn trace_along_pv(&self, pos: &mut Position, pv: &[Move]) -> Vec<EvalTrace> {
        let mut traces = Vec::with_capacity(pv.len());
        let mut states: Vec<StateInfo> = Vec::with_capacity(pv.len());
        for &mv in pv {
            states.push(pos.do_move(mv));
            let (_, trace) = evaluate_with_trace(pos);
            traces.push(trace);
        }
        for (mv, st) in pv.iter().zip(states.iter()).rev() {
            pos.undo_move(*mv, *st);
        }
        traces
    }
}
