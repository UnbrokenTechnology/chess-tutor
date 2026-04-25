//! Core value types used throughout the engine: colors, piece types, pieces,
//! squares, files, ranks, directions, values/scores, bitboards, and moves.
//!
//! Design notes:
//!
//! - `Square` is a newtype over `u8` with the discriminant 64 reserved as a
//!   sentinel (`Square::NONE`). A valid square is always in `0..=63`. This
//!   matches the reference's packed move encoding, where a "none" square
//!   still needs to fit in the 6 bits used for from/to.
//!
//! - `Piece` is a strict enum with discriminants `1..=6` (white) and `9..=14`
//!   (black). The gap at 7-8 isn't arbitrary: `piece >> 3` gives the color
//!   and `piece & 7` gives the piece type, both zero-allocated. Empty squares
//!   are represented by `Option<Piece>` — the niche at 0 lets `Option<Piece>`
//!   still fit in one byte.
//!
//! - `Score` packs two `i16` values (middle-game + end-game) into a single
//!   `i32`. Carrying the pair around as one value is how the reference
//!   amortises the cost of tapered evaluation across search nodes.

use std::ops::{Add, AddAssign, BitAnd, BitOr, BitXor, Mul, Neg, Not, Sub, SubAssign};

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
pub struct Square(u8);

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

    pub const MAX_PLY: i32 = 246;
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

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Color --------------------------------------------------------

    #[test]
    fn color_toggle_is_involutive() {
        assert_eq!(!Color::White, Color::Black);
        assert_eq!(!Color::Black, Color::White);
        assert_eq!(!!Color::White, Color::White);
    }

    // ---- Piece --------------------------------------------------------

    #[test]
    fn piece_new_round_trips_through_color_and_kind() {
        for &color in &Color::both() {
            for &kind in &[
                PieceType::Pawn,
                PieceType::Knight,
                PieceType::Bishop,
                PieceType::Rook,
                PieceType::Queen,
                PieceType::King,
            ] {
                let p = Piece::new(color, kind);
                assert_eq!(p.color(), color);
                assert_eq!(p.kind(), kind);
            }
        }
    }

    #[test]
    fn piece_flip_color_swaps_sides() {
        assert_eq!(Piece::WhitePawn.flip_color(), Piece::BlackPawn);
        assert_eq!(Piece::BlackQueen.flip_color(), Piece::WhiteQueen);
    }

    #[test]
    fn option_piece_fits_in_one_byte() {
        // Niche optimisation: Piece's discriminants skip 0, 7, 8, 15 so
        // Option<Piece> should reuse the zero slot for None.
        assert_eq!(std::mem::size_of::<Option<Piece>>(), 1);
    }

    // ---- Square -------------------------------------------------------

    #[test]
    fn square_file_and_rank_decompose_index() {
        for i in 0u8..64 {
            let sq = Square::from_index(i);
            let f = sq.file().index() as u8;
            let r = sq.rank().index() as u8;
            assert_eq!(f, i & 7);
            assert_eq!(r, i >> 3);
            assert_eq!(Square::new(sq.file(), sq.rank()), sq);
        }
    }

    #[test]
    fn square_flip_vertical_swaps_ranks() {
        assert_eq!(Square::A1.flip_vertical(), Square::A8);
        assert_eq!(Square::H8.flip_vertical(), Square(7)); // H1
    }

    #[test]
    fn square_from_perspective_mirrors_for_black() {
        assert_eq!(Square::E4.from_perspective(Color::White), Square::E4);
        // E4 (rank 4) from Black's view is rank 5, same file => E5.
        assert_eq!(Square::E4.from_perspective(Color::Black), Square::E5);
    }

    #[test]
    fn square_algebraic_roundtrip() {
        for i in 0u8..64 {
            let sq = Square::from_index(i);
            let s = sq.to_algebraic();
            assert_eq!(Square::from_algebraic(&s), Some(sq));
        }
    }

    #[test]
    fn square_algebraic_rejects_garbage() {
        assert_eq!(Square::from_algebraic(""), None);
        assert_eq!(Square::from_algebraic("z3"), None);
        assert_eq!(Square::from_algebraic("a9"), None);
        assert_eq!(Square::from_algebraic("e44"), None);
    }

    #[test]
    fn square_plus_direction_steps_by_offset() {
        assert_eq!(Square::E4 + Direction::NORTH, Square::E5);
        assert_eq!(Square::E4 + Direction::SOUTH, Square(20)); // E3
        assert_eq!(Square::E4 + Direction::EAST, Square(29)); // F4
    }

    // ---- File / Rank --------------------------------------------------

    #[test]
    fn file_folds_to_queenside() {
        assert_eq!(File::A.fold_to_queenside(), File::A);
        assert_eq!(File::D.fold_to_queenside(), File::D);
        assert_eq!(File::E.fold_to_queenside(), File::D);
        assert_eq!(File::H.fold_to_queenside(), File::A);
    }

    #[test]
    fn rank_from_perspective_flips_for_black() {
        assert_eq!(Rank::R1.from_perspective(Color::White), Rank::R1);
        assert_eq!(Rank::R1.from_perspective(Color::Black), Rank::R8);
        assert_eq!(Rank::R4.from_perspective(Color::Black), Rank::R5);
    }

    // ---- Score packing ------------------------------------------------

    #[test]
    fn score_new_round_trips_positive_pairs() {
        let s = Score::new(100, 200);
        assert_eq!(s.mg(), Value(100));
        assert_eq!(s.eg(), Value(200));
    }

    #[test]
    fn score_new_round_trips_negative_mg() {
        // The rounding trick in eg() exists precisely so a negative mg value
        // doesn't drag the eg value down by one.
        let s = Score::new(-1, 200);
        assert_eq!(s.mg(), Value(-1));
        assert_eq!(s.eg(), Value(200));
    }

    #[test]
    fn score_new_round_trips_negative_both() {
        let s = Score::new(-100, -200);
        assert_eq!(s.mg(), Value(-100));
        assert_eq!(s.eg(), Value(-200));
    }

    #[test]
    fn score_new_round_trips_extremes() {
        let s = Score::new(i16::MIN as i32, i16::MAX as i32);
        assert_eq!(s.mg(), Value(i16::MIN as i32));
        assert_eq!(s.eg(), Value(i16::MAX as i32));
    }

    #[test]
    fn score_addition_is_componentwise() {
        let a = Score::new(10, 20);
        let b = Score::new(3, 7);
        let sum = a + b;
        assert_eq!(sum.mg(), Value(13));
        assert_eq!(sum.eg(), Value(27));
    }

    #[test]
    fn score_multiplication_is_componentwise() {
        let s = Score::new(10, -20);
        let tripled = s * 3;
        assert_eq!(tripled.mg(), Value(30));
        assert_eq!(tripled.eg(), Value(-60));
    }

    #[test]
    fn score_division_is_componentwise() {
        let s = Score::new(10, -20);
        let halved = s / 2;
        assert_eq!(halved.mg(), Value(5));
        assert_eq!(halved.eg(), Value(-10));
    }

    #[test]
    fn score_negation_is_componentwise() {
        let s = Score::new(10, -20);
        let n = -s;
        assert_eq!(n.mg(), Value(-10));
        assert_eq!(n.eg(), Value(20));
    }

    // ---- Value --------------------------------------------------------

    #[test]
    fn mate_in_and_mated_in_are_symmetric() {
        for ply in 0..50 {
            assert_eq!(Value::mate_in(ply), -Value::mated_in(ply));
        }
    }

    #[test]
    fn piece_value_table_is_phase_indexed() {
        assert_eq!(PIECE_VALUE[0][Piece::WhiteQueen.index()], Value::QUEEN_MG);
        assert_eq!(PIECE_VALUE[1][Piece::BlackPawn.index()], Value::PAWN_EG);
    }

    // ---- CastlingRights ----------------------------------------------

    #[test]
    fn castling_rights_combine() {
        let all = CastlingRights::WHITE | CastlingRights::BLACK;
        assert_eq!(all, CastlingRights::ALL);
        assert!(all.contains(CastlingRights::WHITE_KING));
        assert!(all.intersects(CastlingRights::QUEEN_SIDE));
    }

    #[test]
    fn castling_rights_not_only_affects_used_bits() {
        let empty = !CastlingRights::ALL;
        assert_eq!(empty, CastlingRights::NONE);
    }

    // ---- Move encoding ------------------------------------------------

    #[test]
    fn move_normal_round_trips_from_and_to() {
        let m = Move::normal(Square::E2, /* E4 */ Square::E4);
        assert_eq!(m.from(), Square::E2);
        assert_eq!(m.to(), Square::E4);
        assert_eq!(m.kind(), MoveKind::Normal);
    }

    #[test]
    fn move_promotion_round_trips_promoted_piece() {
        let from = Square::from_index(48); // A7
        let to = Square::from_index(56); // A8
        for &pt in &[
            PieceType::Knight,
            PieceType::Bishop,
            PieceType::Rook,
            PieceType::Queen,
        ] {
            let m = Move::promotion(from, to, pt);
            assert_eq!(m.kind(), MoveKind::Promotion);
            assert_eq!(m.from(), from);
            assert_eq!(m.to(), to);
            assert_eq!(m.promoted_to(), pt);
        }
    }

    #[test]
    fn move_en_passant_and_castling_tags() {
        let ep = Move::en_passant(Square::from_index(36), Square::from_index(43));
        assert_eq!(ep.kind(), MoveKind::EnPassant);
        let cs = Move::castling(Square::E1, Square::G1);
        assert_eq!(cs.kind(), MoveKind::Castling);
    }

    #[test]
    fn move_none_and_null_are_invalid() {
        assert!(!Move::NONE.is_valid());
        assert!(!Move::NULL.is_valid());
        assert!(Move::normal(Square::E2, Square::E4).is_valid());
    }

    #[test]
    fn move_from_raw_round_trips_packed_bits() {
        // Packing a move and reconstructing from the raw u16 must
        // reproduce an identical `Move`. This is the property the TT
        // relies on when storing and reloading moves.
        let cases = [
            Move::normal(Square::E2, Square::E4),
            Move::en_passant(Square::E5, Square::D6),
            Move::castling(Square::E1, Square::G1),
            Move::promotion(Square::A7, Square::A8, PieceType::Queen),
            Move::NONE,
        ];
        for m in cases {
            assert_eq!(Move::from_raw(m.raw()), m);
        }
    }

    // ---- Depth -------------------------------------------------------

    #[test]
    fn depth_constants_match_reference() {
        // Stockfish 11: DEPTH_NONE = -6, DEPTH_QS_CHECKS = 0,
        // DEPTH_QS_NO_CHECKS = -1, DEPTH_QS_RECAPTURES = -5.
        assert_eq!(Depth::NONE.0, -6);
        assert_eq!(Depth::QS_CHECKS.0, 0);
        assert_eq!(Depth::QS_NO_CHECKS.0, -1);
        assert_eq!(Depth::QS_RECAPTURES.0, -5);
        assert_eq!(Depth::OFFSET, -6);
    }

    #[test]
    fn depth_arithmetic_adjusts_by_integer() {
        assert_eq!((Depth(4) + 2).0, 6);
        assert_eq!((Depth(4) - 2).0, 2);
    }

    // ---- Bound -------------------------------------------------------

    #[test]
    fn bound_from_u8_round_trips_all_variants() {
        // The TT packs the bound into the lower 2 bits of a status byte.
        // Decoding must recover the same variant regardless of the
        // upper bits (which hold unrelated flags).
        for b in [Bound::None, Bound::Upper, Bound::Lower, Bound::Exact] {
            for upper in 0..=255u8 {
                let packed = (upper & !0x3) | b.as_u8();
                assert_eq!(Bound::from_u8(packed), b);
            }
        }
    }
}
