use super::*;

#[test]
fn coaching_view_lists_their_undefended_piece_after_a_blunder() {
    // Black just played a tempo-losing move that hangs their
    // bishop. The coaching panel on white's turn should call out
    // the opportunity.
    let pos = Position::from_fen(
        "rnbqk1nr/pppp1ppp/8/4p3/4P1b1/5N2/PPPP1PPP/RNBQKB1R w KQkq - 0 3",
    )
    .unwrap();
    let view = build_coaching_view(&pos, Color::White);
    assert!(
        view.items
            .iter()
            .any(|it| it.heading == "Look for a capture"),
        "expected an opportunity card for the bishop on g4, got: {:?}",
        view.items.iter().map(|i| &i.heading).collect::<Vec<_>>()
    );
}

#[test]
fn coaching_view_suppresses_unreachable_captures_when_in_check() {
    // The exact case the user reported: it's White's turn, the
    // king on g2 is in check from a rook on f2, and the static
    // hanging-piece scan would flag the undefended pawn on g7 as
    // a capture target. But no legal move reaches g7 (every
    // legal response addresses the check). The opportunity card
    // must not surface.
    let pos = Position::from_fen(
        "2k4r/1ppbR1pp/pqp5/8/B5n1/2P3P1/PP2QrKP/RN6 w - - 0 1",
    )
    .unwrap();
    let view = build_coaching_view(&pos, Color::White);
    assert!(
        !view.items.iter().any(|it| it.heading == "Look for a capture"),
        "no legal capture reaches an undefended piece while in check, \
         expected no opportunity card, got: {:?}",
        view.items.iter().map(|i| &i.heading).collect::<Vec<_>>()
    );
    assert!(
        view.items
            .iter()
            .any(|it| it.heading == "Your king is in check"),
        "expected a check card, got: {:?}",
        view.items.iter().map(|i| &i.heading).collect::<Vec<_>>()
    );
}

#[test]
fn coaching_view_check_card_lists_response_counts() {
    // Spot-check the response-count text. A simple in-check
    // position with a few legal moves should produce a non-empty
    // detail mentioning the available responses.
    let pos = Position::from_fen(
        "2k4r/1ppbR1pp/pqp5/8/B5n1/2P3P1/PP2QrKP/RN6 w - - 0 1",
    )
    .unwrap();
    let view = build_coaching_view(&pos, Color::White);
    let check_card = view
        .items
        .iter()
        .find(|it| it.heading == "Your king is in check")
        .expect("check card");
    // The summary should name the checker square and piece.
    assert!(
        check_card.summary.contains("f2"),
        "summary should call out the rook on f2: {}",
        check_card.summary
    );
    assert!(
        check_card.detail.contains("address"),
        "detail should remind the student to address the check"
    );
}

#[test]
fn coaching_view_surfaces_en_passant_opportunity() {
    // White just played e4 → e5 against black's d-pawn, setting
    // up en passant: 1.e4 d5 2.e5 f5 — now exf6 ep is legal for
    // white. The coaching panel must surface this with the
    // captured pawn's square called out.
    // FEN after 1.e4 d5 2.e5 f5 (black-to-move position would be
    // odd; instead use a position where white can play en passant
    // immediately):
    // Standard 4.exd6 ep example after 1.e4 c5 2.Nf3 d6 3.d4 cxd4
    // 4.Nxd4 — no en passant. Instead: 1.e4 c5 2.e5 d5 — now
    // white can play exd6 ep on move 3.
    let pos = Position::from_fen(
        "rnbqkbnr/pp2pppp/8/2pPp3/8/8/PPPP1PPP/RNBQKBNR w KQkq e6 0 3",
    )
    .unwrap();
    let view = build_coaching_view(&pos, Color::White);
    let ep_card = view
        .items
        .iter()
        .find(|it| it.heading == "En passant capture available")
        .expect("expected en-passant card");
    // The captured pawn is on e5 (same file as the en-passant
    // target e6, same rank as the capturing pawn d5).
    assert!(
        ep_card.summary.contains("e5"),
        "summary should reference the captured pawn on e5: {}",
        ep_card.summary
    );
}

#[test]
fn coaching_view_is_empty_on_startpos() {
    // Standard starting position has no hanging pieces and no
    // structural weaknesses. The coaching panel should have
    // nothing to say beyond an encouragement to think.
    let pos = Position::startpos();
    let view = build_coaching_view(&pos, Color::White);
    assert!(
        view.items.is_empty(),
        "startpos coaching should be empty, got: {:?}",
        view.items.iter().map(|i| &i.heading).collect::<Vec<_>>()
    );
}
