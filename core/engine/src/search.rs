//! Alpha-beta search with iterative deepening and Stockfish 11's
//! pruning stack: null-move, late move reductions (LMR), late move
//! pruning (LMP), futility pruning, SEE pruning, check extensions, and
//! mate-distance pruning. Assembles a principal variation, an
//! accompanying [`EvalTrace`] at the PV leaf, and wires into the
//! [`crate::tt::TranspositionTable`] and [`ButterflyHistory`] for fast
//! subsequent searches.
//!
//! MultiPV follows Stockfish's per-PV-slot pattern: at every iterative
//! deepening depth we walk through [`Search::multi_pv`] slots in order,
//! each time restricting the root move list to those not already claimed
//! by an earlier PV. After each slot's search completes we stable-sort
//! the tail of [`Search::root_moves`] by score descending, promoting the
//! winner into position `pv_idx`. This preserves alpha-beta efficiency
//! within each slot's pass while producing a deterministic top-N ranking.
//! Singular extensions, multi-cut, IID, probcut, razoring, and
//! sophisticated time management are deferred to a follow-up; the
//! scaffolding here should accept them without API churn.

use crate::eval::{evaluate_with_pawn_cache, evaluate_with_trace, EvalTrace};
use crate::opponent::EvalMask;
use crate::movepick::{
    cont_history_update, ButterflyHistory, CaptureHistory, ContHistKeys, ContHistStore,
    CounterMoveTable, MovePicker, BUTTERFLY_HISTORY_BOUND, CAPTURE_HISTORY_BOUND, CONT_HISTORY_BOUND,
};
use crate::pawns;
use crate::position::{Position, StateInfo};
use crate::tt::TranspositionTable;
use crate::types::{Bound, Color, Depth, Move, PieceType, Square, Value};

use crate::engine::{SearchLine, SearchParams, WorkerState};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// Shared stop flag set by the main thread (or by any thread that hits
/// the configured limits) to ask all running searches to bail. Helper
/// threads in a Lazy-SMP search check this between batches of nodes;
/// the single-thread fast path also uses it but only writes to it
/// (never observed by another thread).
pub(crate) type StopFlag = Arc<AtomicBool>;

// =========================================================================
// Constants
// =========================================================================

/// Maximum search depth / ply. Matches `Value::MAX_PLY`.
pub const MAX_PLY: usize = Value::MAX_PLY as usize;

/// How often (in nodes) we check the wall clock / node cap for a stop
/// signal. Keeping this coarse avoids a `now()` syscall per node.
const STOP_CHECK_INTERVAL: u64 = 4096;

/// Node-count interval for the `verbose_progress` "still alive"
/// heartbeat. Picked large enough to not spam stderr in normal search
/// (at ~5 Mnodes/s, 500k = ~100ms between ticks) but small enough that
/// a genuinely-stuck search's last heartbeat is recent.
const VERBOSE_TICK_INTERVAL: u64 = 500_000;

/// Aspiration-window start width. Search widens on fail-high/fail-low.
/// Kept at our pre-SF11-port value of 17 because SF11's score-scaled
/// `21 + |prev|/256` initial regressed FEN 26 d=13 by ~3× (138 k →
/// 447 k); the wider initial costs more in alpha-beta inefficiency
/// than it saves in avoided re-searches. SF11's depth-reduction
/// on consecutive fail-highs (see `aspiration_search`) is the
/// load-bearing piece of the port, not the delta tuning.
const ASPIRATION_DELTA: i32 = 17;

/// Side-to-move-asymmetric bias added to every static evaluation during
/// search. Positive cp when it's the root side's turn; negative when
/// it's the opponent's. Effect at the root: any position the raw
/// evaluator scores as `0` (objectively drawn) returns `+CONTEMPT_CP`
/// after the bias, while a repetition-draw still returns ~0. Drawing
/// is thus a real deficit against the shifted landscape instead of
/// equivalent to playing on, which gives alpha-beta pruning a gradient
/// even in draw-heavy positions. Mimics Stockfish's `Contempt` UCI
/// option.
///
/// **Cross-search caveat:** because contempt is keyed to root_stm and
/// sign-flips between consecutive moves in a game, persisted TT
/// entries carry contempt with the *previous* root's sign. Reads
/// during the next move's search are therefore biased by up to
/// `2 × CONTEMPT_CP`. Keeping the magnitude small (2 cp) bounds that
/// pollution to ±4 cp — small enough to be noise relative to real
/// evaluation differences, while still giving the search a tiny
/// preference for playing on over repeating in balanced positions.
const CONTEMPT_CP: i32 = 2;

/// Depth below which draw values aren't jittered — quiescence-ish
/// regions where a ±1 cp tiebreak would only add noise.
const DRAW_JITTER_MIN_DEPTH: i32 = 4;

/// Minimum depth at which null-move pruning is considered. SF11 has no
/// such floor (it nulls at any depth, diving straight to qsearch when
/// `depth - R <= 0`); we keep a `depth >= 3` gate as a pre-existing,
/// deliberate divergence — low-depth nodes are already covered by
/// razoring (`depth < 2`) and reverse-futility (`depth < 6`).
const NULL_MIN_DEPTH: i32 = 3;

/// SF11 `RazorMargin` (search.cpp:68). At `depth < 2`, when even the
/// refined eval is this far below alpha, the node almost certainly
/// can't raise alpha — drop straight into quiescence.
const RAZOR_MARGIN: i32 = 531;

/// Depth (in plies) at and above which a successful null-move cutoff is
/// re-checked by a verification search with NMP disabled for the
/// cutting side (SF11 search.cpp:869). Below it, the cutoff is trusted
/// directly. Guards against zugzwang where the null move is illusorily
/// good.
const NMP_VERIFY_MIN_DEPTH: i32 = 13;

/// Minimum depth at which LMR activates; earlier moves below it play
/// out at full depth.
const LMR_MIN_DEPTH: i32 = 3;

/// SF11 `ttHitAverageWindow` / `ttHitAverageResolution` (search.cpp:64-65).
/// The running TT-hit average is maintained per search and read by two
/// LMR relaxers (decrease reduction when hits are common; allow
/// capture-LMR when hits are rare). Initialised to half-window.
const TT_HIT_AVERAGE_WINDOW: i64 = 4096;
const TT_HIT_AVERAGE_RESOLUTION: i64 = 1024;
const TT_HIT_AVERAGE_INIT: i64 = TT_HIT_AVERAGE_WINDOW * TT_HIT_AVERAGE_RESOLUTION / 2;

/// When `true`, the LMP threshold (`late_move_prune`) is evaluated at
/// every depth (not just shallow). Once tripped, the flag is threaded
/// into [`MovePicker::next_move`] so the picker stops generating quiet
/// moves entirely for the rest of the node. Mirrors SF11's
/// `moveCountPruning` (search.cpp:1002, threaded into
/// `mp.next_move(moveCountPruning)` at line 964). Landed 2026-05-14
/// (commit `8eafb71`) and confirmed load-bearing on FEN 26 d=13 cold
/// (484 M → 226 k, 2,140×).
const MOVE_COUNT_PRUNING_UNIVERSAL: bool = true;

/// Adjacent-ply |Δwhite-POV-score| below which the PV is considered
/// "settled". In Stockfish-internal centipawns (roughly: PawnEG = 213),
/// so 25 cp is about one-tenth of a pawn — tight enough to treat small
/// positional wobble as noise, wide enough to not get tricked by a 10-cp
/// mobility swing. Tuneable once we see real output on test positions.
pub const SETTLED_THRESHOLD_CP: i32 = 25;

/// Number of sentinel frames prepended to the per-ply stack so that
/// "look back N plies" reads from `ply 0..6` are always in bounds.
/// Stockfish's stack uses the same convention with offset 7.
const STACK_SENTINEL: usize = 7;

/// Trailing padding past the per-ply stack so the SF-style
/// `(ss+2)->statScore = 0` (and `(ss+4)` at root) zero-resets remain
/// in-bounds even when invoked at the maximum legal ply. Sized to cover
/// up to a `+4` write from any in-range ply.
const STACK_LOOKAHEAD: usize = 5;

/// Stockfish 11's `stat_bonus` (search.cpp:86): the depth-dependent
/// bonus applied on β-cutoff to history / continuation-history
/// counters for the cutting move (and to losers tried before it,
/// negated). Bound at ±[`CONT_HISTORY_BOUND`] which the table-update
/// gravity-formula tolerates.
fn stat_bonus(depth: i32) -> i32 {
    if depth > 15 {
        -8
    } else {
        let raw = 19 * depth * depth + 155 * depth - 132;
        raw.clamp(-CONT_HISTORY_BOUND, CONT_HISTORY_BOUND)
    }
}

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
struct RootMove {
    /// The root move itself — the first move of `pv`.
    mv: Move,
    /// Score from the root side-to-move's point of view. Equal to
    /// `-Value::INFINITE` before the first iteration scores it.
    score: Value,
    /// Principal variation starting with `mv`. Captured after each
    /// slot's root-level search completes.
    pv: Vec<Move>,
    /// Score from the previous completed iterative-deepening iteration
    /// — used as the aspiration-window seed for the next iteration.
    prev_score: Value,
}

// =========================================================================
// Per-search state
// =========================================================================

/// Per-search scratchpad: killers, PV table, node counter, stop
/// machinery, repetition path. One `Search` is constructed per
/// `Engine::search` call and thrown away. The TT and history reference
/// shared engine state.
pub(crate) struct Search<'a> {
    tt: &'a TranspositionTable,
    history: &'a mut ButterflyHistory,
    counter_moves: &'a mut CounterMoveTable,
    cont_history: &'a mut ContHistStore,
    capture_history: &'a mut CaptureHistory,
    pawn_cache: &'a mut pawns::Table,

    /// Per-ply search stack with 7 leading sentinel frames so that
    /// `stack[STACK_SENTINEL + ply - i]` for `i ∈ {1, 2, 4, 6}` is
    /// always in-bounds even at ply 0. Sized `MAX_PLY +
    /// STACK_SENTINEL + 1` and allocated once per `Search::new`.
    stack: Vec<StackEntry>,

    /// Killer moves per ply: two slots, `killers[ply][0]` is the latest
    /// fail-high quiet found at that ply.
    killers: Vec<[Move; 2]>,

    /// Flat PV storage: `MAX_PLY` slots per ply, addressed as
    /// `pv[ply * MAX_PLY + idx]`. Paired with `pv_length` per ply.
    pv: Vec<Move>,
    pv_length: Vec<usize>,

    /// Path of position keys from root to current node. Used for
    /// repetition detection inside the search tree.
    path_keys: Vec<u64>,

    /// Every legal move at the root position with its most-recent score
    /// and PV. Stable-sorted by score descending (in the `[pv_idx..]`
    /// range) after each PV slot's search completes.
    root_moves: Vec<RootMove>,
    /// Current PV slot being searched. The root move loop only considers
    /// `root_moves[pv_idx..]` so earlier slots stay fixed.
    pv_idx: usize,
    /// Effective MultiPV count for this search — clamped to the number
    /// of legal root moves when the caller requests more lines than are
    /// available.
    multi_pv: usize,

    nodes: u64,
    /// Per-ply node histogram (TEMPORARY: perf investigation). Indexed
    /// by recursion depth from root (`ply`). Index 0 = root; deeper
    /// indices = nodes visited at that distance from root, including
    /// qsearch and extension-stretched leaves. Sized `MAX_PLY` so the
    /// extension-stretched tail can be observed; ply >= MAX_PLY is
    /// clamped into the last bucket. Reset to zero at every `run()`
    /// start. Exposed via [`Search::nodes_per_ply`].
    nodes_per_ply: Vec<u64>,
    /// Maximum `ply` reached during the most recent `run()` (TEMPORARY:
    /// perf investigation). Mirrors SF's `selDepth` — distinguishes
    /// horizon-stretching (`seldepth >> nominal_depth`) from wide
    /// branching. Reset at every `run()` start.
    seldepth: u32,
    max_nodes: Option<u64>,
    start_time: Instant,
    stop_time: Option<Instant>,
    next_stop_check: u64,
    /// Shared stop flag. In single-thread mode only this thread writes
    /// to it (when its own limits fire); in multi-thread mode the
    /// main thread sets it once iterative deepening finishes so the
    /// helper threads see it and bail. Read via [`should_stop`]
    /// which folds in the local node/time limits too.
    stop_flag: StopFlag,

    /// When `true`, write iterative-deepening and root-move progress
    /// to stderr. Mirrors [`SearchParams::verbose_progress`]; set from
    /// `run()`.
    verbose_progress: bool,

    /// Node count at which the next verbose "still alive" heartbeat
    /// should print. Only used when [`verbose_progress`] is `true`.
    verbose_next_tick: u64,

    /// Side-to-move at the root. Captured at the start of
    /// [`Search::run`] so contempt can be applied asymmetrically
    /// (root prefers playing on; opponent is nudged toward drawing).
    root_stm: Color,

    /// Evaluation-category mask the bot is "blind" to for this
    /// search. [`EvalMask::EMPTY`] is the hot path; populated only
    /// for play-engine searches whose [`crate::engine::SearchParams::
    /// eval_mask`] was set by an [`crate::opponent::OpponentProfile`].
    /// Captured from `params` at `run()` start; passed to every
    /// `evaluate_with_pawn_cache` call inside the search.
    eval_mask: EvalMask,

    /// SF11's `Thread::ttHitAverage` (search.cpp:699-700): a running
    /// exponential average of TT-hit success, in units of
    /// `TT_HIT_AVERAGE_RESOLUTION`. Updated once per `negamax` node
    /// after the probe; read by the LMR relaxer/capture-gate. Reset to
    /// half-window at every `run()`.
    tt_hit_average: i64,

    /// SF11's `Thread::nmpMinPly` (search.cpp:876). While a null-move
    /// *verification* search is active, NMP is disabled for the
    /// verifying side ([`nmp_color`]) until `ply` reaches this value —
    /// this forbids recursive verification. `0` means no verification
    /// is active (NMP allowed everywhere, since `ply >= 0` always
    /// holds). Reset to `0` at every `run()`.
    nmp_min_ply: usize,
    /// SF11's `Thread::nmpColor` (search.cpp:877). The side for which
    /// NMP is suspended during an active verification search. Only
    /// consulted when [`nmp_min_ply`] is non-zero.
    nmp_color: Color,
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

            // Mate found in the leader — no point searching deeper.
            if self.root_moves[0].score.0.abs() >= Value::MATE.0 - Value::MAX_PLY {
                break;
            }
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
    fn run_forced_slots(&mut self, pos: &mut Position, forced: &[Move], max_depth: u32) {
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

                // Mate-in-N termination mirrors the main IDS loop.
                if self.root_moves[new_slot].score.0.abs() >= Value::MATE.0 - Value::MAX_PLY {
                    break;
                }
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

    fn aspiration_search(&mut self, pos: &mut Position, depth: i32, prev_score: Value) -> Value {
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

    // ------------------------------------------------------------------
    // Alpha-beta with pruning stack
    // ------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn negamax(
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
            Value(evaluate_with_pawn_cache(pos, self.pawn_cache, self.eval_mask).0 + bonus)
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
                return Value::ZERO;
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
                    return clamped;
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
                    return Value::ZERO;
                }
                if v >= beta {
                    return clamped;
                }
            }
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
                    return Value::ZERO;
                }

                if value >= raised_beta_v {
                    pc_value = value;
                    break;
                }
            }

            if pc_value != Value::NONE {
                return pc_value;
            }
        }

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
                && {
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
                        let mvp = moved_piece.index() as usize;
                        let mvt = mv.to().index() as usize;
                        let ch0 = cmp_cont_hist_read(self.cont_history, cont_keys[0], mvp, mvt);
                        let ch1 = cmp_cont_hist_read(self.cont_history, cont_keys[1], mvp, mvt);
                        ch0 < 0 && ch1 < 0
                    }
                };
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
                && {
                    let lmr_r = lmr_reduction(depth, move_count, improving);
                    // newDepth = depth - 1 here; extensions are computed
                    // *after* Step 13 in SF11, so the gate is keyed on
                    // pre-extension depth (search.cpp:994, 1008).
                    let lmr_d = ((depth - 1) - lmr_r).max(0);
                    if lmr_d >= 6 {
                        false
                    } else if static_eval.0 + 235 + 172 * lmr_d > alpha.0 {
                        false
                    } else {
                        // Post-`do_move` this read was `pos.side_to_move()`
                        // (the opponent); pre-move we reproduce that exact
                        // value as `!us_at_node` to stay node-neutral.
                        let stm = !us_at_node;
                        let mvp_idx = moved_piece.index() as usize;
                        let mvt_idx = mv.to().index() as usize;
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
                };
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

            // --- Extension chain (SF11 search.cpp:1072-1090) ---
            //
            // Four predicates, mutually-exclusive `else if` for the
            // first three; castling is a separate `if` at the bottom
            // that overrides any prior result. Each fires `+1 ply`.
            // Singular extensions belong here too in SF's full
            // structure but aren't ported yet (see HANDOFF).
            //
            // CHECK EXTENSION: previously blanket — every check got
            // +1 ply. SF's gate is tighter: only extend when the
            // check is either a discovery (moving piece was a
            // blocker for the enemy king, so its departure unblocks
            // a slider check) OR has SEE >= 0 (the checking piece
            // won't simply lose material to a recapture). The
            // filtered-out moves are SEE-negative sac-checks that
            // were noise.
            //
            // ISOLATED-ADDITION CAVEAT: A/B isolation (same session)
            // showed each of the other three extensions (passed-pawn,
            // last-captures, castling) is net-negative in isolation
            // on top of check-gating, sometimes catastrophically so
            // (last-captures alone on pawn-race endgames runs >20 min
            // because every capture drops NPM below 2*ROOK_MG and
            // extends the whole subtree). But all four *together*
            // are net-positive at depth 13 (9× vs blanket-check
            // baseline) and depth 14 (6×). The interaction matters:
            // when only one extension fires per node, its over-
            // extension on pathological positions isn't crowded out
            // by competing extensions firing elsewhere. Don't try to
            // simplify by removing one — the per-extension results
            // are misleading.
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

            self.path_keys.pop();
            pos.undo_move(mv, state);

            if self.is_aborted() {
                return Value::ZERO;
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
                            for q in &quiets_tried {
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
                            for q in &quiets_tried {
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
                        for cap in &captures_tried {
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

    // ------------------------------------------------------------------
    // Quiescence
    // ------------------------------------------------------------------

    fn qsearch(
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
                evaluate_with_pawn_cache(pos, self.pawn_cache, self.eval_mask)
            }
        } else if parent_was_null {
            let parent_raw = self.stack[STACK_SENTINEL + ply - 1].raw_static_eval;
            Value(-parent_raw.0 + 2 * crate::eval::TEMPO.0)
        } else {
            evaluate_with_pawn_cache(pos, self.pawn_cache, self.eval_mask)
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
        let mut picker = MovePicker::new_qs(pos, tt_move, qs_picker_depth, recapture_square, cont_keys);
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

    // ------------------------------------------------------------------
    // Helpers
    // ------------------------------------------------------------------

    fn update_pv(&mut self, ply: usize, mv: Move) {
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
    fn contempt_for_pov(&self, curr_stm: Color) -> i32 {
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
    fn search_eval(&mut self, pos: &Position) -> Value {
        let raw = evaluate_with_pawn_cache(pos, self.pawn_cache, self.eval_mask);
        Value(raw.0 + self.contempt_for_pov(pos.side_to_move()))
    }

    /// Apply contempt to a raw eval pulled from the TT. Mirrors
    /// [`search_eval`] for already-computed values.
    fn apply_contempt(&self, raw: Value, pos: &Position) -> Value {
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
    fn draw_value(&self, depth: i32, pos: &Position) -> Value {
        let contempt = self.contempt_for_pov(pos.side_to_move());
        let jitter = if depth < DRAW_JITTER_MIN_DEPTH {
            0
        } else {
            2 * (self.nodes & 1) as i32 - 1
        };
        Value(contempt + jitter)
    }

    fn is_repetition(&self, pos: &Position) -> bool {
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
    fn is_aborted(&self) -> bool {
        self.stop_flag.load(Ordering::Relaxed)
    }

    fn check_should_stop(&mut self) -> bool {
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
    fn trace_along_pv(&self, pos: &mut Position, pv: &[Move]) -> Vec<EvalTrace> {
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

// =========================================================================
// Settled-ply detection
// =========================================================================

/// Side-to-move at the position reached after playing `ply + 1` moves
/// from a root where `root_stm` was to move. Exposed publicly so the
/// teaching-analysis pipeline (CLI debug renderer, future `MoveAnalysis`
/// assembly) can compute the same alternation without re-deriving it.
///
/// `ply` is a 0-indexed position in a PV: ply 0 is the position reached
/// after the first PV move has been played (so stm has flipped once).
pub fn stm_after_ply(root_stm: crate::types::Color, ply: usize) -> crate::types::Color {
    if ply % 2 == 0 {
        !root_stm
    } else {
        root_stm
    }
}

/// Compute the settled-ply index for a PV's per-ply trace sequence.
///
/// Walks backward from the end of the PV looking for the latest
/// index `i` (≥ 2) where the white-POV score differs from the score
/// **two plies earlier** by at least [`SETTLED_THRESHOLD_CP`]. When
/// such an `i` exists *and* the PV has at least one more ply, we
/// return `i + 1` — the position right after the last shift has
/// fully resolved. When the PV ends mid-shift (the unstable `i` is
/// the leaf), we return `i` itself, since there's no post-resolution
/// trace to land on. When the PV is uniformly quiet, we return 0.
///
/// **Why 2 plies, not 1**: every move temporarily shifts the eval in
/// the mover's favor — their choice is committed but the opponent
/// hasn't responded. Adjacent plies have opposite side-to-move and
/// show the "sawtooth" of these unanswered commitments, routinely
/// 100–300 cp even in quiet positions. Same-side-to-move plies (2
/// apart) represent complete exchanges, so the delta between them
/// reflects what really changed — material swings, positional gains,
/// etc. — not the artificial side-to-move asymmetry.
///
/// **Why land on `i + 1`**: with the 2-ply rule the largest
/// same-side jump often lands on the peak of a mid-exchange position
/// (e.g. white plays Bxe6, ply `i`'s trace shows white temporarily
/// up a bishop, but black's recapture on ply `i + 1` is already
/// part of the PV and restores parity). Consumers that walk the PV
/// up to the settled ply want the *resolved* position, not the
/// peak.
///
/// `root_stm` is the side to move at the PV's root; the helper walks
/// the alternation to pick the right sign for each ply's white-POV
/// normalization.
fn compute_settled_ply(traces: &[EvalTrace], root_stm: crate::types::Color) -> Option<usize> {
    if traces.is_empty() {
        return None;
    }
    if traces.len() == 1 {
        return Some(0);
    }

    let white_pov: Vec<i32> = traces
        .iter()
        .enumerate()
        .map(|(i, t)| t.white_pov_value(stm_after_ply(root_stm, i)).0)
        .collect();

    for i in (2..white_pov.len()).rev() {
        let delta = (white_pov[i] - white_pov[i - 2]).abs();
        if delta >= SETTLED_THRESHOLD_CP {
            // Prefer the post-resolution ply when one exists.
            return if i + 1 < white_pov.len() {
                Some(i + 1)
            } else {
                Some(i)
            };
        }
    }
    Some(0)
}

// =========================================================================
// Tuning helpers
// =========================================================================

fn value_to_tt(v: Value, ply: i32) -> Value {
    if v.0 >= Value::MATE.0 - Value::MAX_PLY {
        Value(v.0 + ply)
    } else if v.0 <= -Value::MATE.0 + Value::MAX_PLY {
        Value(v.0 - ply)
    } else {
        v
    }
}

fn value_from_tt(v: Value, ply: i32) -> Value {
    if v == Value::NONE {
        return Value::NONE;
    }
    if v.0 >= Value::MATE.0 - Value::MAX_PLY {
        Value(v.0 - ply)
    } else if v.0 <= -Value::MATE.0 + Value::MAX_PLY {
        Value(v.0 + ply)
    } else {
        v
    }
}

/// Resolve `stack[ply - offset]` into the cont-hist key tuple
/// `(in_check, was_capture, moved_piece_idx, to_idx)`. The 7-frame
/// sentinel padding makes offset reads up to 6 plies safe even at
/// ply 0 — they return the all-zero sentinel which the cont-hist
/// store treats as "no parent move".
fn cont_key_at(stack: &[StackEntry], ply: usize, offset: usize) -> (bool, bool, u8, u8) {
    let idx = STACK_SENTINEL + ply - offset;
    let e = &stack[idx];
    (e.in_check, e.was_capture, e.moved_piece_idx, e.to_idx)
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
static SF11_REDUCTIONS: std::sync::LazyLock<[i32; 256]> = std::sync::LazyLock::new(|| {
    let mut arr = [0i32; 256];
    for i in 1..256 {
        arr[i] = (24.8 * (i as f64).ln()) as i32;
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
fn lmr_reduction(depth: i32, move_count: usize, improving: bool) -> i32 {
    let d = depth.clamp(0, (SF11_REDUCTIONS.len() - 1) as i32) as usize;
    let mc = move_count.min(SF11_REDUCTIONS.len() - 1);
    let r = SF11_REDUCTIONS[d] * SF11_REDUCTIONS[mc];
    (r + 511) / 1024 + (!improving && r > 1007) as i32
}

fn late_move_prune(depth: i32, move_count: usize, improving: bool) -> bool {
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
fn futility_margin(depth: i32, improving: bool) -> i32 {
    217 * (depth - improving as i32)
}

/// Stockfish 11's `update_continuation_histories`: bumps the
/// `[piece][to]` slot of each parent table referenced by `keys` by
/// `bonus`. Parents for which no real move was played (sentinel slot 0)
/// are skipped — their tables are reserved for "no move" and stay at
/// zero so cont-hist scoring of those plies returns 0.
fn update_cont_histories(
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

fn history_bonus(depth: i32) -> i32 {
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
fn cmp_cont_hist_read(
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

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Engine;
    use crate::types::Square;

    fn search_to_depth(pos: &mut Position, depth: u32) -> SearchLine {
        let mut engine = Engine::new(1);
        let params = SearchParams {
            max_depth: depth,
            ..Default::default()
        };
        let mut lines = engine.search(pos, params);
        assert!(!lines.is_empty(), "search returned no lines");
        lines.remove(0)
    }

    #[test]
    fn search_returns_a_legal_root_move() {
        let mut pos = Position::startpos();
        let line = search_to_depth(&mut pos, 2);
        assert!(!line.pv.is_empty());
        let first = line.pv[0];
        let legal = crate::movegen::pseudo_legal_moves_vec(&pos);
        assert!(legal.contains(&first));
    }

    #[test]
    fn search_finds_mate_in_one() {
        // Classic K+Q mate: white K f6, Q g6, black K h8. White plays
        // Qg7#. The queen is supported by the white king, so black can't
        // capture; g8 and h7 are both covered by the queen.
        let mut pos = Position::from_fen("7k/8/5KQ1/8/8/8/8/8 w - - 0 1").unwrap();
        let line = search_to_depth(&mut pos, 3);
        assert_eq!(line.pv[0], Move::normal(Square::G6, Square::G7));
        assert!(
            line.score.0 >= Value::MATE.0 - Value::MAX_PLY,
            "expected mate score, got {}",
            line.score.0
        );
    }

    #[test]
    fn search_drives_home_kxk_endgame() {
        // White K + Q vs lone black king on the edge. With the KXK
        // evaluator in place, search should find *some* progress-making
        // move rather than shuffling. Specifically: the engine's score
        // should exceed plain queen value at even a modest depth,
        // because PushToEdges / PushClose add ~100–200 on top.
        let mut pos = Position::from_fen("7k/8/5KQ1/8/8/8/8/8 w - - 0 1").unwrap();
        let line = search_to_depth(&mut pos, 4);
        assert!(!line.pv.is_empty());
        assert!(
            line.score.0 > Value::QUEEN_MG.0,
            "KXK endgame should score above raw queen value; got {}",
            line.score.0
        );
    }

    #[test]
    fn search_completes_depth_six_from_startpos() {
        // End-to-end smoke test: the full pruning stack must survive a
        // real opening position at a non-trivial depth. Doesn't assert
        // the best move (that's tuning-sensitive) — just that we get a
        // non-empty PV and a sane score.
        let mut pos = Position::startpos();
        let line = search_to_depth(&mut pos, 6);
        assert!(!line.pv.is_empty());
        assert!(
            line.score.0.abs() < Value::MATE.0 - Value::MAX_PLY,
            "opening eval should not be a mate score, got {}",
            line.score.0
        );
        assert_eq!(line.depth, 6);
    }

    #[test]
    fn search_line_leaf_trace_matches_pv_leaf_static_eval() {
        // After the per-ply refactor, the leaf trace is
        // `ply_traces.last()`. It must still equal a fresh
        // `evaluate_with_trace` at the PV's final position.
        let mut pos = Position::startpos();
        let line = search_to_depth(&mut pos, 3);
        let mut replay = pos.clone();
        let mut states: Vec<StateInfo> = Vec::with_capacity(line.pv.len());
        for mv in &line.pv {
            states.push(replay.do_move(*mv));
        }
        let (_, trace) = evaluate_with_trace(&replay);
        assert_eq!(
            line.ply_traces.last().unwrap(),
            &trace,
            "leaf trace must match a fresh evaluate_with_trace at the PV end"
        );
    }

    #[test]
    fn value_to_from_tt_roundtrip_preserves_non_mate_values() {
        let v = Value(42);
        assert_eq!(value_from_tt(value_to_tt(v, 5), 5), v);
    }

    #[test]
    fn value_to_from_tt_handles_mate_values() {
        let v = Value::mate_in(3);
        assert_eq!(value_from_tt(value_to_tt(v, 3), 3), v);
    }

    #[test]
    fn lmr_reduction_matches_sf11_at_sample_points() {
        // Sample points hand-computed from SF11's formula:
        // `r = Reductions[d] * Reductions[mn]; (r + 511) / 1024 + (!i && r > 1007)`
        // with `Reductions[i] = int(23.4 * ln(i))`.
        //   R[5]=37, R[8]=48, R[10]=53, R[20]=70
        // d=8, mc=5, improving=true:  r=48*37=1776,  (1776+511)/1024 = 2
        assert_eq!(lmr_reduction(8, 5, true), 2);
        // d=10, mc=10, improving=true: r=53*53=2809, (2809+511)/1024 = 3
        assert_eq!(lmr_reduction(10, 10, true), 3);
        // d=20, mc=20, improving=true: r=70*70=4900, (4900+511)/1024 = 5
        assert_eq!(lmr_reduction(20, 20, true), 5);
    }

    #[test]
    fn lmr_reduction_grows_with_depth_and_count() {
        let r_small = lmr_reduction(4, 5, true);
        let r_big = lmr_reduction(10, 20, true);
        assert!(r_big >= r_small);
    }

    #[test]
    fn lmr_reduction_increases_when_not_improving_above_r_gate() {
        // SF11's `!improving && r > 1007` bonus only kicks in once
        // `r = R[d]*R[mn] > 1007`. At (d=10, mc=20) that's
        // 53*70=3710 > 1007, so non-improving adds +1.
        let r_improving = lmr_reduction(10, 20, true);
        let r_not_improving = lmr_reduction(10, 20, false);
        assert_eq!(r_not_improving, r_improving + 1);
    }

    #[test]
    fn history_bonus_respects_butterfly_bound() {
        for d in 1..=20 {
            let b = history_bonus(d);
            assert!((0..=BUTTERFLY_HISTORY_BOUND).contains(&b));
        }
    }

    #[test]
    fn recursion_bails_at_max_ply_without_panicking() {
        // Regression: `pv_length` was sized MAX_PLY and indexed at `ply`
        // before any bail check, and `negamax` had no ply-cap at all.
        // A check-rich position that fed check extensions past MAX_PLY
        // recursion levels crashed with "index out of bounds".
        let tt = TranspositionTable::new(1);
        let mut worker = crate::engine::WorkerState::new();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut search = Search::new(&tt, &mut worker, stop);
        let mut pos = Position::startpos();

        // Both entry points must survive being called at the cap.
        let _ = search.qsearch(&mut pos, -Value::INFINITE, Value::INFINITE, MAX_PLY, 0);
        let _ = search.negamax(
            &mut pos,
            -Value::INFINITE,
            Value::INFINITE,
            1,
            MAX_PLY,
            false,
            false,
            None,
            false,
        );
        // Parent read path: child at MAX_PLY must leave pv_length[MAX_PLY]
        // = 0 so a parent calling update_pv sees an empty child PV.
        assert_eq!(search.pv_length[MAX_PLY], 0);
    }

    #[test]
    fn is_repetition_detects_matches_against_seeded_path_keys() {
        // Direct test of the detection logic: the repetition check
        // compares the current position's key against entries in
        // `path_keys` within the `halfmove_clock` window (positions
        // before the last pawn move / capture can't physically
        // repeat). Seeding `path_keys` with real game history (as
        // `SearchParams::game_history` will do) must make in-tree
        // positions that match that history fire as draws.
        //
        // Using a FEN with halfmove_clock=4 so the scan window
        // actually covers the 2-entry gap between seeded repetitions
        // below. The bit-layout is identical to startpos (the key
        // matches) but the clock honestly reflects "four reversible
        // plies have preceded this position."
        let tt = TranspositionTable::new(1);
        let mut worker = crate::engine::WorkerState::new();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut search = Search::new(&tt, &mut worker, stop);
        let pos =
            Position::from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 4 3").unwrap();

        // Earlier key unrelated to `pos` → not a repetition.
        search.path_keys.clear();
        search.path_keys.push(0xDEAD_BEEF);
        search.path_keys.push(pos.key());
        assert!(!search.is_repetition(&pos));

        // Earlier key equal to `pos.key()` → repetition.
        search.path_keys.clear();
        search.path_keys.push(pos.key());
        search.path_keys.push(0xABCD);
        search.path_keys.push(pos.key());
        assert!(search.is_repetition(&pos));
    }

    #[test]
    fn search_scores_known_repetition_as_draw() {
        // End-to-end: construct a game history where the current
        // position appears twice already (1st in game start, 2nd as the
        // search root). Any move that returns the position to an
        // earlier key is a 3rd occurrence — strictly a draw. With the
        // history seeded, moves in the engine's PV that would cycle
        // back must score as 0 cp.
        //
        // Concrete setup: at the startpos, play Nf3 Nf6 Ng1 Ng8 — four
        // moves that return both sides to the initial position. Feed
        // the engine the keys of every intermediate position as
        // `game_history`, then search. Replaying the knight cycle
        // would detect each of those keys mid-tree and return DRAW.
        // The engine must prefer a non-cycling move (e.g. d4 / e4 /
        // c4), so the score is strictly positive (tempo + whatever the
        // engine normally finds for white).
        let mut pos = Position::startpos();
        let k0 = pos.key();
        pos.do_move(Move::normal(Square::G1, Square::F3));
        let k1 = pos.key();
        pos.do_move(Move::normal(Square::G8, Square::F6));
        let k2 = pos.key();
        pos.do_move(Move::normal(Square::F3, Square::G1));
        let k3 = pos.key();
        pos.do_move(Move::normal(Square::F6, Square::G8));
        // After the cycle we're back at the startpos bit-layout, but
        // `halfmove_clock == 4` now — exactly what the bounded
        // repetition scan needs to see the seeded history. (Undoing
        // back to startpos here would reset hmc to 0 and the scan
        // would never look far enough back to find the repeats.)
        assert_eq!(pos.key(), k0);
        assert_eq!(pos.halfmove_clock(), 4);

        let game_history = vec![k0, k1, k2, k3];

        let mut engine = Engine::new(1);
        let lines = engine.search(
            &mut pos,
            SearchParams {
                max_depth: 4,
                game_history,
                ..Default::default()
            },
        );
        let line = lines.into_iter().next().expect("search returned no lines");
        // Top move must not be Nf3 — that immediately lands on `k1`
        // which is in game history (→ draw by repetition).
        assert_ne!(
            line.pv[0],
            Move::normal(Square::G1, Square::F3),
            "engine should avoid Nf3 when game history makes it a repetition draw"
        );
        // And the score should be positive — the engine found a non-
        // drawing continuation.
        assert!(
            line.score.0 > 0,
            "expected a positive score with a non-repeating continuation, got {}",
            line.score.0
        );
    }

    // ---- MultiPV ----------------------------------------------------

    fn multi_pv_search(pos: &mut Position, depth: u32, multi_pv: usize) -> Vec<SearchLine> {
        let mut engine = Engine::new(1);
        engine.search(
            pos,
            SearchParams {
                max_depth: depth,
                multi_pv,
                ..Default::default()
            },
        )
    }

    #[test]
    fn multi_pv_returns_requested_number_of_lines_from_startpos() {
        // 20 legal moves at the start; asking for 3 must return 3.
        let mut pos = Position::startpos();
        let lines = multi_pv_search(&mut pos, 4, 3);
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn multi_pv_lines_are_sorted_by_score_descending() {
        let mut pos = Position::startpos();
        let lines = multi_pv_search(&mut pos, 4, 5);
        assert_eq!(lines.len(), 5);
        for pair in lines.windows(2) {
            assert!(
                pair[0].score >= pair[1].score,
                "MultiPV must be sorted desc: {:?} then {:?}",
                pair[0].score,
                pair[1].score
            );
        }
    }

    #[test]
    fn multi_pv_first_moves_are_distinct() {
        // Every PV slot is claimed by a distinct root move — no slot
        // ever duplicates another's first move.
        let mut pos = Position::startpos();
        let lines = multi_pv_search(&mut pos, 4, 5);
        let firsts: Vec<Move> = lines.iter().map(|l| l.pv[0]).collect();
        for i in 0..firsts.len() {
            for j in (i + 1)..firsts.len() {
                assert_ne!(
                    firsts[i],
                    firsts[j],
                    "PVs #{} and #{} share first move {:?}",
                    i + 1,
                    j + 1,
                    firsts[i]
                );
            }
        }
    }

    #[test]
    fn multi_pv_clamps_to_legal_move_count() {
        // King + king + queen endgame — very few legal moves for black.
        // White played Qg7+, black's king on h8 is in check. Let's use
        // a position where there are just 2 legal replies but we ask
        // for 10.
        let mut pos = Position::from_fen("7k/6Q1/5K2/8/8/8/8/8 b - - 0 1").unwrap();
        // Black's king can step to g8 (attacked by K) — actually let's
        // not overthink: use a slightly-more-constrained position.
        let legal_count = crate::movegen::legal_moves_vec(&mut pos).len();
        let lines = multi_pv_search(&mut pos, 3, 10);
        assert_eq!(
            lines.len(),
            legal_count,
            "MultiPV should clamp to legal-move count ({} legal moves)",
            legal_count
        );
    }

    #[test]
    fn multi_pv_returns_empty_on_terminal_position() {
        // Fool's-mate-style position: black king checkmated, it's
        // black to move, no legal moves. Return empty.
        //
        // Position: white queen on g7 (protected by Kg6), black king h8.
        // Actually simpler: known checkmate FEN.
        let mut pos = Position::from_fen("7k/5KQ1/8/8/8/8/8/8 b - - 0 1").unwrap();
        let legal = crate::movegen::legal_moves_vec(&mut pos);
        assert!(
            legal.is_empty(),
            "precondition: test FEN must be a terminal position"
        );
        let lines = multi_pv_search(&mut pos, 3, 5);
        assert!(lines.is_empty(), "terminal position should yield 0 PVs");
    }

    #[test]
    fn multi_pv_first_line_matches_single_pv_first_line() {
        // Whether the caller asked for 1 PV or 5, the leading line
        // should agree on the best move. Note: this property is
        // approximate at shallow depths because MultiPV's slot-1..N
        // work at earlier IDS depths leaves extra TT entries that
        // single-PV never produces, and pruning changes (reverse-
        // futility, statScore-driven LMR, NMP gating, CMP, ProbCut,
        // …) can amplify that small state difference into a
        // different move. The test uses a 1 MB TT (high collision
        // rate, amplifies sensitivity); the CLI's much larger
        // default TT typically converges at lower depths than this
        // test requires. Each time a new pruning feature lands the
        // convergence depth bumps; the test sits one step *above*
        // the divergence boundary, not at it. History: depth 4 →
        // 8 (reverse-futility) → 11 (statScore-LMR) → 13 (ProbCut)
        // → 14 (extension refinements).
        let mut pos = Position::startpos();
        let single = multi_pv_search(&mut pos, 14, 1);
        let multi = multi_pv_search(&mut pos, 14, 5);
        assert!(!single.is_empty());
        assert!(!multi.is_empty());
        assert_eq!(
            single[0].pv[0], multi[0].pv[0],
            "MultiPV slot 0's first move must match single-PV"
        );
    }

    #[test]
    fn multi_pv_one_is_backwards_compatible_with_pre_refactor() {
        // Historical contract: multi_pv=1 returns exactly one line for
        // a non-terminal position, and its shape (non-empty PV,
        // non-mate score at a shallow depth) matches what the old
        // single-PV path returned.
        let mut pos = Position::startpos();
        let lines = multi_pv_search(&mut pos, 4, 1);
        assert_eq!(lines.len(), 1);
        let line = &lines[0];
        assert!(!line.pv.is_empty());
        assert!(
            line.score.0.abs() < Value::MATE.0 - Value::MAX_PLY,
            "opening eval shouldn't be mate, got {}",
            line.score.0
        );
    }

    // ---- Per-ply traces + settled-ply ----------------------------------

    #[test]
    fn ply_traces_length_matches_pv_length() {
        let mut pos = Position::startpos();
        let line = search_to_depth(&mut pos, 4);
        assert!(!line.pv.is_empty());
        assert_eq!(
            line.ply_traces.len(),
            line.pv.len(),
            "ply_traces must have exactly one entry per PV move"
        );
    }

    #[test]
    fn ply_traces_agree_with_replay_at_each_index() {
        // For each index i, ply_traces[i] must match a fresh
        // evaluate_with_trace at the position reached by replaying
        // pv[0..=i]. Catches off-by-one errors in the walk.
        let mut pos = Position::startpos();
        let line = search_to_depth(&mut pos, 3);
        let mut replay = pos.clone();
        for (i, mv) in line.pv.iter().enumerate() {
            replay.do_move(*mv);
            let (_, expected) = evaluate_with_trace(&replay);
            assert_eq!(
                line.ply_traces[i], expected,
                "ply_traces[{}] must match a fresh evaluate_with_trace at that ply",
                i
            );
        }
    }

    #[test]
    fn settled_ply_none_on_terminal_position() {
        // Checkmate from black's side: no legal moves, so no PV, so no
        // settled-ply to report.
        let mut pos = Position::from_fen("7k/5KQ1/8/8/8/8/8/8 b - - 0 1").unwrap();
        let lines = multi_pv_search(&mut pos, 3, 1);
        assert!(lines.is_empty());
    }

    #[test]
    fn settled_ply_zero_when_single_move_pv() {
        // Constructed scenario: if the PV has length 1, there's no
        // adjacent delta to evaluate, so settled_ply == 0 trivially.
        // Direct unit-level check of the helper.
        use crate::types::Color;
        let trace = EvalTrace::zero();
        let result = compute_settled_ply(&[trace], Color::White);
        assert_eq!(result, Some(0));
    }

    #[test]
    fn settled_ply_none_when_no_traces() {
        use crate::types::Color;
        let result = compute_settled_ply(&[], Color::White);
        assert_eq!(result, None);
    }

    #[test]
    fn settled_ply_zero_when_every_delta_below_threshold() {
        // Hand-constructed trace sequence where the white-POV score
        // barely moves. Must settle at 0 regardless of length.
        use crate::types::{Color, Value};
        let mut traces = Vec::new();
        for i in 0..6 {
            let mut t = EvalTrace::zero();
            // Alternate sign on final_value per ply to mimic
            // side-to-move oscillation. With i % 2 == 0 meaning stm is
            // black, the white-POV converts to -t.final_value + tempo.
            // We want a stable white-POV of ~+5, so:
            //   - even i (black-to-move): final_value = -(5 - TEMPO) = TEMPO - 5.
            //   - odd  i (white-to-move): final_value = 5 + TEMPO.
            let tempo = t.tempo.0;
            let fv = if i % 2 == 0 { tempo - 5 } else { 5 + tempo };
            t.final_value = Value(fv);
            traces.push(t);
        }
        assert_eq!(compute_settled_ply(&traces, Color::White), Some(0));
    }

    /// Build a trace sequence with the given white-POV targets for
    /// each ply, assuming `root_stm == White` (so the stm-after-ply
    /// pattern is Black, White, Black, ...).
    fn traces_from_white_pov(targets_white_pov: &[i32]) -> Vec<EvalTrace> {
        use crate::types::Value;
        let tempo = EvalTrace::zero().tempo.0;
        targets_white_pov
            .iter()
            .enumerate()
            .map(|(i, &w)| {
                let mut t = EvalTrace::zero();
                // Even i → stm is black (root is White, flipped once).
                // White-POV w means stm_unsigned = -w, and final_value
                // = stm_unsigned + tempo = -w + tempo.
                // Odd i → stm is white. final_value = w + tempo.
                let fv = if i % 2 == 0 { -w + tempo } else { w + tempo };
                t.final_value = Value(fv);
                t
            })
            .collect()
    }

    #[test]
    fn settled_ply_filters_the_single_ply_sawtooth() {
        // A canonical sawtooth: alternating 20/300 white-POV values
        // with every 1-ply delta huge (280 cp) but every 2-ply delta
        // exactly zero. Must settle at 0 — the eval is actually stable
        // across complete exchanges, the 1-ply swings are just the
        // "I moved but you haven't responded yet" asymmetry.
        use crate::types::Color;
        let traces = traces_from_white_pov(&[20, 300, 20, 300, 20, 300]);
        assert_eq!(compute_settled_ply(&traces, Color::White), Some(0));
    }

    #[test]
    fn settled_ply_detects_two_ply_shift_on_top_of_sawtooth() {
        // Same sawtooth as above for the first four plies, then a
        // 180-cp lift: ply 4 = 200 (same side as ply 2 = 20, diff
        // 180), ply 5 = 480 (same side as ply 3 = 300, diff 180).
        // Under 2-ply comparison both plies 4 and 5 show big deltas
        // against their same-side predecessor; scanning backward
        // finds the last unstable at ply 5. PV ends mid-shift (no
        // post-resolution ply available), so we land on 5 itself.
        use crate::types::Color;
        let traces = traces_from_white_pov(&[20, 300, 20, 300, 200, 480]);
        assert_eq!(compute_settled_ply(&traces, Color::White), Some(5));
    }

    #[test]
    fn settled_ply_lands_on_post_resolution_when_available() {
        // Mid-exchange peak modelled on the Nf3 → Bxe6 fxe6 scenario:
        // ply 4 (white-side) shows white temporarily up a bishop
        // (white_pov 950, up from 50 two plies back — 900 cp jump,
        // unstable). Ply 5 (black-side post-recapture) restores
        // parity (white_pov 60, 10 cp from ply 3's 50 — stable).
        // Walking backward, the loop finds ply 4 unstable; with a
        // post-resolution ply available (5), settle there rather
        // than on the peak (4).
        use crate::types::Color;
        let traces = traces_from_white_pov(&[0, 0, 50, 50, 950, 60]);
        assert_eq!(compute_settled_ply(&traces, Color::White), Some(5));
    }

    #[test]
    fn settled_ply_reports_zero_on_short_pv_below_two_plies() {
        // A 2-ply trace sequence cannot use the 2-ply comparison
        // (there's no index >= 2). Settles trivially at 0.
        use crate::types::Color;
        let traces = traces_from_white_pov(&[0, 100]);
        assert_eq!(compute_settled_ply(&traces, Color::White), Some(0));
    }

    #[test]
    fn settled_ply_on_live_search_is_within_bounds() {
        // End-to-end: on a real search, settled_ply must be a valid
        // index into ply_traces (if Some) or None for an empty PV.
        let mut pos = Position::startpos();
        let lines = multi_pv_search(&mut pos, 4, 2);
        for line in &lines {
            match line.settled_ply {
                Some(i) => assert!(
                    i < line.ply_traces.len(),
                    "settled_ply {} out of bounds (ply_traces len {})",
                    i,
                    line.ply_traces.len()
                ),
                None => assert!(line.pv.is_empty()),
            }
        }
    }

    // ---- force_include ------------------------------------------------

    /// Helper: run a search with forced moves and return the resulting lines.
    fn search_with_forced(
        pos: &mut Position,
        depth: u32,
        multi_pv: usize,
        forced: Vec<Move>,
    ) -> Vec<SearchLine> {
        let mut engine = Engine::new(1);
        engine.search(
            pos,
            SearchParams {
                max_depth: depth,
                multi_pv,
                force_include: forced,
                ..Default::default()
            },
        )
    }

    /// Find a legal move that the search definitely won't pick in the
    /// top-k at a given depth. We take the last legal move in the
    /// generated order — from startpos, that's typically a rook or
    /// knight retreat that can't possibly be best.
    fn pick_uninteresting_move(pos: &mut Position) -> Move {
        let legal = crate::movegen::legal_moves_vec(pos);
        *legal.last().expect("startpos must have legal moves")
    }

    #[test]
    fn force_include_empty_matches_plain_multi_pv() {
        // Empty force_include vector must be a no-op.
        let mut pos = Position::startpos();
        let plain = multi_pv_search(&mut pos, 4, 3);
        let forced = search_with_forced(&mut pos, 4, 3, Vec::new());
        assert_eq!(plain.len(), forced.len());
        for (p, f) in plain.iter().zip(forced.iter()) {
            assert_eq!(p.pv[0], f.pv[0], "first-move ordering must match");
        }
    }

    #[test]
    fn force_include_adds_out_of_top_k_move() {
        // Take a startpos move that will not naturally appear in top-3
        // (the last-generated legal move, usually a knight moving to a
        // passive square) and force it into the output.
        let mut pos = Position::startpos();
        let victim = pick_uninteresting_move(&mut pos);

        let plain = multi_pv_search(&mut pos, 4, 3);
        let natural_first_moves: Vec<Move> = plain.iter().map(|l| l.pv[0]).collect();
        assert!(
            !natural_first_moves.contains(&victim),
            "test setup: victim must NOT naturally be in top-3; \
             if this fires, pick a different victim"
        );

        let forced = search_with_forced(&mut pos, 4, 3, vec![victim]);
        let forced_first_moves: Vec<Move> = forced.iter().map(|l| l.pv[0]).collect();
        assert!(
            forced_first_moves.contains(&victim),
            "forced move must appear in output; got {:?}",
            forced_first_moves
        );
    }

    #[test]
    fn force_include_forced_slot_has_valid_score_and_pv() {
        // The forced slot must produce a real score (not -INFINITE) and
        // a PV of length > 1 at depth >= 2 — i.e. the search actually
        // ran, didn't just stub out a one-move PV.
        let mut pos = Position::startpos();
        let victim = pick_uninteresting_move(&mut pos);

        let forced = search_with_forced(&mut pos, 3, 1, vec![victim]);
        let slot = forced
            .iter()
            .find(|l| l.pv[0] == victim)
            .expect("forced move must appear");
        assert_ne!(
            slot.score,
            -Value::INFINITE,
            "forced slot must have real score"
        );
        assert!(slot.pv.len() > 1, "forced PV must extend past ply 1");
        assert_eq!(
            slot.ply_traces.len(),
            slot.pv.len(),
            "forced slot's ply_traces must align with its PV length"
        );
    }

    #[test]
    fn force_include_skips_move_already_in_top_k() {
        // Forcing the natural best move should be a no-op — the output
        // shouldn't have a duplicate of the best move.
        let mut pos = Position::startpos();
        let plain = multi_pv_search(&mut pos, 3, 2);
        let natural_best = plain[0].pv[0];

        let forced = search_with_forced(&mut pos, 3, 2, vec![natural_best]);
        let duplicates = forced.iter().filter(|l| l.pv[0] == natural_best).count();
        assert_eq!(duplicates, 1, "natural best must appear exactly once");
        assert_eq!(forced.len(), plain.len(), "output size must not grow");
    }

    #[test]
    fn force_include_ignores_illegal_moves_silently() {
        // A move that isn't legal at the root (e.g. Move::NONE, or a
        // fabricated move from a wrong-color piece) must be silently
        // dropped — not crash, not return anything extra.
        let mut pos = Position::startpos();
        let plain = multi_pv_search(&mut pos, 3, 2);
        let forced = search_with_forced(&mut pos, 3, 2, vec![Move::NONE]);
        assert_eq!(forced.len(), plain.len());
    }

    #[test]
    fn force_include_deduplicates_within_its_list() {
        // The same forced move listed twice should still produce only
        // one extra output row.
        let mut pos = Position::startpos();
        let victim = pick_uninteresting_move(&mut pos);
        let forced = search_with_forced(&mut pos, 3, 2, vec![victim, victim, victim]);
        let victim_count = forced.iter().filter(|l| l.pv[0] == victim).count();
        assert_eq!(
            victim_count, 1,
            "duplicate forced moves must dedup to one slot"
        );
    }

    #[test]
    fn force_include_multiple_distinct_moves_all_appear() {
        // Force in two distinct out-of-top-k moves; both must show.
        let mut pos = Position::startpos();
        let legal = crate::movegen::legal_moves_vec(&mut pos);
        // Take two tail moves that we expect to be out of top-1.
        let v1 = legal[legal.len() - 1];
        let v2 = legal[legal.len() - 2];

        let forced = search_with_forced(&mut pos, 3, 1, vec![v1, v2]);
        let first_moves: Vec<Move> = forced.iter().map(|l| l.pv[0]).collect();
        assert!(
            first_moves.contains(&v1),
            "v1 must appear: {:?}",
            first_moves
        );
        assert!(
            first_moves.contains(&v2),
            "v2 must appear: {:?}",
            first_moves
        );
    }

    #[test]
    fn force_include_output_is_sorted_by_score_descending() {
        // After the final sort, the whole output (natural + forced)
        // should be monotonically non-increasing in score.
        let mut pos = Position::startpos();
        let victim = pick_uninteresting_move(&mut pos);
        let forced = search_with_forced(&mut pos, 4, 3, vec![victim]);
        for pair in forced.windows(2) {
            assert!(
                pair[0].score.0 >= pair[1].score.0,
                "output must be sorted descending by score; got {} then {}",
                pair[0].score.0,
                pair[1].score.0,
            );
        }
    }

    #[test]
    fn force_include_preserves_natural_top_k() {
        // Forcing an extra move must not change which moves appear in
        // the natural top-k. (They may be reordered by the final sort,
        // but the SET of moves covering the natural top positions
        // plus the forced move should equal natural top-k ∪ {forced}.)
        let mut pos = Position::startpos();
        let victim = pick_uninteresting_move(&mut pos);
        let plain = multi_pv_search(&mut pos, 4, 2);
        let plain_moves: std::collections::HashSet<Move> = plain.iter().map(|l| l.pv[0]).collect();

        let forced = search_with_forced(&mut pos, 4, 2, vec![victim]);
        let forced_moves: std::collections::HashSet<Move> =
            forced.iter().map(|l| l.pv[0]).collect();

        // Everything natural is preserved; plus the victim is now in.
        for m in &plain_moves {
            assert!(
                forced_moves.contains(m),
                "natural move disappeared after force_include"
            );
        }
        assert!(forced_moves.contains(&victim));
    }
}
