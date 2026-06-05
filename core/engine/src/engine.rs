//! The public entry point for consumers of the engine library — the
//! `Engine` struct owns long-lived state (transposition table, history
//! tables) and exposes a single [`Engine::search`] method that runs a
//! search under the constraints in [`SearchParams`] and returns a
//! ranked list of [`SearchLine`]s with per-PV evaluation traces.
//!
//! The struct layout is deliberately small: search-internal scratch
//! (PV tables, killer tables, node counter) lives in a per-search
//! [`crate::search::Search`] so engine-wide state stays shareable for
//! future multi-threaded search. `Engine::search` itself takes `&mut self`
//! only to allow history mutation; the TT is accessed through a shared
//! `&self`.

use crate::endgame::EndgameSkill;
use crate::eval::EvalTrace;
use crate::movepick::{ButterflyHistory, CaptureHistory, ContHistStore, CounterMoveTable};
use crate::opponent::EvalMask;
use crate::pawns;
use crate::position::Position;
use crate::search::{Search, StopFlag};
use crate::tt::{TranspositionTable, DEFAULT_TT_MB};
use crate::types::{Move, Value};

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

// =========================================================================
// Per-thread worker state
// =========================================================================

/// Bundle of per-thread search tables. Each search thread (main +
/// helpers) gets its own [`WorkerState`]; the transposition table is
/// the only thing shared across threads. Persisted across `Engine::
/// search` calls so move-ordering history compounds across moves of
/// the same game (cleared via [`Engine::new_game`]).
#[derive(Clone)]
pub struct WorkerState {
    pub(crate) history: ButterflyHistory,
    pub(crate) counter_moves: CounterMoveTable,
    /// ~8 MB on the heap, partitioned by `[in_check][was_capture]` of
    /// the parent move. Persisted across moves of the same game so
    /// move ordering learning compounds.
    pub(crate) cont_history: ContHistStore,
    /// ~16 KB tiebreaker on top of MVV-LVA when ordering good captures.
    pub(crate) capture_history: CaptureHistory,
    pub(crate) pawn_cache: pawns::Table,
}

impl WorkerState {
    pub(crate) fn new() -> WorkerState {
        WorkerState {
            history: ButterflyHistory::new(),
            counter_moves: CounterMoveTable::new(),
            cont_history: ContHistStore::new(),
            capture_history: CaptureHistory::new(),
            pawn_cache: pawns::Table::new(),
        }
    }

    pub(crate) fn clear(&mut self) {
        self.history.clear();
        self.counter_moves.clear();
        self.cont_history.clear();
        self.capture_history.clear();
        self.pawn_cache.clear();
    }
}

/// Per-search knobs controlling when to stop and how many PVs to return.
/// `Default` yields a sensible interactive setting (depth 10, single PV,
/// no time/node ceiling).
#[derive(Clone, Debug)]
pub struct SearchParams {
    /// Stop after completing this depth (in plies) if reached before any
    /// time/node limit fires. Must be at least 1.
    pub max_depth: u32,
    /// Stop after exploring this many nodes, if `Some`.
    pub max_nodes: Option<u64>,
    /// Stop after this wall-clock duration, if `Some`.
    pub max_time: Option<Duration>,
    /// Number of principal variations to return. `1` yields a single
    /// best line; values > 1 run Stockfish-style per-slot MultiPV and
    /// return that many ranked lines (clamped to the number of legal
    /// moves at the root).
    pub multi_pv: usize,
    /// Zobrist keys of every position reached *before* this search's
    /// root, in the order they were reached. The search seeds its
    /// internal repetition path with these so that a move reaching any
    /// of them inside the tree is treated as a draw by repetition —
    /// without this, the engine cheerfully recommends draws in
    /// positions that are winning but one tempo away from a threefold.
    ///
    /// Do **not** include the root position itself; the search pushes
    /// that separately.
    pub game_history: Vec<u64>,
    /// Root moves that *must* appear in the returned [`SearchLine`]
    /// list, even when they're outside the natural top-`multi_pv`.
    /// Each forced move that isn't already in the top-k gets its own
    /// dedicated single-move IDS pass after the main MultiPV loop
    /// completes, with its own valid score / PV / `ply_traces`.
    ///
    /// The primary use case is **retrospective analysis**: after a
    /// human plays a move, we want to analyze the pre-move position and
    /// compare the human's move against the engine's preferred moves —
    /// but a bad move won't be in the top-k, so without this field it
    /// wouldn't get a real score or PV.
    ///
    /// Entries not legal at the root are silently dropped (no error).
    /// Duplicates and moves already in the natural top-k are deduped.
    pub force_include: Vec<Move>,
    /// Diagnostic knob: when `true`, the search writes progress events
    /// to stderr as it runs — one line per iterative-deepening depth
    /// start/finish and one line per root move as it starts being
    /// searched at the current depth. Intended for investigating slow
    /// or stuck searches; not for normal use (the stderr noise is
    /// substantial). Off by default.
    pub verbose_progress: bool,
    /// How many parallel search threads to run (Stockfish-style Lazy
    /// SMP). The main thread does iterative deepening normally and
    /// returns the result; `threads - 1` helper threads run the same
    /// loop on their own per-thread history tables, contributing to
    /// the shared TT and getting cut off when the main thread
    /// finishes. `1` is the deterministic single-thread path —
    /// required by callers that need bit-identical results across
    /// runs (analytical engine clones, teaching retrospectives).
    pub threads: usize,
    /// Evaluation categories the bot should be "blind" to for this
    /// search. Default [`EvalMask::EMPTY`] runs the standard unbiased
    /// eval. Analytical paths (retrospective, hint, REPL `analyze`)
    /// must keep this `EMPTY` so the student sees true best play in
    /// the feedback layer.
    pub eval_mask: EvalMask,
    /// Quiescence-search horizon cap, in plies of capture resolution.
    /// `None` (default) = full tactical vision (qsearch resolves captures
    /// normally). `Some(n)` limits the bot to `n` plies of capture
    /// resolution before falling back to the static eval — the "tactical
    /// horizon" lever: `Some(0)` makes the bot tactically blind (hangs
    /// pieces, doesn't see recaptures), modelling a sub-600 human.
    /// Play-engine-only, like [`Self::eval_mask`]: analytical paths must
    /// keep this `None` so feedback judges against true best play.
    pub qsearch_max_plies: Option<u32>,

    /// How much closed-form endgame knowledge the bot may use.
    /// [`EndgameSkill::Full`] (the default) consults every specialist;
    /// lower tiers withhold the harder ones so a weak bot misplays
    /// endgames like a human of that level (and, at low tiers, promotes
    /// to a queen instead of the SF-quirk underpromotion). Play-engine-
    /// only, like [`Self::eval_mask`] / [`Self::qsearch_max_plies`].
    pub endgame_skill: EndgameSkill,
}

impl Default for SearchParams {
    fn default() -> Self {
        SearchParams {
            max_depth: 10,
            max_nodes: None,
            max_time: None,
            multi_pv: 1,
            game_history: Vec::new(),
            force_include: Vec::new(),
            verbose_progress: false,
            threads: 1,
            eval_mask: EvalMask::EMPTY,
            qsearch_max_plies: None,
            endgame_skill: EndgameSkill::Full,
        }
    }
}

/// One line of search output — a principal variation, its evaluation,
/// the depth at which it was produced, and an [`EvalTrace`] at every ply
/// of the PV. The trace sequence is the "why" the teaching UI surfaces:
/// the leaf trace tells you what the engine thinks the game is heading
/// toward; the per-ply sequence tells you when that picture settled.
#[derive(Clone, Debug)]
pub struct SearchLine {
    /// Best continuation starting from the root, in the order played.
    /// Empty only when the root position is terminal (checkmate or
    /// stalemate).
    pub pv: Vec<Move>,
    /// Score of the line from the root side-to-move's point of view, in
    /// Stockfish's centipawn-ish scale (see [`Value`]). Mate values use
    /// the `MATE - ply` convention.
    pub score: Value,
    /// Depth (in plies) the search completed at before returning this
    /// line. Useful for "reached depth N" UI hints.
    pub depth: u32,
    /// Per-ply [`EvalTrace`]s along [`pv`]: `ply_traces[i]` is the
    /// evaluation of the position after playing `pv[0..=i]`. The leaf
    /// trace is `ply_traces.last()`. Empty iff `pv` is empty.
    pub ply_traces: Vec<EvalTrace>,
    /// Index into [`ply_traces`] at which the evaluation is deemed to
    /// have "settled" — subsequent plies don't shift the white-POV score
    /// by more than [`crate::search::SETTLED_THRESHOLD_CP`]. `None` when
    /// `pv` is empty. `Some(0)` means the first move already decided
    /// things and the rest is follow-through.
    pub settled_ply: Option<usize>,
}

/// Engine-wide persistent state. Construct once with [`Engine::new`],
/// reuse across searches; call [`Engine::new_game`] between distinct
/// games to clear TT and history learning. `Clone` deep-copies both
/// the TT and history so callers can isolate analytical searches
/// (e.g. CLI `search` / `analyze`) from the play loop's engine.
#[derive(Clone)]
pub struct Engine {
    tt: TranspositionTable,
    /// Per-thread worker state, indexed by thread id (`[0]` is always
    /// the main thread). Grown on demand by [`Engine::ensure_workers`]
    /// when a search asks for more threads than we have. Helper-thread
    /// state persists across calls just like the main thread's, so a
    /// helper that learned a useful history entry in move N can use it
    /// to improve move N+1's move ordering.
    workers: Vec<WorkerState>,
    /// Diagnostic stats from the most recent [`Engine::search`] call.
    /// Both fields are zero before any search has run.
    last_nodes: u64,
    last_elapsed: Duration,
    /// Maximum ply reached during the most recent search (selDepth).
    /// TEMPORARY perf-investigation surface.
    last_seldepth: u32,
    /// Per-ply node histogram from the most recent search. Index = ply
    /// from root. TEMPORARY perf-investigation surface.
    last_nodes_per_ply: Vec<u64>,
}

impl Engine {
    /// Build an engine backed by a transposition table of (at most)
    /// `tt_size_mb` megabytes. Size is rounded down to a whole number of
    /// TT clusters. The worker pool starts with one [`WorkerState`]
    /// (for single-threaded search) and grows lazily when a search
    /// asks for more threads.
    pub fn new(tt_size_mb: usize) -> Engine {
        Engine {
            tt: TranspositionTable::new(tt_size_mb),
            workers: vec![WorkerState::new()],
            last_nodes: 0,
            last_elapsed: Duration::ZERO,
            last_seldepth: 0,
            last_nodes_per_ply: Vec::new(),
        }
    }

    /// Clear everything that accumulated across prior searches: TT and
    /// every worker's history / counter-move / continuation-history /
    /// capture-history / pawn-cache. Call between games so learning
    /// from game N doesn't pollute move ordering in game N+1.
    pub fn new_game(&mut self) {
        self.tt.clear();
        for worker in self.workers.iter_mut() {
            worker.clear();
        }
    }

    /// Ensure the worker pool has at least `n` workers, growing it
    /// with fresh [`WorkerState`]s as needed. New helper-thread state
    /// starts empty (no inherited history); subsequent calls preserve
    /// what each helper has learned.
    fn ensure_workers(&mut self, n: usize) {
        while self.workers.len() < n {
            self.workers.push(WorkerState::new());
        }
    }

    /// Run a search and return at most `params.multi_pv` ranked lines,
    /// sorted by score descending. The provided position is mutated
    /// during the search (do/undo) but is always restored to its
    /// original state before returning. An empty vector indicates a
    /// terminal root position (checkmate or stalemate).
    ///
    /// When `params.threads > 1`, `params.threads - 1` helper threads
    /// run alongside the calling thread (Stockfish-style Lazy SMP):
    /// they share the TT, have their own history tables, and exit
    /// when the main thread finishes. The returned lines come from
    /// the main thread only; helpers contribute via TT writes.
    pub fn search(&mut self, pos: &mut Position, params: SearchParams) -> Vec<SearchLine> {
        let started = Instant::now();
        let n_threads = params.threads.max(1);
        self.ensure_workers(n_threads);

        let stop_flag: StopFlag = Arc::new(AtomicBool::new(false));
        let (main_lines, total_nodes, main_seldepth, main_nodes_per_ply) =
            self.run_threaded(pos, &params, n_threads, stop_flag);

        self.last_nodes = total_nodes;
        self.last_elapsed = started.elapsed();
        self.last_seldepth = main_seldepth;
        self.last_nodes_per_ply = main_nodes_per_ply;
        main_lines
    }

    /// Spawn `n_threads - 1` helper threads and run the main thread's
    /// search on the calling thread. Returns `(main_lines, sum_of_nodes,
    /// main_seldepth, main_nodes_per_ply)`. Helpers run the same
    /// iterative-deepening loop on their own [`WorkerState`] and
    /// contribute to the shared TT; only the main thread's PV / score
    /// are returned, because the main thread is the one tracking
    /// `force_include` and MultiPV ordering.
    fn run_threaded(
        &mut self,
        pos: &mut Position,
        params: &SearchParams,
        n_threads: usize,
        stop_flag: StopFlag,
    ) -> (Vec<SearchLine>, u64, u32, Vec<u64>) {
        // Split the worker pool into one main + many helpers via
        // disjoint mutable references so each thread can mutate its
        // own state without coordination.
        let (main_workers, helper_workers) = self.workers[..n_threads].split_at_mut(1);
        let main_worker = &mut main_workers[0];

        // Single-thread fast path: skip the scope/spawn machinery
        // entirely so deterministic callers pay zero threading
        // overhead.
        if n_threads == 1 {
            let mut search = Search::new(&self.tt, main_worker, stop_flag);
            let lines = search.run(pos, params);
            let nodes = search.node_count();
            let seldepth = search.seldepth();
            let per_ply = search.nodes_per_ply().to_vec();
            return (lines, nodes, seldepth, per_ply);
        }

        let tt = &self.tt;
        std::thread::scope(|scope| {
            // Helpers each get their own position clone, their own
            // worker, and a clone of the params (forced moves and
            // game history are read-only in the search).
            let helper_handles: Vec<_> = helper_workers
                .iter_mut()
                .map(|worker| {
                    let mut local_pos = pos.clone();
                    let local_params = SearchParams {
                        verbose_progress: false,
                        ..params.clone()
                    };
                    let stop_flag = stop_flag.clone();
                    scope.spawn(move || {
                        let mut search = Search::new(tt, worker, stop_flag);
                        // Helpers run for their side effects on the
                        // shared TT; their root_lines are discarded.
                        let _ = search.run(&mut local_pos, &local_params);
                        search.node_count()
                    })
                })
                .collect();

            // Main thread runs on the caller's thread.
            let mut main_search = Search::new(tt, main_worker, stop_flag.clone());
            let main_lines = main_search.run(pos, params);
            let main_nodes = main_search.node_count();
            let main_seldepth = main_search.seldepth();
            let main_per_ply = main_search.nodes_per_ply().to_vec();

            // Signal helpers to stop now that the main thread is
            // done. They check the flag inside their stop-cadence
            // window, so the wait is bounded.
            stop_flag.store(true, std::sync::atomic::Ordering::Relaxed);

            let mut total_nodes = main_nodes;
            for handle in helper_handles {
                total_nodes += handle.join().unwrap_or(0);
            }
            (main_lines, total_nodes, main_seldepth, main_per_ply)
        })
    }

    /// Total node count visited by the most recent [`search`](Self::search)
    /// call (sum across IDS depths and PV slots). Zero before any search
    /// has run.
    pub fn last_nodes(&self) -> u64 {
        self.last_nodes
    }

    /// Wall-clock duration of the most recent [`search`](Self::search)
    /// call. `Duration::ZERO` before any search has run.
    pub fn last_elapsed(&self) -> Duration {
        self.last_elapsed
    }

    /// Maximum ply (selDepth) reached during the most recent search.
    /// TEMPORARY perf-investigation accessor.
    pub fn last_seldepth(&self) -> u32 {
        self.last_seldepth
    }

    /// Per-ply node histogram from the most recent search; index =
    /// recursion depth from root. TEMPORARY perf-investigation
    /// accessor.
    pub fn last_nodes_per_ply(&self) -> &[u64] {
        &self.last_nodes_per_ply
    }

    /// Convenience: nodes per second from the most recent search.
    /// Returns `0.0` if no search has run or the elapsed time is zero
    /// (search returned instantly on a terminal position).
    pub fn last_nps(&self) -> f64 {
        let secs = self.last_elapsed.as_secs_f64();
        if secs > 0.0 {
            self.last_nodes as f64 / secs
        } else {
            0.0
        }
    }

    /// Exposed for diagnostics and tests.
    pub fn tt(&self) -> &TranspositionTable {
        &self.tt
    }

    /// TEMPORARY (perf investigation): pawn-cache hit/miss counts since
    /// engine construction or the last [`reset_pawn_cache_stats`].
    /// Reads the main thread's worker; helper workers' caches aren't
    /// aggregated. Remove once we've used the data to decide on cache
    /// sizing.
    pub fn pawn_cache_stats(&self) -> (u64, u64) {
        self.workers[0].pawn_cache.stats()
    }

    /// TEMPORARY (perf investigation): zero the pawn-cache hit/miss
    /// counters so the next search reports fresh numbers. Resets every
    /// worker's counters so an aggregated read after the reset isn't
    /// pre-populated by helper-thread activity.
    pub fn reset_pawn_cache_stats(&mut self) {
        for worker in self.workers.iter_mut() {
            worker.pawn_cache.reset_stats();
        }
    }
}

impl Default for Engine {
    fn default() -> Engine {
        Engine::new(DEFAULT_TT_MB)
    }
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_new_allocates_tt_and_history() {
        let e = Engine::new(1);
        // cluster_count rounds down: 1 MB / 64 B = 16384 clusters.
        assert!(e.tt.cluster_count() >= 1024);
    }

    #[test]
    fn new_game_clears_state() {
        let mut e = Engine::new(1);
        e.workers[0].history.update(
            crate::types::Color::White,
            crate::types::Square::E2,
            crate::types::Square::E4,
            1000,
        );
        assert_ne!(
            e.workers[0].history.get(
                crate::types::Color::White,
                crate::types::Square::E2,
                crate::types::Square::E4
            ),
            0
        );
        e.new_game();
        assert_eq!(
            e.workers[0].history.get(
                crate::types::Color::White,
                crate::types::Square::E2,
                crate::types::Square::E4
            ),
            0
        );
    }

    #[test]
    fn default_search_params_are_sensible() {
        let p = SearchParams::default();
        assert_eq!(p.max_depth, 10);
        assert_eq!(p.multi_pv, 1);
        assert!(p.max_nodes.is_none());
        assert!(p.max_time.is_none());
    }
}
