//! Move-ordering history tables: butterfly, continuation, capture, and
//! counter-move. All four are heap-allocated (too big for the stack) and
//! engine-owned; the [`MovePicker`](super::MovePicker) reads them at quiet-
//! and capture-scoring time. Gravity-update math and saturation bounds are
//! the factual Stockfish 11 parameters, used under the idea/expression split.

use crate::types::{Color, Move, PieceType, Square};
use std::mem::MaybeUninit;

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
