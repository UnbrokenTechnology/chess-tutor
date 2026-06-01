use super::*;
use chess_tutor_engine::types::Value;

/// The eval bar labels the *viewed* position, but its score is the
/// analysis of the move that reached it — rooted one ply earlier. So a
/// position that is mate-in-1 is reached by a move whose line is
/// `mate_in(2)`; the bar must drop that played ply and read "M1", in
/// moves, not the old raw-plies "M2".
#[test]
fn eval_bar_mate_drops_the_played_ply_and_renders_moves() {
    // White-POV mate score for the move that *reached* a mate-in-1
    // position (line = played move + the mate = 2 plies from the root).
    let (ratio, label) = eval_bar_fill_and_label(Some(Value::mate_in(2)));
    assert_eq!(label, "M1");
    assert_eq!(ratio, 1.0);
}

#[test]
fn eval_bar_mate_on_board_shows_bare_hash() {
    // The reaching move itself delivered mate (line = 1 ply): the viewed
    // position is checkmate, so there's no countdown to show.
    let (_, label) = eval_bar_fill_and_label(Some(Value::mate_in(1)));
    assert_eq!(label, "#");
}

#[test]
fn eval_bar_longer_mate_renders_moves_after_dropping_a_ply() {
    // 7 plies from the analysis root -> 6 from the viewed position ->
    // mate in 3 moves.
    let (_, label) = eval_bar_fill_and_label(Some(Value::mate_in(7)));
    assert_eq!(label, "M3");
}

#[test]
fn eval_bar_black_mate_is_signed() {
    // Black winning: mated_in(3) is 3 plies from the root -> 2 from the
    // viewed position -> mate in 1 move, on the black side.
    let (ratio, label) = eval_bar_fill_and_label(Some(Value::mated_in(3)));
    assert_eq!(label, "-M1");
    assert_eq!(ratio, 0.0);
}

#[test]
fn eval_bar_cp_score_uses_pawn_eg_scale() {
    // One endgame pawn reads as +1.00 (chess.com-aligned), not raw cp.
    let (_, label) = eval_bar_fill_and_label(Some(Value(Value::PAWN_EG.0)));
    assert_eq!(label, "+1.00");
}

#[test]
fn eval_bar_none_is_neutral_dash() {
    let (ratio, label) = eval_bar_fill_and_label(None);
    assert_eq!(label, "—");
    assert_eq!(ratio, 0.5);
}
