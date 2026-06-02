//! Learning-mode preferences — how (and whether) the session pauses on
//! mistakes, whether the engine's preferred moves are ever revealed,
//! and whether the Hint pop-over auto-opens each move.
//!
//! The axes are deliberately independent so the user can mix them:
//! "auto-coach each move + silent during mistakes + blunder safety on"
//! is just as valid as "silent everything." The [`LearningPreset`] enum
//! collapses common combinations into named presets for the
//! Start/Options UI; advanced users tune the axes directly.
//!
//! **The coaching/hint model (PLAN §"coaching/hint model").** Coaching
//! is *on-demand*, not a persistent panel-swapping mode. The student
//! presses **Hint** to pop a transient "what to notice" panel (fed by
//! [`crate::coaching_view::build_coaching_view`]); an optional
//! [`LearningPreferences::auto_coach`] toggle auto-opens it each move
//! for maximum hand-holding. The old persistent `AssistanceLevel`
//! (Off / Prophylactic / Coached) axis is **gone** — coaching no longer
//! shares the side-panel slot with the backward-looking retrospective,
//! so the two can coexist by construction (retrospective in the panel,
//! coaching in the pop-over).
//!
//! **Pedagogical principle.** The engine knows the best move; the
//! student needs to *develop* the skill of finding it. Revealing the
//! engine's choice short-circuits that practice and trains rote
//! memorisation — so every reveal is opt-in, every pause is gated to
//! genuine teaching moments (not every non-best move), the Hint
//! pop-over names patterns/squares but never the move, and the
//! retrospective never tells the student what to do, only what they
//! missed.

mod terms;
pub(crate) use terms::term_prompt_copy;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;


/// What happens after the user commits a non-best move.
///
/// The gate for what counts as "non-best enough to interrupt for"
/// lives in [`chess_tutor_engine::analysis::classify_user_move`];
/// see that module for the dominant-term / share-of-drop rules.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MistakeHandling {
    /// Silent during play; everything goes to the retrospective. The
    /// strongest evidence-aligned mode (Dvoretsky's annotated-game
    /// method): the student plays through their own decisions and
    /// reviews afterward.
    #[default]
    SilentRetrospective,
    /// Pause on detected teaching moments — moves with a dominant,
    /// teachable eval-term shift. **Not every non-best move.** The
    /// classifier filters out noise / engine subtlety so the prompt
    /// only fires when there's a concrete chess concept the user
    /// could have spotted.
    TeachingMoments,
    /// Pause on every non-best move. High crutch risk; intended for
    /// short onboarding walkthroughs, not regular play.
    AllMistakes,
}

/// Safety net for material loss — independent of any pedagogical
/// dimension.
///
/// The student already *knows* when they hang a piece; the safety net
/// saves the game's time rather than teaching anything. Off-by-default
/// could be the right call if we want students to develop their own
/// blunder-check habit; on-by-default fits a "tool that respects my
/// time" framing. The user-facing toggle lets each player choose.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BlunderSafety {
    /// No safety net — every move stands, including blunders. Forces
    /// the student to develop their own pre-commit blunder check.
    #[default]
    Off,
    /// After a blunder is committed, offer "take back" with no
    /// teaching content (the student doesn't need to be told they
    /// hung a piece). Acceptance reverts the move; declining lets
    /// the game continue.
    OfferTakeback,
}

/// Bundle of all learning-mode preferences. Stored on
/// [`crate::session::Session`]; persists across moves within a game
/// and across games for the session. (Persistence to disk is a
/// future deliverable.)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LearningPreferences {
    pub mistake_handling: MistakeHandling,
    pub blunder_safety: BlunderSafety,
    /// When `true`, the Hint pop-over auto-opens at the start of every
    /// one of the user's turns — maximum hand-holding without the
    /// student having to remember to press Hint. Off by default (the
    /// pop-over is on-demand). The auto-open itself is wired through
    /// the session; the toggle's setup UI lands with the Options
    /// screen (PLAN build-order step 5).
    pub auto_coach: bool,
    /// Whether the engine's preferred move is ever shown in the
    /// retrospective (text label and arrow on the board). Off by
    /// default to keep the retrospective focused on *why* the user's
    /// move was an inaccuracy rather than *what* they should have
    /// played — telling the student the answer trains memorisation,
    /// not understanding.
    pub reveal_best_moves: bool,
}

/// Named combinations of the three axes, surfaced as a single picker
/// in the New Game UI. "Custom" is the catch-all for users who tuned
/// the axes individually.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LearningPreset {
    /// Silent during play; retrospective only. The default for
    /// students above true beginner.
    #[default]
    Practicing,
    /// Silent during play, but pauses on detected teaching moments
    /// (with a takeback option after blunders).
    Supported,
    /// Auto-coach: the Hint pop-over ("what to notice" — features named,
    /// never moves) auto-opens each move, plus teaching pauses + blunder
    /// safety. The most-help mode short of revealing engine moves.
    Coached,
    /// Bespoke combination of axes.
    Custom,
}

/// State of an active mid-game intervention, owned by
/// [`crate::session::Session`]. While `Some`, the engine reply is
/// held until the user dismisses or takes back.
///
/// Both the blunder and teaching dimensions can be populated
/// simultaneously (one move can trip both gates). The UI typically
/// shows the blunder prompt with priority and surfaces the teaching
/// dimension in the retrospective once the user continues.
#[derive(Clone, Debug)]
pub struct PendingIntervention {
    /// Index into `Session::history` of the move that triggered this
    /// intervention. Used so a take-back doesn't try to undo the wrong
    /// move if state shifts unexpectedly.
    pub at_history_index: usize,
    /// The move the user committed.
    pub original_move: chess_tutor_engine::types::Move,
    /// Structured assessment from the engine classifier.
    pub assessment: chess_tutor_engine::analysis::MoveAssessment,
    /// `true` once the user has clicked "Show me what I missed" so
    /// the renderer expands the concept-reveal panel. Game state is
    /// untouched.
    pub concept_revealed: bool,
}

/// Build the renderer-facing [`crate::view::InterventionPanelView`]
/// from a pending intervention. Blunder takes priority — when both
/// gates fired, the in-game prompt is about the material loss and
/// the teaching dimension surfaces in the retrospective. The
/// returned panel never names the engine's preferred move.
pub fn build_intervention_panel(
    pending: &PendingIntervention,
) -> crate::view::InterventionPanelView {
    use crate::view::{InterventionAction, InterventionPanelKind, InterventionPanelView};
    if let Some(b) = pending.assessment.blunder {
        let headline = match b.lost_piece_square {
            Some(sq) => format!(
                "Your piece on {} is in danger.",
                sq.to_algebraic()
            ),
            None => "That move loses material.".to_string(),
        };
        let summary = format!(
            "About {:.1} pawns at risk.",
            (b.material_loss_cp as f32) / 100.0
        );
        return InterventionPanelView {
            kind: InterventionPanelKind::BlunderSafety,
            headline,
            summary,
            concept: None,
            actions: vec![InterventionAction::TakeBack, InterventionAction::Continue],
        };
    }
    // ALLOWED-not-MISSED (PLAN §3) takes priority over a plain
    // missed-point teaching prompt: it is the more specific, more
    // actionable framing — "your move let your opponent do something."
    // The concept reveal names the *pattern* (a discovered attack, a
    // fork) but never the engine's preferred move, preserving the
    // find-it-yourself principle.
    if let Some(allowed) = pending.assessment.allowed.as_ref() {
        let (headline, summary, concept) = allowed_prompt(allowed);
        let mut actions = vec![InterventionAction::TakeBack];
        if !pending.concept_revealed {
            actions.push(InterventionAction::RevealConcept);
        }
        actions.push(InterventionAction::Continue);
        return InterventionPanelView {
            kind: InterventionPanelKind::TeachingMoment,
            headline,
            summary,
            concept: if pending.concept_revealed {
                Some(concept)
            } else {
                None
            },
            actions,
        };
    }
    let teaching = pending
        .assessment
        .teaching
        .expect("intervention requires a blunder, allowed, or teaching dimension");
    let (headline, summary, concept) = teaching_prompt(teaching);
    let mut actions = vec![InterventionAction::TakeBack];
    if !pending.concept_revealed {
        actions.push(InterventionAction::RevealConcept);
    }
    actions.push(InterventionAction::Continue);
    crate::view::InterventionPanelView {
        kind: InterventionPanelKind::TeachingMoment,
        headline,
        summary,
        concept: if pending.concept_revealed {
            Some(concept)
        } else {
            None
        },
        actions,
    }
}

/// Build the (headline, summary, concept_reveal) triple for an
/// ALLOWED-not-MISSED prompt. The framing mirrors the retrospective's
/// ALLOWED reframe (PLAN §4.1) and the CLI's `print_allowed_banner`,
/// kept question-shaped so the student is asked *"what did I let my
/// opponent do?"* rather than told a move. The headline / summary stay
/// pattern-and-swing only; the concept reveal names the pattern (a
/// discovered attack, a fork) — never the engine's preferred move.
fn allowed_prompt(
    allowed: &chess_tutor_engine::analysis::AllowedInfo,
) -> (String, String, String) {
    let pattern = allowed.walked_into.pattern;
    let headline = "Your move let your opponent do something — what did you let them do?".to_string();
    let summary = format!(
        "You were doing well, but this move swings about {:.1} pawns the other way. \
         The question isn't \"what better move did I have?\" — it's \"what reply did I just allow?\"",
        (allowed.conceded_cp as f32) / 100.0,
    );
    // Concept reveal: name the pattern using the engine's own phrasing,
    // framed as the thing the move allowed. No squares, no engine move.
    let concept = format!(
        "Your move allowed {}. Look for the opponent's reply that fires it, \
         and ask whether a move that addressed it first was available.",
        allowed_pattern_phrase_pub(pattern),
    );
    (headline, summary, concept)
}

/// Lower-cased noun phrase for a tactic pattern, for the ALLOWED
/// prompt's "you allowed {…}" sentence. Mirrors the retrospective's
/// `pattern_phrase`; kept local so `learning_mode` has no dependency on
/// the retrospective renderer. Falls back to the engine's own
/// [`chess_tutor_engine::analysis::TacticPattern::heading`] for any
/// pattern without a bespoke phrase. `pub(crate)` so the game-review
/// headline builder can share the same wording.
pub(crate) fn allowed_pattern_phrase_pub(
    pattern: chess_tutor_engine::analysis::TacticPattern,
) -> String {
    use chess_tutor_engine::analysis::TacticPattern as P;
    let phrase = match pattern {
        P::Fork => "a fork",
        P::HangingCapture => "a free capture",
        P::RemovingDefender => "removing one of your defenders",
        P::TrappedPiece => "a trap on one of your pieces",
        P::Pin => "a pin",
        P::RelativePin => "a relative pin",
        P::Skewer => "a skewer",
        P::DiscoveredAttack => "a discovered attack",
        P::DiscoveredCheck => "a discovered check",
        P::DoubleCheck => "a double check",
        P::Deflection => "a deflection",
        P::Attraction => "an attraction",
        P::Interference => "interference",
        _ => return pattern.heading().to_string(),
    };
    phrase.to_string()
}

/// Build the (headline, summary, concept_reveal) triple for a
/// teaching-moment prompt. The headline never names the engine's
/// preferred move; the concept reveal names the *specific* concept
/// (one or two granular [`chess_tutor_engine::analysis::TermId`]s)
/// and frames it without naming squares or pieces by coordinate.
fn teaching_prompt(
    info: chess_tutor_engine::analysis::TeachingInfo,
) -> (String, String, String) {
    let (area_a, concept_a) = term_prompt_copy(info.dominant.term);
    match info.secondary {
        None => {
            let headline = format!("I noticed something about {}.", area_a);
            let summary = format!(
                "About {:.1} pawns of swing concentrated here.",
                (info.dominant.severity_cp as f32) / 100.0
            );
            (headline, summary, concept_a.to_string())
        }
        Some(secondary) => {
            let (area_b, concept_b) = term_prompt_copy(secondary.term);
            let headline = format!(
                "I noticed two things — {} and {}.",
                area_a, area_b
            );
            let combined_pawns =
                ((info.dominant.severity_cp + secondary.severity_cp) as f32) / 100.0;
            let summary = format!(
                "About {:.1} pawns of swing split between both.",
                combined_pawns
            );
            // Two separate concept paragraphs, prefixed so the student
            // can tell which is which when reading. Capitalise the
            // first letter of each area for the headers.
            let concept = format!(
                "{}: {}\n\n{}: {}",
                capitalize_first(area_a),
                concept_a,
                capitalize_first(area_b),
                concept_b,
            );
            (headline, summary, concept)
        }
    }
}

/// Uppercase the first character of `s`, leaving the rest unchanged.
/// Used to title-case the area phrase as a multi-term concept header.
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Decide whether `assessment` should pause the game given the
/// user's `prefs`. Returns `true` if any active gate fires under the
/// preferences they've set.
///
/// The ALLOWED-not-MISSED dimension shares the `MistakeHandling` gate
/// with the teaching dimension (both are "your move had a teachable
/// cost", just framed differently — see [`build_intervention_panel`]).
/// The silent-sequencing suppressor has already been applied inside
/// [`chess_tutor_engine::analysis::classify_user_move`], so an
/// `assessment` with a depth-only, detector-less verdict arrives here
/// already cleared and will not pause.
pub fn intervention_required(
    assessment: &chess_tutor_engine::analysis::MoveAssessment,
    prefs: &LearningPreferences,
) -> bool {
    let blunder_active = matches!(prefs.blunder_safety, BlunderSafety::OfferTakeback)
        && assessment.blunder.is_some();
    let teaching_gate = matches!(
        prefs.mistake_handling,
        MistakeHandling::TeachingMoments | MistakeHandling::AllMistakes
    );
    let teaching_active =
        teaching_gate && (assessment.teaching.is_some() || assessment.allowed.is_some());
    blunder_active || teaching_active
}

/// Build the [`chess_tutor_engine::analysis::GatingConfig`] that
/// matches the user's [`MistakeHandling`] preference. AllMistakes
/// loosens every gate so any non-best move with a teachable
/// component fires; TeachingMoments uses the default (strict) gates.
pub fn gating_config_for(
    handling: MistakeHandling,
) -> chess_tutor_engine::analysis::GatingConfig {
    use chess_tutor_engine::analysis::GatingConfig;
    match handling {
        MistakeHandling::AllMistakes => GatingConfig {
            noise_floor_cp: 1,
            dominant_term_share_min: 0.0,
            teaching_term_severity_min_cp: 1,
            teaching_term_severity_escape_cp: 1,
            multi_term_coverage_min: 0.0,
            ..GatingConfig::default()
        },
        MistakeHandling::TeachingMoments => GatingConfig::default(),
        // SilentRetrospective never calls the classifier; the return
        // value here is unused but we still need to satisfy the
        // function signature.
        MistakeHandling::SilentRetrospective => GatingConfig::default(),
    }
}

impl LearningPreset {
    /// Map a preset to its concrete axis settings.
    pub fn to_preferences(self) -> LearningPreferences {
        match self {
            LearningPreset::Practicing => LearningPreferences {
                mistake_handling: MistakeHandling::SilentRetrospective,
                blunder_safety: BlunderSafety::Off,
                auto_coach: false,
                reveal_best_moves: false,
            },
            LearningPreset::Supported => LearningPreferences {
                mistake_handling: MistakeHandling::TeachingMoments,
                blunder_safety: BlunderSafety::OfferTakeback,
                auto_coach: false,
                reveal_best_moves: false,
            },
            // "Coached" now means the Hint pop-over auto-opens each move
            // (on-demand coaching, maximum hand-holding) plus the
            // teaching pause + blunder safety. The old persistent
            // coaching panel is gone.
            LearningPreset::Coached => LearningPreferences {
                mistake_handling: MistakeHandling::TeachingMoments,
                blunder_safety: BlunderSafety::OfferTakeback,
                auto_coach: true,
                reveal_best_moves: false,
            },
            // Custom returns the default; callers using Custom should
            // have already populated their own preferences and ignore
            // this path.
            LearningPreset::Custom => LearningPreferences::default(),
        }
    }
}
