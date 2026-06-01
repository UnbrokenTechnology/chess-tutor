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
    let view = build_coaching_view(&pos, Color::White, None, None);
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
    let view = build_coaching_view(&pos, Color::White, None, None);
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
    let view = build_coaching_view(&pos, Color::White, None, None);
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
    let view = build_coaching_view(&pos, Color::White, None, None);
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
    let view = build_coaching_view(&pos, Color::White, None, None);
    assert!(
        view.items.is_empty(),
        "startpos coaching should be empty, got: {:?}",
        view.items.iter().map(|i| &i.heading).collect::<Vec<_>>()
    );
}

#[test]
fn coaching_tactic_card_names_pattern_without_square_annotations() {
    use chess_tutor_engine::analysis::Confidence;
    use chess_tutor_engine::types::Square;
    // Synthesize a fork hit and feed it into a clean startpos so the
    // tactic card is the *only* item that fires.
    let pos = Position::startpos();
    let hit = TacticHit {
        pattern: TacticPattern::Fork,
        pv_ply: 0,
        primary_piece: Square::F7,
        targets: vec![Square::E5, Square::D8],
        material_gain: Some(300),
        confidence: Confidence::High,
        sacrifice: false,
        mate_pattern: None,
        key_move: None,
    };
    let view = build_coaching_view(&pos, Color::White, Some(&hit), None);
    let card = view
        .items
        .iter()
        .find(|i| i.heading.starts_with("There's"))
        .expect("tactic card fires");
    assert_eq!(card.heading, "There's a fork available");
    // Pedagogical rule: pre-move coaching never names squares — the
    // tactic card must produce zero board annotations.
    assert!(
        card.annotations.is_empty(),
        "coaching tactic card must not leak the squares: {:?}",
        card.annotations
    );
}

#[test]
fn coaching_tactic_card_suppressed_when_medium_confidence() {
    use chess_tutor_engine::analysis::Confidence;
    use chess_tutor_engine::types::Square;
    let pos = Position::startpos();
    let hit = TacticHit {
        pattern: TacticPattern::Fork,
        pv_ply: 0,
        primary_piece: Square::F7,
        targets: vec![Square::E5, Square::D8],
        material_gain: None,
        confidence: Confidence::Medium,
        sacrifice: false,
        mate_pattern: None,
        key_move: None,
    };
    let view = build_coaching_view(&pos, Color::White, Some(&hit), None);
    assert!(
        view.items.iter().all(|i| !i.heading.starts_with("There's")),
        "Medium-confidence hits stay off the coaching surface (retrospective only)"
    );
}

#[test]
fn coaching_view_surfaces_overloaded_enemy_defender() {
    // OVERLOAD fixture from overloading_tests.rs: black knight on d8
    // is the sole defender of both bishops on c6 and e6, each attacked
    // by a white rook. White (the user) is to move; the coaching panel
    // should name the overload and highlight all three squares.
    let pos =
        Position::from_fen("3n2k1/8/2b1b3/8/8/8/8/2R1R1K1 w - - 0 1").unwrap();
    let view = build_coaching_view(&pos, Color::White, None, None);
    let card = view
        .items
        .iter()
        .find(|i| i.heading == "Their piece is overloaded")
        .expect("overloaded card fires");
    assert_eq!(card.sentiment, Sentiment::Positive);
    // Highlights cover defender + both duties + arrows for each duty.
    use chess_tutor_engine::types::Square;
    let highlight_at = |sq, kind| {
        card.annotations.iter().any(|a| {
            matches!(a, BoardAnnotation::SquareHighlight { square, kind: k }
                if *square == sq && *k == kind)
        })
    };
    assert!(highlight_at(Square::D8, AnnotationKind::BadPiece));
    assert!(highlight_at(Square::C6, AnnotationKind::Threat));
    assert!(highlight_at(Square::E6, AnnotationKind::Threat));
}

#[test]
fn coaching_mate_hint_uses_named_heading_for_back_rank() {
    use chess_tutor_engine::analysis::{Confidence, MatePattern};
    use chess_tutor_engine::types::Square;
    let pos = Position::startpos();
    let hit = TacticHit {
        pattern: TacticPattern::Checkmate,
        pv_ply: 0,
        primary_piece: Square::H7,
        targets: vec![Square::G8],
        material_gain: None,
        confidence: Confidence::High,
        sacrifice: false,
        mate_pattern: Some(MatePattern::BackRank),
        key_move: None,
    };
    let view = build_coaching_view(&pos, Color::White, Some(&hit), None);
    assert!(
        view.items.iter().any(|i| i.heading.contains("back-rank mate")),
        "Back-rank mate must use its named heading on the coaching surface"
    );
}

#[test]
fn coaching_view_latent_threat_leads_and_demotes_positional_when_live() {
    // The discovered-attack-after-qxe6 case study FEN (White to move,
    // after …Qxe6). CLI confirms Black has a DiscoveredAttack loaded on
    // White's Re1 (fires via …Bxh2+) plus a RelativePin on Ra1, and a
    // hanging white Qc4 — i.e. the tactical-mode gate is LIVE. The
    // latent-threat card must lead and any positional (pawn) card must
    // be demoted under the quiet-position fold.
    //
    // Verified with: chess-tutor tactics
    //   "1r4nr/p3k3/4qpp1/4b2p/2Q5/8/PPPP1PPP/R1B1R1K1 w - - 0 1" --latent
    let pos = Position::from_fen(
        "1r4nr/p3k3/4qpp1/4b2p/2Q5/8/PPPP1PPP/R1B1R1K1 w - - 0 1",
    )
    .unwrap();
    let view = build_coaching_view(&pos, Color::White, None, None);

    // A latent-threat card must be present and must NOT be demoted.
    let latent_idx = view
        .items
        .iter()
        .position(|it| it.heading.starts_with("Your opponent has"))
        .expect("expected a latent-threat card to lead");
    assert!(
        !view.items[latent_idx].demoted,
        "the latent-threat card must lead, not be demoted"
    );
    // Pre-move pedagogical rule: the latent-threat card names no squares.
    assert!(
        view.items[latent_idx].annotations.is_empty(),
        "the latent-threat card must not leak squares: {:?}",
        view.items[latent_idx].annotations
    );
    assert_eq!(view.items[latent_idx].sentiment, Sentiment::Negative);

    // Any positional pawn-weakness card present must be demoted (the
    // gate is live), and must appear AFTER the latent-threat card.
    for (i, it) in view.items.iter().enumerate() {
        if it.category == RetrospectiveCategory::PawnStructure {
            assert!(
                it.demoted,
                "positional card '{}' must be demoted when the gate is live",
                it.heading
            );
            assert!(
                i > latent_idx,
                "positional cards must follow the tactical cards"
            );
        }
    }
}

#[test]
fn coaching_view_does_not_demote_in_quiet_position() {
    // A quiet endgame: Black has doubled + isolated c-pawns (a real
    // positional weakness the pawn-weakness card fires on) but there are
    // NO tactical reasons — no check, no latent threat, no check-
    // followup, no king-hunt, no tactic, no loose piece. The gate is
    // NOT live, so the positional card must lead and nothing is demoted.
    //
    // Verified with: chess-tutor tactics
    //   "4k3/p1p2ppp/2p5/8/8/8/PP3PPP/4K3 w - - 0 1" --latent
    //   (no tactic, no latent threats) and chess-tutor eval (Black
    //   isolated mg -15, doubled mg -11).
    let pos =
        Position::from_fen("4k3/p1p2ppp/2p5/8/8/8/PP3PPP/4K3 w - - 0 1").unwrap();
    let view = build_coaching_view(&pos, Color::White, None, None);

    // A pawn-weakness card must be present (Black's c-pawns).
    let pawn_card = view
        .items
        .iter()
        .find(|it| it.category == RetrospectiveCategory::PawnStructure)
        .expect("expected a pawn-weakness card on the doubled/isolated c-pawns");
    // Nothing is demoted in a quiet position.
    assert!(
        view.items.iter().all(|it| !it.demoted),
        "no card may be demoted when the tactical-mode gate is not live"
    );
    assert_eq!(pawn_card.sentiment, Sentiment::Positive); // theirs = opportunity
}
