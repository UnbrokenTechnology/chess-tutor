//! `direction` types, split out of the types module.

use std::ops::{Add, Sub};

use super::*;


// =========================================================================
// Direction
// =========================================================================

/// Signed offset between two squares when their difference is a single-step
/// move in one of the eight king-move directions (or a double pawn push).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Direction(pub i8);

impl Direction {
    pub const NORTH: Direction = Direction(8);
    pub const SOUTH: Direction = Direction(-8);
    pub const EAST: Direction = Direction(1);
    pub const WEST: Direction = Direction(-1);
    pub const NORTH_EAST: Direction = Direction(9);
    pub const NORTH_WEST: Direction = Direction(7);
    pub const SOUTH_EAST: Direction = Direction(-7);
    pub const SOUTH_WEST: Direction = Direction(-9);

    /// Single-step pawn push direction for the given color.
    pub const fn pawn_push(color: Color) -> Direction {
        match color {
            Color::White => Direction::NORTH,
            Color::Black => Direction::SOUTH,
        }
    }
}

impl Add<Direction> for Square {
    type Output = Square;
    fn add(self, d: Direction) -> Square {
        Square((self.0 as i16 + d.0 as i16) as u8)
    }
}

impl Sub<Direction> for Square {
    type Output = Square;
    fn sub(self, d: Direction) -> Square {
        Square((self.0 as i16 - d.0 as i16) as u8)
    }
}

impl Add<Direction> for Direction {
    type Output = Direction;
    fn add(self, rhs: Direction) -> Direction {
        Direction(self.0 + rhs.0)
    }
}
