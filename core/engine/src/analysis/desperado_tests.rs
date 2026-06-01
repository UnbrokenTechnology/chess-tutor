use super::*;
use crate::position::Position;

/// A position where White's knight on f5 is the doomed piece (Black is
/// massively up after grabbing material) but `Nxg7+` is a legal
/// capture-with-check — the case study's `Nxg7+` desperado. The knight can
/// cash itself for the g7 pawn *with check* before it falls.
const DESPERADO_FEN: &str = "r1b1kb1r/1p3ppp/p5pp/4pNB1/4n3/2N5/PPP2PPP/R2Q1RK1 w kq - 0 2";

#[test]
fn finds_nxg7_check_desperado() {
    let pos = Position::from_fen(DESPERADO_FEN).unwrap();
    let f5 = Square::F5;
    let d = find_desperado(&pos, f5, Color::White).expect("Nxg7+ is a capture-with-check");
    assert_eq!(d.piece, Square::F5);
    assert_eq!(d.captures_on, Square::G7);
    assert_eq!(d.captured, PieceType::Pawn);
    assert!(d.recovered_cp > 0, "a pawn recovery must be positive cp");
}

#[test]
fn no_desperado_when_not_side_to_move() {
    let pos = Position::from_fen(DESPERADO_FEN).unwrap();
    // It's White to move; asking for a Black desperado must return None
    // (the desperado is the *owner's* move).
    assert!(find_desperado(&pos, Square::F5, Color::Black).is_none());
}

#[test]
fn no_desperado_for_quiet_piece() {
    // Startpos: no piece has a capture-with-check available.
    let pos = Position::startpos();
    for sq in [Square::B1, Square::G1, Square::E2] {
        assert!(
            find_desperado(&pos, sq, Color::White).is_none(),
            "no capture-with-check exists from {sq:?} in the start position"
        );
    }
}

#[test]
fn no_desperado_for_empty_square() {
    let pos = Position::from_fen(DESPERADO_FEN).unwrap();
    // d4 is empty — nothing to be a desperado.
    assert!(find_desperado(&pos, Square::D4, Color::White).is_none());
}
