//! Stockfish 11 per-piece-type weight tables. Numerical parameters carry
//! over from the reference verbatim — they are factual data, not
//! expression. Consumed by the scoring functions in [`super`].

use crate::types::Score;

// =========================================================================
// Weight tables
// =========================================================================

/// Mobility bonuses by attacked-square count, one row per piece type.
/// Unused entries beyond each piece's maximum mobility are never indexed
/// in practice but kept zero for safety.
pub(super) const MOBILITY_KNIGHT: [Score; 9] = [
    Score::new(-62, -81),
    Score::new(-53, -56),
    Score::new(-12, -30),
    Score::new(-4, -14),
    Score::new(3, 8),
    Score::new(13, 15),
    Score::new(22, 23),
    Score::new(28, 27),
    Score::new(33, 33),
];
pub(super) const MOBILITY_BISHOP: [Score; 14] = [
    Score::new(-48, -59),
    Score::new(-20, -23),
    Score::new(16, -3),
    Score::new(26, 13),
    Score::new(38, 24),
    Score::new(51, 42),
    Score::new(55, 54),
    Score::new(63, 57),
    Score::new(63, 65),
    Score::new(68, 73),
    Score::new(81, 78),
    Score::new(81, 86),
    Score::new(91, 88),
    Score::new(98, 97),
];
pub(super) const MOBILITY_ROOK: [Score; 15] = [
    Score::new(-58, -76),
    Score::new(-27, -18),
    Score::new(-15, 28),
    Score::new(-10, 55),
    Score::new(-5, 69),
    Score::new(-2, 82),
    Score::new(9, 112),
    Score::new(16, 118),
    Score::new(30, 132),
    Score::new(29, 142),
    Score::new(32, 155),
    Score::new(38, 165),
    Score::new(46, 166),
    Score::new(48, 169),
    Score::new(58, 171),
];
pub(super) const MOBILITY_QUEEN: [Score; 28] = [
    Score::new(-39, -36),
    Score::new(-21, -15),
    Score::new(3, 8),
    Score::new(3, 18),
    Score::new(14, 34),
    Score::new(22, 54),
    Score::new(28, 61),
    Score::new(41, 73),
    Score::new(43, 79),
    Score::new(48, 92),
    Score::new(56, 94),
    Score::new(60, 104),
    Score::new(60, 113),
    Score::new(66, 120),
    Score::new(67, 123),
    Score::new(70, 126),
    Score::new(71, 133),
    Score::new(73, 136),
    Score::new(79, 140),
    Score::new(88, 143),
    Score::new(88, 148),
    Score::new(99, 166),
    Score::new(102, 170),
    Score::new(102, 175),
    Score::new(106, 184),
    Score::new(109, 191),
    Score::new(113, 206),
    Score::new(116, 212),
];

/// King-attack weight per piece type. Indexed by `PieceType::index()`
/// (1..=6); slot 0 unused.
pub(super) const KING_ATTACK_WEIGHT: [i32; 7] = [0, 0, 81, 52, 44, 10, 0];

/// Rook on a semi-open / fully-open file.
pub(super) const ROOK_ON_FILE: [Score; 2] = [Score::new(21, 4), Score::new(47, 25)];

pub(super) const BISHOP_PAWNS: Score = Score::new(3, 7);
pub(super) const KING_PROTECTOR: Score = Score::new(7, 8);
pub(super) const LONG_DIAGONAL_BISHOP: Score = Score::new(45, 0);
pub(super) const MINOR_BEHIND_PAWN: Score = Score::new(18, 3);
pub(super) const OUTPOST: Score = Score::new(30, 21);
pub(super) const REACHABLE_OUTPOST: Score = Score::new(32, 10);
pub(super) const ROOK_ON_QUEEN_FILE: Score = Score::new(7, 6);
pub(super) const TRAPPED_ROOK: Score = Score::new(52, 10);
pub(super) const WEAK_QUEEN: Score = Score::new(49, 15);
