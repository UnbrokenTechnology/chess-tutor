use super::super::test_support::ma_with_pv;
use super::*;
use crate::types::{Color, Move, Square};

// Royal fork: white knight on b5 plays Nc7+, forking the black king on
// e8 and the rook on a8. Used by several tests.
const ROYAL_FORK_FEN: &str = "r3k3/8/8/1N6/8/8/8/6K1 w - - 0 1";

fn pos(fen: &str) -> Position {
    Position::from_fen(fen).unwrap()
}

// ---- detect_fork: positive cases ------------------------------------

#[test]
fn knight_royal_fork_fires() {
    let pre = pos(ROYAL_FORK_FEN);
    let nc7 = Move::normal(Square::B5, Square::C7);
    let hit = detect_line_tactic(&pre, &[nc7], Color::White, 0).expect("fork should fire");

    assert_eq!(hit.pattern, TacticPattern::Fork);
    assert_eq!(hit.pv_ply, 0);
    assert_eq!(hit.primary_piece, Square::C7);
    assert_eq!(hit.targets, vec![Square::A8, Square::E8]);
    // Single move, no capture in window → not yet realized.
    assert_eq!(hit.confidence, Confidence::Medium);
}

#[test]
fn fork_with_capture_continuation_is_high_confidence() {
    let pre = pos(ROYAL_FORK_FEN);
    // Nc7+ Kd8 Nxa8 — the rook falls within the four-ply window.
    let pv = [
        Move::normal(Square::B5, Square::C7),
        Move::normal(Square::E8, Square::D8),
        Move::normal(Square::C7, Square::A8),
    ];
    let hit = detect_line_tactic(&pre, &pv, Color::White, 0).expect("fork should fire");
    assert_eq!(hit.confidence, Confidence::High);
    assert_eq!(hit.material_gain, Some(Value::ROOK_MG.0));
}

#[test]
fn pawn_fork_fires_via_pawn_attack_pattern() {
    // White pawn d4 pushes to d5, forking two black rooks on c6 and e6.
    // Neither rook attacks d5, so the pawn is safe and both rooks outvalue it.
    let pre = pos("k7/8/2r1r3/8/3P4/8/8/7K w - - 0 1");
    let d5 = Move::normal(Square::D4, Square::D5);
    let hit = detect_line_tactic(&pre, &[d5], Color::White, 0).expect("pawn fork should fire");
    assert_eq!(hit.pattern, TacticPattern::Fork);
    assert_eq!(hit.primary_piece, Square::D5);
    assert_eq!(hit.targets, vec![Square::C6, Square::E6]);
}

#[test]
fn queen_forks_two_hanging_knights_via_hanging_branch() {
    // White queen a1 → d4 attacks two undefended black knights on b4 and
    // f4. Both are worth less than the queen, so they qualify only through
    // the "hanging and can't recapture" arm, not the value arm. The king
    // is on a8 — off every line from d4 — so it isn't a third target.
    let pre = pos("k7/8/8/8/1n3n2/8/8/Q6K w - - 0 1");
    let qd4 = Move::normal(Square::A1, Square::D4);
    let hit = detect_line_tactic(&pre, &[qd4], Color::White, 0).expect("queen fork should fire");
    assert_eq!(hit.pattern, TacticPattern::Fork);
    assert_eq!(hit.targets, vec![Square::B4, Square::F4]);
}

// ---- detect_fork: negative cases ------------------------------------

#[test]
fn king_mover_is_not_a_fork() {
    // A king landing next to a black rook (c5) and knight (e5) is never
    // treated as a fork — a king can't be the forking piece.
    let pre = pos("7k/8/8/2r1n3/3K4/8/8/8 w - - 0 1");
    let kd5 = Move::normal(Square::D4, Square::D5);
    assert!(detect_line_tactic(&pre, &[kd5], Color::White, 0).is_none());
}

#[test]
fn single_target_is_not_a_fork() {
    // Knight check with only one valuable target (the king) — one target is
    // not a fork.
    let pre = pos("4k3/8/8/1N6/8/8/8/6K1 w - - 0 1");
    let nc7 = Move::normal(Square::B5, Square::C7);
    assert!(detect_line_tactic(&pre, &[nc7], Color::White, 0).is_none());
}

#[test]
fn forker_in_bad_spot_is_not_a_fork() {
    // Same royal-fork geometry, but a black bishop on a5 attacks c7. The
    // knight would be hanging there, so the fork is illusory.
    let pre = pos("r3k3/8/8/bN6/8/8/8/6K1 w - - 0 1");
    let nc7 = Move::normal(Square::B5, Square::C7);
    assert!(detect_line_tactic(&pre, &[nc7], Color::White, 0).is_none());
}

// ---- compute_tactic_outcome: the three slots ------------------------

#[test]
fn outcome_reports_user_played_fork() {
    let pre = pos(ROYAL_FORK_FEN);
    let pv = vec![
        Move::normal(Square::B5, Square::C7),
        Move::normal(Square::E8, Square::D8),
        Move::normal(Square::C7, Square::A8),
    ];
    let ma = ma_with_pv(pv, Some(2));
    let outcome = compute_tactic_outcome(&ma, &ma, &pre, Color::White);

    let hit = outcome.user_played_tactic.expect("user played a fork");
    assert_eq!(hit.pattern, TacticPattern::Fork);
    assert_eq!(hit.confidence, Confidence::High);
    // Same move as best → nothing "missed".
    assert!(outcome.user_missed_tactic.is_none());
    assert!(outcome.user_walked_into.is_none());
}

#[test]
fn outcome_reports_missed_fork_when_user_chose_another_move() {
    let pre = pos(ROYAL_FORK_FEN);
    let best = ma_with_pv(vec![Move::normal(Square::B5, Square::C7)], Some(0));
    // User shuffled the king instead of forking.
    let user = ma_with_pv(vec![Move::normal(Square::G1, Square::F1)], Some(0));

    let outcome = compute_tactic_outcome(&best, &user, &pre, Color::White);
    let missed = outcome.user_missed_tactic.expect("best line had a fork");
    assert_eq!(missed.pattern, TacticPattern::Fork);
    assert!(outcome.user_played_tactic.is_none());
}

#[test]
fn outcome_has_no_missed_tactic_when_user_played_best() {
    let pre = pos(ROYAL_FORK_FEN);
    let ma = ma_with_pv(vec![Move::normal(Square::B5, Square::C7)], Some(0));
    let outcome = compute_tactic_outcome(&ma, &ma, &pre, Color::White);
    assert!(outcome.user_missed_tactic.is_none());
}

#[test]
fn outcome_reports_walked_into_fork() {
    // White (the user) pushes a quiet pawn; black replies with Nc2+,
    // forking the white king on e1 and the rook on a1.
    let pre = pos("4k3/8/8/8/1n6/8/7P/R3K3 w - - 0 1");
    let pv = vec![
        Move::normal(Square::H2, Square::H3),
        Move::normal(Square::B4, Square::C2),
    ];
    let ma = ma_with_pv(pv, Some(1));
    let outcome = compute_tactic_outcome(&ma, &ma, &pre, Color::White);

    let walked = outcome.user_walked_into.expect("walked into a fork");
    assert_eq!(walked.pattern, TacticPattern::Fork);
    assert_eq!(walked.pv_ply, 1);
    assert_eq!(walked.primary_piece, Square::C2);
    assert_eq!(walked.targets, vec![Square::A1, Square::E1]);
    // The quiet pawn push itself is no tactic.
    assert!(outcome.user_played_tactic.is_none());
}

// ---- detect_hanging_capture -----------------------------------------

// White rook on d1 captures an undefended black bishop on d5.
const HANGING_BISHOP_FEN: &str = "4k3/8/8/3b4/8/8/8/3RK3 w - - 0 1";

#[test]
fn capturing_an_undefended_piece_fires_hanging_capture() {
    let pre = pos(HANGING_BISHOP_FEN);
    let rxd5 = Move::normal(Square::D1, Square::D5);
    let hit = detect_line_tactic(&pre, &[rxd5], Color::White, 0).expect("hanging capture");
    assert_eq!(hit.pattern, TacticPattern::HangingCapture);
    assert_eq!(hit.pv_ply, 0);
    assert_eq!(hit.primary_piece, Square::D5);
    assert_eq!(hit.targets, vec![Square::D5]);
    assert_eq!(hit.confidence, Confidence::High);
    assert_eq!(hit.material_gain, Some(Value::BISHOP_MG.0));
}

#[test]
fn capturing_a_defended_piece_is_not_a_hanging_capture() {
    // A black pawn on e6 defends (and would recapture on) d5.
    let pre = pos("4k3/8/4p3/3b4/8/8/8/3RK3 w - - 0 1");
    let rxd5 = Move::normal(Square::D1, Square::D5);
    assert!(detect_line_tactic(&pre, &[rxd5], Color::White, 0).is_none());
}

#[test]
fn capturing_a_pawn_is_not_a_hanging_capture() {
    // Pawns are excluded — "you won a free pawn" isn't the lesson.
    let pre = pos("4k3/8/8/3p4/8/8/8/3RK3 w - - 0 1");
    let rxd5 = Move::normal(Square::D1, Square::D5);
    assert!(detect_line_tactic(&pre, &[rxd5], Color::White, 0).is_none());
}

#[test]
fn quiet_move_next_to_a_hanging_piece_is_not_a_capture() {
    // Rd1-d4 attacks the hanging bishop but doesn't capture it.
    let pre = pos(HANGING_BISHOP_FEN);
    let rd4 = Move::normal(Square::D1, Square::D4);
    assert!(detect_line_tactic(&pre, &[rd4], Color::White, 0).is_none());
}

#[test]
fn outcome_reports_user_played_hanging_capture() {
    let pre = pos(HANGING_BISHOP_FEN);
    let ma = ma_with_pv(vec![Move::normal(Square::D1, Square::D5)], Some(0));
    let outcome = compute_tactic_outcome(&ma, &ma, &pre, Color::White);
    let hit = outcome.user_played_tactic.expect("free-piece capture");
    assert_eq!(hit.pattern, TacticPattern::HangingCapture);
}

// ---- ported util helpers --------------------------------------------

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
