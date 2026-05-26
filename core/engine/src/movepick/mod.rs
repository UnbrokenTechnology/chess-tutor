//! Move picker — staged move ordering for alpha-beta search.
//!
//! The picker hands moves to search one at a time in an order designed to
//! produce fast beta-cutoffs: transposition-table move first, then winning
//! captures (SEE ≥ 0, sorted by MVV-LVA), then killer moves, then quiet
//! moves sorted by history, then losing captures last. When the side to
//! move is in check it follows a separate "evasion" pipeline. Quiescence
//! search uses its own shorter pipeline that returns captures only
//! (optionally restricted to a recapture square at the deepest qs plies).
//!
//! MVP scope is deliberately narrower than Stockfish 11's: we keep the
//! main stage structure but drop continuation-history and counter-move
//! heuristics, leaving butterfly history as the only quiet-scoring term.
//! These additions slot in later without changing the public surface.
//!
//! Every returned move is *pseudo-legal* (including the TT move, which is
//! validated on construction). The search is responsible for filtering
//! legality via do/undo — the picker never mutates the position.

mod helpers;
mod history;
mod picker;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

pub use history::*;

use crate::movegen::MAX_MOVES;
use crate::types::{Depth, Move, Square};
use std::cell::RefCell;
use std::mem::MaybeUninit;
use std::ops::{Deref, DerefMut};

// =========================================================================
// Internal types
// =========================================================================

#[derive(Copy, Clone, Debug)]
struct ScoredMove {
    mv: Move,
    score: i32,
}

/// All four scored-move buffers used by a single [`MovePicker`], packed
/// together so the pool checks them out as one heap allocation.
struct MoveBufs {
    captures: [ScoredMove; MAX_MOVES],
    bad_captures: [ScoredMove; MAX_MOVES],
    quiets: [ScoredMove; MAX_MOVES],
    evasions: [ScoredMove; MAX_MOVES],
}

impl MoveBufs {
    /// Allocate a fresh `MoveBufs` directly on the heap. Avoids the stack
    /// blow-up of `Box::new(MoveBufs { ... })` (which would materialize the
    /// ~32 KB struct on the stack first) by zero-initializing in place.
    /// Sound because the all-zero bit pattern is a valid `ScoredMove`:
    /// `Move::NONE = Move(0)` and `score: i32` zero.
    fn new_boxed() -> Box<MoveBufs> {
        let mut b: Box<MaybeUninit<MoveBufs>> = Box::new_uninit();
        unsafe {
            std::ptr::write_bytes(b.as_mut_ptr(), 0u8, 1);
            b.assume_init()
        }
    }
}

thread_local! {
    /// Per-thread pool of [`MoveBufs`]. [`MovePicker`] checks one out on
    /// construction and returns it on `Drop`, so steady-state use is
    /// zero-alloc: the pool grows to ≈ recursion depth (typically 30–50
    /// at the search peak) and stays at that high-water mark for the
    /// duration of the thread.
    static MOVE_BUFS_POOL: RefCell<Vec<Box<MoveBufs>>> = const { RefCell::new(Vec::new()) };
}

fn checkout_move_bufs() -> Box<MoveBufs> {
    MOVE_BUFS_POOL.with(|p| p.borrow_mut().pop().unwrap_or_else(MoveBufs::new_boxed))
}

fn return_move_bufs(bufs: Box<MoveBufs>) {
    MOVE_BUFS_POOL.with(|p| p.borrow_mut().push(bufs));
}

/// Fixed-capacity scored-move buffer view. Stores a raw pointer into a
/// [`MoveBufs`] checkout (heap-allocated, see [`MovePicker::bufs`]) plus
/// a populated-prefix length. The pointer is valid for as long as the
/// owning `MovePicker` keeps its `bufs` Box alive — moves of the
/// `MovePicker` don't invalidate it because the heap allocation it
/// references doesn't move with the Box pointer.
///
/// `Deref<Target = [ScoredMove]>` exposes `len`, indexing, `swap`,
/// iteration, etc. against the populated prefix `[..len]`, so the
/// internal call sites that previously held a `Vec<ScoredMove>` field
/// stay unchanged.
struct MoveBuf {
    storage: *mut [ScoredMove; MAX_MOVES],
    len: usize,
}

impl MoveBuf {
    /// SAFETY: `storage` must point to a `[ScoredMove; MAX_MOVES]` that
    /// outlives the constructed `MoveBuf` and is not aliased by any
    /// other `MoveBuf` for the duration.
    #[inline]
    unsafe fn from_storage(storage: *mut [ScoredMove; MAX_MOVES]) -> Self {
        Self { storage, len: 0 }
    }

    #[inline]
    fn push(&mut self, m: ScoredMove) {
        debug_assert!(self.len < MAX_MOVES, "MoveBuf overflow");
        // SAFETY: `storage` is valid by the constructor's invariant; the
        // index is in-bounds by the assert.
        unsafe {
            (&mut *self.storage)[self.len] = m;
        }
        self.len += 1;
    }

    #[inline]
    fn clear(&mut self) {
        self.len = 0;
    }
}

impl Deref for MoveBuf {
    type Target = [ScoredMove];
    #[inline]
    fn deref(&self) -> &[ScoredMove] {
        // SAFETY: storage is valid by the constructor's invariant; the
        // prefix length is bounded by `MAX_MOVES` via `push`'s assert.
        unsafe { &(&*self.storage)[..self.len] }
    }
}

impl DerefMut for MoveBuf {
    #[inline]
    fn deref_mut(&mut self) -> &mut [ScoredMove] {
        // SAFETY: as above.
        unsafe { &mut (&mut *self.storage)[..self.len] }
    }
}

/// The sorting threshold Stockfish uses: quiets with a score below
/// `-3000 * depth` are left unordered in the tail. Depth here is the
/// remaining alpha-beta depth, so deeper searches demand a tighter cutoff.
const QUIET_SORT_BASE: i32 = -3000;

/// Evasion quiets are pushed below evasion captures by subtracting this
/// large constant. Matches Stockfish 11's scoring.
const EVASION_QUIET_PENALTY: i32 = 1 << 28;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Stage {
    MainTt,
    CaptureInit,
    GoodCapture,
    Killer0,
    Killer1,
    CounterMove,
    QuietInit,
    Quiet,
    BadCapture,

    EvasionTt,
    EvasionInit,
    Evasion,

    QSearchTt,
    QCaptureInit,
    QCapture,

    Done,
}

// =========================================================================
// MovePicker
// =========================================================================

/// Staged pseudo-legal move picker. Consume with
/// `next_move(pos, skip_quiets)` in a loop until it returns `Move::NONE`;
/// never call again after that. The position is threaded through each
/// call rather than held as a field so search code can freely
/// `do_move`/`undo_move` between `next_move`s without borrow conflicts.
/// Callers must pass the *same* position (by value equality, not
/// identity) on every call — the picker's buffers assume generation
/// happened against that position.
/// Continuation-history lookup keys for the four ply offsets Stockfish
/// 11 reads at quiet scoring time (1, 2, 4, 6 plies ago). Each entry is
/// `(in_check, was_capture, parent_piece_idx, parent_to_idx)`. A
/// sentinel "no parent" slot uses `parent_piece_idx = 0`, which selects
/// a sub-table that is never updated and reads as all zeros.
pub type ContHistKeys = [(bool, bool, u8, u8); 4];

/// Sentinel keys for callers that have no parent move (qsearch entry,
/// root, regression tests). Reads from this set always score zero.
pub const NO_CONT_HIST: ContHistKeys = [(false, false, 0, 0); 4];

pub struct MovePicker {
    tt_move: Move,
    killers: [Move; 2],
    counter_move: Move,
    /// Keys identifying the four parent-move sub-tables to read at
    /// quiet scoring time. Set at construction; resolved against
    /// `&ContinuationHistory` only inside [`MovePicker::generate_quiets`]
    /// so the caller's mutable borrow on the cont-history store can
    /// coexist with the move loop's β-cutoff updates.
    cont_keys: ContHistKeys,
    depth: Depth,
    recapture_square: Option<Square>,
    stage: Stage,
    cur: usize,

    /// Heap-allocated buffer storage, checked out from a thread-local
    /// pool on construction and returned on `Drop`. The four [`MoveBuf`]
    /// views below alias into disjoint fields of this Box's contents;
    /// they remain valid because moving `MovePicker` only moves the Box
    /// pointer, not the heap allocation it references. Always `Some`
    /// for the lifetime of the `MovePicker`; the `Option` exists only so
    /// `Drop` can `take()` ownership without leaving a dangling slot.
    bufs: Option<Box<MoveBufs>>,

    // Populated by `CaptureInit`. Drained during `GoodCapture`; losing
    // captures are shifted into `bad_captures` for later.
    captures: MoveBuf,
    // Losing captures held aside during `GoodCapture`; tried last in the
    // main pipeline.
    bad_captures: MoveBuf,
    // Populated by `QuietInit`; drained during `Quiet` in insertion-sorted
    // order (moves with score >= sort threshold are sorted descending).
    quiets: MoveBuf,
    // Populated by `EvasionInit`; drained during `Evasion` with pick-best.
    evasions: MoveBuf,
}

impl Drop for MovePicker {
    fn drop(&mut self) {
        if let Some(bufs) = self.bufs.take() {
            return_move_bufs(bufs);
        }
    }
}

/// Build the four buffer views that alias into `bufs`. Splits the
/// mutable borrow into disjoint fields, then erases the lifetimes via
/// raw pointers so the resulting [`MoveBuf`]s can be stored alongside
/// the owning `Box` in the same struct without self-referential
/// lifetime gymnastics. Sound because each pointer addresses a distinct
/// field and the Box keeps the heap allocation alive.
fn split_bufs(bufs: &mut MoveBufs) -> (MoveBuf, MoveBuf, MoveBuf, MoveBuf) {
    let captures_ptr = &mut bufs.captures as *mut _;
    let bad_captures_ptr = &mut bufs.bad_captures as *mut _;
    let quiets_ptr = &mut bufs.quiets as *mut _;
    let evasions_ptr = &mut bufs.evasions as *mut _;
    unsafe {
        (
            MoveBuf::from_storage(captures_ptr),
            MoveBuf::from_storage(bad_captures_ptr),
            MoveBuf::from_storage(quiets_ptr),
            MoveBuf::from_storage(evasions_ptr),
        )
    }
}
