//! Tests for the overloaded-defender scan. Hand-built positions (lichess has
//! no reference predicate — `cook.py:overloading` is a stub).

use super::*;

/// Black knight on d8 is the sole defender of both bishops (c6, e6), each hit
/// by a white rook up its file — a textbook overload.
const OVERLOAD: &str = "3n2k1/8/2b1b3/8/8/8/8/2R1R1K1 w - - 0 1";

#[test]
fn sole_defender_of_two_attacked_pieces_is_overloaded() {
    let pos = Position::from_fen(OVERLOAD).unwrap();
    let found = find_overloaded(&pos, Color::Black);
    assert_eq!(
        found,
        vec![OverloadedPiece {
            piece: Square::D8,
            duties: vec![Square::C6, Square::E6],
        }]
    );
}

#[test]
fn scan_is_colour_correct() {
    // The same position has no overloaded *white* piece (Black attacks none).
    let pos = Position::from_fen(OVERLOAD).unwrap();
    assert!(find_overloaded(&pos, Color::White).is_empty());
}

#[test]
fn one_duty_is_not_an_overload() {
    // Drop the e-file rook: the knight is now sole defender of only c6, so it
    // carries a single duty — not overloaded.
    let pos = Position::from_fen("3n2k1/8/2b1b3/8/8/8/8/2R3K1 w - - 0 1").unwrap();
    assert!(find_overloaded(&pos, Color::Black).is_empty());
}

#[test]
fn over_defended_target_is_not_a_sole_duty() {
    // Add a black pawn on d7: it also defends c6 and e6, so neither target has
    // a *sole* defender — no overload, even though d7/d8 both guard two pieces.
    let pos = Position::from_fen("3n2k1/3p4/2b1b3/8/8/8/8/2R1R1K1 w - - 0 1").unwrap();
    assert!(find_overloaded(&pos, Color::Black).is_empty());
}
