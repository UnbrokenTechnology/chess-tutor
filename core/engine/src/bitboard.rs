//! 64-bit bitboards and the const-expr operations over them.
//!
//! A `Bitboard` is a set of squares represented as a `u64` with bit `i`
//! meaning "square `i` is in the set," using the row-major layout defined in
//! `types.rs` (bit 0 = a1, bit 7 = h1, bit 56 = a8, bit 63 = h8).
//!
//! Only operations that don't need runtime-populated tables live here:
//! masks of files/ranks/regions, directional shifts, pawn attack projections,
//! bit manipulation primitives, and king-move distance. The runtime tables
//! (slider magics, pseudo-attacks by piece type, line-between, per-pair
//! chebyshev distance) will land in a separate module once we need them for
//! move generation and evaluation.

use std::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, BitXor, BitXorAssign, Not, Shl, Shr};

use crate::types::{Color, Direction, File, Rank, Square};

// =========================================================================
// Bitboard type
// =========================================================================

/// A set of squares encoded in 64 bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[repr(transparent)]
pub struct Bitboard(pub u64);

impl Bitboard {
    pub const EMPTY: Bitboard = Bitboard(0);
    pub const ALL: Bitboard = Bitboard(!0u64);

    /// `true` iff no square is set.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// `true` iff at least one square is set.
    pub const fn any(self) -> bool {
        self.0 != 0
    }

    /// `true` iff `square` is in the set.
    pub const fn contains(self, square: Square) -> bool {
        (self.0 & (1u64 << square.raw())) != 0
    }

    /// Add `square` to the set.
    pub const fn with(self, square: Square) -> Bitboard {
        Bitboard(self.0 | (1u64 << square.raw()))
    }

    /// Remove `square` from the set.
    pub const fn without(self, square: Square) -> Bitboard {
        Bitboard(self.0 & !(1u64 << square.raw()))
    }

    /// Number of squares in the set.
    pub const fn popcount(self) -> u32 {
        self.0.count_ones()
    }

    /// `true` iff more than one square is set. Cheaper than `popcount() > 1`.
    pub const fn more_than_one(self) -> bool {
        (self.0 & self.0.wrapping_sub(1)) != 0
    }

    /// The least-significant square in the set. Caller must ensure `self` is
    /// non-empty; `trailing_zeros` on zero is 64 which is `Square::NONE`, but
    /// production code should guard it.
    pub const fn lsb(self) -> Square {
        Square::from_index(self.0.trailing_zeros() as u8)
    }

    /// The most-significant square in the set. Caller must ensure `self` is
    /// non-empty.
    pub const fn msb(self) -> Square {
        Square::from_index(63 - self.0.leading_zeros() as u8)
    }

    /// Remove and return the least-significant square. `self` must be non-empty.
    pub fn pop_lsb(&mut self) -> Square {
        let s = self.lsb();
        self.0 &= self.0 - 1;
        s
    }

    /// The front-most square from `color`'s point of view: MSB for white
    /// (largest rank number), LSB for black (smallest rank number).
    pub const fn frontmost(self, color: Color) -> Square {
        match color {
            Color::White => self.msb(),
            Color::Black => self.lsb(),
        }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }

    // -- Directional shifts --------------------------------------------

    pub const fn shift_north(self) -> Bitboard {
        Bitboard(self.0 << 8)
    }

    pub const fn shift_south(self) -> Bitboard {
        Bitboard(self.0 >> 8)
    }

    pub const fn shift_east(self) -> Bitboard {
        Bitboard((self.0 & !FILE_H.0) << 1)
    }

    pub const fn shift_west(self) -> Bitboard {
        Bitboard((self.0 & !FILE_A.0) >> 1)
    }

    pub const fn shift_north_east(self) -> Bitboard {
        Bitboard((self.0 & !FILE_H.0) << 9)
    }

    pub const fn shift_north_west(self) -> Bitboard {
        Bitboard((self.0 & !FILE_A.0) << 7)
    }

    pub const fn shift_south_east(self) -> Bitboard {
        Bitboard((self.0 & !FILE_H.0) >> 7)
    }

    pub const fn shift_south_west(self) -> Bitboard {
        Bitboard((self.0 & !FILE_A.0) >> 9)
    }

    pub const fn shift_north_north(self) -> Bitboard {
        Bitboard(self.0 << 16)
    }

    pub const fn shift_south_south(self) -> Bitboard {
        Bitboard(self.0 >> 16)
    }

    /// Dispatch a shift by one of the eight king-move directions (or N+N, S+S).
    /// Unknown directions return the empty set.
    pub fn shift(self, direction: Direction) -> Bitboard {
        match direction.0 {
            8 => self.shift_north(),
            -8 => self.shift_south(),
            1 => self.shift_east(),
            -1 => self.shift_west(),
            9 => self.shift_north_east(),
            7 => self.shift_north_west(),
            -7 => self.shift_south_east(),
            -9 => self.shift_south_west(),
            16 => self.shift_north_north(),
            -16 => self.shift_south_south(),
            _ => Bitboard::EMPTY,
        }
    }

    /// Bitboard of squares attacked by any of the pawns in `self`, of the given
    /// color. Cheaper than iterating pawns one at a time because both pawns on
    /// the same rank share the same shift.
    pub const fn pawn_attacks(self, color: Color) -> Bitboard {
        match color {
            Color::White => Bitboard(self.shift_north_west().0 | self.shift_north_east().0),
            Color::Black => Bitboard(self.shift_south_west().0 | self.shift_south_east().0),
        }
    }

    /// Bitboard of squares attacked *by two or more* pawns in `self`. Useful
    /// for king-safety and outpost evaluation.
    pub const fn pawn_double_attacks(self, color: Color) -> Bitboard {
        match color {
            Color::White => Bitboard(self.shift_north_west().0 & self.shift_north_east().0),
            Color::Black => Bitboard(self.shift_south_west().0 & self.shift_south_east().0),
        }
    }
}

/// Single-square bitboard.
pub const fn square_bb(square: Square) -> Bitboard {
    Bitboard(1u64 << square.raw())
}

// =========================================================================
// Bitwise operator impls
// =========================================================================

impl BitAnd for Bitboard {
    type Output = Bitboard;
    fn bitand(self, rhs: Bitboard) -> Bitboard {
        Bitboard(self.0 & rhs.0)
    }
}

impl BitOr for Bitboard {
    type Output = Bitboard;
    fn bitor(self, rhs: Bitboard) -> Bitboard {
        Bitboard(self.0 | rhs.0)
    }
}

impl BitXor for Bitboard {
    type Output = Bitboard;
    fn bitxor(self, rhs: Bitboard) -> Bitboard {
        Bitboard(self.0 ^ rhs.0)
    }
}

impl Not for Bitboard {
    type Output = Bitboard;
    fn not(self) -> Bitboard {
        Bitboard(!self.0)
    }
}

impl BitAndAssign for Bitboard {
    fn bitand_assign(&mut self, rhs: Bitboard) {
        self.0 &= rhs.0;
    }
}

impl BitOrAssign for Bitboard {
    fn bitor_assign(&mut self, rhs: Bitboard) {
        self.0 |= rhs.0;
    }
}

impl BitXorAssign for Bitboard {
    fn bitxor_assign(&mut self, rhs: Bitboard) {
        self.0 ^= rhs.0;
    }
}

impl BitAnd<Square> for Bitboard {
    type Output = Bitboard;
    fn bitand(self, rhs: Square) -> Bitboard {
        self & square_bb(rhs)
    }
}

impl BitOr<Square> for Bitboard {
    type Output = Bitboard;
    fn bitor(self, rhs: Square) -> Bitboard {
        self | square_bb(rhs)
    }
}

impl BitXor<Square> for Bitboard {
    type Output = Bitboard;
    fn bitxor(self, rhs: Square) -> Bitboard {
        self ^ square_bb(rhs)
    }
}

impl Shl<u32> for Bitboard {
    type Output = Bitboard;
    fn shl(self, rhs: u32) -> Bitboard {
        Bitboard(self.0 << rhs)
    }
}

impl Shr<u32> for Bitboard {
    type Output = Bitboard;
    fn shr(self, rhs: u32) -> Bitboard {
        Bitboard(self.0 >> rhs)
    }
}

// =========================================================================
// Iteration
// =========================================================================

/// Iterator that yields the squares of a bitboard in ascending order and
/// consumes the bitboard as it goes.
pub struct Squares(pub Bitboard);

impl Iterator for Squares {
    type Item = Square;
    fn next(&mut self) -> Option<Square> {
        if self.0.is_empty() {
            None
        } else {
            Some(self.0.pop_lsb())
        }
    }
}

impl IntoIterator for Bitboard {
    type Item = Square;
    type IntoIter = Squares;
    fn into_iter(self) -> Squares {
        Squares(self)
    }
}

// =========================================================================
// File / rank / region masks
// =========================================================================

pub const FILE_A: Bitboard = Bitboard(0x0101_0101_0101_0101);
pub const FILE_B: Bitboard = Bitboard(0x0101_0101_0101_0101 << 1);
pub const FILE_C: Bitboard = Bitboard(0x0101_0101_0101_0101 << 2);
pub const FILE_D: Bitboard = Bitboard(0x0101_0101_0101_0101 << 3);
pub const FILE_E: Bitboard = Bitboard(0x0101_0101_0101_0101 << 4);
pub const FILE_F: Bitboard = Bitboard(0x0101_0101_0101_0101 << 5);
pub const FILE_G: Bitboard = Bitboard(0x0101_0101_0101_0101 << 6);
pub const FILE_H: Bitboard = Bitboard(0x0101_0101_0101_0101 << 7);

pub const RANK_1: Bitboard = Bitboard(0xFF);
pub const RANK_2: Bitboard = Bitboard(0xFF << 8);
pub const RANK_3: Bitboard = Bitboard(0xFF << 16);
pub const RANK_4: Bitboard = Bitboard(0xFF << 24);
pub const RANK_5: Bitboard = Bitboard(0xFF << 32);
pub const RANK_6: Bitboard = Bitboard(0xFF << 40);
pub const RANK_7: Bitboard = Bitboard(0xFF << 48);
pub const RANK_8: Bitboard = Bitboard(0xFF << 56);

/// The a1..h8 diagonal's dark-square pattern. Doubles as a "half the board"
/// checker useful for bishop-color tests and mop-up endgames.
pub const DARK_SQUARES: Bitboard = Bitboard(0xAA55_AA55_AA55_AA55);
pub const LIGHT_SQUARES: Bitboard = Bitboard(0x55AA_55AA_55AA_55AA);

pub const QUEEN_SIDE: Bitboard = Bitboard(FILE_A.0 | FILE_B.0 | FILE_C.0 | FILE_D.0);
pub const CENTER_FILES: Bitboard = Bitboard(FILE_C.0 | FILE_D.0 | FILE_E.0 | FILE_F.0);
pub const KING_SIDE: Bitboard = Bitboard(FILE_E.0 | FILE_F.0 | FILE_G.0 | FILE_H.0);
pub const CENTER: Bitboard = Bitboard((FILE_D.0 | FILE_E.0) & (RANK_4.0 | RANK_5.0));

/// The "king flank" for each file: roughly the three files centred on the
/// king's file, clamped at the edges. Indexed by `File::index()`.
pub const KING_FLANK: [Bitboard; 8] = [
    Bitboard(QUEEN_SIDE.0 ^ FILE_D.0),
    Bitboard(QUEEN_SIDE.0),
    Bitboard(QUEEN_SIDE.0),
    Bitboard(CENTER_FILES.0),
    Bitboard(CENTER_FILES.0),
    Bitboard(KING_SIDE.0),
    Bitboard(KING_SIDE.0),
    Bitboard(KING_SIDE.0 ^ FILE_E.0),
];

const FILE_MASKS: [Bitboard; 8] = [
    FILE_A, FILE_B, FILE_C, FILE_D, FILE_E, FILE_F, FILE_G, FILE_H,
];
const RANK_MASKS: [Bitboard; 8] = [
    RANK_1, RANK_2, RANK_3, RANK_4, RANK_5, RANK_6, RANK_7, RANK_8,
];

pub const fn file_bb(file: File) -> Bitboard {
    FILE_MASKS[file.index()]
}

pub const fn rank_bb(rank: Rank) -> Bitboard {
    RANK_MASKS[rank.index()]
}

/// Files immediately adjacent to `square`'s file. For a-file returns file b;
/// for h-file returns file g; otherwise returns both neighbours.
pub const fn adjacent_files_bb(square: Square) -> Bitboard {
    let f = file_bb(square.file());
    Bitboard(f.shift_east().0 | f.shift_west().0)
}

/// All squares on ranks strictly in front of `square` from `color`'s point of
/// view. For white on d3 that's ranks 4..=8; for black on d3 that's ranks 1..=2.
pub const fn forward_ranks_bb(color: Color, square: Square) -> Bitboard {
    let rank = square.rank().index();
    match color {
        // All ranks above the current rank. Rank 8 has nothing in front.
        Color::White => {
            let keep_mask: u64 = if rank == 7 {
                0
            } else {
                u64::MAX << (8 * (rank + 1))
            };
            Bitboard(keep_mask)
        }
        // All ranks below the current rank. Rank 1 has nothing in front.
        Color::Black => {
            let keep_mask: u64 = if rank == 0 {
                0
            } else {
                (1u64 << (8 * rank)) - 1
            };
            Bitboard(keep_mask)
        }
    }
}

/// The file in front of `square` from `color`'s point of view, not including
/// `square` itself.
pub const fn forward_file_bb(color: Color, square: Square) -> Bitboard {
    Bitboard(forward_ranks_bb(color, square).0 & file_bb(square.file()).0)
}

/// All squares on adjacent files that a pawn on `square` (of `color`) could
/// ever attack if it advances down its file. Used for backward-pawn detection.
pub const fn pawn_attack_span(color: Color, square: Square) -> Bitboard {
    Bitboard(forward_ranks_bb(color, square).0 & adjacent_files_bb(square).0)
}

/// All squares an enemy pawn or pawn-defender must occupy to stop a pawn on
/// `square` (of `color`) from promoting: the file in front plus both adjacent
/// files in front. Used for passed-pawn detection.
pub const fn passed_pawn_span(color: Color, square: Square) -> Bitboard {
    Bitboard(
        forward_ranks_bb(color, square).0
            & (adjacent_files_bb(square).0 | file_bb(square.file()).0),
    )
}

/// True when two squares are on differently-coloured tiles.
pub const fn opposite_colors(a: Square, b: Square) -> bool {
    let ra = (a.raw() & 7) ^ (a.raw() >> 3);
    let rb = (b.raw() & 7) ^ (b.raw() >> 3);
    (ra & 1) != (rb & 1)
}

// =========================================================================
// King-step distance (file / rank / chebyshev)
// =========================================================================

/// The file-distance between two squares.
pub const fn file_distance(a: Square, b: Square) -> u8 {
    (a.raw() & 7).abs_diff(b.raw() & 7)
}

/// The rank-distance between two squares.
pub const fn rank_distance(a: Square, b: Square) -> u8 {
    (a.raw() >> 3).abs_diff(b.raw() >> 3)
}

/// Chebyshev (king-step) distance between two squares.
pub const fn king_distance(a: Square, b: Square) -> u8 {
    let fd = file_distance(a, b);
    let rd = rank_distance(a, b);
    if fd > rd {
        fd
    } else {
        rd
    }
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
#[path = "bitboard_tests.rs"]
mod tests;
