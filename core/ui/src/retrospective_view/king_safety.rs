//! King-safety card builder.
//!
//! The prose (heading + pre→post detail, with the "you" / "they"
//! reframe and the flank-aware / direction-aware wording) is produced
//! by the shared teaching translator ([`chess_tutor_teaching`]) from a
//! [`Claim::KingSafety`]; the shared salience (per-side direction with
//! exposure-over-safer precedence, the attacker-count and threshold-
//! gated shelter clauses, the endgame shelter suppression) lives in
//! [`king_safety_claims`]. This builder owns only the *structured* card
//! surface the translator deliberately doesn't carry — the sentiment,
//! the terse stat summary, and the per-square board annotations.
//!
//! Split out of `retrospective_view`; the orchestrator
//! ([`super::build_retrospective_view`]) assembles the cards.

use chess_tutor_engine::analysis::KingSafetyOutcome;

use chess_tutor_teaching::claim::{
    king_safety_claims, Claim, CountShift, KingSide, SafetyDirection, ShelterShift,
};
use chess_tutor_teaching::phrasing::{phrase, Locale, Perspective, PhrasingContext, Verbosity};

use crate::view::{
    AnnotationKind, BoardAnnotation, RetrospectiveCategory, RetrospectiveItem, Sentiment,
};

// ---------------------------------------------------------------------
// King safety
// ---------------------------------------------------------------------

/// Build every king-safety card for one analysed move. `perspective`
/// selects "you" vs "they" and drives the student-POV sentiment colour.
pub(super) fn build_king_safety_items(
    outcome: &KingSafetyOutcome,
    perspective: Perspective,
) -> Vec<RetrospectiveItem> {
    let ctx = PhrasingContext {
        perspective,
        locale: Locale::En,
        verbosity: Verbosity::Normal,
        reveal_moves: false,
    };
    king_safety_claims(outcome)
        .into_iter()
        .map(|claim| king_safety_item(&claim, &ctx))
        .collect()
}

/// Turn one [`Claim::KingSafety`] into a card — prose from the
/// translator, structured surface (sentiment, stat summary,
/// annotations) computed here from the claim's payload.
fn king_safety_item(claim: &Claim, ctx: &PhrasingContext) -> RetrospectiveItem {
    let phrasing = phrase(claim, ctx);
    let Claim::KingSafety {
        side,
        direction,
        attackers,
        shield,
        king_sq,
    } = claim
    else {
        unreachable!("king_safety_claims always returns Claim::KingSafety");
    };

    // The shifted king is the user's when the moving side is the user
    // (Player + Mover); the player's POV is fixed here.
    let king_is_user =
        (*side == KingSide::Mover) == (ctx.perspective == Perspective::Player);

    // Sentiment is a function of "good for the user?" — exposing the
    // user's own king is bad; exposing the opponent's is good.
    let sentiment = match (direction, king_is_user) {
        (SafetyDirection::MoreExposed, true) => Sentiment::Negative,
        (SafetyDirection::MoreExposed, false) => Sentiment::Positive,
        (SafetyDirection::Safer, true) => Sentiment::Positive,
        (SafetyDirection::Safer, false) => Sentiment::Negative,
    };

    // The KingRing highlight marks an exposure (the danger zone); a
    // safer shift highlights the king as a now-secure piece.
    let highlight_kind = match direction {
        SafetyDirection::MoreExposed => AnnotationKind::KingRing,
        SafetyDirection::Safer => AnnotationKind::GoodPiece,
    };

    RetrospectiveItem {
        category: RetrospectiveCategory::KingSafety,
        heading: phrasing.summary,
        summary: stat_summary(*direction, attackers.as_ref(), shield.as_ref()),
        detail: phrasing.detail.unwrap_or_default(),
        score_delta_pawns: None,
        sentiment,
        annotations: vec![BoardAnnotation::SquareHighlight {
            square: *king_sq,
            kind: highlight_kind,
        }],
    }
}

/// The terse, perspective-neutral stat line shown under the heading —
/// the structured summary the translator's prose deliberately omits.
fn stat_summary(
    direction: SafetyDirection,
    attackers: Option<&CountShift>,
    shield: Option<&ShelterShift>,
) -> String {
    let mut parts = Vec::new();
    if let Some(c) = attackers {
        match direction {
            SafetyDirection::MoreExposed => {
                parts.push(format!("{} attackers (up from {})", c.post, c.pre))
            }
            SafetyDirection::Safer => {
                parts.push(format!("attackers down to {} (from {})", c.post, c.pre))
            }
        }
    }
    if let Some(s) = shield {
        parts.push(format!("shield {:+.2}", (s.post_mg - s.pre_mg) as f32 / 100.0));
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chess_tutor_engine::analysis::KingSafetySnapshot;
    use chess_tutor_engine::types::Square;

    /// Build a [`KingSafetyOutcome`] from per-side `(attackers,
    /// shield_mg)` pre/post tuples; king squares default to central
    /// (e1 / e8) so the flank wording falls back to "king ring".
    fn ks(
        ours: ((i32, i32), (i32, i32)),
        theirs: ((i32, i32), (i32, i32)),
    ) -> KingSafetyOutcome {
        ks_kings((Square::E1, Square::E1), (Square::E8, Square::E8), ours, theirs)
    }

    fn ks_kings(
        ours_kings: (Square, Square),
        theirs_kings: (Square, Square),
        ours: ((i32, i32), (i32, i32)),
        theirs: ((i32, i32), (i32, i32)),
    ) -> KingSafetyOutcome {
        let snap = |king_sq: Square, (atk, shield): (i32, i32)| KingSafetySnapshot {
            king_sq,
            attackers_count: atk,
            attacks_count: 0,
            pawn_shield_mg: shield,
            pawn_shield_eg: 0,
            pawn_storm_mg: 0,
            pawn_storm_eg: 0,
            king_pawn_distance_eg: 0,
        };
        KingSafetyOutcome {
            ours_pre: snap(ours_kings.0, ours.0),
            ours_post: snap(ours_kings.1, ours.1),
            theirs_pre: snap(theirs_kings.0, theirs.0),
            theirs_post: snap(theirs_kings.1, theirs.1),
            phase: 128,
        }
    }

    #[test]
    fn no_shift_yields_no_card() {
        let items = build_king_safety_items(&ks(((1, 80), (1, 80)), ((0, 80), (0, 80))), Perspective::Player);
        assert!(items.is_empty());
    }

    #[test]
    fn our_king_exposed_is_negative_with_translator_heading() {
        let items = build_king_safety_items(&ks(((1, 80), (3, 80)), ((0, 80), (0, 80))), Perspective::Player);
        let card = items.first().expect("an exposure card");
        assert_eq!(card.heading, "Your king is more exposed: 3 attackers on the king ring (up from 1).");
        assert_eq!(card.summary, "3 attackers (up from 1)");
        assert_eq!(card.sentiment, Sentiment::Negative);
        assert!(matches!(
            card.annotations[0],
            BoardAnnotation::SquareHighlight { kind: AnnotationKind::KingRing, .. }
        ));
    }

    #[test]
    fn exposing_their_king_is_positive_opportunity() {
        let items = build_king_safety_items(&ks(((0, 80), (0, 80)), ((0, 80), (2, 80))), Perspective::Player);
        let card = items.first().expect("a their-exposure card");
        assert!(card.heading.starts_with("You expose the opponent's king"), "{}", card.heading);
        assert_eq!(card.sentiment, Sentiment::Positive);
    }

    #[test]
    fn our_king_safer_is_positive_with_good_piece_highlight() {
        let items = build_king_safety_items(&ks(((3, 80), (1, 80)), ((0, 80), (0, 80))), Perspective::Player);
        let card = items.first().expect("a safer card");
        assert!(card.heading.starts_with("Your king is safer"), "{}", card.heading);
        assert_eq!(card.summary, "attackers down to 1 (from 3)");
        assert_eq!(card.sentiment, Sentiment::Positive);
        assert!(matches!(
            card.annotations[0],
            BoardAnnotation::SquareHighlight { kind: AnnotationKind::GoodPiece, .. }
        ));
    }

    #[test]
    fn shelter_clause_suppressed_in_endgame() {
        let mut outcome = ks(((1, 80), (1, 20)), ((0, 80), (0, 80)));
        outcome.phase = 16;
        assert!(build_king_safety_items(&outcome, Perspective::Player).is_empty());
    }

    #[test]
    fn flank_label_after_castling() {
        let outcome = ks_kings(
            (Square::E1, Square::G1),
            (Square::E8, Square::E8),
            ((0, 80), (2, 80)),
            ((0, 80), (0, 80)),
        );
        let items = build_king_safety_items(&outcome, Perspective::Player);
        let card = items.first().expect("an exposure card");
        assert!(card.heading.contains("kingside"), "{}", card.heading);
    }
}
