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

use crate::types::Value;


use std::sync::atomic::AtomicBool;
use std::sync::Arc;

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

mod loop_helpers;
mod move_loop;
mod move_search;
mod negamax;
mod pre_loop;
mod qsearch;
mod run;
mod settled;
mod state;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

// External API surface (preserve `crate::search::X` paths).
pub use settled::stm_after_ply;

// Re-export submodule items as crate-internal so the sibling submodules and
// the test module resolve them via `use super::*`. Glob re-exports (not
// explicit lists) so `cargo fix` won't trim a re-export that the lib reaches
// only through `super::*` (e.g. a helper used in one sibling and the tests).
pub(crate) use loop_helpers::*;
pub(crate) use settled::*;
pub(crate) use state::*;
