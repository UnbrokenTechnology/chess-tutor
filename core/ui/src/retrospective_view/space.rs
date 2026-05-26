//! Space card builders.
//!
//! Split out of `retrospective_view`; the orchestrator
//! ([`super::build_retrospective_view`]) assembles the cards.

use chess_tutor_engine::analysis::SpaceOutcome;

use crate::view::{
    AnnotationKind, BoardAnnotation, RetrospectiveCategory,
    RetrospectiveItem, Sentiment,
};


/// Default firing threshold for a Space card. One reinforced-square
/// change at full piece count moves the score by ~14 cp, so the
/// integer threshold is set just above to avoid surfacing the
/// always-on rocking caused by pawn-push-induced behind-set shifts.
/// Under "Show all signals" we drop to 1 cp so the +14 single-square
/// gains do surface.
const SPACE_DELTA_THRESHOLD_CP: i32 = 15;

/// Build Space cards. Up to two cards per move: one for our side and
/// one for the opponent's, fired independently when each side's
/// `|space_delta_mg|` crosses the threshold. Each card paints only
/// that side's post-move space (front + reinforced) — the renderer
/// shows them sequentially as the user clicks through, rather than
/// crowding both sides' highlights onto one board.
pub(super) fn build_space_items(outcome: &SpaceOutcome, show_all: bool) -> Vec<RetrospectiveItem> {
    let threshold = if show_all { 1 } else { SPACE_DELTA_THRESHOLD_CP };

    let mut items = Vec::new();
    if let Some(it) = build_space_item_ours(outcome, threshold) {
        items.push(it);
    }
    if let Some(it) = build_space_item_theirs(outcome, threshold) {
        items.push(it);
    }
    items
}

pub(super) fn build_space_item_ours(outcome: &SpaceOutcome, threshold: i32) -> Option<RetrospectiveItem> {
    let delta = outcome.ours_space_delta_mg();
    if delta.abs() < threshold {
        return None;
    }
    let (heading, sentiment) = if delta > 0 {
        ("You gained space", Sentiment::Positive)
    } else {
        ("You lost space", Sentiment::Negative)
    };
    Some(make_space_card(
        heading,
        sentiment,
        delta,
        outcome.ours_safe_post,
        outcome.ours_reinforced_post,
    ))
}

pub(super) fn build_space_item_theirs(outcome: &SpaceOutcome, threshold: i32) -> Option<RetrospectiveItem> {
    let delta = outcome.theirs_space_delta_mg();
    if delta.abs() < threshold {
        return None;
    }
    let (heading, sentiment) = if delta > 0 {
        ("Opponent gained space", Sentiment::Negative)
    } else {
        ("You squeezed the opponent's space", Sentiment::Positive)
    };
    // Score-delta is from the user's POV — opponent gaining space
    // hurts the user, so flip the sign.
    Some(make_space_card(
        heading,
        sentiment,
        -delta,
        outcome.theirs_safe_post,
        outcome.theirs_reinforced_post,
    ))
}

pub(super) fn make_space_card(
    heading: &str,
    sentiment: Sentiment,
    score_delta_mg: i32,
    safe_post: chess_tutor_engine::bitboard::Bitboard,
    reinforced_post: chess_tutor_engine::bitboard::Bitboard,
) -> RetrospectiveItem {
    let summary = format!("{:+.2} pawns", score_delta_mg as f32 / 100.0);
    let detail = "Stockfish's space term scores the central c–f files across the three ranks \
                  in front of your back row. Squares the enemy pawns attack don't count; \
                  squares on or behind your own pawn that no enemy piece attacks count \
                  twice. The bonus is squared by piece count, so space matters most when \
                  the board is still full."
        .to_string();

    // Front-only (highlighted as `SpaceFront`) vs. reinforced (the
    // doubly-rewarded subset, highlighted as `SpaceReinforced`).
    // Reinforced is always a subset of safe, so we subtract it out of
    // the front tier to keep each square painted exactly once.
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

    RetrospectiveItem {
        category: RetrospectiveCategory::Space,
        heading: heading.to_string(),
        summary,
        detail,
        score_delta_pawns: Some(score_delta_mg as f32 / 100.0),
        sentiment,
        annotations,
    }
}

