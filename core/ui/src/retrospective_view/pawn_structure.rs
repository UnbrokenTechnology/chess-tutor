//! Pawn-structure card builder.
//!
//! The prose (heading + detail, with the "you" / "they" reframe and the
//! worsened/improved sub-term wording) is produced by the shared teaching
//! translator ([`chess_tutor_teaching`]) from a [`Claim::PawnStructure`];
//! the shared salience (per-sub-term threshold gating, worsened-over-
//! improved precedence per side) lives in [`pawn_structure_claims`]. This
//! builder owns only the *structured* card surface the translator
//! deliberately doesn't carry — the sentiment and the terse summary.
//!
//! Split out of `retrospective_view`; the orchestrator
//! ([`super::build_retrospective_view`]) assembles the cards.

use chess_tutor_engine::analysis::PawnStructureOutcome;

use chess_tutor_teaching::claim::{
    pawn_structure_claims, Claim, PawnSide, StructureDirection,
};
use chess_tutor_teaching::phrasing::{phrase, Locale, Perspective, PhrasingContext, Verbosity};

use crate::view::{RetrospectiveCategory, RetrospectiveItem, Sentiment};

// ---------------------------------------------------------------------
// Pawn structure
// ---------------------------------------------------------------------

/// Build the pawn-structure card for one analysed move. `perspective`
/// selects "you" vs "they" and drives the student-POV sentiment colour.
///
/// Returns at most one card: `pawn_structure_claims` can emit one claim
/// per side (mover + opponent), but the card surface is a single
/// heading + detail, so the mover-side claim wins when present and the
/// opponent-side claim is the fallback. (The CLI prints both lines; the
/// GUI keeps one scannable card, matching the prior single-card shape.)
pub(super) fn build_pawn_structure_item(
    outcome: &PawnStructureOutcome,
    perspective: Perspective,
) -> Option<RetrospectiveItem> {
    let ctx = PhrasingContext {
        perspective,
        locale: Locale::En,
        verbosity: Verbosity::Normal,
        reveal_moves: false,
    };
    // Prefer the mover-side claim (the move the user just made acting on
    // their own structure is the more direct teaching point); fall back
    // to the opponent-side claim when only that fired.
    let claims = pawn_structure_claims(outcome);
    let claim = claims
        .iter()
        .find(|c| matches!(c, Claim::PawnStructure { side: PawnSide::Mover, .. }))
        .or_else(|| claims.first())?;
    Some(pawn_structure_item(claim, &ctx))
}

/// Turn one [`Claim::PawnStructure`] into a card — prose from the
/// translator, structured surface (sentiment, terse summary) computed
/// here from the claim's payload.
fn pawn_structure_item(claim: &Claim, ctx: &PhrasingContext) -> RetrospectiveItem {
    let phrasing = phrase(claim, ctx);
    let Claim::PawnStructure {
        side,
        direction,
        categories,
    } = claim
    else {
        unreachable!("pawn_structure_claims always returns Claim::PawnStructure");
    };

    // The structure is the user's when the moving side is the user
    // (Player + Mover); the player's POV is fixed here.
    let structure_is_user =
        (*side == PawnSide::Mover) == (ctx.perspective == Perspective::Player);

    // Sentiment is a function of "good for the user?" — worsening the
    // user's own structure is bad; worsening the opponent's is good.
    let sentiment = match (direction, structure_is_user) {
        (StructureDirection::Worsened, true) => Sentiment::Negative,
        (StructureDirection::Worsened, false) => Sentiment::Positive,
        (StructureDirection::Improved, true) => Sentiment::Positive,
        (StructureDirection::Improved, false) => Sentiment::Negative,
    };

    RetrospectiveItem {
        category: RetrospectiveCategory::PawnStructure,
        heading: phrasing.summary,
        summary: format!("{} sub-term(s) shifted", categories.len()),
        detail: phrasing.detail.unwrap_or_default(),
        score_delta_pawns: None,
        sentiment,
        annotations: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chess_tutor_engine::eval::PawnsBreakdown;
    use chess_tutor_engine::types::Score;

    fn pb(
        connected: i32,
        isolated: i32,
        backward: i32,
        doubled: i32,
        weak_unopposed: i32,
        weak_lever: i32,
    ) -> PawnsBreakdown {
        PawnsBreakdown {
            connected: Score::new(connected, 0),
            isolated: Score::new(isolated, 0),
            backward: Score::new(backward, 0),
            doubled: Score::new(doubled, 0),
            weak_unopposed: Score::new(weak_unopposed, 0),
            weak_lever: Score::new(weak_lever, 0),
        }
    }

    fn outcome(
        ours_pre: PawnsBreakdown,
        ours_post: PawnsBreakdown,
        theirs_pre: PawnsBreakdown,
        theirs_post: PawnsBreakdown,
    ) -> PawnStructureOutcome {
        PawnStructureOutcome {
            ours_pre,
            ours_post,
            theirs_pre,
            theirs_post,
        }
    }

    #[test]
    fn no_shift_yields_no_card() {
        let o = outcome(
            pb(0, 0, 0, 0, 0, 0),
            pb(0, 0, 0, 0, 0, 0),
            pb(0, 0, 0, 0, 0, 0),
            pb(0, 0, 0, 0, 0, 0),
        );
        assert!(build_pawn_structure_item(&o, Perspective::Player).is_none());
    }

    #[test]
    fn our_doubled_pawn_is_negative_with_translator_heading() {
        let o = outcome(
            pb(0, 0, 0, 0, 0, 0),
            pb(0, 0, 0, -20, 0, 0),
            pb(0, 0, 0, 0, 0, 0),
            pb(0, 0, 0, 0, 0, 0),
        );
        let card = build_pawn_structure_item(&o, Perspective::Player).expect("a worsened card");
        assert_eq!(
            card.heading,
            "Your pawn structure weakened: doubled a pawn."
        );
        assert_eq!(card.sentiment, Sentiment::Negative);
    }

    #[test]
    fn weakening_theirs_is_positive_opportunity() {
        let o = outcome(
            pb(0, 0, 0, 0, 0, 0),
            pb(0, 0, 0, 0, 0, 0),
            pb(0, 0, 0, 0, 0, 0),
            pb(0, 0, 0, -20, 0, 0),
        );
        let card = build_pawn_structure_item(&o, Perspective::Player).expect("a their-weakened card");
        assert!(
            card.heading
                .starts_with("You weakened the opponent's pawn structure"),
            "{}",
            card.heading
        );
        assert_eq!(card.sentiment, Sentiment::Positive);
    }

    #[test]
    fn improving_our_structure_is_positive() {
        let o = outcome(
            pb(0, 0, 0, -20, 0, 0),
            pb(0, 0, 0, 0, 0, 0),
            pb(0, 0, 0, 0, 0, 0),
            pb(0, 0, 0, 0, 0, 0),
        );
        let card = build_pawn_structure_item(&o, Perspective::Player).expect("an improved card");
        assert!(
            card.heading.starts_with("Your pawn structure improved"),
            "{}",
            card.heading
        );
        assert_eq!(card.sentiment, Sentiment::Positive);
    }

    #[test]
    fn mover_side_wins_when_both_fire() {
        // Both sides moved a sub-term; the card shows the mover side.
        let o = outcome(
            pb(0, 0, 0, 0, 0, 0),
            pb(0, 0, 0, -20, 0, 0),
            pb(0, 0, 0, 0, 0, 0),
            pb(0, 0, 0, -20, 0, 0),
        );
        let card = build_pawn_structure_item(&o, Perspective::Player).expect("a card");
        assert!(card.heading.starts_with("Your pawn structure weakened"));
    }
}
