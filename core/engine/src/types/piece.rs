//! `piece` types, split out of the types module.


use super::*;


// =========================================================================
// PieceType
// =========================================================================

/// A piece kind without color information. Discriminants match the reference
/// so a `Piece` can be decomposed with a single `& 7`.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PieceType {
    Pawn = 1,
    Knight = 2,
    Bishop = 3,
    Rook = 4,
    Queen = 5,
    King = 6,
}

impl PieceType {
    pub const NB: usize = 6;

    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn is_slider(self) -> bool {
        matches!(self, PieceType::Bishop | PieceType::Rook | PieceType::Queen)
    }

    pub const fn is_minor(self) -> bool {
        matches!(self, PieceType::Knight | PieceType::Bishop)
    }

    /// Classical "point value" used in pedagogical contexts where the
    /// student thinks in P:1 / N:3 / B:3 / R:5 / Q:9. Distinct from
    /// the engine's tapered cp piece values (which differ by a few cp
    /// between N and B, with phase-dependent endgame inflation). The
    /// classical scale is what shows up in teaching prose, in the
    /// "is this an even trade?" parity check, and in the
    /// recapture-aware filters in the analysis crate. Returns 0 for
    /// the King (kings are never captured; the result is unused but
    /// has to be defined for exhaustiveness).
    pub const fn classical_points(self) -> u8 {
        match self {
            PieceType::Pawn => 1,
            PieceType::Knight | PieceType::Bishop => 3,
            PieceType::Rook => 5,
            PieceType::Queen => 9,
            PieceType::King => 0,
        }
    }
}

// =========================================================================
// Piece
// =========================================================================

/// A piece with its color. Empty squares use `Option<Piece>`.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Piece {
    WhitePawn = 1,
    WhiteKnight = 2,
    WhiteBishop = 3,
    WhiteRook = 4,
    WhiteQueen = 5,
    WhiteKing = 6,
    BlackPawn = 9,
    BlackKnight = 10,
    BlackBishop = 11,
    BlackRook = 12,
    BlackQueen = 13,
    BlackKing = 14,
}

impl Piece {
    /// Build a piece from a color and a piece type.
    pub const fn new(color: Color, kind: PieceType) -> Piece {
        // SAFETY: color << 3 is 0 (White) or 8 (Black); kind is 1..=6; sum is
        // always one of the 12 defined discriminants.
        let raw = ((color as u8) << 3) | (kind as u8);
        unsafe { std::mem::transmute::<u8, Piece>(raw) }
    }

    pub const fn color(self) -> Color {
        if (self as u8) >> 3 == 0 {
            Color::White
        } else {
            Color::Black
        }
    }

    pub const fn kind(self) -> PieceType {
        // SAFETY: for any defined Piece discriminant, `& 7` produces a value in
        // 1..=6 which is always a valid PieceType.
        let raw = (self as u8) & 7;
        unsafe { std::mem::transmute::<u8, PieceType>(raw) }
    }

    pub const fn index(self) -> usize {
        self as usize
    }

    /// Flip to the same piece type in the opposite color.
    pub const fn flip_color(self) -> Piece {
        let raw = (self as u8) ^ 8;
        unsafe { std::mem::transmute::<u8, Piece>(raw) }
    }
}
