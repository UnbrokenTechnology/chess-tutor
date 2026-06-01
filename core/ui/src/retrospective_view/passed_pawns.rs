//! Passed-pawns card builder.
//!
//! The prose (heading + detail, with the "you" / "they" reframe and the
//! advanced/lost-ground wording) is produced by the shared teaching
//! translator ([`chess_tutor_teaching`]) from a [`Claim::PassedPawns`];
//! the shared salience (aggregate threshold gating per side) lives in
//! [`passed_pawns_claims`]. This builder owns only the *structured* card
//! surface the translator deliberately doesn't carry — the sentiment, the
//! terse cp summary, and the user-POV score delta.
//!
//! Split out of `retrospective_view`; the orchestrator
//! ([`super::build_retrospective_view`]) assembles the cards.

use chess_tutor_engine::analysis::PassedPawnsOutcome;

use chess_tutor_teaching::claim::{passed_pawns_claims, Claim, PawnSide, StructureDirection};
use chess_tutor_teaching::phrasing::{phrase, Locale, Perspective, PhrasingContext, Verbosity};

use crate::view::{RetrospectiveCategory, RetrospectiveItem, Sentiment};

// ---------------------------------------------------------------------
// Passed pawns
// ---------------------------------------------------------------------

/// Build the passed-pawns card for one analysed move. `perspective`
/// selects "you" vs "they" and drives the student-POV sentiment colour.
///
/// `passed_pawns_claims` can emit one claim per side; the card surface is
/// a single heading, so the side with the larger absolute shift wins
/// (matching the prior single-card "whose passers moved most" rule).
pub(super) fn build_passed_pawns_item(
    outcome: &PassedPawnsOutcome,
    perspective: Perspective,
) -> Option<RetrospectiveItem> {
    let ctx = PhrasingContext {
        perspective,
        locale: Locale::En,
        verbosity: Verbosity::Normal,
        reveal_moves: false,
    };
    // Largest-magnitude shift wins the single card.
    let claim = passed_pawns_claims(outcome).into_iter().max_by_key(|c| {
        let Claim::PassedPawns { delta_mg, .. } = c else {
            unreachable!("passed_pawns_claims always returns Claim::PassedPawns");
        };
        delta_mg.abs()
    })?;
    Some(passed_pawns_item(&claim, &ctx))
}

/// Turn one [`Claim::PassedPawns`] into a card — prose from the
/// translator, structured surface (sentiment, cp summary, user-POV score
/// delta) computed here from the claim's payload.
fn passed_pawns_item(claim: &Claim, ctx: &PhrasingContext) -> RetrospectiveItem {
    let phrasing = phrase(claim, ctx);
    let Claim::PassedPawns {
        side,
        direction,
        delta_mg,
    } = claim
    else {
        unreachable!("passed_pawns_claims always returns Claim::PassedPawns");
    };

    // The passers are the user's when the moving side is the user
    // (Player + Mover); the player's POV is fixed here.
    let passers_are_user =
        (*side == PawnSide::Mover) == (ctx.perspective == Perspective::Player);

    // Sentiment is a function of "good for the user?" — the user's own
    // passers advancing is good; the opponent's advancing is bad.
    let sentiment = match (direction, passers_are_user) {
        (StructureDirection::Improved, true) => Sentiment::Positive,
        (StructureDirection::Worsened, true) => Sentiment::Negative,
        (StructureDirection::Improved, false) => Sentiment::Negative,
        (StructureDirection::Worsened, false) => Sentiment::Positive,
    };

    // `delta_mg` is the owning side's signed shift; the card's score
    // delta is from the *user's* POV — the user's own passers gaining is
    // positive, the opponent's passers gaining is a loss for the user, so
    // flip the sign on the opponent side.
    let score_delta = match side {
        PawnSide::Mover => *delta_mg as f32 / 100.0,
        PawnSide::Opponent => -*delta_mg as f32 / 100.0,
    };

    RetrospectiveItem {
        category: RetrospectiveCategory::PassedPawns,
        heading: phrasing.summary,
        summary: format!("passed-pawn value {:+.2}", *delta_mg as f32 / 100.0),
        detail: phrasing.detail.unwrap_or_default(),
        score_delta_pawns: Some(score_delta),
        sentiment,
        annotations: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chess_tutor_engine::eval::PassedBreakdown;
    use chess_tutor_engine::types::Score;

    fn pa(rank: i32, king_prox: i32, free_adv: i32, stopper: i32) -> PassedBreakdown {
        PassedBreakdown {
            rank_bonus: Score::new(rank, 0),
            king_proximity: Score::new(king_prox, 0),
            free_advance: Score::new(free_adv, 0),
            stopper_penalty: Score::new(stopper, 0),
        }
    }

    fn outcome(
        ours_pre: PassedBreakdown,
        ours_post: PassedBreakdown,
        theirs_pre: PassedBreakdown,
        theirs_post: PassedBreakdown,
    ) -> PassedPawnsOutcome {
        PassedPawnsOutcome {
            ours_pre,
            ours_post,
            theirs_pre,
            theirs_post,
        }
    }

    #[test]
    fn no_shift_yields_no_card() {
        let o = outcome(
            pa(0, 0, 0, 0),
            pa(10, 0, 0, 0),
            pa(0, 0, 0, 0),
            pa(0, 0, 0, 0),
        );
        assert!(build_passed_pawns_item(&o, Perspective::Player).is_none());
    }

    #[test]
    fn our_passer_advancing_is_positive() {
        let o = outcome(
            pa(50, 0, 0, 0),
            pa(90, 0, 0, 0),
            pa(0, 0, 0, 0),
            pa(0, 0, 0, 0),
        );
        let card = build_passed_pawns_item(&o, Perspective::Player).expect("an advanced card");
        assert_eq!(card.heading, "Your passed pawns advanced.");
        assert_eq!(card.sentiment, Sentiment::Positive);
        assert_eq!(card.score_delta_pawns, Some(0.40));
    }

    #[test]
    fn blunting_their_passer_is_positive_with_flipped_delta() {
        let o = outcome(
            pa(0, 0, 0, 0),
            pa(0, 0, 0, 0),
            pa(80, 0, 0, 0),
            pa(40, 0, 0, 0),
        );
        let card = build_passed_pawns_item(&o, Perspective::Player).expect("a blunted card");
        assert!(
            card.heading
                .starts_with("You blunted the opponent's passed pawns"),
            "{}",
            card.heading
        );
        assert_eq!(card.sentiment, Sentiment::Positive);
        // Their passers lost 40 cp; from the user's POV that's a +0.40 gain.
        assert_eq!(card.score_delta_pawns, Some(0.40));
    }

    #[test]
    fn their_passer_advancing_is_negative() {
        let o = outcome(
            pa(0, 0, 0, 0),
            pa(0, 0, 0, 0),
            pa(50, 0, 0, 0),
            pa(90, 0, 0, 0),
        );
        let card = build_passed_pawns_item(&o, Perspective::Player).expect("a their-advanced card");
        assert!(
            card.heading
                .starts_with("The opponent's passed pawns advanced"),
            "{}",
            card.heading
        );
        assert_eq!(card.sentiment, Sentiment::Negative);
    }
}
