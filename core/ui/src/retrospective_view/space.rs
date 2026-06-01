//! Space card builders.
//!
//! The prose (heading + detail, with the "you" / "they" reframe) is
//! produced by the shared teaching translator ([`chess_tutor_teaching`])
//! from a [`Claim::Space`]; the shared salience (per-side threshold
//! gating) lives in [`space_claims`]. This builder owns only the
//! *structured* card surface the translator deliberately doesn't carry —
//! the sentiment, the score chip, and the board-highlight annotations
//! (the post-move space bitboards, a render concern that stays on the
//! [`SpaceOutcome`]).
//!
//! Split out of `retrospective_view`; the orchestrator
//! ([`super::build_retrospective_view`]) assembles the cards.

use chess_tutor_engine::analysis::SpaceOutcome;
use chess_tutor_engine::bitboard::Bitboard;

use chess_tutor_teaching::claim::{space_claims, Claim, SpaceDirection, SpaceSide, SPACE_DEFAULT_THRESHOLD_CP};
use chess_tutor_teaching::phrasing::{phrase, Locale, Perspective, PhrasingContext, Verbosity};

use crate::view::{
    AnnotationKind, BoardAnnotation, RetrospectiveCategory, RetrospectiveItem, Sentiment,
};

/// Build Space cards for one analysed move. Up to two cards: one per
/// side, fired independently when each side's `|space_delta_mg|` crosses
/// the threshold. Each card paints only that side's post-move space
/// (front + reinforced) — the renderer shows them sequentially as the
/// user clicks through, rather than crowding both sides' highlights onto
/// one board.
///
/// `perspective` selects "you" vs "they" and drives the student-POV
/// sentiment colour. `show_all` drops the firing floor to 1 cp so the
/// +14 single-square gains surface under "Show all signals".
pub(super) fn build_space_items(
    outcome: &SpaceOutcome,
    show_all: bool,
    perspective: Perspective,
) -> Vec<RetrospectiveItem> {
    let threshold = if show_all { 1 } else { SPACE_DEFAULT_THRESHOLD_CP };
    let ctx = PhrasingContext {
        perspective,
        locale: Locale::En,
        verbosity: Verbosity::Normal,
        reveal_moves: false,
    };
    space_claims(outcome, threshold)
        .iter()
        .map(|claim| space_item(claim, outcome, &ctx))
        .collect()
}

/// Turn one [`Claim::Space`] into a card — prose from the translator,
/// structured surface (sentiment, score chip, the side's post-move
/// space highlight) computed here from the claim's payload and the
/// outcome's bitboards.
fn space_item(claim: &Claim, outcome: &SpaceOutcome, ctx: &PhrasingContext) -> RetrospectiveItem {
    let phrasing = phrase(claim, ctx);
    let Claim::Space {
        side,
        direction,
        delta_mg,
    } = claim
    else {
        unreachable!("space_claims always returns Claim::Space");
    };

    // The space is the user's when the moving side is the user
    // (Player + Mover); the player's POV is fixed here.
    let space_is_user = (*side == SpaceSide::Mover) == (ctx.perspective == Perspective::Player);

    // Sentiment is "good for the user?": gaining your own space is
    // good, gaining the opponent's hurts you, squeezing theirs helps.
    let sentiment = match (direction, space_is_user) {
        (SpaceDirection::Gained, true) => Sentiment::Positive,
        (SpaceDirection::Lost, true) => Sentiment::Negative,
        (SpaceDirection::Gained, false) => Sentiment::Negative,
        (SpaceDirection::Lost, false) => Sentiment::Positive,
    };

    // User-POV score chip: the claim's `delta_mg` is side-relative
    // (positive = that side gained). For the user's own side that maps
    // straight through; for the opponent's it flips (their gain hurts).
    let score_delta_mg = if space_is_user { *delta_mg } else { -*delta_mg };

    // Highlight that side's post-move space.
    let (safe_post, reinforced_post) = match side {
        SpaceSide::Mover => (outcome.ours_safe_post, outcome.ours_reinforced_post),
        SpaceSide::Opponent => (outcome.theirs_safe_post, outcome.theirs_reinforced_post),
    };

    RetrospectiveItem {
        category: RetrospectiveCategory::Space,
        heading: phrasing.summary,
        summary: format!("{:+.2} pawns", score_delta_mg as f32 / 100.0),
        detail: phrasing.detail.unwrap_or_default(),
        score_delta_pawns: Some(score_delta_mg as f32 / 100.0),
        sentiment,
        annotations: space_annotations(safe_post, reinforced_post),
    }
}

/// The two-tier post-move space highlight: "front" squares (the safe
/// box minus the reinforced subset) plus the doubly-rewarded
/// "reinforced" squares. Reinforced is always a subset of safe, so we
/// subtract it out of the front tier to keep each square painted once.
fn space_annotations(safe_post: Bitboard, reinforced_post: Bitboard) -> Vec<BoardAnnotation> {
    let front_only = safe_post & !reinforced_post;
    let mut annotations: Vec<BoardAnnotation> = Vec::new();
    for sq in front_only {
        annotations.push(BoardAnnotation::SquareHighlight {
            square: sq,
            kind: AnnotationKind::SpaceFront,
        });
    }
    for sq in reinforced_post {
        annotations.push(BoardAnnotation::SquareHighlight {
            square: sq,
            kind: AnnotationKind::SpaceReinforced,
        });
    }
    annotations
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::too_many_arguments)]
    fn outcome(
        ours_pre: i32,
        ours_post: i32,
        theirs_pre: i32,
        theirs_post: i32,
    ) -> SpaceOutcome {
        SpaceOutcome {
            ours_space_pre_mg: ours_pre,
            ours_space_post_mg: ours_post,
            theirs_space_pre_mg: theirs_pre,
            theirs_space_post_mg: theirs_post,
            ours_piece_count_pre: 16,
            ours_piece_count_post: 16,
            theirs_piece_count_pre: 16,
            theirs_piece_count_post: 16,
            ours_safe_post: Bitboard::EMPTY,
            ours_reinforced_post: Bitboard::EMPTY,
            theirs_safe_post: Bitboard::EMPTY,
            theirs_reinforced_post: Bitboard::EMPTY,
        }
    }

    #[test]
    fn our_gain_is_positive_with_translator_heading() {
        // Our space grew 40 cp; theirs unchanged.
        let o = outcome(20, 60, 0, 0);
        let cards = build_space_items(&o, false, Perspective::Player);
        let card = cards.first().expect("a space card");
        assert_eq!(card.heading, "You gained space");
        assert_eq!(card.sentiment, Sentiment::Positive);
        assert_eq!(card.score_delta_pawns, Some(0.40));
    }

    #[test]
    fn squeezing_theirs_is_positive_opportunity() {
        // Opponent's space shrank 40 cp; the user-POV chip flips so a
        // squeeze reads as a gain for the user.
        let o = outcome(0, 0, 60, 20);
        let cards = build_space_items(&o, false, Perspective::Player);
        let card = cards.first().expect("a space card");
        assert_eq!(card.heading, "You squeezed the opponent's space");
        assert_eq!(card.sentiment, Sentiment::Positive);
        assert_eq!(card.score_delta_pawns, Some(0.40));
    }

    #[test]
    fn opponent_gaining_space_is_negative() {
        let o = outcome(0, 0, 20, 60);
        let cards = build_space_items(&o, false, Perspective::Player);
        let card = cards.first().expect("a space card");
        assert_eq!(card.heading, "The opponent gained space");
        assert_eq!(card.sentiment, Sentiment::Negative);
        assert_eq!(card.score_delta_pawns, Some(-0.40));
    }

    #[test]
    fn below_threshold_yields_no_card() {
        let o = outcome(0, 10, 0, 0);
        assert!(build_space_items(&o, false, Perspective::Player).is_empty());
    }
}
