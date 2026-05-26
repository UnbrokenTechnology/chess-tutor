//! `square` types, split out of the types module.


use super::*;


// =========================================================================
// File / Rank
// =========================================================================

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum File {
    A = 0,
    B = 1,
    C = 2,
    D = 3,
    E = 4,
    F = 5,
    G = 6,
    H = 7,
}

impl File {
    pub const NB: usize = 8;

    pub const fn index(self) -> usize {
        self as usize
    }

    /// Map `a..=h` → `a..=d` by reflecting across the middle. Useful for
    /// pawn/king symmetry in evaluation tables.
    pub const fn fold_to_queenside(self) -> File {
        let i = self as u8;
        let folded = if i < 4 { i } else { 7 - i };
        unsafe { std::mem::transmute::<u8, File>(folded) }
    }

    /// Try to build a `File` from its 0..=7 index. Returns `None` if out of range.
    pub const fn from_index(i: u8) -> Option<File> {
        if i < 8 {
            Some(unsafe { std::mem::transmute::<u8, File>(i) })
        } else {
            None
        }
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rank {
    R1 = 0,
    R2 = 1,
    R3 = 2,
    R4 = 3,
    R5 = 4,
    R6 = 5,
    R7 = 6,
    R8 = 7,
}

impl Rank {
    pub const NB: usize = 8;

    pub const fn index(self) -> usize {
        self as usize
    }

    /// Rank as seen from `color`'s point of view. White's 1st rank stays
    /// rank 1; for Black it's rank 8.
    pub const fn from_perspective(self, color: Color) -> Rank {
        let i = (self as u8) ^ ((color as u8) * 7);
        unsafe { std::mem::transmute::<u8, Rank>(i) }
    }

    pub const fn from_index(i: u8) -> Option<Rank> {
        if i < 8 {
            Some(unsafe { std::mem::transmute::<u8, Rank>(i) })
        } else {
            None
        }
    }
}

// =========================================================================
// Square
// =========================================================================

/// A board square. Valid squares are `0..=63`, laid out row-major from a1 to
/// h8. The sentinel `Square::NONE` (=64) is used where a move or state needs
/// to express "no square here" while remaining a plain integer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Square(pub(crate) u8);

impl Square {
    pub const NB: usize = 64;
    pub const NONE: Square = Square(64);

    pub const A1: Square = Square(0);
    pub const B1: Square = Square(1);
    pub const C1: Square = Square(2);
    pub const D1: Square = Square(3);
    pub const E1: Square = Square(4);
    pub const F1: Square = Square(5);
    pub const G1: Square = Square(6);
    pub const H1: Square = Square(7);
    pub const A2: Square = Square(8);
    pub const B2: Square = Square(9);
    pub const C2: Square = Square(10);
    pub const D2: Square = Square(11);
    pub const E2: Square = Square(12);
    pub const F2: Square = Square(13);
    pub const G2: Square = Square(14);
    pub const H2: Square = Square(15);
    pub const A3: Square = Square(16);
    pub const B3: Square = Square(17);
    pub const C3: Square = Square(18);
    pub const D3: Square = Square(19);
    pub const E3: Square = Square(20);
    pub const F3: Square = Square(21);
    pub const G3: Square = Square(22);
    pub const H3: Square = Square(23);
    pub const A4: Square = Square(24);
    pub const B4: Square = Square(25);
    pub const C4: Square = Square(26);
    pub const D4: Square = Square(27);
    pub const E4: Square = Square(28);
    pub const F4: Square = Square(29);
    pub const G4: Square = Square(30);
    pub const H4: Square = Square(31);
    pub const A5: Square = Square(32);
    pub const B5: Square = Square(33);
    pub const C5: Square = Square(34);
    pub const D5: Square = Square(35);
    pub const E5: Square = Square(36);
    pub const F5: Square = Square(37);
    pub const G5: Square = Square(38);
    pub const H5: Square = Square(39);
    pub const A6: Square = Square(40);
    pub const B6: Square = Square(41);
    pub const C6: Square = Square(42);
    pub const D6: Square = Square(43);
    pub const E6: Square = Square(44);
    pub const F6: Square = Square(45);
    pub const G6: Square = Square(46);
    pub const H6: Square = Square(47);
    pub const A7: Square = Square(48);
    pub const B7: Square = Square(49);
    pub const C7: Square = Square(50);
    pub const D7: Square = Square(51);
    pub const E7: Square = Square(52);
    pub const F7: Square = Square(53);
    pub const G7: Square = Square(54);
    pub const H7: Square = Square(55);
    pub const A8: Square = Square(56);
    pub const B8: Square = Square(57);
    pub const C8: Square = Square(58);
    pub const D8: Square = Square(59);
    pub const E8: Square = Square(60);
    pub const F8: Square = Square(61);
    pub const G8: Square = Square(62);
    pub const H8: Square = Square(63);

    pub const fn new(file: File, rank: Rank) -> Square {
        Square(((rank as u8) << 3) | (file as u8))
    }

    /// Construct from a raw 0..=63 index. Callers must ensure validity.
    pub const fn from_index(i: u8) -> Square {
        debug_assert!(i < 64);
        Square(i)
    }

    /// Same as `from_index`, returning `None` outside 0..=63.
    pub const fn try_from_index(i: u8) -> Option<Square> {
        if i < 64 {
            Some(Square(i))
        } else {
            None
        }
    }

    pub const fn index(self) -> usize {
        self.0 as usize
    }

    pub const fn raw(self) -> u8 {
        self.0
    }

    pub const fn file(self) -> File {
        unsafe { std::mem::transmute::<u8, File>(self.0 & 7) }
    }

    pub const fn rank(self) -> Rank {
        unsafe { std::mem::transmute::<u8, Rank>(self.0 >> 3) }
    }

    pub const fn is_on_board(self) -> bool {
        self.0 < 64
    }

    /// Vertical flip: `a1 ↔ a8`.
    pub const fn flip_vertical(self) -> Square {
        Square(self.0 ^ 56)
    }

    /// Square as seen from `color`'s point of view. For White it's unchanged;
    /// for Black it's vertically flipped.
    pub const fn from_perspective(self, color: Color) -> Square {
        Square(self.0 ^ ((color as u8) * 56))
    }

    /// Parse algebraic coordinates like `"e4"`.
    pub fn from_algebraic(s: &str) -> Option<Square> {
        let bytes = s.as_bytes();
        if bytes.len() != 2 {
            return None;
        }
        let file = match bytes[0] {
            b'a'..=b'h' => bytes[0] - b'a',
            _ => return None,
        };
        let rank = match bytes[1] {
            b'1'..=b'8' => bytes[1] - b'1',
            _ => return None,
        };
        Some(Square((rank << 3) | file))
    }

    pub fn to_algebraic(self) -> String {
        let file = (b'a' + (self.0 & 7)) as char;
        let rank = (b'1' + (self.0 >> 3)) as char;
        let mut s = String::with_capacity(2);
        s.push(file);
        s.push(rank);
        s
    }
}
