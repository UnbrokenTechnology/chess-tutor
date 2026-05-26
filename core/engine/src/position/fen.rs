//! FEN parsing and serialisation, plus the `compute_*_from_scratch`
//! oracles used by `from_fen` to initialise the incrementally-maintained
//! hash, psq, and material totals. Those same oracles double as test
//! cross-checks against the incremental maintenance done by `do_move`.

use std::fmt;

use super::Position;
use crate::bitboard::{square_bb, Bitboard};
use crate::psqt::psq_score;
use crate::types::{CastlingRights, Color, File, Piece, PieceType, Rank, Score, Square, Value};
use crate::zobrist;

/// An error encountered while parsing a FEN string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FenError {
    /// A required whitespace-separated field wasn't present.
    MissingField(&'static str),
    /// The piece-placement field wasn't eight ranks, or contained an illegal
    /// character or run-length.
    BadPiecePlacement(String),
    /// The side-to-move field wasn't "w" or "b".
    BadSideToMove,
    /// The castling-rights field contained something other than "-" or a
    /// subset of "KQkq".
    BadCastlingRights,
    /// The en-passant field wasn't "-" or a valid algebraic square.
    BadEnPassant,
    /// The halfmove or fullmove field didn't parse as a number.
    BadClock(&'static str),
    /// A color was missing its king (every legal position has exactly one
    /// king per side).
    MissingKing(Color),
}

impl fmt::Display for FenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FenError::MissingField(name) => write!(f, "FEN is missing the {} field", name),
            FenError::BadPiecePlacement(msg) => write!(f, "bad piece placement: {}", msg),
            FenError::BadSideToMove => write!(f, "side-to-move must be 'w' or 'b'"),
            FenError::BadCastlingRights => {
                write!(f, "castling rights must be '-' or a subset of 'KQkq'")
            }
            FenError::BadEnPassant => {
                write!(f, "en-passant square is not a valid algebraic square")
            }
            FenError::BadClock(which) => write!(f, "{} clock did not parse as a number", which),
            FenError::MissingKing(color) => write!(f, "position has no {:?} king", color),
        }
    }
}

impl std::error::Error for FenError {}

impl Position {
    /// Parse a FEN record.
    pub fn from_fen(fen: &str) -> Result<Position, FenError> {
        let mut fields = fen.split_ascii_whitespace();
        let placement = fields
            .next()
            .ok_or(FenError::MissingField("piece placement"))?;
        let side = fields
            .next()
            .ok_or(FenError::MissingField("side to move"))?;
        let castling = fields
            .next()
            .ok_or(FenError::MissingField("castling rights"))?;
        let ep = fields.next().ok_or(FenError::MissingField("en passant"))?;
        let halfmove = fields
            .next()
            .ok_or(FenError::MissingField("halfmove clock"))?;
        let fullmove = fields
            .next()
            .ok_or(FenError::MissingField("fullmove number"))?;

        let (board, by_kind, by_color) = parse_placement(placement)?;

        let side_to_move = match side {
            "w" => Color::White,
            "b" => Color::Black,
            _ => return Err(FenError::BadSideToMove),
        };

        let castling_rights = parse_castling(castling)?;

        let en_passant = if ep == "-" {
            None
        } else {
            Some(Square::from_algebraic(ep).ok_or(FenError::BadEnPassant)?)
        };

        let halfmove_clock: u16 = halfmove
            .parse()
            .map_err(|_| FenError::BadClock("halfmove"))?;
        let fullmove_number: u16 = fullmove
            .parse()
            .map_err(|_| FenError::BadClock("fullmove"))?;

        let mut position = Position {
            board,
            by_kind,
            by_color,
            side_to_move,
            castling_rights,
            en_passant,
            halfmove_clock,
            fullmove_number,
            key: 0,
            pawn_key: 0,
            psq: Score::ZERO,
            non_pawn_material: [Value::ZERO; 2],
            checkers: Bitboard::EMPTY,
            king_blockers: [Bitboard::EMPTY; 2],
            king_pinners: [Bitboard::EMPTY; 2],
        };

        // Exactly one king per color is a hard prerequisite for any of the
        // king-dependent queries (king_square, check detection, legality)
        // to work, so enforce it up front.
        for &color in &Color::both() {
            if position.pieces_of(color, PieceType::King).popcount() != 1 {
                return Err(FenError::MissingKing(color));
            }
        }

        // An en-passant square is only meaningful if a side-to-move pawn
        // can actually capture onto it (SF11 position.cpp:262-273). Drop a
        // "phantom" ep so `key()` matches SF and stays consistent with
        // do_move's gated ep handling (P1) — otherwise a FEN-loaded
        // position and the same position reached by playing moves would
        // hash differently, breaking TT/repetition matches. The capturing
        // side is the side to move; the pushed pawn is the opponent's, so
        // a pusher-coloured pawn on ep_sq attacks exactly the squares the
        // capturing pawns would sit on.
        if let Some(ep_sq) = position.en_passant {
            let capturer = position.side_to_move;
            let capturer_pawns = position.pieces_of(capturer, PieceType::Pawn);
            if (crate::attacks::pawn_attacks_from(!capturer, ep_sq) & capturer_pawns).is_empty() {
                position.en_passant = None;
            }
        }

        position.key = position.compute_key_from_scratch();
        position.pawn_key = position.compute_pawn_key_from_scratch();
        position.psq = position.compute_psq_from_scratch();
        position.non_pawn_material = position.compute_non_pawn_material_from_scratch();
        position.compute_check_info();
        Ok(position)
    }

    /// Recompute the pawn-only Zobrist key from the current board. Used by
    /// `from_fen` and as a test oracle against the incremental update.
    pub(crate) fn compute_pawn_key_from_scratch(&self) -> u64 {
        let mut key = zobrist::no_pawns_key();
        for i in 0u8..64 {
            let sq = Square::from_index(i);
            if let Some(piece) = self.board[sq.index()] {
                if piece.kind() == PieceType::Pawn {
                    key ^= zobrist::piece_square_key(piece, sq);
                }
            }
        }
        key
    }

    /// Recompute the piece-square-table score from scratch. Used by
    /// `from_fen` and as a test oracle against the incremental maintenance.
    pub(crate) fn compute_psq_from_scratch(&self) -> Score {
        let mut total = Score::ZERO;
        for i in 0u8..64 {
            let sq = Square::from_index(i);
            if let Some(piece) = self.board[sq.index()] {
                total += psq_score(piece, sq);
            }
        }
        total
    }

    /// Recompute non-pawn material from scratch. Test oracle; `from_fen`
    /// uses it to initialize and the incremental maintenance in
    /// remove/put keeps it in sync thereafter.
    pub(crate) fn compute_non_pawn_material_from_scratch(&self) -> [Value; 2] {
        let mut totals = [Value::ZERO; 2];
        for i in 0u8..64 {
            if let Some(piece) = self.board[i as usize] {
                let kind = piece.kind();
                if kind != PieceType::Pawn && kind != PieceType::King {
                    totals[piece.color().index()] += Value::mg_of_piece(kind);
                }
            }
        }
        totals
    }

    /// Compute the Zobrist key from the current piece placement, castling
    /// rights, en-passant target, and side to move. Used by `from_fen` and
    /// as a correctness oracle in tests. Not called during search: the key
    /// is maintained incrementally by `do_move` and `undo_move`.
    pub(crate) fn compute_key_from_scratch(&self) -> u64 {
        let mut key: u64 = 0;
        for i in 0u8..64 {
            let sq = Square::from_index(i);
            if let Some(piece) = self.board[sq.index()] {
                key ^= zobrist::piece_square_key(piece, sq);
            }
        }
        key ^= zobrist::castling_key(self.castling_rights);
        if let Some(ep) = self.en_passant {
            key ^= zobrist::ep_key(ep);
        }
        if self.side_to_move == Color::Black {
            key ^= zobrist::side_to_move_key();
        }
        key
    }

    /// Render this position as a FEN record.
    pub fn to_fen(&self) -> String {
        let mut out = String::with_capacity(90);

        // Placement: ranks 8 down to 1, each rank files a through h.
        for rank_idx in 0..8u8 {
            let rank = 7 - rank_idx;
            let mut run: u8 = 0;
            for file in 0..8u8 {
                let sq = Square::new(
                    File::from_index(file).unwrap(),
                    Rank::from_index(rank).unwrap(),
                );
                match self.board[sq.index()] {
                    None => run += 1,
                    Some(piece) => {
                        if run > 0 {
                            out.push(char::from_digit(run as u32, 10).unwrap());
                            run = 0;
                        }
                        out.push(piece_to_char(piece));
                    }
                }
            }
            if run > 0 {
                out.push(char::from_digit(run as u32, 10).unwrap());
            }
            if rank_idx < 7 {
                out.push('/');
            }
        }

        out.push(' ');
        out.push(match self.side_to_move {
            Color::White => 'w',
            Color::Black => 'b',
        });

        out.push(' ');
        if self.castling_rights == CastlingRights::NONE {
            out.push('-');
        } else {
            if self.castling_rights.contains(CastlingRights::WHITE_KING) {
                out.push('K');
            }
            if self.castling_rights.contains(CastlingRights::WHITE_QUEEN) {
                out.push('Q');
            }
            if self.castling_rights.contains(CastlingRights::BLACK_KING) {
                out.push('k');
            }
            if self.castling_rights.contains(CastlingRights::BLACK_QUEEN) {
                out.push('q');
            }
        }

        out.push(' ');
        match self.en_passant {
            None => out.push('-'),
            Some(sq) => out.push_str(&sq.to_algebraic()),
        }

        out.push_str(&format!(
            " {} {}",
            self.halfmove_clock, self.fullmove_number
        ));
        out
    }
}

// =========================================================================
// FEN helper functions
// =========================================================================

type Placement = ([Option<Piece>; 64], [Bitboard; 7], [Bitboard; 2]);

fn parse_placement(placement: &str) -> Result<Placement, FenError> {
    let ranks: Vec<&str> = placement.split('/').collect();
    if ranks.len() != 8 {
        return Err(FenError::BadPiecePlacement(format!(
            "expected 8 ranks separated by '/', got {}",
            ranks.len()
        )));
    }

    let mut board: [Option<Piece>; 64] = [None; 64];
    let mut by_kind = [Bitboard::EMPTY; 7];
    let mut by_color = [Bitboard::EMPTY; 2];

    // FEN places rank 8 first, then 7, down to 1. We'll walk ranks 8→1 and
    // files a→h.
    for (rank_offset, rank_str) in ranks.iter().enumerate() {
        let rank = 7u8 - rank_offset as u8;
        let mut file: u8 = 0;

        for ch in rank_str.chars() {
            if let Some(digit) = ch.to_digit(10) {
                // Empty-square run. Advance the file cursor.
                if digit == 0 || digit > 8 {
                    return Err(FenError::BadPiecePlacement(format!(
                        "bad empty-square run '{}' on rank {}",
                        ch,
                        rank + 1
                    )));
                }
                file = file.saturating_add(digit as u8);
                if file > 8 {
                    return Err(FenError::BadPiecePlacement(format!(
                        "rank {} overflows past file h",
                        rank + 1
                    )));
                }
            } else {
                if file >= 8 {
                    return Err(FenError::BadPiecePlacement(format!(
                        "rank {} has too many pieces",
                        rank + 1
                    )));
                }
                let piece = piece_from_char(ch).ok_or_else(|| {
                    FenError::BadPiecePlacement(format!("unknown piece character '{}'", ch))
                })?;
                let sq = Square::new(
                    File::from_index(file).unwrap(),
                    Rank::from_index(rank).unwrap(),
                );
                board[sq.index()] = Some(piece);
                by_kind[piece.kind().index()] |= square_bb(sq);
                by_color[piece.color().index()] |= square_bb(sq);
                file += 1;
            }
        }

        if file != 8 {
            return Err(FenError::BadPiecePlacement(format!(
                "rank {} has {} files worth of content, expected 8",
                rank + 1,
                file
            )));
        }
    }

    Ok((board, by_kind, by_color))
}

fn parse_castling(s: &str) -> Result<CastlingRights, FenError> {
    if s == "-" {
        return Ok(CastlingRights::NONE);
    }
    let mut rights = CastlingRights::NONE;
    for ch in s.chars() {
        let bit = match ch {
            'K' => CastlingRights::WHITE_KING,
            'Q' => CastlingRights::WHITE_QUEEN,
            'k' => CastlingRights::BLACK_KING,
            'q' => CastlingRights::BLACK_QUEEN,
            _ => return Err(FenError::BadCastlingRights),
        };
        rights = rights | bit;
    }
    Ok(rights)
}

fn piece_from_char(ch: char) -> Option<Piece> {
    Some(match ch {
        'P' => Piece::WhitePawn,
        'N' => Piece::WhiteKnight,
        'B' => Piece::WhiteBishop,
        'R' => Piece::WhiteRook,
        'Q' => Piece::WhiteQueen,
        'K' => Piece::WhiteKing,
        'p' => Piece::BlackPawn,
        'n' => Piece::BlackKnight,
        'b' => Piece::BlackBishop,
        'r' => Piece::BlackRook,
        'q' => Piece::BlackQueen,
        'k' => Piece::BlackKing,
        _ => return None,
    })
}

fn piece_to_char(piece: Piece) -> char {
    match piece {
        Piece::WhitePawn => 'P',
        Piece::WhiteKnight => 'N',
        Piece::WhiteBishop => 'B',
        Piece::WhiteRook => 'R',
        Piece::WhiteQueen => 'Q',
        Piece::WhiteKing => 'K',
        Piece::BlackPawn => 'p',
        Piece::BlackKnight => 'n',
        Piece::BlackBishop => 'b',
        Piece::BlackRook => 'r',
        Piece::BlackQueen => 'q',
        Piece::BlackKing => 'k',
    }
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- FEN roundtrip -----------------------------------------------

    const STARTPOS_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

    #[test]
    fn fen_roundtrips_startpos() {
        let p = Position::from_fen(STARTPOS_FEN).unwrap();
        assert_eq!(p.to_fen(), STARTPOS_FEN);
    }

    #[test]
    fn fen_roundtrips_italian_after_four_halfmoves() {
        // 1. e4 e5 2. Nf3 Nc6
        let fen = "r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3";
        let p = Position::from_fen(fen).unwrap();
        assert_eq!(p.to_fen(), fen);
        assert_eq!(p.piece_on(Square::E4), Some(Piece::WhitePawn));
        assert_eq!(p.piece_on(Square::E5), Some(Piece::BlackPawn));
        assert_eq!(p.piece_on(Square::F3), Some(Piece::WhiteKnight));
        assert_eq!(p.piece_on(Square::C6), Some(Piece::BlackKnight));
        assert_eq!(p.side_to_move(), Color::White);
    }

    #[test]
    fn fen_roundtrips_en_passant_when_capturable() {
        // En passant is only recorded when a side-to-move pawn can
        // actually capture onto it (SF11 position.cpp:262-273). White's
        // e5 pawn can play exd6, so d6 is a real ep target and survives
        // the round-trip.
        let fen = "4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1";
        let p = Position::from_fen(fen).unwrap();
        assert_eq!(p.en_passant(), Some(Square::D6));
        assert_eq!(p.to_fen(), fen);
    }

    #[test]
    fn fen_drops_phantom_en_passant_with_no_capturer() {
        // After 1. e4 from the start no black pawn can capture on e3, so
        // the ep square is dropped — matching SF11 and keeping key() in
        // sync with the do_move path (the same position reached by playing
        // 1. e4 also has no ep square).
        let fen = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1";
        let p = Position::from_fen(fen).unwrap();
        assert_eq!(p.en_passant(), None);
        assert_eq!(
            p.to_fen(),
            "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1"
        );
    }

    #[test]
    fn fen_roundtrips_no_castling_rights() {
        let fen = "4k3/8/8/8/8/8/8/4K3 w - - 0 1";
        let p = Position::from_fen(fen).unwrap();
        assert_eq!(p.castling_rights(), CastlingRights::NONE);
        assert_eq!(p.to_fen(), fen);
    }

    #[test]
    fn fen_roundtrips_partial_castling_rights() {
        // White can only castle kingside, black can only castle queenside.
        let fen = "r3k2r/8/8/8/8/8/8/R3K2R w Kq - 0 1";
        let p = Position::from_fen(fen).unwrap();
        assert!(p.castling_rights().contains(CastlingRights::WHITE_KING));
        assert!(!p.castling_rights().contains(CastlingRights::WHITE_QUEEN));
        assert!(!p.castling_rights().contains(CastlingRights::BLACK_KING));
        assert!(p.castling_rights().contains(CastlingRights::BLACK_QUEEN));
        assert_eq!(p.to_fen(), fen);
    }

    // ---- FEN validation ---------------------------------------------

    #[test]
    fn fen_rejects_missing_fields() {
        assert!(matches!(
            Position::from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR"),
            Err(FenError::MissingField(_))
        ));
    }

    #[test]
    fn fen_rejects_wrong_rank_count() {
        let bad = "rnbqkbnr/pppppppp/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        assert!(matches!(
            Position::from_fen(bad),
            Err(FenError::BadPiecePlacement(_))
        ));
    }

    #[test]
    fn fen_rejects_rank_with_too_many_files() {
        let bad = "rnbqkbnr1/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        assert!(matches!(
            Position::from_fen(bad),
            Err(FenError::BadPiecePlacement(_))
        ));
    }

    #[test]
    fn fen_rejects_rank_with_too_few_files() {
        let bad = "rnbqkbnr/ppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        assert!(matches!(
            Position::from_fen(bad),
            Err(FenError::BadPiecePlacement(_))
        ));
    }

    #[test]
    fn fen_rejects_unknown_piece_char() {
        let bad = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPXPP/RNBQKBNR w KQkq - 0 1";
        assert!(matches!(
            Position::from_fen(bad),
            Err(FenError::BadPiecePlacement(_))
        ));
    }

    #[test]
    fn fen_rejects_bad_side_to_move() {
        let bad = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR q KQkq - 0 1";
        assert_eq!(Position::from_fen(bad), Err(FenError::BadSideToMove));
    }

    #[test]
    fn fen_rejects_bad_castling_rights() {
        let bad = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkx - 0 1";
        assert_eq!(Position::from_fen(bad), Err(FenError::BadCastlingRights));
    }

    #[test]
    fn fen_rejects_bad_en_passant_square() {
        let bad = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq z9 0 1";
        assert_eq!(Position::from_fen(bad), Err(FenError::BadEnPassant));
    }

    #[test]
    fn fen_rejects_non_numeric_clock() {
        let bad = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - x 1";
        assert!(matches!(
            Position::from_fen(bad),
            Err(FenError::BadClock(_))
        ));
    }

    #[test]
    fn fen_rejects_position_with_no_white_king() {
        // Two kings on d8/e8 but no white king.
        let bad = "4kkk1/8/8/8/8/8/8/8 w - - 0 1";
        assert_eq!(
            Position::from_fen(bad),
            Err(FenError::MissingKing(Color::White))
        );
    }

    #[test]
    fn fen_rejects_position_with_two_white_kings() {
        let bad = "4k3/8/8/8/8/8/8/KK6 w - - 0 1";
        // Two white kings => popcount != 1 on white king bb.
        assert_eq!(
            Position::from_fen(bad),
            Err(FenError::MissingKing(Color::White))
        );
    }

    // ---- Zobrist initial key -----------------------------------------

    #[test]
    fn startpos_key_is_non_zero_and_stable() {
        let p = Position::startpos();
        assert_ne!(p.key(), 0);
        // The key must match a from-scratch recomputation.
        assert_eq!(p.key(), p.compute_key_from_scratch());
    }

    #[test]
    fn different_positions_have_different_keys() {
        let a = Position::startpos();
        let b = Position::from_fen("rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1")
            .unwrap();
        assert_ne!(a.key(), b.key());
    }

    // ---- Pawn-only Zobrist key ---------------------------------------

    #[test]
    fn startpos_pawn_key_matches_scratch_and_is_non_zero() {
        let p = Position::startpos();
        assert_ne!(p.pawn_key(), 0);
        assert_eq!(p.pawn_key(), p.compute_pawn_key_from_scratch());
    }

    #[test]
    fn pawn_key_is_equal_for_identical_pawn_structures_only() {
        // Two different full positions that happen to share the same pawn
        // placement must produce the same pawn_key, even though their main
        // keys differ.
        let a =
            Position::from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap();
        let b = Position::from_fen("4k3/pppppppp/8/8/8/8/PPPPPPPP/4K3 w - - 0 1").unwrap();
        assert_eq!(a.pawn_key(), b.pawn_key());
        assert_ne!(a.key(), b.key());
    }

    #[test]
    fn pawn_key_differs_when_pawns_differ() {
        let a = Position::startpos();
        // Same startpos but with the e2 pawn pushed to e4 — different pawn
        // structure, so pawn_key must change.
        let b = Position::from_fen("rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1")
            .unwrap();
        assert_ne!(a.pawn_key(), b.pawn_key());
    }

    #[test]
    fn empty_pawn_position_has_no_pawns_base_key() {
        // Kings alone — no pawns to XOR in, so pawn_key equals the
        // `noPawns` base constant exactly.
        let p = Position::from_fen("4k3/8/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        assert_eq!(p.pawn_key(), crate::zobrist::no_pawns_key());
    }
}
