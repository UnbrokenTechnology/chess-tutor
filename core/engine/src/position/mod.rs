//! The board state: piece placement, side to move, castling rights, en
//! passant, move clocks, and Zobrist key. Plus FEN parse/serialize, basic
//! queries every other module needs (piece lookups, occupancy, attackers),
//! and `do_move` / `undo_move` with a `StateInfo` carrying everything
//! needed to reverse a move.
//!
//! Still deferred: pin/checker detection (for legality and check-evasion
//! move generation) and repetition tracking.
//!
//! Invariant: all the redundant representations — the `[Option<Piece>; 64]`
//! mailbox, the bitboards indexed by color and piece type, the Zobrist
//! `key`, and the piece-square-table `psq` score — are always in sync.
//! `do_move` / `undo_move` maintain them incrementally; the
//! `compute_*_from_scratch()` helpers exist as test oracles to catch drift.

mod attack_queries;
mod blockers;
mod fen;
mod make_move;
mod queries;
mod see;

pub use fen::FenError;
pub use make_move::StateInfo;

use crate::bitboard::Bitboard;
use crate::types::{CastlingRights, Color, Piece, PieceType, Score, Square, Value};

// =========================================================================
// The Position type
// =========================================================================

/// A fully-specified chess position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Position {
    /// Piece-on-square mailbox. `None` on empty squares.
    pub(crate) board: [Option<Piece>; 64],
    /// Bitboard per piece type. Indexed by `PieceType::index()` which runs
    /// 1..=6, so slot 0 is unused. Paying 8 bytes of padding to skip a
    /// subtraction on every query is fine.
    pub(crate) by_kind: [Bitboard; 7],
    /// Bitboard per color. Indexed by `Color::index()`.
    pub(crate) by_color: [Bitboard; 2],

    pub(crate) side_to_move: Color,
    pub(crate) castling_rights: CastlingRights,
    /// The target square for a legal en-passant capture (the empty square
    /// the capturing pawn would land on), or `None`.
    pub(crate) en_passant: Option<Square>,
    /// Halfmoves since the last pawn move or capture (the 50-move rule
    /// counts this to 100).
    pub(crate) halfmove_clock: u16,
    /// Fullmove counter, starting at 1 and incrementing after each black move.
    pub(crate) fullmove_number: u16,
    /// Zobrist hash of the current position. Maintained incrementally by
    /// `do_move` / `undo_move`; set from scratch by `from_fen`.
    pub(crate) key: u64,
    /// Zobrist hash over pawn placement only. Two positions with identical
    /// pawns hash to the same `pawn_key`, which is what lets a pawn-structure
    /// evaluator cache its result across positions that share only their
    /// pawns. Maintained incrementally alongside `key` but XOR-ing only
    /// pawn-on-square contributions. Initialised to the `no_pawns` base so
    /// empty-pawn-structure positions still hash to a stable non-zero value.
    pub(crate) pawn_key: u64,
    /// Piece-square-table score: sum of `psq_score(piece, square)` over
    /// every occupied square. Includes both material value and positional
    /// preference. White pieces contribute positive, black negative, so a
    /// symmetric position is zero. Maintained incrementally.
    pub(crate) psq: Score,
    /// Sum of middle-game material values for non-pawn, non-king pieces of
    /// each color. Drives game-phase interpolation (lots of non-pawn
    /// material = middlegame; little = endgame). Maintained incrementally.
    pub(crate) non_pawn_material: [Value; 2],

    // --- Cached check info (B3) -----------------------------------------
    // Recomputed once per `do_move` / `do_null_move` / `from_fen` by
    // `compute_check_info`, saved/restored across undo via `StateInfo`.
    // Lets the per-move `checkers()` / `blockers_for_king()` / SEE /
    // `legal()` / `gives_check()` reads be O(1) instead of recomputing
    // `attackers_to` / `slider_blockers` each call.
    /// Enemy pieces giving check to the side-to-move's king.
    pub(crate) checkers: Bitboard,
    /// Per color: that color's own pieces pinned/blocking against its king.
    pub(crate) king_blockers: [Bitboard; 2],
    /// Per color: the enemy sliders pinning a `king_blockers` piece.
    pub(crate) king_pinners: [Bitboard; 2],
}

impl Default for Position {
    /// The standard starting position.
    fn default() -> Position {
        Position::startpos()
    }
}

impl Position {
    /// The standard starting position.
    pub fn startpos() -> Position {
        // Unwrap is safe: the startpos FEN is a compile-time constant and is
        // valid by construction.
        Position::from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap()
    }

    // ----- accessors -------------------------------------------------------

    pub fn side_to_move(&self) -> Color {
        self.side_to_move
    }

    pub fn castling_rights(&self) -> CastlingRights {
        self.castling_rights
    }

    pub fn en_passant(&self) -> Option<Square> {
        self.en_passant
    }

    pub fn halfmove_clock(&self) -> u16 {
        self.halfmove_clock
    }

    pub fn fullmove_number(&self) -> u16 {
        self.fullmove_number
    }

    /// The Zobrist hash of this position.
    pub fn key(&self) -> u64 {
        self.key
    }

    /// The Zobrist hash of this position's pawn structure alone.
    pub fn pawn_key(&self) -> u64 {
        self.pawn_key
    }

    /// The incrementally-maintained piece-square-table score (material +
    /// positional preference). White is positive, black negative.
    pub fn psq_score(&self) -> Score {
        self.psq
    }

    /// Sum of middle-game material values for this color's non-pawn pieces.
    /// Kings contribute zero.
    pub fn non_pawn_material(&self, color: Color) -> Value {
        self.non_pawn_material[color.index()]
    }

    pub fn piece_on(&self, square: Square) -> Option<Piece> {
        self.board[square.index()]
    }

    /// Bitboard of every piece of the given type, both colors.
    pub fn pieces(&self, piece_type: PieceType) -> Bitboard {
        self.by_kind[piece_type.index()]
    }

    /// Bitboard of every piece of the given color, any type.
    pub fn pieces_by_color(&self, color: Color) -> Bitboard {
        self.by_color[color.index()]
    }

    /// Bitboard of pieces matching both a color and a type.
    pub fn pieces_of(&self, color: Color, piece_type: PieceType) -> Bitboard {
        self.by_kind[piece_type.index()] & self.by_color[color.index()]
    }

    /// Every occupied square on the board.
    pub fn occupied(&self) -> Bitboard {
        self.by_color[0] | self.by_color[1]
    }

    /// The square the king of the given color is on. Every legal position
    /// has exactly one king per color, so this is well-defined.
    pub fn king_square(&self, color: Color) -> Square {
        self.pieces_of(color, PieceType::King).lsb()
    }

    /// Number of pieces of the given `(color, type)` combination.
    pub fn count(&self, color: Color, piece_type: PieceType) -> u32 {
        self.pieces_of(color, piece_type).popcount()
    }
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startpos_has_32_pieces_in_the_expected_places() {
        let p = Position::startpos();
        assert_eq!(p.occupied().popcount(), 32);
        assert_eq!(p.pieces(PieceType::Pawn).popcount(), 16);
        assert_eq!(p.pieces_of(Color::White, PieceType::King).popcount(), 1);
        assert_eq!(p.pieces_of(Color::Black, PieceType::King).popcount(), 1);
        assert_eq!(p.king_square(Color::White), Square::E1);
        assert_eq!(p.king_square(Color::Black), Square::E8);
        assert_eq!(p.piece_on(Square::A1), Some(Piece::WhiteRook));
        assert_eq!(p.piece_on(Square::D1), Some(Piece::WhiteQueen));
        assert_eq!(p.piece_on(Square::E4), None);
        assert_eq!(p.side_to_move(), Color::White);
        assert_eq!(p.castling_rights(), CastlingRights::ALL);
        assert_eq!(p.en_passant(), None);
        assert_eq!(p.halfmove_clock(), 0);
        assert_eq!(p.fullmove_number(), 1);
    }

    #[test]
    fn default_is_startpos() {
        assert_eq!(Position::default(), Position::startpos());
    }

    #[test]
    fn bitboards_and_mailbox_agree_on_startpos() {
        let p = Position::startpos();
        for i in 0u8..64 {
            let sq = Square::from_index(i);
            match p.piece_on(sq) {
                None => {
                    assert!(
                        !p.occupied().contains(sq),
                        "mailbox empty but occupied bit set"
                    );
                }
                Some(piece) => {
                    assert!(p.occupied().contains(sq));
                    assert!(p.pieces(piece.kind()).contains(sq));
                    assert!(p.pieces_by_color(piece.color()).contains(sq));
                }
            }
        }
    }

    #[test]
    fn pieces_of_is_the_intersection() {
        let p = Position::startpos();
        for &color in &Color::both() {
            for &pt in &[
                PieceType::Pawn,
                PieceType::Knight,
                PieceType::Bishop,
                PieceType::Rook,
                PieceType::Queen,
                PieceType::King,
            ] {
                assert_eq!(
                    p.pieces_of(color, pt),
                    p.pieces(pt) & p.pieces_by_color(color),
                );
            }
        }
    }

    #[test]
    fn count_returns_piece_counts() {
        let p = Position::startpos();
        assert_eq!(p.count(Color::White, PieceType::Pawn), 8);
        assert_eq!(p.count(Color::Black, PieceType::Pawn), 8);
        assert_eq!(p.count(Color::White, PieceType::Knight), 2);
        assert_eq!(p.count(Color::White, PieceType::King), 1);
    }
}
