//! Sibling tests for [`super`] (`piece_fmt.rs`).

use super::*;
use chess_tutor_engine::types::{Color, Piece, PieceType, Square};

#[test]
fn white_pieces_take_uppercase_letters() {
    assert_eq!(piece_letter(Piece::WhiteKnight), 'N');
    assert_eq!(piece_letter(Piece::WhiteKing), 'K');
    assert_eq!(piece_letter(Piece::WhiteQueen), 'Q');
    assert_eq!(piece_letter(Piece::WhiteRook), 'R');
    assert_eq!(piece_letter(Piece::WhiteBishop), 'B');
    assert_eq!(piece_letter(Piece::WhitePawn), 'P');
}

#[test]
fn black_pieces_take_lowercase_letters() {
    assert_eq!(piece_letter(Piece::BlackKnight), 'n');
    assert_eq!(piece_letter(Piece::BlackQueen), 'q');
}

#[test]
fn piece_label_encodes_colour_in_case() {
    assert_eq!(piece_label(Piece::WhiteKing, Square::E1), "Ke1");
    assert_eq!(piece_label(Piece::BlackQueen, Square::D8), "qd8");
    assert_eq!(piece_label(Piece::WhiteKnight, Square::F3), "Nf3");
}

#[test]
fn piece_type_name_is_lowercase() {
    assert_eq!(piece_type_name(PieceType::Rook), "rook");
    assert_eq!(piece_type_name(PieceType::Pawn), "pawn");
}

#[test]
fn color_name_capitalises() {
    assert_eq!(color_name(Color::White), "White");
    assert_eq!(color_name(Color::Black), "Black");
}
