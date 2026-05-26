//! `misc` types, split out of the types module.

use std::ops::{Add, BitAnd, BitOr, BitXor, Not, Sub};

use super::*;


// =========================================================================
// Phase / ScaleFactor / Bound
// =========================================================================

/// Game phase interpolation weight. 0 = pure endgame, 128 = pure middle game.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Phase(pub i32);

impl Phase {
    pub const ENDGAME: Phase = Phase(0);
    pub const MIDGAME: Phase = Phase(128);
}

/// Scale factor for interpolating between a tapered score and zero in drawish
/// material configurations. 64 = normal (no scaling), 0 = draw, 128 = maximum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScaleFactor(pub i32);

impl ScaleFactor {
    pub const DRAW: ScaleFactor = ScaleFactor(0);
    pub const NORMAL: ScaleFactor = ScaleFactor(64);
    pub const MAX: ScaleFactor = ScaleFactor(128);
    pub const NONE: ScaleFactor = ScaleFactor(255);
}

/// Transposition-table bound kind.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bound {
    None = 0,
    Upper = 1,
    Lower = 2,
    Exact = 3,
}

impl Bound {
    pub const fn from_u8(raw: u8) -> Bound {
        match raw & 0x3 {
            1 => Bound::Upper,
            2 => Bound::Lower,
            3 => Bound::Exact,
            _ => Bound::None,
        }
    }

    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

// =========================================================================
// Depth
// =========================================================================

/// A search depth, measured in plies. Positive values are regular
/// alpha-beta depths; zero and negative values are used by the
/// quiescence search to distinguish "still checking checks" from "only
/// recapturing" and the like. Matches Stockfish 11's `Depth` enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Depth(pub i32);

impl Depth {
    /// Amount added to a `Depth` before it's written to an 8-bit TT slot,
    /// so the smallest legal depth (`NONE`) maps to zero in the packed
    /// representation. Matches the reference's `DEPTH_OFFSET`.
    pub const OFFSET: i32 = -6;

    pub const NONE: Depth = Depth(-6);
    pub const QS_CHECKS: Depth = Depth(0);
    pub const QS_NO_CHECKS: Depth = Depth(-1);
    pub const QS_RECAPTURES: Depth = Depth(-5);

    pub const fn from_raw(raw: i32) -> Depth {
        Depth(raw)
    }
}

impl Add<i32> for Depth {
    type Output = Depth;
    fn add(self, rhs: i32) -> Depth {
        Depth(self.0 + rhs)
    }
}

impl Sub<i32> for Depth {
    type Output = Depth;
    fn sub(self, rhs: i32) -> Depth {
        Depth(self.0 - rhs)
    }
}

// =========================================================================
// CastlingRights
// =========================================================================

/// Castling-rights bitset. Four independent flags (WhiteKing, WhiteQueen,
/// BlackKing, BlackQueen) packed into the low four bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct CastlingRights(pub u8);

impl CastlingRights {
    pub const NONE: CastlingRights = CastlingRights(0);
    pub const WHITE_KING: CastlingRights = CastlingRights(0b0001);
    pub const WHITE_QUEEN: CastlingRights = CastlingRights(0b0010);
    pub const BLACK_KING: CastlingRights = CastlingRights(0b0100);
    pub const BLACK_QUEEN: CastlingRights = CastlingRights(0b1000);

    pub const WHITE: CastlingRights = CastlingRights(0b0011);
    pub const BLACK: CastlingRights = CastlingRights(0b1100);
    pub const KING_SIDE: CastlingRights = CastlingRights(0b0101);
    pub const QUEEN_SIDE: CastlingRights = CastlingRights(0b1010);
    pub const ALL: CastlingRights = CastlingRights(0b1111);

    pub const fn contains(self, other: CastlingRights) -> bool {
        (self.0 & other.0) == other.0
    }

    pub const fn intersects(self, other: CastlingRights) -> bool {
        (self.0 & other.0) != 0
    }

    pub const fn for_color(color: Color) -> CastlingRights {
        match color {
            Color::White => CastlingRights::WHITE,
            Color::Black => CastlingRights::BLACK,
        }
    }
}

impl BitOr for CastlingRights {
    type Output = CastlingRights;
    fn bitor(self, rhs: CastlingRights) -> CastlingRights {
        CastlingRights(self.0 | rhs.0)
    }
}

impl BitAnd for CastlingRights {
    type Output = CastlingRights;
    fn bitand(self, rhs: CastlingRights) -> CastlingRights {
        CastlingRights(self.0 & rhs.0)
    }
}

impl BitXor for CastlingRights {
    type Output = CastlingRights;
    fn bitxor(self, rhs: CastlingRights) -> CastlingRights {
        CastlingRights(self.0 ^ rhs.0)
    }
}

impl Not for CastlingRights {
    type Output = CastlingRights;
    fn not(self) -> CastlingRights {
        CastlingRights(!self.0 & 0b1111)
    }
}
