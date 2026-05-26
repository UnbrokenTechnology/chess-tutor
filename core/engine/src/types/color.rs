//! `color` types, split out of the types module.

use std::ops::Not;



// =========================================================================
// Color
// =========================================================================

/// One of the two sides in a chess game.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Color {
    White = 0,
    Black = 1,
}

impl Color {
    pub const NB: usize = 2;

    /// Iterate over both colors, white then black.
    pub const fn both() -> [Color; 2] {
        [Color::White, Color::Black]
    }

    pub const fn index(self) -> usize {
        self as usize
    }
}

impl Not for Color {
    type Output = Color;
    fn not(self) -> Color {
        match self {
            Color::White => Color::Black,
            Color::Black => Color::White,
        }
    }
}
