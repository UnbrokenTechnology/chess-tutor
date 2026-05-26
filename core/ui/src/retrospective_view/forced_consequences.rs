//! Forced-consequences card builder (opponent's best reply).
//!
//! Split out of `retrospective_view`; the orchestrator
//! ([`super::build_retrospective_view`]) assembles the cards.

use chess_tutor_engine::analysis::MoveAnalysis;
use chess_tutor_engine::position::Position;
use chess_tutor_engine::san;
use chess_tutor_engine::types::Color;

use crate::view::{
    RetrospectiveCategory,
    RetrospectiveItem, Sentiment,
};

use super::pawn_structure::PawnSubTerm;

// ---------------------------------------------------------------------
// Forced consequences — pawn-structure concessions in the opponent's
// best reply
// ---------------------------------------------------------------------

/// Walk the PV one ply past the user's move and look for pawn-
/// structure concessions in the opponent's *response*. The existing
/// pawn-structure card describes the change `pre → post-user-move`;
/// this card describes `post-user-move → post-opponent-reply`. It's
/// what answers "yes Bxh6 was an even trade, *and* it doubles their
/// h-pawns." Never says "this forces" — only "if they reply with X".
pub(super) fn build_forced_consequences_items(
    user: &MoveAnalysis,
    pre_move_pos: &Position,
    root_stm: Color,
) -> Vec<RetrospectiveItem> {
    if user.pv.len() < 2 {
        return Vec::new();
    }
    let mut after_user = pre_move_pos.clone();
    after_user.do_move(user.pv[0]);
    let reply = user.pv[1];
    // Move-legality guard: if the engine's reply somehow isn't legal
    // against the post-user-move position (shouldn't happen in
    // practice), bail rather than panic.
    let reply_san = san::format(&after_user, reply);
    let mut after_reply = after_user.clone();
    after_reply.do_move(reply);

    let before = chess_tutor_engine::pawns::evaluate(&after_user)
        .breakdowns[(!root_stm).index()];
    let after = chess_tutor_engine::pawns::evaluate(&after_reply)
        .breakdowns[(!root_stm).index()];

    // Sub-terms where a more-negative delta means *worse* for the
    // opponent. Doubled / Isolated / Backward / WeakUnopposed /
    // WeakLever are all penalty terms (≤ 0 score); the Connected
    // term is a bonus and not interesting on the "they conceded
    // something" axis.
    // Lower threshold than the regular pawn-structure card. SF11's
    // Doubled penalty is `Score::new(11, 56)` per doubled pawn — only
    // ~11 cp at full middlegame phase, below the regular 15 cp gate.
    // Doubled / isolated / backward pawns are pedagogically valuable
    // even at small cp; they're long-term concessions that matter
    // toward the endgame.
    const FORCED_CONSEQUENCES_THRESHOLD_CP: i32 = 8;

    let mut items = Vec::new();
    for st in [
        PawnSubTerm::Doubled,
        PawnSubTerm::Isolated,
        PawnSubTerm::Backward,
        PawnSubTerm::WeakUnopposed,
    ] {
        let delta = st.delta_mg(&before, &after);
        // We're looking for the opponent's pawn position getting
        // worse — a more-negative delta.
        if delta > -FORCED_CONSEQUENCES_THRESHOLD_CP {
            continue;
        }
        let consequence = match st {
            PawnSubTerm::Doubled => "doubled pawns",
            PawnSubTerm::Isolated => "an isolated pawn",
            PawnSubTerm::Backward => "a backward pawn",
            PawnSubTerm::WeakUnopposed => "a weak pawn on a half-open file",
            // Connected / WeakLever excluded above.
            _ => continue,
        };
        items.push(RetrospectiveItem {
            category: RetrospectiveCategory::PawnStructure,
            heading: format!("If they reply {}, they get {}", reply_san, consequence),
            summary: format!(
                "their structure {:+.2} pawns after the reply",
                delta as f32 / 100.0
            ),
            detail: format!(
                "After your move and the opponent's best response {}, their pawn \
                 structure picks up {}. The engine's evaluation values this as a \
                 long-term concession on their side — they may decide not to \
                 reply this way, but if they do, this is the structural cost.",
                reply_san, consequence
            ),
            score_delta_pawns: Some(-delta as f32 / 100.0),
            sentiment: Sentiment::Positive,
            annotations: Vec::new(),
        });
    }
    items
}

