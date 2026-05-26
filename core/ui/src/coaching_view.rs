//! Live-coaching view: features-to-notice on the position the user
//! is about to move from. Never names a move — surfaces structural
//! observations (their piece is undefended, your pawn is backward,
//! the d5 square is contested) so the student picks the move
//! themselves.
//!
//! Distinct from the retrospective:
//! - **Retrospective** describes what *changed* on the last move.
//!   Inputs: pre-move and post-move positions plus a `MoveAnalysis`.
//! - **Coaching** describes the *current* position. Inputs: just the
//!   `Position`. No search runs; the snapshot uses pawn / threats
//!   helpers directly.
//!
//! Compute is intentionally cheap (sub-ms in release) so it can rebuild
//! every frame without any worker round-trip. If a future Coached
//! feature needs a real search (e.g. "this outpost is reachable by
//! …"), we can promote to the worker thread the same way the hint
//! panel does.

use std::collections::HashSet;

use chess_tutor_engine::analysis::{list_hanging, HangingPiece};
use chess_tutor_engine::movegen::legal_moves_vec;
use chess_tutor_engine::pawns::PawnsEval;
use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::{Color, Move, MoveKind, PieceType, Square};

use crate::view::{
    AnnotationKind, BoardAnnotation, CoachingItem, CoachingViewModel, RetrospectiveCategory,
    Sentiment,
};

/// Build the structured coaching view from the live position. `user_color`
/// is the side the student is playing — surface "their" cards as
/// opportunities and "ours" cards as risks. The view model is empty
/// (no items) when nothing notable is on the board, in which case
/// the renderer paints an encouraging neutral message.
pub fn build_coaching_view(pos: &Position, user_color: Color) -> CoachingViewModel {
    let mut items: Vec<CoachingItem> = Vec::new();

    // Compute legal moves once. Used for:
    //   - Surfacing the "you're in check" card with concrete
    //     check-addressing context.
    //   - Filtering "Look for a capture" to only opportunities a
    //     legal move can actually take. Without this, the static
    //     hanging-piece list overcounts: pinned attackers and being-
    //     in-check are both cases where we *attack* a square but
    //     can't legally capture there.
    let mut scratch = pos.clone();
    let legal_moves = legal_moves_vec(&mut scratch);
    let legal_destinations: HashSet<Square> =
        legal_moves.iter().map(|m| m.to()).collect();

    // Check card first — when the king is under attack, that's the
    // only thing the student should be thinking about. Everything
    // else is filtered to keep the panel honest.
    if pos.in_check() && pos.side_to_move() == user_color {
        items.push(check_card(pos, &legal_moves));
    }

    // En passant: high-signal, easy to miss, deserves its own card
    // when available. The static `list_hanging` scan won't flag an
    // en-passant-capturable pawn because en passant isn't a normal
    // attack — the captured pawn sits on a square no enemy piece
    // directly attacks. We have to detect it from the legal-move
    // list itself.
    let en_passant_moves: Vec<Move> = legal_moves
        .iter()
        .copied()
        .filter(|m| m.kind() == MoveKind::EnPassant)
        .collect();
    if !en_passant_moves.is_empty() {
        items.push(en_passant_card(&en_passant_moves));
    }

    // Opportunity scan: undefended opponent pieces *we can actually
    // reach* with a legal move. The static `list_hanging` only checks
    // attacker bitboards, so a pinned attacker or a check-blocked
    // capture would still appear — we filter those out here.
    let theirs_hanging = list_hanging(pos, !user_color);
    let theirs_capturable: Vec<HangingPiece> = theirs_hanging
        .into_iter()
        .filter(|h| legal_destinations.contains(&h.location.square))
        .collect();
    if !theirs_capturable.is_empty() {
        items.push(opportunity_card(&theirs_capturable));
    }

    // Risk scan: our undefended pieces. We don't legal-move-filter
    // these because the threat is about the opponent's *next* turn
    // (which the engine can't fully evaluate without searching) —
    // a loose piece warrants the student's attention regardless of
    // whose turn it is right now.
    let ours_hanging = list_hanging(pos, user_color);
    if !ours_hanging.is_empty() {
        items.push(risk_card(&ours_hanging));
    }

    // Pawn weakness scan for both sides. Builds at most one card per
    // side per weakness kind (doubled / isolated / backward), so the
    // panel stays scannable.
    let pawns = chess_tutor_engine::pawns::evaluate(pos);
    items.extend(pawn_weakness_cards(&pawns, user_color, true));
    items.extend(pawn_weakness_cards(&pawns, !user_color, false));

    CoachingViewModel { items }
}

/// Build the "Your king is in check" card. Highlights the king and
/// every checking piece, and counts how many of the legal moves are
/// king moves vs. blocks/captures so the student knows what kind of
/// response options exist. Never names a specific move.
fn check_card(pos: &Position, legal_moves: &[Move]) -> CoachingItem {
    let us = pos.side_to_move();
    let king_sq = pos.king_square(us);
    let checkers = pos.checkers();

    let mut annotations = Vec::new();
    annotations.push(BoardAnnotation::SquareHighlight {
        square: king_sq,
        kind: AnnotationKind::Threat,
    });
    let mut checker_locations: Vec<(Square, PieceType)> = Vec::new();
    for sq in checkers {
        if let Some(piece) = pos.piece_on(sq) {
            checker_locations.push((sq, piece.kind()));
            annotations.push(BoardAnnotation::Arrow {
                from: sq,
                to: king_sq,
                kind: AnnotationKind::Attacker,
            });
        }
    }

    // Count response shapes from the legal moves. King moves =
    // "run." Other moves = "block or capture the checker." We don't
    // distinguish block vs capture in the count because the student
    // shouldn't need that split to reason — the categories are
    // really "move the king" vs "everything else."
    let king_move_count = legal_moves
        .iter()
        .filter(|m| m.from() == king_sq)
        .count();
    let other_count = legal_moves.len() - king_move_count;

    let summary = if checker_locations.len() == 1 {
        let (sq, piece) = checker_locations[0];
        format!(
            "checked by {} on {}",
            piece_name(piece),
            sq.to_algebraic()
        )
    } else {
        format!("double check by {} pieces", checker_locations.len())
    };

    let response_text = match (king_move_count, other_count) {
        (0, 0) => "No legal moves — this is checkmate.".to_string(),
        (k, 0) => format!(
            "{} king move{} available — the king must move (double check, or no piece can block or capture).",
            k,
            if k == 1 { "" } else { "s" }
        ),
        (0, n) => format!(
            "{} blocking-or-capturing move{} available — the king has nowhere safe to go.",
            n,
            if n == 1 { "" } else { "s" }
        ),
        (k, n) => format!(
            "{} response{} total: {} king move{} and {} block-or-capture move{}.",
            k + n,
            if k + n == 1 { "" } else { "s" },
            k,
            if k == 1 { "" } else { "s" },
            n,
            if n == 1 { "" } else { "s" }
        ),
    };

    let detail = format!(
        "Your king is in check — the only legal responses are moving the king \
         to safety, blocking the line of attack, or capturing the checking piece. \
         {} Other observations on the position only matter once the check is \
         addressed.",
        response_text
    );

    CoachingItem {
        category: RetrospectiveCategory::KingSafety,
        heading: "Your king is in check".to_string(),
        summary,
        detail,
        sentiment: Sentiment::Negative,
        annotations,
    }
}

/// Build the "En passant capture available" card from one or more
/// legal en-passant moves. Almost always one move in practice
/// (two pawns rarely have simultaneous en-passant chances), but we
/// handle the list generically.
///
/// Geometry: an en-passant move's `to()` is the empty square *behind*
/// the captured pawn from the captured side's POV. The captured pawn
/// itself sits on `Square::new(to.file(), from.rank())` — same file
/// as the destination, same rank as the capturing pawn before it
/// moved.
///
/// The card never names the capturing move; it just points at the
/// pawn that can be taken and reminds the student that en passant
/// is one-shot — only available on the move immediately after the
/// opponent's two-square push.
fn en_passant_card(moves: &[Move]) -> CoachingItem {
    let mut annotations: Vec<BoardAnnotation> = Vec::new();
    let mut captured_squares: Vec<Square> = Vec::new();
    for m in moves {
        let captured_sq = Square::new(m.to().file(), m.from().rank());
        captured_squares.push(captured_sq);
        // Highlight the captured pawn — that's what the student
        // needs to *see*. Also draw the attacker arrow so the
        // geometry of the en-passant move is visually obvious.
        annotations.push(BoardAnnotation::SquareHighlight {
            square: captured_sq,
            kind: AnnotationKind::GoodPiece,
        });
        annotations.push(BoardAnnotation::Arrow {
            from: m.from(),
            to: m.to(),
            kind: AnnotationKind::Attacker,
        });
        // Also highlight the en-passant destination square subtly so
        // the student sees where the capturing pawn would actually
        // land — en passant geometry surprises people because the
        // capturing piece doesn't end up on the captured piece's
        // square.
        annotations.push(BoardAnnotation::SquareHighlight {
            square: m.to(),
            kind: AnnotationKind::NewMobility,
        });
    }
    let summary = if captured_squares.len() == 1 {
        format!("pawn on {}", captured_squares[0].to_algebraic())
    } else {
        let strs: Vec<String> = captured_squares
            .iter()
            .map(|s| s.to_algebraic().to_string())
            .collect();
        format!("pawns on {}", strs.join(", "))
    };
    let detail = "Your opponent's last move was a two-square pawn push that landed \
                  alongside one of your pawns. You can capture it *as if* it had \
                  only moved one square — your pawn moves diagonally to the square \
                  the pawn would have stopped on if it'd only pushed one. \
                  \n\nEn passant is one-shot: it's only legal on the move \
                  immediately following the opponent's two-square push. If you \
                  don't take it now, the opportunity is gone."
        .to_string();
    CoachingItem {
        category: RetrospectiveCategory::Threats,
        heading: "En passant capture available".to_string(),
        summary,
        detail,
        sentiment: Sentiment::Positive,
        annotations,
    }
}

fn opportunity_card(hangs: &[HangingPiece]) -> CoachingItem {
    let summary = if hangs.len() == 1 {
        format!(
            "{} on {}",
            piece_name(hangs[0].location.piece),
            hangs[0].location.square.to_algebraic()
        )
    } else {
        format!("{} pieces", hangs.len())
    };
    let mut detail_lines = Vec::new();
    let mut annotations = Vec::new();
    for h in hangs {
        annotations.push(BoardAnnotation::SquareHighlight {
            square: h.location.square,
            kind: AnnotationKind::GoodPiece,
        });
        for a in &h.attackers {
            annotations.push(BoardAnnotation::Arrow {
                from: a.square,
                to: h.location.square,
                kind: AnnotationKind::Attacker,
            });
        }
        let attacker_text: Vec<String> = h
            .attackers
            .iter()
            .map(|a| format!("{} on {}", piece_name(a.piece), a.square.to_algebraic()))
            .collect();
        detail_lines.push(format!(
            "{} on {} — attacked by {}, no defenders. Check whether you can capture, \
             and whether the opponent can defend, counter-attack, or threaten back.",
            capitalize(piece_name(h.location.piece)),
            h.location.square.to_algebraic(),
            join_with_and(&attacker_text)
        ));
    }
    CoachingItem {
        category: RetrospectiveCategory::Threats,
        heading: "Look for a capture".to_string(),
        summary,
        detail: detail_lines.join("\n"),
        sentiment: Sentiment::Positive,
        annotations,
    }
}

fn risk_card(hangs: &[HangingPiece]) -> CoachingItem {
    let summary = if hangs.len() == 1 {
        format!(
            "{} on {}",
            piece_name(hangs[0].location.piece),
            hangs[0].location.square.to_algebraic()
        )
    } else {
        format!("{} pieces", hangs.len())
    };
    let mut detail_lines = Vec::new();
    let mut annotations = Vec::new();
    for h in hangs {
        annotations.push(BoardAnnotation::SquareHighlight {
            square: h.location.square,
            kind: AnnotationKind::Threat,
        });
        for a in &h.attackers {
            annotations.push(BoardAnnotation::Arrow {
                from: a.square,
                to: h.location.square,
                kind: AnnotationKind::Attacker,
            });
        }
        let attacker_text: Vec<String> = h
            .attackers
            .iter()
            .map(|a| format!("{} on {}", piece_name(a.piece), a.square.to_algebraic()))
            .collect();
        detail_lines.push(format!(
            "{} on {} — attacked by {}, no defenders. Consider defending it, moving it, \
             or making a stronger counter-threat.",
            capitalize(piece_name(h.location.piece)),
            h.location.square.to_algebraic(),
            join_with_and(&attacker_text)
        ));
    }
    CoachingItem {
        category: RetrospectiveCategory::Threats,
        heading: "Watch your loose piece".to_string(),
        summary,
        detail: detail_lines.join("\n"),
        sentiment: Sentiment::Negative,
        annotations,
    }
}

/// Doubled / isolated / backward pawn snapshot. `is_ours` flips the
/// framing: our weakness reads as a risk, theirs reads as something
/// to exploit.
fn pawn_weakness_cards(
    eval: &PawnsEval,
    color: Color,
    is_ours: bool,
) -> Vec<CoachingItem> {
    let bd = &eval.breakdowns[color.index()];
    let mut items = Vec::new();
    // Threshold matches the forced-consequences card (8 cp) — same
    // pedagogical level: small but real structural facts that
    // matter long-term.
    const COACHING_PAWN_THRESHOLD_CP: i32 = -8;
    let (doubled_mg, isolated_mg, backward_mg) =
        (bd.doubled.mg().0, bd.isolated.mg().0, bd.backward.mg().0);
    let (heading_us, heading_them) = (
        ("Your pawns are weak", "Their pawns are weak"),
        ("Your pawns are strong", "Their pawns are strong"),
    );
    let _ = heading_them.1; // silence warning; future "strong pawn" card
    let mut weaknesses: Vec<&str> = Vec::new();
    if doubled_mg <= COACHING_PAWN_THRESHOLD_CP {
        weaknesses.push("doubled pawn");
    }
    if isolated_mg <= COACHING_PAWN_THRESHOLD_CP {
        weaknesses.push("isolated pawn");
    }
    if backward_mg <= COACHING_PAWN_THRESHOLD_CP {
        weaknesses.push("backward pawn");
    }
    if weaknesses.is_empty() {
        return items;
    }
    let (heading, sentiment) = if is_ours {
        (heading_us.0, Sentiment::Negative)
    } else {
        (heading_us.1, Sentiment::Positive)
    };
    let summary = weaknesses.join(", ");
    let detail = if is_ours {
        format!(
            "Your pawn structure has {}. These are long-term weaknesses — they're \
             hard to defend in the endgame. When you choose a move, consider \
             whether you can resolve the weakness or whether the move makes it \
             worse.",
            join_with_and(&weaknesses.iter().map(|s| (*s).to_string()).collect::<Vec<_>>())
        )
    } else {
        format!(
            "Their pawn structure has {}. These are targets you can pressure — \
             they're hard for the opponent to defend, especially as material \
             comes off the board.",
            join_with_and(&weaknesses.iter().map(|s| (*s).to_string()).collect::<Vec<_>>())
        )
    };
    items.push(CoachingItem {
        category: RetrospectiveCategory::PawnStructure,
        heading: heading.to_string(),
        summary,
        detail,
        sentiment,
        annotations: Vec::new(),
    });
    items
}

fn piece_name(pt: PieceType) -> &'static str {
    match pt {
        PieceType::Pawn => "pawn",
        PieceType::Knight => "knight",
        PieceType::Bishop => "bishop",
        PieceType::Rook => "rook",
        PieceType::Queen => "queen",
        PieceType::King => "king",
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn join_with_and(items: &[String]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].clone(),
        2 => format!("{} and {}", items[0], items[1]),
        _ => {
            let head = &items[..items.len() - 1];
            format!("{}, and {}", head.join(", "), items[items.len() - 1])
        }
    }
}

#[cfg(test)]
mod tests {
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
}
