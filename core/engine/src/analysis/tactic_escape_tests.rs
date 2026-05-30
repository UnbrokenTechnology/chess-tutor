//! Sibling tests for [`super`] (`tactic_escape.rs`).

use super::*;
use crate::analysis::find_best_tactic_in_position;
use crate::position::Position;
use crate::types::{Color, Square};

/// The case-study position. The static `tactics` surface flags `Rxe5`
/// (rook takes the e5 bishop, pinning the e6 queen to the king) as a
/// High-confidence Pin worth a piece — but the "pin" is broken by `Qxe5`,
/// the pinned queen capturing the pinner along the pin line. Without
/// escape detection the tool recommends a move that loses the exchange.
#[test]
fn rxe5_pin_is_broken_by_qxe5() {
    let pos = Position::from_fen("1r4nr/p3k3/4qpp1/4b2p/2Q5/8/PPPP1PPP/R1B1R1K1 w - - 0 1")
        .expect("valid FEN");

    let hit = find_best_tactic_in_position(&pos, Color::White, None)
        .expect("a high-confidence tactic is detected here");
    assert_eq!(hit.pattern, TacticPattern::Pin, "expected the pin to surface");

    let key = hit.key_move.expect("key_move is stamped on the hit");
    assert_eq!(key.from(), Square::E1, "tactic move is the rook from e1");
    assert_eq!(key.to(), Square::E5, "tactic move lands on e5");

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
