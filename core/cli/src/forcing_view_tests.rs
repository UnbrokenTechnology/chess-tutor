//! Sibling tests for [`super`] (`forcing_view.rs`).

use super::*;
use chess_tutor_engine::position::Position;

#[test]
fn startpos_has_zero_forcing_moves() {
    // No piece is in check, no captures available, no promotion rank
    // reachable in one move.
    let pos = Position::startpos();
    let view = build(&pos);
    assert!(view.white.checks.is_empty());
    assert!(view.white.captures.is_empty());
    assert!(view.white.promotions.is_empty());
    assert!(view.black.checks.is_empty());
}

#[test]
fn case_study_white_to_move_finds_qxe6_with_check() {
    // Discovered-attack case study. White to move; Qxe6+ is the
    // load-bearing forcing move (the engine's pick at +6.09).
    let pos =
        Position::from_fen("1r4nr/p3k3/4qpp1/4b2p/2Q5/8/PPPP1PPP/R1B1R1K1 w - - 0 1").unwrap();
    let view = build(&pos);
    assert!(view.white.is_to_move);
    // Qxe6+ should appear in BOTH checks and captures.
    let qxe6_check = view
        .white
        .checks
        .iter()
        .find(|m| m.san.starts_with("Qxe6"))
        .unwrap_or_else(|| panic!("no Qxe6+ in white checks: {view:#?}"));
    assert!(qxe6_check.gives_check);
    assert!(qxe6_check.captures.is_some());
    let cap = qxe6_check.captures.as_ref().unwrap();
    assert_eq!(cap.piece, "qe6");
    assert_eq!(cap.classical_points, 9);
}

#[test]
fn opponent_via_null_move_lists_blacks_standing_options() {
    // After 1. e4 (white played), black is to move. We query for
    // white's forcing options *as if* they got another move — and
    // white has none, since the position is quiet.
    let pos = Position::from_fen("rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1")
        .unwrap();
    let view = build(&pos);
    assert!(view.black.is_to_move);
    assert!(!view.white.is_to_move);
    // White (the side not to move) has no forcing options here.
    assert!(view.white.checks.is_empty());
    assert!(view.white.captures.is_empty());
}

#[test]
fn promotion_move_surfaces_with_promoted_piece_letter() {
    // White pawn on g7, black king parked on a8, white king on a1.
    // White can promote g7-g8=Q (or =R/=B/=N).
    let pos = Position::from_fen("k7/6P1/8/8/8/8/8/K7 w - - 0 1").unwrap();
    let view = build(&pos);
    let promos = &view.white.promotions;
    assert!(!promos.is_empty(), "expected promotions: {view:#?}");
    // All four promotion pieces should appear.
    let pieces: Vec<&str> = promos
        .iter()
        .filter_map(|m| m.promotion_piece.as_deref())
        .collect();
    assert!(pieces.contains(&"q"));
    assert!(pieces.contains(&"r"));
    assert!(pieces.contains(&"b"));
    assert!(pieces.contains(&"n"));
}

#[test]
fn side_in_check_has_empty_opponent_forcing_list() {
    // Side to move is in check → null move not legal → opponent's
    // forcing list returns empty (with the same "side label, no
    // forcing moves" surface).
    let pos = Position::from_fen("4r2k/8/8/8/8/8/8/4K3 w - - 0 1").unwrap();
    let view = build(&pos);
    assert!(view.white.is_to_move);
    // Black (opponent here) gets reported with the null-move stub.
    assert!(!view.black.is_to_move);
    assert!(view.black.checks.is_empty());
    assert!(view.black.captures.is_empty());
}
