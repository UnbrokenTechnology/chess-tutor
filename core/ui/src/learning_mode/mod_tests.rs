use super::*;
use chess_tutor_engine::analysis::{
    AllowedInfo, Confidence, MoveAssessment, TacticHit, TacticPattern,
};
use chess_tutor_engine::types::Square;

/// A representative ALLOWED-not-MISSED assessment: the user gave away a
/// winning position to a discovered attack the move failed to address.
/// Mirrors the shape `classify_user_move` produces on the `Qc5+` case.
fn allowed_assessment() -> MoveAssessment {
    let walked_into = TacticHit {
        pattern: TacticPattern::DiscoveredAttack,
        pv_ply: 1,
        primary_piece: Square::E6,
        targets: vec![Square::E1],
        material_gain: Some(500),
        confidence: Confidence::Medium,
        sacrifice: false,
        mate_pattern: None,
        key_move: None,
    };
    MoveAssessment {
        blunder: None,
        teaching: None,
        allowed: Some(AllowedInfo {
            walked_into,
            conceded_cp: 330,
        }),
    }
}

#[test]
fn intervention_required_fires_on_allowed_under_teaching_moments() {
    let assessment = allowed_assessment();
    let prefs = LearningPreset::Supported.to_preferences();
    // Supported = TeachingMoments handling, so the ALLOWED dimension
    // (which shares the teaching gate) must pause.
    assert!(intervention_required(&assessment, &prefs));
}

#[test]
fn intervention_not_required_for_allowed_when_silent_retrospective() {
    let assessment = allowed_assessment();
    let prefs = LearningPreset::Practicing.to_preferences();
    // Practicing = SilentRetrospective: no pause, even on ALLOWED.
    assert!(!intervention_required(&assessment, &prefs));
}

#[test]
fn allowed_panel_uses_what_did_you_let_them_do_framing() {
    let pending = PendingIntervention {
        at_history_index: 0,
        original_move: chess_tutor_engine::types::Move::normal(Square::C4, Square::C5),
        assessment: allowed_assessment(),
        concept_revealed: false,
    };
    let panel = build_intervention_panel(&pending);
    // The headline asks what the move *allowed*, not what was *missed*.
    assert!(
        panel.headline.to_lowercase().contains("let your opponent"),
        "ALLOWED headline must use the let-your-opponent framing, got: {}",
        panel.headline
    );
    assert!(
        !panel.headline.to_lowercase().contains("better move"),
        "ALLOWED prompt must not lead with a missed-move framing",
    );
    // Concept is withheld until the student asks (RevealConcept action
    // is offered while not yet revealed).
    assert!(panel.concept.is_none());
    assert!(panel
        .actions
        .iter()
        .any(|a| matches!(a, crate::view::InterventionAction::RevealConcept)));
}

#[test]
fn allowed_panel_concept_names_the_pattern_not_a_move() {
    let mut pending = PendingIntervention {
        at_history_index: 0,
        original_move: chess_tutor_engine::types::Move::normal(Square::C4, Square::C5),
        assessment: allowed_assessment(),
        concept_revealed: true,
    };
    pending.concept_revealed = true;
    let panel = build_intervention_panel(&pending);
    let concept = panel.concept.expect("revealed concept present");
    // Names the pattern (a discovered attack) but never an engine move
    // (preserving the find-it-yourself principle).
    assert!(
        concept.to_lowercase().contains("discovered attack"),
        "concept must name the allowed pattern, got: {concept}"
    );
}

#[test]
fn blunder_takes_priority_over_allowed_in_panel() {
    use chess_tutor_engine::analysis::BlunderInfo;
    let mut assessment = allowed_assessment();
    assessment.blunder = Some(BlunderInfo {
        material_loss_cp: 900,
        lost_piece_square: Some(Square::E1),
    });
    let pending = PendingIntervention {
        at_history_index: 0,
        original_move: chess_tutor_engine::types::Move::normal(Square::C4, Square::C5),
        assessment,
        concept_revealed: false,
    };
    let panel = build_intervention_panel(&pending);
    // When both fire, the in-game prompt is the material-loss takeback
    // (BlunderSafety), per the documented priority.
    assert!(matches!(
        panel.kind,
        crate::view::InterventionPanelKind::BlunderSafety
    ));
}
