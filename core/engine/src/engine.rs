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

use crate::eval::EvalTrace;
use crate::movepick::{ButterflyHistory, CaptureHistory, ContHistStore, CounterMoveTable};
use crate::pawns;
use crate::position::Position;
use crate::search::Search;
use crate::tt::{TranspositionTable, DEFAULT_TT_MB};
use crate::types::{Move, Value};

use std::time::{Duration, Instant};

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
    history: ButterflyHistory,
    counter_moves: CounterMoveTable,
    /// Stockfish-style continuation history: ~8 MB on the heap,
    /// partitioned by `[in_check][was_capture]` of the parent move.
    /// Persisted across moves of the same game so move ordering
    /// learning compounds; cleared in [`Engine::new_game`].
    cont_history: ContHistStore,
    /// Capture-history table (~16 KB) used as a tiebreaker on top of
    /// MVV-LVA when ordering good captures. Persists across moves of
    /// the same game.
    capture_history: CaptureHistory,
    pawn_cache: pawns::Table,
    /// Diagnostic stats from the most recent [`Engine::search`] call.
    /// Both fields are zero before any search has run.
    last_nodes: u64,
    last_elapsed: Duration,
}

impl Engine {
    /// Build an engine backed by a transposition table of (at most)
    /// `tt_size_mb` megabytes. Size is rounded down to a whole number of
    /// TT clusters.
    pub fn new(tt_size_mb: usize) -> Engine {
        Engine {
            tt: TranspositionTable::new(tt_size_mb),
            history: ButterflyHistory::new(),
            counter_moves: CounterMoveTable::new(),
            cont_history: ContHistStore::new(),
            capture_history: CaptureHistory::new(),
            pawn_cache: pawns::Table::new(),
            last_nodes: 0,
            last_elapsed: Duration::ZERO,
        }
    }

    /// Clear everything that accumulated across prior searches: TT,
    /// butterfly history, counter-move table, continuation history,
    /// capture history, pawn cache. Call between games so learning
    /// from game N doesn't pollute move ordering in game N+1.
    pub fn new_game(&mut self) {
        self.tt.clear();
        self.history.clear();
        self.counter_moves.clear();
        self.cont_history.clear();
        self.capture_history.clear();
        self.pawn_cache.clear();
    }

    /// Run a search and return at most `params.multi_pv` ranked lines,
    /// sorted by score descending. The provided position is mutated
    /// during the search (do/undo) but is always restored to its
    /// original state before returning. An empty vector indicates a
    /// terminal root position (checkmate or stalemate).
    pub fn search(&mut self, pos: &mut Position, params: SearchParams) -> Vec<SearchLine> {
        let started = Instant::now();
        let mut search = Search::new(
            &self.tt,
            &mut self.history,
            &mut self.counter_moves,
            &mut self.cont_history,
            &mut self.capture_history,
            &mut self.pawn_cache,
        );
        let lines = search.run(pos, &params);
        self.last_nodes = search.node_count();
        self.last_elapsed = started.elapsed();
        lines
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
    /// Remove once we've used the data to decide on cache sizing.
    pub fn pawn_cache_stats(&self) -> (u64, u64) {
        self.pawn_cache.stats()
    }

    /// TEMPORARY (perf investigation): zero the pawn-cache hit/miss
    /// counters so the next search reports fresh numbers.
    pub fn reset_pawn_cache_stats(&mut self) {
        self.pawn_cache.reset_stats();
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
        e.history.update(
            crate::types::Color::White,
            crate::types::Square::E2,
            crate::types::Square::E4,
            1000,
        );
        assert_ne!(
            e.history.get(
                crate::types::Color::White,
                crate::types::Square::E2,
                crate::types::Square::E4
            ),
            0
        );
        e.new_game();
        assert_eq!(
            e.history.get(
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
