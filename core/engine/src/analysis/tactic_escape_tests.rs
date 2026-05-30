//! Sibling tests for [`super`] (`tactic_escape.rs`).

use super::*;
use crate::analysis::find_best_tactic_in_position;
use crate::analysis::tactic_outcome::detect_line_tactic;
use crate::position::Position;
use crate::types::{Color, Move, Square};

const CASE_STUDY_FEN: &str = "1r4nr/p3k3/4qpp1/4b2p/2Q5/8/PPPP1PPP/R1B1R1K1 w - - 0 1";

/// The case-study position. Previously the `tactics` surface crowned
/// `Rxe5` (rook takes the e5 bishop to pin the e6 queen to the king) as a
/// High-confidence Pin worth a piece — a phantom, since `Qxe5` recaptures
/// the pinner. With `detect_relative_pin` plus escape-aware ranking, the
/// **best tactic** is now the genuine relative pin: `d4` attacks the
/// bishop that is pinned (along Re1's file) to the queen. Its escape is
/// the *forcing* `…Bxh2+`, which turns the bishop's departure into a
/// discovered attack on Re1 — exactly the resource that makes the pin
/// unsafe to rely on.
#[test]
fn relative_pin_surfaces_as_best_tactic_with_forcing_escape() {
    let pos = Position::from_fen(CASE_STUDY_FEN).expect("valid FEN");

    let hit = find_best_tactic_in_position(&pos, Color::White, None)
        .expect("a high-confidence tactic is detected here");
    assert_eq!(
        hit.pattern,
        TacticPattern::RelativePin,
        "the relative pin (d4) should win over the refuted Rxe5 absolute pin"
    );

    let key = hit.key_move.expect("key_move is stamped on the hit");
    assert_eq!(key.from(), Square::D2, "the pin-pressing move is the d-pawn");
    assert_eq!(key.to(), Square::D4, "advancing to attack the pinned bishop");
    // targets are front-then-rear: the pinned bishop and the queen behind.
    assert_eq!(hit.targets, vec![Square::E5, Square::E6]);

    let escape = find_tactic_escape(&pos, &hit, Color::White)
        .expect("the relative pin has a forcing escape");
    assert_eq!(escape.refutation.from(), Square::E5, "the bishop breaks the pin");
    assert_eq!(escape.refutation.to(), Square::H2, "with the forcing Bxh2+");
    assert_eq!(
        escape.kind,
        EscapeKind::ForcingCheck,
        "it's a check the owner must answer, not a quiet move"
    );
}

/// The absolute-pin escape machinery still works when the `Rxe5` pin is
/// detected directly (we no longer *surface* it as best, but it remains a
/// valid hit whose escape — the pinned queen recapturing the pinner — must
/// be found).
#[test]
fn rxe5_absolute_pin_is_still_broken_by_qxe5() {
    let pos = Position::from_fen(CASE_STUDY_FEN).expect("valid FEN");
    let rxe5 = Move::normal(Square::E1, Square::E5);
    let hit = detect_line_tactic(&pos, &[rxe5], Color::White, 0, None)
        .expect("Rxe5 pins the queen to the king");
    assert_eq!(hit.pattern, TacticPattern::Pin, "queen pinned to king = absolute pin");

    let escape = find_tactic_escape(&pos, &hit, Color::White)
        .expect("Rxe5's pin has a clean escape");
    assert_eq!(escape.refutation.from(), Square::E6, "the queen escapes");
    assert_eq!(escape.refutation.to(), Square::E5, "by capturing the pinner");
    assert_eq!(escape.kind, EscapeKind::Zwischenzug, "it's a capture, not a check");
    assert_eq!(escape.expected_target, Square::E6, "we expected to win the queen");
}

/// A non-analysable pattern (or a position with no tactic) yields no
/// escape — the function is silent rather than guessing.
#[test]
fn quiet_startpos_has_no_tactic_to_escape() {
    let pos = Position::startpos();
    assert!(find_best_tactic_in_position(&pos, Color::White, None).is_none());
}
