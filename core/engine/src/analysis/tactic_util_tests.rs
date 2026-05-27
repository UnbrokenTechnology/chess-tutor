use super::*;
use crate::types::{Color, Move, Square};

fn pos(fen: &str) -> Position {
    Position::from_fen(fen).unwrap()
}

const ROYAL_FORK_FEN: &str = "r3k3/8/8/1N6/8/8/8/6K1 w - - 0 1";

// A corner knight, fenced in and attacked. Black knight a8 is attacked by
// the white bishop d5 (along a8-d5); its only two squares — b6 and c7 —
// are each covered by a white pawn (c5 covers b6, d6 covers c7). Black to
// move and the knight cannot be saved.
const TRAPPED_KNIGHT_FEN: &str = "n6k/8/3P4/2PB4/8/8/8/6K1 b - - 0 1";

// ---- is_in_bad_spot / is_defended / is_hanging ----------------------

#[test]
fn is_in_bad_spot_detects_hanging_attacked_piece() {
    // After Nc7 with a bishop on a5 raking c7, the knight is in a bad spot.
    let mut post = pos("r3k3/8/8/bN6/8/8/8/6K1 w - - 0 1");
    post.do_move(Move::normal(Square::B5, Square::C7));
    assert!(is_in_bad_spot(&post, Square::C7));
}

#[test]
fn is_in_bad_spot_false_for_safe_piece() {
    let p = pos(ROYAL_FORK_FEN);
    // The white king on g1 is unattacked.
    assert!(!is_in_bad_spot(&p, Square::G1));
}

#[test]
fn is_defended_recognizes_ray_defense_through_enemy_slider() {
    // White pawn d5 is attacked by a black bishop on f7. A white bishop on
    // g8 sits behind it on the same diagonal: removing the black bishop
    // reveals the defender, so the pawn is defended, not hanging.
    let p = pos("6B1/5b2/8/3P4/8/8/8/k6K w - - 0 1");
    assert!(is_defended(&p, Square::D5, Color::White));
    assert!(!is_hanging(&p, Square::D5, Color::White));
}

#[test]
fn is_hanging_true_for_undefended_attacked_piece() {
    // Lone white pawn d5 attacked by a black bishop f7, no defender behind.
    let p = pos("8/5b2/8/3P4/8/8/8/k6K w - - 0 1");
    assert!(is_hanging(&p, Square::D5, Color::White));
}

// ---- is_trapped: positive case --------------------------------------

#[test]
fn trapped_corner_knight_fires() {
    let p = pos(TRAPPED_KNIGHT_FEN);
    assert!(is_trapped(&p, Square::A8));
}

// ---- is_trapped: exclusions -----------------------------------------

#[test]
fn trapped_false_when_an_escape_is_safe() {
    // Remove the d6 pawn: now c7 is an unattacked escape square.
    let p = pos("n6k/8/8/2PB4/8/8/8/6K1 b - - 0 1");
    assert!(!is_trapped(&p, Square::A8));
}

#[test]
fn trapped_false_for_piece_not_to_move() {
    // Same fenced knight, but it's White's turn — the predicate only
    // reasons about the side to move, so the black knight is not reported.
    let p = pos("n6k/8/3P4/2PB4/8/8/8/6K1 w - - 0 1");
    assert!(!is_trapped(&p, Square::A8));
}

#[test]
fn trapped_false_for_pawn_and_king() {
    // Neither pawns nor kings are ever "trapped" in lichess's sense.
    let p = pos("6k1/8/8/8/8/8/2p5/6K1 b - - 0 1");
    assert!(!is_trapped(&p, Square::C2)); // pawn
    assert!(!is_trapped(&p, Square::G8)); // king
}

#[test]
fn trapped_false_when_piece_can_trade_out_evenly() {
    // Black knight h1 is attacked by the white king, but it can capture the
    // white rook on g3 — an equal-or-greater trade — so it isn't trapped.
    let p = pos("7k/8/8/8/8/6R1/8/6Kn b - - 0 1");
    assert!(!is_trapped(&p, Square::H1));
}

#[test]
fn trapped_false_when_in_check() {
    // The fenced knight again, but a white rook on h7 checks the black king:
    // "trapped" is moot while the side to move must answer a check.
    let p = pos("n6k/7R/3P4/2PB4/8/8/8/6K1 b - - 0 1");
    assert!(!is_trapped(&p, Square::A8));
}

#[test]
fn trapped_false_for_safe_piece() {
    // A queen sitting in the open, unattacked, is not trapped.
    let p = pos("6k1/8/8/3q4/8/8/8/6K1 b - - 0 1");
    assert!(!is_trapped(&p, Square::D5));
}
