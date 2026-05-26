//! `moves` types, split out of the types module.


use super::*;


// =========================================================================
// Move
// =========================================================================

/// A move, encoded in 16 bits:
///
/// ```text
/// bits  0- 5 : destination square
/// bits  6-11 : origin square
/// bits 12-13 : promotion piece type, offset so knight=0, bishop=1, rook=2, queen=3
/// bits 14-15 : special kind (0 = normal, 1 = promotion, 2 = en passant, 3 = castling)
/// ```
///
/// Two special cases `NONE` and `NULL` share from == to, which is never true
/// for a legal move, and lets them piggyback on the same storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Move(u16);

/// The special-move tag stored in bits 14-15 of a `Move`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveKind {
    Normal,
    Promotion,
    EnPassant,
    Castling,
}

impl MoveKind {
    pub const fn tag(self) -> u16 {
        match self {
            MoveKind::Normal => 0,
            MoveKind::Promotion => 1 << 14,
            MoveKind::EnPassant => 2 << 14,
            MoveKind::Castling => 3 << 14,
        }
    }

    const fn from_tag(tag: u16) -> MoveKind {
        match tag >> 14 {
            0 => MoveKind::Normal,
            1 => MoveKind::Promotion,
            2 => MoveKind::EnPassant,
            3 => MoveKind::Castling,
            _ => MoveKind::Normal,
        }
    }
}

impl Move {
    pub const NONE: Move = Move(0);
    pub const NULL: Move = Move(65);

    /// Build a plain (non-special) move between two squares.
    pub const fn normal(from: Square, to: Square) -> Move {
        Move(((from.0 as u16) << 6) | (to.0 as u16))
    }

    pub const fn promotion(from: Square, to: Square, promoted_to: PieceType) -> Move {
        let kind_bits = MoveKind::Promotion.tag();
        // Knight=1 in PieceType, but we want it to map to 0 in bits 12-13.
        let promo_bits = ((promoted_to as u16).wrapping_sub(PieceType::Knight as u16)) << 12;
        Move(kind_bits | promo_bits | ((from.0 as u16) << 6) | (to.0 as u16))
    }

    pub const fn en_passant(from: Square, to: Square) -> Move {
        Move(MoveKind::EnPassant.tag() | ((from.0 as u16) << 6) | (to.0 as u16))
    }

    pub const fn castling(from: Square, to: Square) -> Move {
        Move(MoveKind::Castling.tag() | ((from.0 as u16) << 6) | (to.0 as u16))
    }

    pub const fn from(self) -> Square {
        Square(((self.0 >> 6) & 0x3F) as u8)
    }

    pub const fn to(self) -> Square {
        Square((self.0 & 0x3F) as u8)
    }

    pub const fn kind(self) -> MoveKind {
        MoveKind::from_tag(self.0 & (3 << 14))
    }

    /// Only valid when `kind() == Promotion`.
    pub const fn promoted_to(self) -> PieceType {
        let raw = ((self.0 >> 12) & 3) + PieceType::Knight as u16;
        unsafe { std::mem::transmute::<u8, PieceType>(raw as u8) }
    }

    pub const fn raw(self) -> u16 {
        self.0
    }

    /// Reconstruct a `Move` from its packed 16-bit form. Callers should
    /// only pass values originally produced by another `Move` (e.g., by
    /// reading from a TT slot); arbitrary inputs aren't validated here.
    pub const fn from_raw(raw: u16) -> Move {
        Move(raw)
    }

    /// Returns true if this slot holds an actual move, not `NONE`/`NULL`.
    pub const fn is_valid(self) -> bool {
        ((self.0 >> 6) & 0x3F) != (self.0 & 0x3F)
    }
}
