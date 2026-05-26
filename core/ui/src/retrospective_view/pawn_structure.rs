//! Pawn-structure card builder.
//!
//! Split out of `retrospective_view`; the orchestrator
//! ([`super::build_retrospective_view`]) assembles the cards.

use chess_tutor_engine::analysis::PawnStructureOutcome;
use chess_tutor_engine::eval::PawnsBreakdown;

use crate::view::{
    RetrospectiveCategory,
    RetrospectiveItem, Sentiment,
};


// ---------------------------------------------------------------------
// Pawn structure
// ---------------------------------------------------------------------

const PAWN_STRUCTURE_DELTA_THRESHOLD_CP: i32 = 15;

#[derive(Copy, Clone, Debug)]
pub(super) enum PawnSubTerm {
    Connected,
    Isolated,
    Backward,
    Doubled,
    WeakUnopposed,
    WeakLever,
}

impl PawnSubTerm {
    const ALL: [PawnSubTerm; 6] = [
        PawnSubTerm::Connected,
        PawnSubTerm::Isolated,
        PawnSubTerm::Backward,
        PawnSubTerm::Doubled,
        PawnSubTerm::WeakUnopposed,
        PawnSubTerm::WeakLever,
    ];
    pub(super) fn delta_mg(self, pre: &PawnsBreakdown, post: &PawnsBreakdown) -> i32 {
        match self {
            PawnSubTerm::Connected => post.connected.mg().0 - pre.connected.mg().0,
            PawnSubTerm::Isolated => post.isolated.mg().0 - pre.isolated.mg().0,
            PawnSubTerm::Backward => post.backward.mg().0 - pre.backward.mg().0,
            PawnSubTerm::Doubled => post.doubled.mg().0 - pre.doubled.mg().0,
            PawnSubTerm::WeakUnopposed => post.weak_unopposed.mg().0 - pre.weak_unopposed.mg().0,
            PawnSubTerm::WeakLever => post.weak_lever.mg().0 - pre.weak_lever.mg().0,
        }
    }
    fn worsened_phrase(self) -> &'static str {
        match self {
            PawnSubTerm::Connected => "broke pawn connections",
            PawnSubTerm::Isolated => "isolated a pawn",
            PawnSubTerm::Backward => "created a backward pawn",
            PawnSubTerm::Doubled => "doubled a pawn",
            PawnSubTerm::WeakUnopposed => "exposed a weak pawn",
            PawnSubTerm::WeakLever => "walked into a pawn lever",
        }
    }
    fn improved_phrase(self) -> &'static str {
        match self {
            PawnSubTerm::Connected => "connected pawns",
            PawnSubTerm::Isolated => "reconnected an isolated pawn",
            PawnSubTerm::Backward => "freed a backward pawn",
            PawnSubTerm::Doubled => "resolved a doubled pawn",
            PawnSubTerm::WeakUnopposed => "covered a weak pawn",
            PawnSubTerm::WeakLever => "resolved a pawn lever",
        }
    }
}

pub(super) fn pawn_clauses(pre: &PawnsBreakdown, post: &PawnsBreakdown) -> (Vec<&'static str>, Vec<&'static str>) {
    let mut worsened = Vec::new();
    let mut improved = Vec::new();
    for st in PawnSubTerm::ALL.iter() {
        let d = st.delta_mg(pre, post);
        if d <= -PAWN_STRUCTURE_DELTA_THRESHOLD_CP {
            worsened.push(st.worsened_phrase());
        } else if d >= PAWN_STRUCTURE_DELTA_THRESHOLD_CP {
            improved.push(st.improved_phrase());
        }
    }
    (worsened, improved)
}

pub(super) fn build_pawn_structure_item(outcome: &PawnStructureOutcome) -> Option<RetrospectiveItem> {
    let (ours_worsened, ours_improved) = pawn_clauses(&outcome.ours_pre, &outcome.ours_post);
    let (theirs_worsened, theirs_improved) =
        pawn_clauses(&outcome.theirs_pre, &outcome.theirs_post);

    if ours_worsened.is_empty()
        && ours_improved.is_empty()
        && theirs_worsened.is_empty()
        && theirs_improved.is_empty()
    {
        return None;
    }

    // Sentiment: worsened on our side hurts; worsened on theirs helps.
    let net_our = ours_improved.len() as i32 - ours_worsened.len() as i32;
    let net_their = theirs_worsened.len() as i32 - theirs_improved.len() as i32;
    let net = net_our + net_their;
    let (heading, sentiment) = if !ours_worsened.is_empty() {
        ("Your pawn structure weakened", Sentiment::Negative)
    } else if !theirs_worsened.is_empty() {
        ("Weakened their pawn structure", Sentiment::Positive)
    } else if !ours_improved.is_empty() {
        ("Your pawn structure improved", Sentiment::Positive)
    } else if net < 0 {
        ("Their pawn structure improved", Sentiment::Negative)
    } else {
        ("Pawn structure changed", Sentiment::Mixed)
    };

    let summary_clauses: &[&'static str] = if !ours_worsened.is_empty() {
        &ours_worsened
    } else if !theirs_worsened.is_empty() {
        &theirs_worsened
    } else if !ours_improved.is_empty() {
        &ours_improved
    } else {
        &theirs_improved
    };
    let summary = summary_clauses.join(", ");

    let mut detail_lines = Vec::new();
    if !ours_worsened.is_empty() {
        detail_lines.push(format!("You: {}.", ours_worsened.join(", ")));
    }
    if !ours_improved.is_empty() {
        detail_lines.push(format!("You: {}.", ours_improved.join(", ")));
    }
    if !theirs_worsened.is_empty() {
        detail_lines.push(format!("Opponent: {}.", theirs_worsened.join(", ")));
    }
    if !theirs_improved.is_empty() {
        detail_lines.push(format!("Opponent: {}.", theirs_improved.join(", ")));
    }

    Some(RetrospectiveItem {
        category: RetrospectiveCategory::PawnStructure,
        heading: heading.to_string(),
        summary,
        detail: detail_lines.join("\n"),
        score_delta_pawns: None,
        sentiment,
        annotations: Vec::new(),
    })
}

