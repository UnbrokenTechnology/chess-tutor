//! `value` types, split out of the types module.

use std::ops::{Add, AddAssign, Mul, Neg, Sub, SubAssign};

use super::*;


// =========================================================================
// Value (centipawn-ish scalar with mate / infinite constants)
// =========================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Value(pub i32);

impl Value {
    pub const ZERO: Value = Value(0);
    pub const DRAW: Value = Value(0);
    pub const KNOWN_WIN: Value = Value(10_000);
    pub const MATE: Value = Value(32_000);
    pub const INFINITE: Value = Value(32_001);
    pub const NONE: Value = Value(32_002);

    pub const MAX_PLY: i32 = 64;
    pub const MATE_IN_MAX_PLY: Value = Value(32_000 - 2 * Self::MAX_PLY);
    pub const MATED_IN_MAX_PLY: Value = Value(-32_000 + 2 * Self::MAX_PLY);

    // Piece values from the reference (middle-game, end-game). These are
    // factual parameters of the classical evaluator, not copyrightable code.
    pub const PAWN_MG: Value = Value(128);
    pub const PAWN_EG: Value = Value(213);
    pub const KNIGHT_MG: Value = Value(781);
    pub const KNIGHT_EG: Value = Value(854);
    pub const BISHOP_MG: Value = Value(825);
    pub const BISHOP_EG: Value = Value(915);
    pub const ROOK_MG: Value = Value(1276);
    pub const ROOK_EG: Value = Value(1380);
    pub const QUEEN_MG: Value = Value(2538);
    pub const QUEEN_EG: Value = Value(2682);

    pub const MIDGAME_LIMIT: Value = Value(15_258);
    pub const ENDGAME_LIMIT: Value = Value(3_915);

    pub const fn mate_in(ply: i32) -> Value {
        Value(32_000 - ply)
    }

    pub const fn mated_in(ply: i32) -> Value {
        Value(-32_000 + ply)
    }

    pub const fn abs(self) -> Value {
        Value(self.0.abs())
    }

    /// Middle-game material value of a single piece of the given kind. Kings
    /// contribute zero — the position can't exist without both kings, so
    /// there's no "king material value" in the classical-eval sense.
    pub const fn mg_of_piece(piece_type: PieceType) -> Value {
        match piece_type {
            PieceType::Pawn => Value::PAWN_MG,
            PieceType::Knight => Value::KNIGHT_MG,
            PieceType::Bishop => Value::BISHOP_MG,
            PieceType::Rook => Value::ROOK_MG,
            PieceType::Queen => Value::QUEEN_MG,
            PieceType::King => Value::ZERO,
        }
    }

    /// End-game material value of a single piece of the given kind.
    pub const fn eg_of_piece(piece_type: PieceType) -> Value {
        match piece_type {
            PieceType::Pawn => Value::PAWN_EG,
            PieceType::Knight => Value::KNIGHT_EG,
            PieceType::Bishop => Value::BISHOP_EG,
            PieceType::Rook => Value::ROOK_EG,
            PieceType::Queen => Value::QUEEN_EG,
            PieceType::King => Value::ZERO,
        }
    }
}

impl Add<Value> for Value {
    type Output = Value;
    fn add(self, rhs: Value) -> Value {
        Value(self.0 + rhs.0)
    }
}

impl Sub<Value> for Value {
    type Output = Value;
    fn sub(self, rhs: Value) -> Value {
        Value(self.0 - rhs.0)
    }
}

impl Add<i32> for Value {
    type Output = Value;
    fn add(self, rhs: i32) -> Value {
        Value(self.0 + rhs)
    }
}

impl Sub<i32> for Value {
    type Output = Value;
    fn sub(self, rhs: i32) -> Value {
        Value(self.0 - rhs)
    }
}

impl AddAssign<Value> for Value {
    fn add_assign(&mut self, rhs: Value) {
        self.0 += rhs.0;
    }
}

impl SubAssign<Value> for Value {
    fn sub_assign(&mut self, rhs: Value) {
        self.0 -= rhs.0;
    }
}

impl Neg for Value {
    type Output = Value;
    fn neg(self) -> Value {
        Value(-self.0)
    }
}

/// Per-`(color, piece_type)` material value in each game phase. Indexed by
/// the `Phase::index()` then `Piece::index()`. The array is sized to 16 so
/// the gap at discriminants 0, 7, 8, 15 can be indexed by the raw piece byte.
pub const PIECE_VALUE: [[Value; 16]; 2] = {
    let mut mg = [Value::ZERO; 16];
    let mut eg = [Value::ZERO; 16];
    mg[Piece::WhitePawn as usize] = Value::PAWN_MG;
    mg[Piece::WhiteKnight as usize] = Value::KNIGHT_MG;
    mg[Piece::WhiteBishop as usize] = Value::BISHOP_MG;
    mg[Piece::WhiteRook as usize] = Value::ROOK_MG;
    mg[Piece::WhiteQueen as usize] = Value::QUEEN_MG;
    mg[Piece::BlackPawn as usize] = Value::PAWN_MG;
    mg[Piece::BlackKnight as usize] = Value::KNIGHT_MG;
    mg[Piece::BlackBishop as usize] = Value::BISHOP_MG;
    mg[Piece::BlackRook as usize] = Value::ROOK_MG;
    mg[Piece::BlackQueen as usize] = Value::QUEEN_MG;
    eg[Piece::WhitePawn as usize] = Value::PAWN_EG;
    eg[Piece::WhiteKnight as usize] = Value::KNIGHT_EG;
    eg[Piece::WhiteBishop as usize] = Value::BISHOP_EG;
    eg[Piece::WhiteRook as usize] = Value::ROOK_EG;
    eg[Piece::WhiteQueen as usize] = Value::QUEEN_EG;
    eg[Piece::BlackPawn as usize] = Value::PAWN_EG;
    eg[Piece::BlackKnight as usize] = Value::KNIGHT_EG;
    eg[Piece::BlackBishop as usize] = Value::BISHOP_EG;
    eg[Piece::BlackRook as usize] = Value::ROOK_EG;
    eg[Piece::BlackQueen as usize] = Value::QUEEN_EG;
    [mg, eg]
};

// =========================================================================
// Score (packed mg+eg)
// =========================================================================

/// A pair of 16-bit values — one for the middle-game phase, one for the
/// endgame — packed into a single 32-bit integer. The lower 16 bits hold the
/// mg value; the upper 16 bits hold the eg value. Addition and subtraction
/// can be done in a single `i32` op; multiplication by an integer is likewise
/// componentwise because each component is sign-extended independently.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Score(pub i32);

impl Score {
    pub const ZERO: Score = Score(0);

    /// Pack a `(mg, eg)` pair. Values outside `-32768..=32767` will wrap.
    pub const fn new(mg: i32, eg: i32) -> Score {
        Score((((eg as u32) << 16).wrapping_add(mg as u32)) as i32)
    }

    /// Extract the middle-game component (sign-extended from 16 bits).
    pub const fn mg(self) -> Value {
        Value((self.0 as i16) as i32)
    }

    /// Extract the endgame component. Uses the same round-half-up trick as
    /// the reference: `(raw + 0x8000) >> 16`, sign-extended. The +0x8000
    /// offset folds any carry from the mg field into the eg field so the
    /// eg component doesn't drift when mg is negative.
    pub const fn eg(self) -> Value {
        let adjusted = (self.0 as u32).wrapping_add(0x8000);
        Value((((adjusted >> 16) as u16) as i16) as i32)
    }
}

impl Add<Score> for Score {
    type Output = Score;
    fn add(self, rhs: Score) -> Score {
        Score(self.0 + rhs.0)
    }
}

impl Sub<Score> for Score {
    type Output = Score;
    fn sub(self, rhs: Score) -> Score {
        Score(self.0 - rhs.0)
    }
}

impl AddAssign<Score> for Score {
    fn add_assign(&mut self, rhs: Score) {
        self.0 += rhs.0;
    }
}

impl SubAssign<Score> for Score {
    fn sub_assign(&mut self, rhs: Score) {
        self.0 -= rhs.0;
    }
}

impl Neg for Score {
    type Output = Score;
    fn neg(self) -> Score {
        Score(-self.0)
    }
}

/// Multiplying a Score by an integer is componentwise. We can do it in the
/// packed representation because mg and eg are independently sign-extended
/// when read back, and `i32` multiplication distributes over both halves.
impl Mul<i32> for Score {
    type Output = Score;
    fn mul(self, rhs: i32) -> Score {
        Score(self.0 * rhs)
    }
}

/// Dividing a Score by an integer must decompose and recombine, because
/// integer division does not distribute over the packed mg/eg layout
/// (the round-half-up trick used when extracting `eg` doesn't cleanly
/// invert through arithmetic on the whole packed `i32`).
impl std::ops::Div<i32> for Score {
    type Output = Score;
    fn div(self, rhs: i32) -> Score {
        Score::new(self.mg().0 / rhs, self.eg().0 / rhs)
    }
}
