//! Forced-consequences card builder (opponent's best reply).
//!
//! The prose (heading + summary + detail, with the "you" / "they"
//! reframe) is produced by the shared teaching translator
//! ([`chess_tutor_teaching`]) from a [`Claim::ForcedConsequence`]; the
//! shared salience (the PV walk one ply past the move, the replier-side
//! pawn diff, the per-concession threshold) lives in
//! [`forced_consequence_claims`]. This builder owns only the *structured*
//! card surface the translator deliberately doesn't carry — the
//! category, sentiment, and the white-POV score chip.
//!
//! Split out of `retrospective_view`; the orchestrator
//! ([`super::build_retrospective_view`]) assembles the cards.

use chess_tutor_engine::analysis::MoveAnalysis;
use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::Color;

use chess_tutor_teaching::claim::{forced_consequence_claims, Claim};
use chess_tutor_teaching::phrasing::{phrase, Locale, Perspective, PhrasingContext, Verbosity};

use crate::view::{RetrospectiveCategory, RetrospectiveItem, Sentiment};

// ---------------------------------------------------------------------
// Forced consequences — pawn-structure concessions in the opponent's
// best reply
// ---------------------------------------------------------------------

/// Build the forced-consequences cards: structural concessions the
/// opponent's best reply creates *on their own side*. The existing
/// pawn-structure card describes the change `pre → post-user-move`; these
/// describe `post-user-move → post-opponent-reply`. It's what answers
/// "yes Bxh6 was an even trade, *and* it doubles their h-pawns." Never
/// says "this forces" — only "if they reply with X".
///
/// `perspective` selects "you" vs "they" in the translator's prose and,
/// here, which side the concession lands on (the *replier* = non-mover):
/// in the Player perspective the replier is the opponent (the student's
/// opportunity, Positive); in the Opponent perspective the replier is the
/// user (a concession on the student's side, Negative).
pub(super) fn build_forced_consequences_items(
    user: &MoveAnalysis,
    pre_move_pos: &Position,
    root_stm: Color,
    perspective: Perspective,
) -> Vec<RetrospectiveItem> {
    let ctx = PhrasingContext {
        perspective,
        locale: Locale::En,
        verbosity: Verbosity::Normal,
        reveal_moves: false,
    };
    forced_consequence_claims(pre_move_pos, user, root_stm)
        .iter()
        .map(|claim| forced_consequence_item(claim, &ctx))
        .collect()
}

/// Turn one [`Claim::ForcedConsequence`] into a card — prose from the
/// translator, structured surface computed here from the claim payload.
fn forced_consequence_item(claim: &Claim, ctx: &PhrasingContext) -> RetrospectiveItem {
    let phrasing = phrase(claim, ctx);
    let Claim::ForcedConsequence { delta_mg, .. } = claim else {
        unreachable!("forced_consequence_claims always returns Claim::ForcedConsequence");
    };
    // The concession lands on the replier (the non-moving side). From the
    // Player perspective that's the opponent — *our* opportunity (Positive,
    // and the eval chip flips sign to read student-POV). From the Opponent
    // perspective the replier is the user, so the concession is on the
    // student's own side (Negative, chip unflipped).
    let (replier_poss, sentiment, chip) = match ctx.perspective {
        Perspective::Player => ("their", Sentiment::Positive, -*delta_mg as f32 / 100.0),
        Perspective::Opponent => ("your", Sentiment::Negative, *delta_mg as f32 / 100.0),
    };
    RetrospectiveItem {
        category: RetrospectiveCategory::PawnStructure,
        heading: phrasing.summary,
        summary: format!(
            "{replier_poss} structure {:+.2} pawns after the reply",
            *delta_mg as f32 / 100.0
        ),
        detail: phrasing.detail.unwrap_or_default(),
        score_delta_pawns: Some(chip),
        sentiment,
        annotations: Vec::new(),
    }
}
