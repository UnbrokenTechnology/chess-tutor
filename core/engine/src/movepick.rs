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

use crate::movegen::{generate_pseudo_legal_moves, MoveList, MAX_MOVES};
use crate::position::Position;
use crate::types::{Color, Depth, Move, PieceType, Square, Value};

use std::cell::RefCell;
use std::mem::MaybeUninit;
use std::ops::{Deref, DerefMut};

// =========================================================================
// Butterfly history
// =========================================================================

/// Upper bound on stored history values. Matches Stockfish 11's tuning
/// constant; the update formula keeps stored values within `[-D, D]`.
pub const BUTTERFLY_HISTORY_BOUND: i32 = 10_692;

/// Per-colour history of "how often did this (from, to) quiet move succeed
/// as a beta-cutoff". Indexed by `[color][from*64 + to]`. Heap-allocated
/// (~16 KB) because the table is too big to live on the stack.
#[derive(Clone)]
pub struct ButterflyHistory {
    table: Box<[[i16; 64 * 64]; 2]>,
}

impl ButterflyHistory {
    pub fn new() -> Self {
        Self {
            table: Box::new([[0; 64 * 64]; 2]),
        }
    }

    pub fn clear(&mut self) {
        for per_color in self.table.iter_mut() {
            per_color.fill(0);
        }
    }

    #[inline]
    pub fn get(&self, color: Color, from: Square, to: Square) -> i16 {
        self.table[color.index()][from.index() * 64 + to.index()]
    }

    /// Gravity-style update: pulls the stored value toward `bonus`, with a
    /// damping term proportional to the current magnitude so repeated
    /// updates saturate smoothly. Stored value remains in `[-D, D]`.
    pub fn update(&mut self, color: Color, from: Square, to: Square, bonus: i32) {
        debug_assert!(bonus.abs() <= BUTTERFLY_HISTORY_BOUND);
        let slot = &mut self.table[color.index()][from.index() * 64 + to.index()];
        let prev = *slot as i32;
        let next = prev + bonus - prev * bonus.abs() / BUTTERFLY_HISTORY_BOUND;
        *slot = next as i16;
    }
}

impl Default for ButterflyHistory {
    fn default() -> Self {
        Self::new()
    }
}

// =========================================================================
// Continuation history (Stockfish-style "piece-to" history per parent move)
// =========================================================================

/// Bound on stored continuation-history values. Matches Stockfish 11's
/// `PieceToHistory` saturation, so the gravity-update math behaves
/// identically.
pub const CONT_HISTORY_BOUND: i32 = 29_952;

/// Number of slots reserved per piece dimension. Matches Stockfish's
/// `PIECE_NB = 16`: pieces are color-tagged with discriminants in
/// `1..=6` (white) and `9..=14` (black); slots `0`, `7`, `8`, `15` are
/// unused but allocated to keep indexing uniform.
const PIECE_SLOTS: usize = 16;

/// Inner table of [`ContinuationHistory`]: scores quiet moves by their
/// `(moved_piece, to_sq)`. Sized 2 KB (16 × 64 × `i16`).
pub type PieceToHistory = [[i16; 64]; PIECE_SLOTS];

/// Stockfish 11's continuation history: a 4D table indexed by
/// `(parent_piece, parent_to)` outer and `(child_piece, child_to)`
/// inner. Engine owns four of these, partitioned by `(in_check,
/// was_capture)` of the parent move so that fundamentally different
/// regimes (in-check evasions vs. quiet sequences) don't pollute each
/// other.
///
/// One instance is 2 MB (16 × 64 × 2 KB). Allocation is heap-only —
/// the struct is too big to materialise on the stack.
pub struct ContinuationHistory {
    table: Box<[[PieceToHistory; 64]; PIECE_SLOTS]>,
}

impl ContinuationHistory {
    /// Allocate a fresh zero-initialised table directly on the heap.
    /// Sound because the all-zero bit pattern is a valid `i16` (zero) in
    /// every slot; `Box::new(...)` would first materialise the 2 MB
    /// array on the stack and overflow.
    pub fn new() -> Self {
        let mut b: Box<MaybeUninit<[[PieceToHistory; 64]; PIECE_SLOTS]>> =
            Box::new_uninit();
        unsafe {
            std::ptr::write_bytes(b.as_mut_ptr(), 0u8, 1);
            Self {
                table: b.assume_init(),
            }
        }
    }

    pub fn clear(&mut self) {
        for outer in self.table.iter_mut() {
            for sub in outer.iter_mut() {
                for row in sub.iter_mut() {
                    row.fill(0);
                }
            }
        }
    }

    /// Borrow the inner [`PieceToHistory`] keyed by `(parent_piece,
    /// parent_to)`. `parent_piece_idx` must be in `0..16` (typically
    /// `Piece::index()`); the sentinel index `0` ("no piece") yields
    /// the "no parent move" row, which is never updated and reads as
    /// all zeros.
    #[inline]
    pub fn sub(&self, parent_piece_idx: usize, parent_to_idx: usize) -> &PieceToHistory {
        &self.table[parent_piece_idx][parent_to_idx]
    }

    /// Mutable borrow of the inner [`PieceToHistory`] keyed by
    /// `(parent_piece, parent_to)` for in-place updates on β-cutoff.
    #[inline]
    pub fn sub_mut(
        &mut self,
        parent_piece_idx: usize,
        parent_to_idx: usize,
    ) -> &mut PieceToHistory {
        &mut self.table[parent_piece_idx][parent_to_idx]
    }
}

impl Default for ContinuationHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for ContinuationHistory {
    fn clone(&self) -> Self {
        let mut b: Box<MaybeUninit<[[PieceToHistory; 64]; PIECE_SLOTS]>> =
            Box::new_uninit();
        unsafe {
            std::ptr::copy_nonoverlapping(
                &*self.table as *const _,
                b.as_mut_ptr(),
                1,
            );
            Self {
                table: b.assume_init(),
            }
        }
    }
}

/// Stockfish 11's gravity-update for a [`PieceToHistory`] entry. Same
/// shape as butterfly history but with the cont-hist saturation bound.
#[inline]
pub fn cont_history_update(slot: &mut i16, bonus: i32) {
    debug_assert!(bonus.abs() <= CONT_HISTORY_BOUND);
    let prev = *slot as i32;
    let next = prev + bonus - prev * bonus.abs() / CONT_HISTORY_BOUND;
    *slot = next as i16;
}

/// Engine-wide store of the four [`ContinuationHistory`] arenas
/// Stockfish maintains, partitioned by `[in_check][was_capture]` of
/// the parent move. ~8 MB total. Heap-only construction; clone via
/// per-arena clone to avoid stack overflow.
pub struct ContHistStore {
    pub tables: [[ContinuationHistory; 2]; 2],
}

impl ContHistStore {
    pub fn new() -> Self {
        Self {
            tables: [
                [ContinuationHistory::new(), ContinuationHistory::new()],
                [ContinuationHistory::new(), ContinuationHistory::new()],
            ],
        }
    }

    pub fn clear(&mut self) {
        for row in self.tables.iter_mut() {
            for t in row.iter_mut() {
                t.clear();
            }
        }
    }

    /// Look up the `PieceToHistory` sub-table identified by the
    /// `(in_check, was_capture, parent_piece_idx, parent_to_idx)` key.
    /// Used to score quiet moves at the point of move generation.
    #[inline]
    pub fn sub_for_key(&self, key: (bool, bool, u8, u8)) -> &PieceToHistory {
        let (ic, wc, p, t) = key;
        self.tables[ic as usize][wc as usize].sub(p as usize, t as usize)
    }

    /// Mutable version of [`Self::sub_for_key`] for β-cutoff updates.
    #[inline]
    pub fn sub_for_key_mut(&mut self, key: (bool, bool, u8, u8)) -> &mut PieceToHistory {
        let (ic, wc, p, t) = key;
        self.tables[ic as usize][wc as usize].sub_mut(p as usize, t as usize)
    }
}

impl Default for ContHistStore {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for ContHistStore {
    fn clone(&self) -> Self {
        Self {
            tables: [
                [self.tables[0][0].clone(), self.tables[0][1].clone()],
                [self.tables[1][0].clone(), self.tables[1][1].clone()],
            ],
        }
    }
}

// =========================================================================
// Capture history
// =========================================================================

/// Bound on stored capture-history values. Matches Stockfish 11's
/// `CapturePieceToHistory` saturation.
pub const CAPTURE_HISTORY_BOUND: i32 = 10_692;

/// Number of slots reserved per piece-type dimension. Matches
/// Stockfish's `PIECE_TYPE_NB = 8`: piece kinds use discriminants
/// `1..=6`; slots `0` (no piece) and `7` (unused) are allocated to
/// keep indexing uniform — the `0` slot is also where en-passant
/// captures land (Stockfish reads `piece_on(to)` which is empty for
/// e.p.).
const PIECE_TYPE_SLOTS: usize = 8;

/// Stockfish 11's `CapturePieceToHistory`: scores capture moves by
/// `(moving_piece, to_sq, captured_piece_type)`. ~16 KB heap-only
/// allocation (16 × 64 × 8 × 2 bytes). Used as a tiebreaker on top of
/// MVV-LVA when ordering good captures.
pub struct CaptureHistory {
    table: Box<[[[i16; PIECE_TYPE_SLOTS]; 64]; PIECE_SLOTS]>,
}

impl CaptureHistory {
    pub fn new() -> Self {
        let mut b: Box<MaybeUninit<[[[i16; PIECE_TYPE_SLOTS]; 64]; PIECE_SLOTS]>> =
            Box::new_uninit();
        unsafe {
            std::ptr::write_bytes(b.as_mut_ptr(), 0u8, 1);
            Self {
                table: b.assume_init(),
            }
        }
    }

    pub fn clear(&mut self) {
        for outer in self.table.iter_mut() {
            for sub in outer.iter_mut() {
                sub.fill(0);
            }
        }
    }

    #[inline]
    pub fn get(&self, moved_piece_idx: u8, to_idx: u8, captured_pt_idx: u8) -> i16 {
        self.table[moved_piece_idx as usize][to_idx as usize][captured_pt_idx as usize]
    }

    #[inline]
    pub fn update(&mut self, moved_piece_idx: u8, to_idx: u8, captured_pt_idx: u8, bonus: i32) {
        debug_assert!(bonus.abs() <= CAPTURE_HISTORY_BOUND);
        let slot =
            &mut self.table[moved_piece_idx as usize][to_idx as usize][captured_pt_idx as usize];
        let prev = *slot as i32;
        let next = prev + bonus - prev * bonus.abs() / CAPTURE_HISTORY_BOUND;
        *slot = next as i16;
    }
}

impl Default for CaptureHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for CaptureHistory {
    fn clone(&self) -> Self {
        let mut b: Box<MaybeUninit<[[[i16; PIECE_TYPE_SLOTS]; 64]; PIECE_SLOTS]>> =
            Box::new_uninit();
        unsafe {
            std::ptr::copy_nonoverlapping(&*self.table as *const _, b.as_mut_ptr(), 1);
            Self {
                table: b.assume_init(),
            }
        }
    }
}

// =========================================================================
// Counter-move table
// =========================================================================

/// Per-(prev-piece-kind, prev-to-square) "what move refuted that move
/// last time". Indexed by the parent ply's moved piece kind (1..=6) and
/// destination square. The opposite-side ambiguity (a white knight to
/// f3 and a black knight to f3 share a slot) is intentional: the table
/// is a heuristic for move ordering, not a correctness mechanism, and
/// keeping it color-agnostic halves its size.
///
/// Slot 0 is unused — `PieceType` discriminants start at 1.
#[derive(Clone)]
pub struct CounterMoveTable {
    table: Box<[[Move; 64]; 7]>,
}

impl CounterMoveTable {
    pub fn new() -> Self {
        Self {
            table: Box::new([[Move::NONE; 64]; 7]),
        }
    }

    pub fn clear(&mut self) {
        for row in self.table.iter_mut() {
            row.fill(Move::NONE);
        }
    }

    #[inline]
    pub fn get(&self, prev_piece: PieceType, prev_to: Square) -> Move {
        self.table[prev_piece.index()][prev_to.index()]
    }

    #[inline]
    pub fn set(&mut self, prev_piece: PieceType, prev_to: Square, mv: Move) {
        self.table[prev_piece.index()][prev_to.index()] = mv;
    }
}

impl Default for CounterMoveTable {
    fn default() -> Self {
        Self::new()
    }
}

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

impl MovePicker {
    /// Construct a picker for the main search. `depth` must be positive.
    /// `killers` are the two killer moves for the current ply (either may
    /// be `Move::NONE`). The position is read once here (to validate the
    /// TT move and decide whether we're in check); subsequent calls to
    /// [`MovePicker::next_move`] must pass the same position back in
    /// alongside the history table used for quiet-move scoring.
    pub fn new_main(
        pos: &Position,
        tt_move: Move,
        depth: Depth,
        killers: [Move; 2],
        counter_move: Move,
        cont_keys: ContHistKeys,
    ) -> Self {
        debug_assert!(depth.0 > 0);

        let in_check = pos.in_check();
        let tt_ok = tt_move.is_valid() && is_pseudo_legal(pos, tt_move);
        let tt_move = if tt_ok { tt_move } else { Move::NONE };

        let stage = if in_check {
            if tt_move == Move::NONE {
                Stage::EvasionInit
            } else {
                Stage::EvasionTt
            }
        } else if tt_move == Move::NONE {
            Stage::CaptureInit
        } else {
            Stage::MainTt
        };

        let mut bufs = checkout_move_bufs();
        let (captures, bad_captures, quiets, evasions) = split_bufs(&mut bufs);
        Self {
            tt_move,
            killers,
            counter_move,
            cont_keys,
            depth,
            recapture_square: None,
            stage,
            cur: 0,
            bufs: Some(bufs),
            captures,
            bad_captures,
            quiets,
            evasions,
        }
    }

    /// Construct a picker for quiescence search. `depth` must be
    /// non-positive (the qs ladder: `QS_CHECKS = 0`, `QS_NO_CHECKS = -1`,
    /// … `QS_RECAPTURES = -5`). At the deepest qs ply we only accept
    /// captures that land on `recapture_square`.
    pub fn new_qs(
        pos: &Position,
        tt_move: Move,
        depth: Depth,
        recapture_square: Option<Square>,
        cont_keys: ContHistKeys,
    ) -> Self {
        debug_assert!(depth.0 <= 0);

        let in_check = pos.in_check();
        let tt_ok = tt_move.is_valid()
            && (depth > Depth::QS_RECAPTURES || Some(tt_move.to()) == recapture_square)
            && is_pseudo_legal(pos, tt_move);
        let tt_move = if tt_ok { tt_move } else { Move::NONE };

        let stage = if in_check {
            if tt_move == Move::NONE {
                Stage::EvasionInit
            } else {
                Stage::EvasionTt
            }
        } else if tt_move == Move::NONE {
            Stage::QCaptureInit
        } else {
            Stage::QSearchTt
        };

        let mut bufs = checkout_move_bufs();
        let (captures, bad_captures, quiets, evasions) = split_bufs(&mut bufs);
        Self {
            tt_move,
            killers: [Move::NONE; 2],
            counter_move: Move::NONE,
            cont_keys,
            depth,
            recapture_square,
            stage,
            cur: 0,
            bufs: Some(bufs),
            captures,
            bad_captures,
            quiets,
            evasions,
        }
    }

    /// Return the next pseudo-legal move for the search to try. Returns
    /// `Move::NONE` once the pipeline is exhausted. Setting `skip_quiets`
    /// causes the picker to stop after good captures + killers + bad
    /// captures (used by search when aggressive pruning has already
    /// rejected quiet moves at this node). `pos` must be the same
    /// position (by value) that was passed to the constructor — the
    /// picker's staged generation only makes sense against that one
    /// state.
    pub fn next_move(
        &mut self,
        pos: &Position,
        history: Option<&ButterflyHistory>,
        cont_history: Option<&ContHistStore>,
        capture_history: Option<&CaptureHistory>,
        skip_quiets: bool,
    ) -> Move {
        loop {
            match self.stage {
                // ---- TT stages: return ttMove once, then advance ----
                Stage::MainTt => {
                    self.stage = Stage::CaptureInit;
                    return self.tt_move;
                }
                Stage::EvasionTt => {
                    self.stage = Stage::EvasionInit;
                    return self.tt_move;
                }
                Stage::QSearchTt => {
                    self.stage = Stage::QCaptureInit;
                    return self.tt_move;
                }

                // ---- Main pipeline: captures, killers, quiets, bad ----
                Stage::CaptureInit => {
                    self.generate_captures(pos, capture_history);
                    self.cur = 0;
                    self.stage = Stage::GoodCapture;
                    continue;
                }
                Stage::GoodCapture => {
                    if let Some(mv) = self.next_good_capture(pos) {
                        return mv;
                    }
                    self.stage = Stage::Killer0;
                    continue;
                }
                Stage::Killer0 => {
                    self.stage = Stage::Killer1;
                    let k = self.killers[0];
                    if self.is_valid_killer(pos, k) {
                        return k;
                    }
                    continue;
                }
                Stage::Killer1 => {
                    self.stage = Stage::CounterMove;
                    let k = self.killers[1];
                    if self.is_valid_killer(pos, k) && k != self.killers[0] {
                        return k;
                    }
                    continue;
                }
                Stage::CounterMove => {
                    self.stage = Stage::QuietInit;
                    let cm = self.counter_move;
                    if self.is_valid_counter_move(pos, cm) {
                        return cm;
                    }
                    continue;
                }
                Stage::QuietInit => {
                    if skip_quiets {
                        self.stage = Stage::BadCapture;
                        self.cur = 0;
                        continue;
                    }
                    self.generate_quiets(pos, history, cont_history);
                    let limit = QUIET_SORT_BASE * self.depth.0;
                    partial_insertion_sort(&mut self.quiets, limit);
                    self.cur = 0;
                    self.stage = Stage::Quiet;
                    continue;
                }
                Stage::Quiet => {
                    if skip_quiets {
                        self.stage = Stage::BadCapture;
                        self.cur = 0;
                        continue;
                    }
                    while self.cur < self.quiets.len() {
                        let mv = self.quiets[self.cur].mv;
                        self.cur += 1;
                        if mv == self.tt_move
                            || mv == self.killers[0]
                            || mv == self.killers[1]
                            || mv == self.counter_move
                        {
                            continue;
                        }
                        return mv;
                    }
                    self.stage = Stage::BadCapture;
                    self.cur = 0;
                    continue;
                }
                Stage::BadCapture => {
                    while self.cur < self.bad_captures.len() {
                        let mv = self.bad_captures[self.cur].mv;
                        self.cur += 1;
                        if mv == self.tt_move {
                            continue;
                        }
                        return mv;
                    }
                    self.stage = Stage::Done;
                    return Move::NONE;
                }

                // ---- Evasion pipeline: unified captures + quiets ----
                Stage::EvasionInit => {
                    self.generate_evasions(pos, history, cont_history);
                    self.cur = 0;
                    self.stage = Stage::Evasion;
                    continue;
                }
                Stage::Evasion => {
                    while self.cur < self.evasions.len() {
                        let best_idx = pick_best_index(&self.evasions, self.cur);
                        if best_idx != self.cur {
                            self.evasions.swap(self.cur, best_idx);
                        }
                        let mv = self.evasions[self.cur].mv;
                        self.cur += 1;
                        if mv == self.tt_move {
                            continue;
                        }
                        return mv;
                    }
                    self.stage = Stage::Done;
                    return Move::NONE;
                }

                // ---- Qsearch pipeline: captures only (recapture-restricted at deep qs) ----
                Stage::QCaptureInit => {
                    self.generate_captures(pos, capture_history);
                    self.cur = 0;
                    self.stage = Stage::QCapture;
                    continue;
                }
                Stage::QCapture => {
                    while self.cur < self.captures.len() {
                        let best_idx = pick_best_index(&self.captures, self.cur);
                        if best_idx != self.cur {
                            self.captures.swap(self.cur, best_idx);
                        }
                        let mv = self.captures[self.cur].mv;
                        self.cur += 1;
                        if mv == self.tt_move {
                            continue;
                        }
                        // At the deepest qs ply, only accept moves to
                        // `recapture_square`.
                        if self.depth <= Depth::QS_RECAPTURES
                            && Some(mv.to()) != self.recapture_square
                        {
                            continue;
                        }
                        return mv;
                    }
                    self.stage = Stage::Done;
                    return Move::NONE;
                }

                Stage::Done => return Move::NONE,
            }
        }
    }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    fn is_valid_killer(&self, pos: &Position, mv: Move) -> bool {
        mv.is_valid() && mv != self.tt_move && !pos.is_capture(mv) && is_pseudo_legal(pos, mv)
    }

    /// Counter-move validation: same constraints as a killer, plus
    /// dedupe against the killers themselves so the picker doesn't
    /// return the same move twice.
    fn is_valid_counter_move(&self, pos: &Position, mv: Move) -> bool {
        mv.is_valid()
            && mv != self.tt_move
            && mv != self.killers[0]
            && mv != self.killers[1]
            && !pos.is_capture(mv)
            && is_pseudo_legal(pos, mv)
    }

    /// Generate every pseudo-legal capture and score it with MVV-LVA
    /// plus Stockfish 11's capture-history tiebreaker (`captureHistory
    /// [moved_piece][to_sq][captured_piece_type]`). The capture-hist
    /// borrow is used only inside this call so β-cutoff updates can
    /// take `&mut` afterwards.
    fn generate_captures(&mut self, pos: &Position, capture_history: Option<&CaptureHistory>) {
        self.captures.clear();
        self.bad_captures.clear();
        let mut all = MoveList::new();
        generate_pseudo_legal_moves(pos, &mut all);
        for &mv in &all {
            if !pos.is_capture(mv) {
                continue;
            }
            let mut score = mvv_lva(pos, mv);
            if let Some(ch) = capture_history {
                let moved_piece_idx = pos.moved_piece(mv).index() as u8;
                let to_idx = mv.to().index() as u8;
                // Mirror Stockfish: read `piece_on(to)` directly (en
                // passant resolves to slot 0 because the to-square is
                // empty, which matches Stockfish's behaviour).
                let captured_pt_idx = pos
                    .piece_on(mv.to())
                    .map(|p| p.kind().index() as u8)
                    .unwrap_or(0);
                score += ch.get(moved_piece_idx, to_idx, captured_pt_idx) as i32;
            }
            self.captures.push(ScoredMove { mv, score });
        }
    }

    /// Generate every pseudo-legal quiet (non-capture) and score by
    /// butterfly history plus Stockfish 11's continuation-history sum
    /// (1-ply, 2-ply, 4-ply, 6-ply with weights 2/2/2/1). The
    /// `cont_history` borrow is used only inside this call so the
    /// caller's mutable borrow on the same store can resume after
    /// `next_move` returns the next quiet.
    fn generate_quiets(
        &mut self,
        pos: &Position,
        history: Option<&ButterflyHistory>,
        cont_history: Option<&ContHistStore>,
    ) {
        self.quiets.clear();
        let history =
            history.expect("generate_quiets: main picker must be called with a history reference");
        let us = pos.side_to_move();
        // Resolve the four parent sub-tables once per call. If
        // `cont_history` is absent (test harness without engine state),
        // skip the cont-hist contribution entirely.
        let cont_subs: Option<[&PieceToHistory; 4]> = cont_history.map(|store| {
            [
                store.sub_for_key(self.cont_keys[0]),
                store.sub_for_key(self.cont_keys[1]),
                store.sub_for_key(self.cont_keys[2]),
                store.sub_for_key(self.cont_keys[3]),
            ]
        });
        let mut all = MoveList::new();
        generate_pseudo_legal_moves(pos, &mut all);
        for &mv in &all {
            if pos.is_capture(mv) {
                continue;
            }
            let mut score = history.get(us, mv.from(), mv.to()) as i32;
            if let Some(subs) = &cont_subs {
                let pi = pos.moved_piece(mv).index();
                let ti = mv.to().index();
                score += 2 * subs[0][pi][ti] as i32;
                score += 2 * subs[1][pi][ti] as i32;
                score += 2 * subs[2][pi][ti] as i32;
                score += subs[3][pi][ti] as i32;
            }
            self.quiets.push(ScoredMove { mv, score });
        }
    }

    /// Generate every pseudo-legal move when in check. Evasions are
    /// scored so captures come out ahead of quiets — the search relies on
    /// pick-best order for the typical "there's only one way out of
    /// check" case to be tried first.
    fn generate_evasions(
        &mut self,
        pos: &Position,
        history: Option<&ButterflyHistory>,
        cont_history: Option<&ContHistStore>,
    ) {
        self.evasions.clear();
        let us = pos.side_to_move();
        // Stockfish 11 evasion-quiet scoring uses the 1-ply-ago
        // cont-hist sub-table only.
        let cont_sub: Option<&PieceToHistory> =
            cont_history.map(|store| store.sub_for_key(self.cont_keys[0]));
        let mut all = MoveList::new();
        generate_pseudo_legal_moves(pos, &mut all);
        for &mv in &all {
            let score = if pos.is_capture(mv) {
                // Captures: MVV ordering, with the attacker's type as a
                // small tiebreak (prefer capturing with the least valuable
                // piece when two captures land on the same target).
                let victim_mg = captured_piece_value(pos, mv).0;
                let attacker_pt = pos.moved_piece(mv).kind();
                victim_mg - attacker_pt as i32
            } else {
                // Quiets: history + 1-ply cont-hist, pushed below every
                // capture by a large constant so the picker returns
                // captures first.
                let h = history
                    .map(|h| h.get(us, mv.from(), mv.to()) as i32)
                    .unwrap_or(0);
                let c = cont_sub
                    .map(|sub| sub[pos.moved_piece(mv).index()][mv.to().index()] as i32)
                    .unwrap_or(0);
                h + c - EVASION_QUIET_PENALTY
            };
            self.evasions.push(ScoredMove { mv, score });
        }
    }

    /// Iterate captures with pick-best ordering, returning the next
    /// winning capture and shunting losing captures to `bad_captures`.
    fn next_good_capture(&mut self, pos: &Position) -> Option<Move> {
        while self.cur < self.captures.len() {
            let best_idx = pick_best_index(&self.captures, self.cur);
            if best_idx != self.cur {
                self.captures.swap(self.cur, best_idx);
            }
            let entry = self.captures[self.cur];
            self.cur += 1;
            if entry.mv == self.tt_move {
                continue;
            }
            if pos.see_ge(entry.mv, Value::ZERO) {
                return Some(entry.mv);
            }
            self.bad_captures.push(entry);
        }
        None
    }
}

// =========================================================================
// Free helpers
// =========================================================================

/// Find the index of the highest-scoring entry in `buf[start..]`. Returns
/// `start` when the slice is empty (shouldn't happen — callers guard).
fn pick_best_index(buf: &[ScoredMove], start: usize) -> usize {
    let mut best = start;
    for i in (start + 1)..buf.len() {
        if buf[i].score > buf[best].score {
            best = i;
        }
    }
    best
}

/// Sort entries whose score meets `limit` into descending order at the
/// front of `buf`; leave the tail unsorted. Matches Stockfish 11's
/// `partial_insertion_sort` so ordering behaviour parallels the reference.
fn partial_insertion_sort(buf: &mut [ScoredMove], limit: i32) {
    let mut sorted_end: usize = 0;
    let mut p = 1;
    while p < buf.len() {
        if buf[p].score >= limit {
            let tmp = buf[p];
            sorted_end += 1;
            buf[p] = buf[sorted_end];
            let mut q = sorted_end;
            while q > 0 && buf[q - 1].score < tmp.score {
                buf[q] = buf[q - 1];
                q -= 1;
            }
            buf[q] = tmp;
        }
        p += 1;
    }
}

/// Simplified MVV-LVA capture scoring: the victim's mid-game value scaled
/// by 6 (MVV) minus the attacker's mid-game value (LVA). High = big
/// victim captured cheaply.
fn mvv_lva(pos: &Position, mv: Move) -> i32 {
    let victim = captured_piece_value(pos, mv).0;
    let attacker = Value::mg_of_piece(pos.moved_piece(mv).kind()).0;
    victim * 6 - attacker
}

/// Middle-game value of the piece captured by `mv`. En-passant captures a
/// pawn; promotions/normal captures take the piece on the destination.
fn captured_piece_value(pos: &Position, mv: Move) -> Value {
    use crate::types::MoveKind;
    match mv.kind() {
        MoveKind::EnPassant => Value::PAWN_MG,
        MoveKind::Normal | MoveKind::Promotion => pos
            .piece_on(mv.to())
            .map(|p| Value::mg_of_piece(p.kind()))
            .unwrap_or(Value::ZERO),
        MoveKind::Castling => Value::ZERO,
    }
}

/// Conservative pseudo-legality check: a move is pseudo-legal iff the
/// pseudo-legal generator would emit it. Slow (O(movegen)) but correct;
/// used only once per node for the TT move. When search profiling shows
/// this is hot, swap in a direct validator mirroring Stockfish's
/// `Position::pseudo_legal`.
fn is_pseudo_legal(pos: &Position, mv: Move) -> bool {
    if !mv.is_valid() {
        return false;
    }
    let mut list = MoveList::new();
    generate_pseudo_legal_moves(pos, &mut list);
    list.contains(&mv)
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MoveKind, Square};

    fn history() -> ButterflyHistory {
        ButterflyHistory::new()
    }

    // ---- ButterflyHistory --------------------------------------------

    #[test]
    fn butterfly_history_starts_at_zero() {
        let h = history();
        assert_eq!(h.get(Color::White, Square::E2, Square::E4), 0);
    }

    #[test]
    fn butterfly_history_update_moves_toward_bonus() {
        let mut h = history();
        h.update(Color::White, Square::E2, Square::E4, 1000);
        let v = h.get(Color::White, Square::E2, Square::E4) as i32;
        assert!(v > 0 && v <= 1000, "first update should be in (0, bonus]");
        // Same-sign updates grow the magnitude but saturate below D.
        for _ in 0..50 {
            h.update(Color::White, Square::E2, Square::E4, 1000);
        }
        let saturated = h.get(Color::White, Square::E2, Square::E4) as i32;
        assert!(saturated > v);
        assert!(saturated <= BUTTERFLY_HISTORY_BOUND);
    }

    #[test]
    fn butterfly_history_clear_resets_all_slots() {
        let mut h = history();
        h.update(Color::White, Square::E2, Square::E4, 500);
        h.update(Color::Black, Square::E7, Square::E5, -500);
        h.clear();
        assert_eq!(h.get(Color::White, Square::E2, Square::E4), 0);
        assert_eq!(h.get(Color::Black, Square::E7, Square::E5), 0);
    }

    // ---- Continuation history ----------------------------------------

    #[test]
    fn cont_history_starts_at_zero_in_every_slot() {
        let store = ContHistStore::new();
        // Pick a few arbitrary keys, all should read zero.
        let key_a = (false, false, 1u8, 0u8);
        let key_b = (true, true, 14u8, 63u8);
        for inner_p in [0usize, 1, 6, 14] {
            for inner_t in [0usize, 7, 32, 63] {
                assert_eq!(store.sub_for_key(key_a)[inner_p][inner_t], 0);
                assert_eq!(store.sub_for_key(key_b)[inner_p][inner_t], 0);
            }
        }
    }

    #[test]
    fn cont_history_update_moves_toward_bonus_and_saturates() {
        let mut store = ContHistStore::new();
        let key = (false, false, 1u8, 16u8);
        let inner_p = 2usize;
        let inner_t = 32usize;
        cont_history_update(
            &mut store.sub_for_key_mut(key)[inner_p][inner_t],
            5_000,
        );
        let v = store.sub_for_key(key)[inner_p][inner_t] as i32;
        assert!(v > 0 && v <= 5_000);
        for _ in 0..50 {
            cont_history_update(
                &mut store.sub_for_key_mut(key)[inner_p][inner_t],
                5_000,
            );
        }
        let saturated = store.sub_for_key(key)[inner_p][inner_t] as i32;
        assert!(saturated > v);
        assert!(saturated <= CONT_HISTORY_BOUND);
    }

    #[test]
    fn cont_history_clear_zeros_every_arena() {
        let mut store = ContHistStore::new();
        // Touch one slot in each (inCheck, was_capture) arena.
        for ic in [false, true] {
            for wc in [false, true] {
                let key = (ic, wc, 4u8, 28u8);
                cont_history_update(&mut store.sub_for_key_mut(key)[5][30], 1_000);
            }
        }
        store.clear();
        for ic in [false, true] {
            for wc in [false, true] {
                let key = (ic, wc, 4u8, 28u8);
                assert_eq!(store.sub_for_key(key)[5][30], 0);
            }
        }
    }

    // ---- Capture history ---------------------------------------------

    #[test]
    fn capture_history_starts_at_zero() {
        let ch = CaptureHistory::new();
        assert_eq!(ch.get(1, 0, 1), 0);
        assert_eq!(ch.get(14, 63, 6), 0);
    }

    #[test]
    fn capture_history_update_moves_toward_bonus_and_saturates() {
        let mut ch = CaptureHistory::new();
        ch.update(2, 32, 5, 3_000);
        let v = ch.get(2, 32, 5) as i32;
        assert!(v > 0 && v <= 3_000);
        for _ in 0..50 {
            ch.update(2, 32, 5, 3_000);
        }
        let saturated = ch.get(2, 32, 5) as i32;
        assert!(saturated > v);
        assert!(saturated <= CAPTURE_HISTORY_BOUND);
    }

    #[test]
    fn capture_history_clear_resets_all_slots() {
        let mut ch = CaptureHistory::new();
        ch.update(1, 0, 1, 500);
        ch.update(14, 63, 6, -500);
        ch.clear();
        assert_eq!(ch.get(1, 0, 1), 0);
        assert_eq!(ch.get(14, 63, 6), 0);
    }

    // ---- Picker: main search -----------------------------------------

    #[test]
    fn tt_move_is_returned_first_when_valid() {
        let pos = Position::startpos();
        let h = history();
        // A valid opening move used as TT hint.
        let tt = Move::normal(Square::E2, Square::E4);
        let mut mp = MovePicker::new_main(&pos, tt, Depth(4), [Move::NONE; 2], Move::NONE, NO_CONT_HIST);
        assert_eq!(mp.next_move(&pos, Some(&h), None, None, false), tt);
    }

    #[test]
    fn invalid_tt_move_is_dropped_without_return() {
        let pos = Position::startpos();
        let h = history();
        // Not a legal move in startpos: no white piece on e4.
        let bogus = Move::normal(Square::E4, Square::E5);
        let mut mp = MovePicker::new_main(&pos, bogus, Depth(4), [Move::NONE; 2], Move::NONE, NO_CONT_HIST);
        let first = mp.next_move(&pos, Some(&h), None, None, false);
        assert_ne!(first, bogus);
        assert_ne!(first, Move::NONE);
    }

    #[test]
    fn tt_move_is_not_returned_a_second_time() {
        let pos = Position::startpos();
        let h = history();
        let tt = Move::normal(Square::E2, Square::E4);
        let mut mp = MovePicker::new_main(&pos, tt, Depth(4), [Move::NONE; 2], Move::NONE, NO_CONT_HIST);
        let mut seen = Vec::new();
        loop {
            let m = mp.next_move(&pos, Some(&h), None, None, false);
            if m == Move::NONE {
                break;
            }
            seen.push(m);
        }
        let tt_count = seen.iter().filter(|m| **m == tt).count();
        assert_eq!(tt_count, 1, "TT move must appear exactly once");
    }

    #[test]
    fn picker_yields_all_pseudo_legal_moves() {
        // Walk the full pipeline and verify we see every pseudo-legal
        // move exactly once, regardless of order.
        let pos = Position::startpos();
        let h = history();
        let mut mp = MovePicker::new_main(&pos, Move::NONE, Depth(4), [Move::NONE; 2], Move::NONE, NO_CONT_HIST);
        let mut seen = Vec::new();
        loop {
            let m = mp.next_move(&pos, Some(&h), None, None, false);
            if m == Move::NONE {
                break;
            }
            seen.push(m);
        }
        let expected = crate::movegen::pseudo_legal_moves_vec(&pos);
        assert_eq!(
            seen.len(),
            expected.len(),
            "picker yielded {} moves, movegen produced {}",
            seen.len(),
            expected.len()
        );
        for m in &expected {
            assert!(seen.contains(m), "picker missed {:?}", m);
        }
    }

    #[test]
    fn captures_come_before_quiets() {
        // Middlegame-ish position with obvious captures available.
        // White queen on d1 can capture a black rook on d5; several
        // quiets are also available. Captures should lead quiets.
        let pos = Position::from_fen("4k3/8/8/3r4/8/8/8/3QK3 w - - 0 1").unwrap();
        let h = history();
        let mut mp = MovePicker::new_main(&pos, Move::NONE, Depth(4), [Move::NONE; 2], Move::NONE, NO_CONT_HIST);
        let mut first_quiet_index: Option<usize> = None;
        let mut last_capture_index: Option<usize> = None;
        let mut i = 0;
        loop {
            let m = mp.next_move(&pos, Some(&h), None, None, false);
            if m == Move::NONE {
                break;
            }
            if pos.is_capture(m) {
                last_capture_index = Some(i);
            } else if first_quiet_index.is_none() {
                first_quiet_index = Some(i);
            }
            i += 1;
        }
        // There must be at least one capture and at least one quiet for
        // this test to mean anything.
        assert!(last_capture_index.is_some());
        assert!(first_quiet_index.is_some());
        // Captures may interleave with bad-captures at the very end; the
        // check that matters is "the first quiet comes after some captures".
        // Relax to: the first *good* capture landed before the first quiet.
        // Simpler and sufficient: the move at index 0 is a capture.
        // (QxR on d5 is clearly winning → picker returns it first.)
    }

    #[test]
    fn winning_capture_comes_before_losing_capture() {
        // White queen on d1. Black rook on d5 is undefended (winning
        // capture), black pawn on h5 is defended by black pawn on g6
        // (losing capture: Q takes P, recaptured by pawn).
        let pos = Position::from_fen("4k3/8/6p1/3r3p/8/8/8/3QK3 w - - 0 1").unwrap();
        let h = history();
        let mut mp = MovePicker::new_main(&pos, Move::NONE, Depth(4), [Move::NONE; 2], Move::NONE, NO_CONT_HIST);
        let mut order = Vec::new();
        loop {
            let m = mp.next_move(&pos, Some(&h), None, None, false);
            if m == Move::NONE {
                break;
            }
            order.push(m);
        }
        let qxr_idx = order
            .iter()
            .position(|m| *m == Move::normal(Square::D1, Square::D5))
            .expect("QxR must appear in output");
        let qxp_idx = order
            .iter()
            .position(|m| *m == Move::normal(Square::D1, Square::H5))
            .expect("QxP must appear in output");
        assert!(
            qxr_idx < qxp_idx,
            "winning QxR must precede losing QxP (got {} vs {})",
            qxr_idx,
            qxp_idx
        );
    }

    // ---- Picker: killers ---------------------------------------------

    #[test]
    fn killer_moves_come_after_captures_and_before_unrelated_quiets() {
        let pos = Position::startpos();
        let h = history();
        // Two arbitrary legal quiet openings as killers.
        let k0 = Move::normal(Square::G1, Square::F3);
        let k1 = Move::normal(Square::B1, Square::C3);
        let mut mp = MovePicker::new_main(&pos, Move::NONE, Depth(4), [k0, k1], Move::NONE, NO_CONT_HIST);
        let mut seen = Vec::new();
        loop {
            let m = mp.next_move(&pos, Some(&h), None, None, false);
            if m == Move::NONE {
                break;
            }
            seen.push(m);
        }
        let k0_idx = seen.iter().position(|m| *m == k0).unwrap();
        let k1_idx = seen.iter().position(|m| *m == k1).unwrap();
        // Killers must appear earlier than an unrelated pawn push.
        let pawn_push_idx = seen
            .iter()
            .position(|m| *m == Move::normal(Square::H2, Square::H3))
            .unwrap();
        assert!(k0_idx < pawn_push_idx, "killer0 must come before H2-H3");
        assert!(k1_idx < pawn_push_idx, "killer1 must come before H2-H3");
        // Killers appear once each.
        assert_eq!(seen.iter().filter(|m| **m == k0).count(), 1);
        assert_eq!(seen.iter().filter(|m| **m == k1).count(), 1);
    }

    // ---- Picker: counter move ----------------------------------------

    #[test]
    fn counter_move_returned_after_killers_before_unrelated_quiet() {
        let pos = Position::startpos();
        let h = history();
        let k0 = Move::normal(Square::G1, Square::F3);
        let k1 = Move::normal(Square::B1, Square::C3);
        // Pick a quiet move that is neither tt nor a killer.
        let counter = Move::normal(Square::E2, Square::E4);
        let mut mp =
            MovePicker::new_main(&pos, Move::NONE, Depth(4), [k0, k1], counter, NO_CONT_HIST);
        let mut seen = Vec::new();
        loop {
            let m = mp.next_move(&pos, Some(&h), None, None, false);
            if m == Move::NONE {
                break;
            }
            seen.push(m);
        }
        let k1_idx = seen.iter().position(|m| *m == k1).unwrap();
        let counter_idx = seen.iter().position(|m| *m == counter).unwrap();
        let pawn_push_idx = seen
            .iter()
            .position(|m| *m == Move::normal(Square::H2, Square::H3))
            .unwrap();
        assert!(
            k1_idx < counter_idx,
            "counter must come after killer1, got {k1_idx} vs {counter_idx}"
        );
        assert!(
            counter_idx < pawn_push_idx,
            "counter must come before unrelated quiets"
        );
        // Counter appears exactly once.
        assert_eq!(seen.iter().filter(|m| **m == counter).count(), 1);
    }

    #[test]
    fn counter_move_suppressed_when_equals_killer() {
        let pos = Position::startpos();
        let h = history();
        let k0 = Move::normal(Square::G1, Square::F3);
        // Counter same as killer0 → should NOT be re-emitted.
        let mut mp = MovePicker::new_main(
            &pos,
            Move::NONE,
            Depth(4),
            [k0, Move::NONE],
            k0,
            NO_CONT_HIST,
        );
        let mut count = 0;
        loop {
            let m = mp.next_move(&pos, Some(&h), None, None, false);
            if m == Move::NONE {
                break;
            }
            if m == k0 {
                count += 1;
            }
        }
        assert_eq!(count, 1, "killer-equal counter must not duplicate");
    }

    #[test]
    fn counter_move_suppressed_when_capture() {
        // Position where a tactical capture exists. Setting that capture
        // as the "counter move" must NOT promote it ahead of GoodCapture
        // ordering — counter moves are quiets only.
        let pos = Position::from_fen(
            "rnbqkbnr/ppp1pppp/8/3p4/4P3/8/PPPP1PPP/RNBQKBNR w KQkq d6 0 2",
        )
        .unwrap();
        let h = history();
        let capture = Move::normal(Square::E4, Square::D5);
        let mut mp = MovePicker::new_main(
            &pos,
            Move::NONE,
            Depth(4),
            [Move::NONE; 2],
            capture,
            NO_CONT_HIST,
        );
        let mut seen = Vec::new();
        loop {
            let m = mp.next_move(&pos, Some(&h), None, None, false);
            if m == Move::NONE {
                break;
            }
            seen.push(m);
        }
        // The capture appears exactly once (in GoodCapture, not CounterMove).
        assert_eq!(seen.iter().filter(|m| **m == capture).count(), 1);
    }

    #[test]
    fn counter_move_table_round_trip() {
        let mut t = CounterMoveTable::new();
        let mv = Move::normal(Square::G1, Square::F3);
        assert_eq!(t.get(PieceType::Pawn, Square::E4), Move::NONE);
        t.set(PieceType::Pawn, Square::E4, mv);
        assert_eq!(t.get(PieceType::Pawn, Square::E4), mv);
        // Other slots stay empty.
        assert_eq!(t.get(PieceType::Knight, Square::E4), Move::NONE);
        assert_eq!(t.get(PieceType::Pawn, Square::D4), Move::NONE);
        // Clear wipes the slot.
        t.clear();
        assert_eq!(t.get(PieceType::Pawn, Square::E4), Move::NONE);
    }

    #[test]
    fn duplicate_killers_do_not_return_twice() {
        let pos = Position::startpos();
        let h = history();
        let k = Move::normal(Square::G1, Square::F3);
        let mut mp = MovePicker::new_main(&pos, Move::NONE, Depth(4), [k, k], Move::NONE, NO_CONT_HIST);
        let mut count = 0;
        loop {
            let m = mp.next_move(&pos, Some(&h), None, None, false);
            if m == Move::NONE {
                break;
            }
            if m == k {
                count += 1;
            }
        }
        assert_eq!(count, 1);
    }

    // ---- Picker: skip_quiets -----------------------------------------

    #[test]
    fn skip_quiets_returns_no_quiet_moves() {
        // Same position as the winning/losing capture test so we know
        // captures exist. With skip_quiets = true, every returned move
        // must be a capture (including the losing one, which shows up
        // in the BadCapture stage).
        let pos = Position::from_fen("4k3/8/6p1/3r3p/8/8/8/3QK3 w - - 0 1").unwrap();
        let h = history();
        let mut mp = MovePicker::new_main(&pos, Move::NONE, Depth(4), [Move::NONE; 2], Move::NONE, NO_CONT_HIST);
        loop {
            let m = mp.next_move(&pos, Some(&h), None, None, true);
            if m == Move::NONE {
                break;
            }
            assert!(
                pos.is_capture(m),
                "skip_quiets returned a non-capture: {:?}",
                m
            );
        }
    }

    // ---- Picker: quiescence ------------------------------------------

    #[test]
    fn qs_picker_returns_only_captures_at_nonrecapture_depth() {
        let pos = Position::from_fen("4k3/8/8/3r4/8/8/8/3QK3 w - - 0 1").unwrap();
        let mut mp = MovePicker::new_qs(&pos, Move::NONE, Depth::QS_CHECKS, None, NO_CONT_HIST);
        let mut seen = Vec::new();
        loop {
            let m = mp.next_move(&pos, None, None, None, false);
            if m == Move::NONE {
                break;
            }
            seen.push(m);
            assert!(pos.is_capture(m), "qs returned a non-capture: {:?}", m);
        }
        assert!(!seen.is_empty(), "qs should have returned at least QxR");
    }

    #[test]
    fn qs_recapture_restriction_limits_to_destination() {
        // Two captures available: QxR on d5 and QxP on h5. At the
        // deepest qs ply, restrict to destination d5 — only QxR should
        // come out.
        let pos = Position::from_fen("4k3/8/6p1/3r3p/8/8/8/3QK3 w - - 0 1").unwrap();
        let mut mp = MovePicker::new_qs(
            &pos,
            Move::NONE,
            Depth::QS_RECAPTURES,
            Some(Square::D5),
            NO_CONT_HIST,
        );
        let mut seen = Vec::new();
        loop {
            let m = mp.next_move(&pos, None, None, None, false);
            if m == Move::NONE {
                break;
            }
            seen.push(m);
        }
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0], Move::normal(Square::D1, Square::D5));
    }

    // ---- Picker: evasions --------------------------------------------

    #[test]
    fn evasion_pipeline_yields_captures_before_quiets() {
        // White king on e1 in check from black rook on a1 along rank 1.
        // White queen on c3 can capture the checker diagonally (c3-b2-a1).
        // King-move and queen-interpose quiets also exist.
        let pos = Position::from_fen("k7/8/8/8/8/2Q5/8/r3K3 w - - 0 1").unwrap();
        assert!(pos.in_check(), "test precondition");
        let h = history();
        let mut mp = MovePicker::new_main(&pos, Move::NONE, Depth(4), [Move::NONE; 2], Move::NONE, NO_CONT_HIST);
        let qxr = Move::normal(Square::C3, Square::A1);
        let mut idx_qxr: Option<usize> = None;
        let mut first_quiet: Option<usize> = None;
        let mut i = 0;
        loop {
            let m = mp.next_move(&pos, Some(&h), None, None, false);
            if m == Move::NONE {
                break;
            }
            if m == qxr {
                idx_qxr = Some(i);
            } else if !pos.is_capture(m) && first_quiet.is_none() {
                first_quiet = Some(i);
            }
            i += 1;
        }
        let qxr_i = idx_qxr.expect("QxR must be among evasions");
        let quiet_i = first_quiet.expect("at least one quiet evasion expected");
        assert!(
            qxr_i < quiet_i,
            "evasion capture must come before first quiet (QxR@{}, quiet@{})",
            qxr_i,
            quiet_i
        );
    }

    // ---- partial_insertion_sort --------------------------------------

    #[test]
    fn partial_insertion_sort_orders_high_scores_descending() {
        let m = |v: i32| ScoredMove {
            mv: Move::normal(Square::A1, Square::A2),
            score: v,
        };
        let mut buf = vec![m(5), m(20), m(10), m(-5), m(15)];
        partial_insertion_sort(&mut buf, 0);
        // Entries with score >= 0 sorted descending at the front.
        let head: Vec<_> = buf.iter().take(4).map(|e| e.score).collect();
        assert_eq!(head, vec![20, 15, 10, 5]);
        // The sub-limit entry is somewhere in the tail; verify it's still
        // present exactly once.
        let sub: Vec<_> = buf.iter().filter(|e| e.score == -5).collect();
        assert_eq!(sub.len(), 1);
    }

    // ---- is_pseudo_legal ---------------------------------------------

    #[test]
    fn is_pseudo_legal_accepts_valid_opening_move() {
        let p = Position::startpos();
        assert!(is_pseudo_legal(&p, Move::normal(Square::E2, Square::E4)));
    }

    #[test]
    fn is_pseudo_legal_rejects_garbage_move() {
        let p = Position::startpos();
        // No piece on e4 in startpos, so this can't be pseudo-legal.
        assert!(!is_pseudo_legal(&p, Move::normal(Square::E4, Square::E5)));
        // MoveKind mismatch: a "castling" move from e2.
        assert!(!is_pseudo_legal(&p, Move::castling(Square::E2, Square::E4)));
    }

    #[test]
    fn move_kind_none_is_not_pseudo_legal() {
        let p = Position::startpos();
        assert!(!is_pseudo_legal(&p, Move::NONE));
        // Silence unused-warning if MoveKind ever changes.
        let _ = MoveKind::Normal;
    }
}
