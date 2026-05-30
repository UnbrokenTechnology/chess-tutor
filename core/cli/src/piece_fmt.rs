//! Shared piece / square / color formatting for the agent-facing
//! geometric-query commands (Phase B).
//!
//! The convention used everywhere these helpers are called:
//!
//! - `piece_at_sq(pos, sq)` → `"Nf3"` / `"qe6"` — uppercase for white,
//!   lowercase for black, mirroring FEN. An agent reading the line can
//!   tell colour without a second field.
//! - `piece_name_word(piece)` → `"white knight"` / `"black queen"` —
//!   plain-English for prose contexts (mostly tests / fallback labels).
//!
//! Keep this file small: every other Phase-B module pulls from here, so
//! a change in convention propagates everywhere consistently.

use chess_tutor_engine::types::{Color, Piece, PieceType, Square};

/// One-character piece code, uppercase for white / lowercase for black.
/// `K Q R B N P` for white, `k q r b n p` for black.
pub fn piece_letter(piece: Piece) -> char {
    let letter = match piece.kind() {
        PieceType::King => 'K',
        PieceType::Queen => 'Q',
        PieceType::Rook => 'R',
        PieceType::Bishop => 'B',
        PieceType::Knight => 'N',
        PieceType::Pawn => 'P',
    };
    if piece.color() == Color::White {
        letter
    } else {
        letter.to_ascii_lowercase()
    }
}

/// Piece + square, e.g. `"Nf3"` for a white knight on f3 or `"qe6"`
/// for a black queen on e6. The colour is encoded in the case of the
/// leading letter so the entire identifier is one token (no `(W)`/
/// `(B)` suffix to parse).
pub fn piece_label(piece: Piece, sq: Square) -> String {
    format!("{}{}", piece_letter(piece), sq.to_algebraic())
}

/// Plain-English colour name (`"White"` / `"Black"`).
pub fn color_name(c: Color) -> &'static str {
    match c {
        Color::White => "White",
        Color::Black => "Black",
    }
}

/// Plain-English piece-type name (lowercase, no colour).
pub fn piece_type_name(pt: PieceType) -> &'static str {
    match pt {
        PieceType::King => "king",
        PieceType::Queen => "queen",
        PieceType::Rook => "rook",
        PieceType::Bishop => "bishop",
        PieceType::Knight => "knight",
        PieceType::Pawn => "pawn",
    }
}

#[cfg(test)]
#[path = "piece_fmt_tests.rs"]
mod tests;
