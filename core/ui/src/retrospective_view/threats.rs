//! Threats card builders (hanging / SEE-losing / pressure).
//!
//! Split out of `retrospective_view`; the orchestrator
//! ([`super::build_retrospective_view`]) assembles the cards.

use chess_tutor_engine::analysis::{
    HangingPiece, ThreatsOutcome,
};
use chess_tutor_engine::types::Square;

use crate::view::{
    AnnotationKind, BoardAnnotation, RetrospectiveCategory,
    RetrospectiveItem, Sentiment,
};

use super::helpers::*;

// ---------------------------------------------------------------------
// Threats
// ---------------------------------------------------------------------

pub(super) fn threats_items_empty(outcome: &ThreatsOutcome) -> bool {
    outcome.ours_hanging.is_empty()
        && outcome.theirs_hanging.is_empty()
        && outcome.ours_see_losing.is_empty()
        && outcome.theirs_see_losing.is_empty()
        && outcome.ours_pressured.is_empty()
        && outcome.theirs_pressured.is_empty()
}

pub(super) fn build_threat_items(
    outcome: &ThreatsOutcome,
    user_captures_by_square: &[(Square, u8)],
) -> Vec<RetrospectiveItem> {
    let mut items = Vec::new();

    // Highest-value guaranteed counter-threat — used to suppress
    // ours_hanging entries whose loss is irrelevant because the
    // opponent has a bigger problem on the board. Computed once
    // across both guaranteed-hanging and guaranteed-SEE-losing lists
    // since either qualifies as a winning counter-threat.
    let max_counter_threat_points =
        max_target_points(&outcome.theirs_hanging_guaranteed)
            .max(max_target_points(&outcome.theirs_see_losing_guaranteed));

    // Our hanging pieces — filter for misleading entries. Two cases
    // get filtered out:
    //   (1) "Planned recapture" — we just captured a piece of ≥ equal
    //       point value on the same square. The bishop on h6 right
    //       after Bxh6 is the second leg of a trade we initiated.
    //   (2) "Compensating counter-attack" — we have a guaranteed
    //       higher-value win elsewhere. The opponent has to address
    //       that bigger problem; our hanging bishop is no longer
    //       their best response.
    let ours_hanging_filtered = filter_misleading_hangs(
        &outcome.ours_hanging,
        user_captures_by_square,
        max_counter_threat_points,
    );
    if !ours_hanging_filtered.is_empty() {
        items.push(threat_item_from_hangs(
            &ours_hanging_filtered,
            "Your piece is hanging",
            Sentiment::Negative,
            true,
        ));
    }

    // "You can win material" only fires off the *guaranteed* list —
    // entries that survive every legal opponent response. The raw
    // theirs_hanging is a static snapshot and would mis-teach the
    // student about defensible threats (Nf3 attacks e5 but ...Nc6
    // defends, etc.).
    if !outcome.theirs_hanging_guaranteed.is_empty() {
        items.push(threat_item_from_hangs(
            &outcome.theirs_hanging_guaranteed,
            "You can win material",
            Sentiment::Positive,
            false,
        ));
    }

    let ours_see_losing_filtered = filter_misleading_hangs(
        &outcome.ours_see_losing,
        user_captures_by_square,
        max_counter_threat_points,
    );
    if !ours_see_losing_filtered.is_empty() {
        items.push(threat_item_from_hangs(
            &ours_see_losing_filtered,
            "Your piece loses to a trade",
            Sentiment::Negative,
            true,
        ));
    }
    if !outcome.theirs_see_losing_guaranteed.is_empty() {
        items.push(threat_item_from_hangs(
            &outcome.theirs_see_losing_guaranteed,
            "Their piece loses to a trade",
            Sentiment::Positive,
            false,
        ));
    }

    items
}

/// Drop hanging-piece entries that would mislead the student. Two
/// suppression cases:
///
/// 1. **Planned recapture**: we just captured a piece of ≥ equal
///    classical point value (P:1, N:3, B:3, R:5, Q:9) on the same
///    square the piece now sits on. The exchange is fair or
///    favorable — calling it a hang frames the student's deliberate
///    trade as a mistake.
///
/// 2. **Compensating counter-attack**: there's a guaranteed
///    higher-value win elsewhere on the board — `counter_threat_pts`
///    is greater than our hanging piece's points. The opponent can't
///    both address the bigger threat and capture our piece — they have
///    to choose the bigger problem, so our piece isn't really
///    hanging. This catches the classic "leave the bishop hanging,
///    threatening the queen" zwischenzug pattern, when the queen
///    is in the *guaranteed* win list (i.e. opponent can't save it
///    even by capturing the bishop, because the bishop wasn't the
///    only attacker).
///
/// Both checks use classical point values, not cp, so the student
/// reasons in the same units the cards present.
pub(super) fn filter_misleading_hangs(
    hangs: &[HangingPiece],
    user_captures_by_square: &[(Square, u8)],
    counter_threat_pts: u8,
) -> Vec<HangingPiece> {
    hangs
        .iter()
        .filter(|h| {
            let our_points = h.location.piece.classical_points();
            // Case 1: planned recapture on the same square.
            let planned_recapture =
                user_captures_by_square.iter().any(|(sq, captured_points)| {
                    *sq == h.location.square && *captured_points >= our_points
                });
            // Case 2: a guaranteed counter-threat of strictly higher
            // value than the hanging piece. "Strictly higher" because
            // an equal-value counter-threat is a wash — opponent
            // could plausibly take ours and accept the loss elsewhere
            // as compensation.
            let compensated_by_counter = counter_threat_pts > our_points;
            !(planned_recapture || compensated_by_counter)
        })
        .cloned()
        .collect()
}

/// Maximum classical point value across a hanging-piece list.
/// Returns 0 when the list is empty — used as the "no
/// counter-threat" sentinel by [`filter_misleading_hangs`].
pub(super) fn max_target_points(hangs: &[HangingPiece]) -> u8 {
    hangs
        .iter()
        .map(|h| h.location.piece.classical_points())
        .max()
        .unwrap_or(0)
}

pub(super) fn threat_item_from_hangs(
    hangs: &[HangingPiece],
    heading: &str,
    sentiment: Sentiment,
    target_is_ours: bool,
) -> RetrospectiveItem {
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
        let mut attacker_strs = Vec::new();
        for a in &h.attackers {
            attacker_strs.push(format!("{} on {}", piece_name(a.piece), a.square.to_algebraic()));
            annotations.push(BoardAnnotation::Arrow {
                from: a.square,
                to: h.location.square,
                kind: AnnotationKind::Attacker,
            });
        }
        let target_kind = if target_is_ours {
            AnnotationKind::Threat
        } else {
            AnnotationKind::GoodPiece
        };
        annotations.push(BoardAnnotation::SquareHighlight {
            square: h.location.square,
            kind: target_kind,
        });
        detail_lines.push(format!(
            "{} on {} — attacked by {}.",
            capitalize(piece_name(h.location.piece)),
            h.location.square.to_algebraic(),
            join_with_and(&attacker_strs),
        ));
    }

    RetrospectiveItem {
        category: RetrospectiveCategory::Threats,
        heading: heading.to_string(),
        summary,
        detail: detail_lines.join("\n"),
        score_delta_pawns: None,
        sentiment,
        annotations,
    }
}

