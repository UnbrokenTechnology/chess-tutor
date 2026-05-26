//! Passed-pawns card builder.
//!
//! Split out of `retrospective_view`; the orchestrator
//! ([`super::build_retrospective_view`]) assembles the cards.

use chess_tutor_engine::analysis::PassedPawnsOutcome;
use chess_tutor_engine::eval::PassedBreakdown;

use crate::view::{
    RetrospectiveCategory,
    RetrospectiveItem, Sentiment,
};


// ---------------------------------------------------------------------
// Passed pawns
// ---------------------------------------------------------------------

const PASSED_DELTA_THRESHOLD_CP: i32 = 20;

pub(super) fn passed_total_mg(bd: &PassedBreakdown) -> i32 {
    bd.rank_bonus.mg().0
        + bd.king_proximity.mg().0
        + bd.free_advance.mg().0
        + bd.stopper_penalty.mg().0
}

pub(super) fn build_passed_pawns_item(outcome: &PassedPawnsOutcome) -> Option<RetrospectiveItem> {
    let ours_pre = passed_total_mg(&outcome.ours_pre);
    let ours_post = passed_total_mg(&outcome.ours_post);
    let theirs_pre = passed_total_mg(&outcome.theirs_pre);
    let theirs_post = passed_total_mg(&outcome.theirs_post);
    let ours_delta = ours_post - ours_pre;
    let theirs_delta = theirs_post - theirs_pre;

    if ours_delta.abs() < PASSED_DELTA_THRESHOLD_CP
        && theirs_delta.abs() < PASSED_DELTA_THRESHOLD_CP
    {
        return None;
    }

    let (heading, sentiment, net_for_user) = if ours_delta.abs() >= theirs_delta.abs() {
        if ours_delta > 0 {
            ("Your passed pawns advanced", Sentiment::Positive, ours_delta)
        } else {
            ("Your passed pawns lost ground", Sentiment::Negative, ours_delta)
        }
    } else if theirs_delta > 0 {
        ("Opponent's passed pawns advanced", Sentiment::Negative, -theirs_delta)
    } else {
        ("You blunted their passed pawns", Sentiment::Positive, -theirs_delta)
    };

    let summary = format!(
        "yours {:+.2}, theirs {:+.2}",
        ours_delta as f32 / 100.0,
        theirs_delta as f32 / 100.0
    );
    let detail = "Passed pawns are pawns with no enemy pawns on the same file or \
                  adjacent files ahead of them. The engine scores them by rank, \
                  king proximity, and clear-path bonuses."
        .to_string();

    Some(RetrospectiveItem {
        category: RetrospectiveCategory::PassedPawns,
        heading: heading.to_string(),
        summary,
        detail,
        score_delta_pawns: Some(net_for_user as f32 / 100.0),
        sentiment,
        annotations: Vec::new(),
    })
}

// ---------------------------------------------------------------------
// Space
// ---------------------------------------------------------------------

