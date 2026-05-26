//! King-safety card builders.
//!
//! Split out of `retrospective_view`; the orchestrator
//! ([`super::build_retrospective_view`]) assembles the cards.

use chess_tutor_engine::analysis::KingSafetyOutcome;

use crate::view::{
    AnnotationKind, BoardAnnotation, RetrospectiveCategory,
    RetrospectiveItem, Sentiment,
};


// ---------------------------------------------------------------------
// King safety
// ---------------------------------------------------------------------

const KING_SHELTER_DELTA_THRESHOLD_CP: i32 = 25;
const KING_SHELTER_ENDGAME_PHASE_CUTOFF: i32 = 32;

pub(super) fn build_king_safety_items(outcome: &KingSafetyOutcome) -> Vec<RetrospectiveItem> {
    let mut items = Vec::new();
    let shelter_relevant = outcome.phase >= KING_SHELTER_ENDGAME_PHASE_CUTOFF;

    let ours_attackers_up = outcome.ours_attackers_delta() > 0;
    let ours_shield_down = shelter_relevant
        && outcome.ours_pawn_shield_mg_delta() <= -KING_SHELTER_DELTA_THRESHOLD_CP;
    let ours_attackers_down = outcome.ours_attackers_delta() < 0;
    let ours_shield_up = shelter_relevant
        && outcome.ours_pawn_shield_mg_delta() >= KING_SHELTER_DELTA_THRESHOLD_CP;

    if ours_attackers_up || ours_shield_down {
        items.push(RetrospectiveItem {
            category: RetrospectiveCategory::KingSafety,
            heading: "Your king is more exposed".to_string(),
            summary: king_safety_summary_exposure(
                outcome.ours_post.attackers_count,
                outcome.ours_pre.attackers_count,
                outcome.ours_pawn_shield_mg_delta(),
                ours_shield_down,
            ),
            detail: king_safety_detail(
                outcome.ours_pre.attackers_count,
                outcome.ours_post.attackers_count,
                outcome.ours_pre.pawn_shield_mg,
                outcome.ours_post.pawn_shield_mg,
                ours_attackers_up,
                ours_shield_down,
            ),
            score_delta_pawns: None,
            sentiment: Sentiment::Negative,
            annotations: vec![BoardAnnotation::SquareHighlight {
                square: outcome.ours_post.king_sq,
                kind: AnnotationKind::KingRing,
            }],
        });
    } else if ours_attackers_down || ours_shield_up {
        items.push(RetrospectiveItem {
            category: RetrospectiveCategory::KingSafety,
            heading: "Your king is safer".to_string(),
            summary: king_safety_summary_safer(
                outcome.ours_post.attackers_count,
                outcome.ours_pre.attackers_count,
                outcome.ours_pawn_shield_mg_delta(),
                ours_shield_up,
            ),
            detail: king_safety_detail(
                outcome.ours_pre.attackers_count,
                outcome.ours_post.attackers_count,
                outcome.ours_pre.pawn_shield_mg,
                outcome.ours_post.pawn_shield_mg,
                ours_attackers_down,
                ours_shield_up,
            ),
            score_delta_pawns: None,
            sentiment: Sentiment::Positive,
            annotations: vec![BoardAnnotation::SquareHighlight {
                square: outcome.ours_post.king_sq,
                kind: AnnotationKind::GoodPiece,
            }],
        });
    }

    let theirs_attackers_up = outcome.theirs_attackers_delta() > 0;
    let theirs_shield_down = shelter_relevant
        && outcome.theirs_pawn_shield_mg_delta() <= -KING_SHELTER_DELTA_THRESHOLD_CP;
    let theirs_attackers_down = outcome.theirs_attackers_delta() < 0;
    let theirs_shield_up = shelter_relevant
        && outcome.theirs_pawn_shield_mg_delta() >= KING_SHELTER_DELTA_THRESHOLD_CP;

    if theirs_attackers_up || theirs_shield_down {
        items.push(RetrospectiveItem {
            category: RetrospectiveCategory::KingSafety,
            heading: "You expose the opponent's king".to_string(),
            summary: king_safety_summary_exposure(
                outcome.theirs_post.attackers_count,
                outcome.theirs_pre.attackers_count,
                outcome.theirs_pawn_shield_mg_delta(),
                theirs_shield_down,
            ),
            detail: king_safety_detail(
                outcome.theirs_pre.attackers_count,
                outcome.theirs_post.attackers_count,
                outcome.theirs_pre.pawn_shield_mg,
                outcome.theirs_post.pawn_shield_mg,
                theirs_attackers_up,
                theirs_shield_down,
            ),
            score_delta_pawns: None,
            sentiment: Sentiment::Positive,
            annotations: vec![BoardAnnotation::SquareHighlight {
                square: outcome.theirs_post.king_sq,
                kind: AnnotationKind::KingRing,
            }],
        });
    } else if theirs_attackers_down || theirs_shield_up {
        items.push(RetrospectiveItem {
            category: RetrospectiveCategory::KingSafety,
            heading: "The opponent's king is safer".to_string(),
            summary: king_safety_summary_safer(
                outcome.theirs_post.attackers_count,
                outcome.theirs_pre.attackers_count,
                outcome.theirs_pawn_shield_mg_delta(),
                theirs_shield_up,
            ),
            detail: king_safety_detail(
                outcome.theirs_pre.attackers_count,
                outcome.theirs_post.attackers_count,
                outcome.theirs_pre.pawn_shield_mg,
                outcome.theirs_post.pawn_shield_mg,
                theirs_attackers_down,
                theirs_shield_up,
            ),
            score_delta_pawns: None,
            sentiment: Sentiment::Negative,
            annotations: vec![BoardAnnotation::SquareHighlight {
                square: outcome.theirs_post.king_sq,
                kind: AnnotationKind::GoodPiece,
            }],
        });
    }

    items
}

pub(super) fn king_safety_summary_exposure(
    post_atk: i32,
    pre_atk: i32,
    shield_delta_cp: i32,
    shield_changed: bool,
) -> String {
    let mut parts = Vec::new();
    if post_atk > pre_atk {
        parts.push(format!("{} attackers (up from {})", post_atk, pre_atk));
    }
    if shield_changed {
        parts.push(format!(
            "shield {:+.2}",
            shield_delta_cp as f32 / 100.0
        ));
    }
    parts.join(", ")
}

pub(super) fn king_safety_summary_safer(
    post_atk: i32,
    pre_atk: i32,
    shield_delta_cp: i32,
    shield_changed: bool,
) -> String {
    let mut parts = Vec::new();
    if post_atk < pre_atk {
        parts.push(format!("attackers down to {} (from {})", post_atk, pre_atk));
    }
    if shield_changed {
        parts.push(format!(
            "shield {:+.2}",
            shield_delta_cp as f32 / 100.0
        ));
    }
    parts.join(", ")
}

pub(super) fn king_safety_detail(
    pre_atk: i32,
    post_atk: i32,
    pre_shield: i32,
    post_shield: i32,
    show_attackers: bool,
    show_shield: bool,
) -> String {
    let mut parts = Vec::new();
    if show_attackers {
        parts.push(format!(
            "Attackers on the king ring: {} → {}.",
            pre_atk, post_atk
        ));
    }
    if show_shield {
        parts.push(format!(
            "Pawn shield: {:+.2} → {:+.2}.",
            pre_shield as f32 / 100.0,
            post_shield as f32 / 100.0,
        ));
    }
    parts.join("\n")
}

