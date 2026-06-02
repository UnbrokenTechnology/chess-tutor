//! [`Session::dispatch`] — the semantic UI-intent event handler — and
//! its cancel handling.

use super::*;

use chess_tutor_engine::opponent::{EvalMask, NoiseProfile};

use crate::event::Event;
use crate::learning_mode::LearningPreset;

impl Session {
    /// Apply a renderer-emitted intent. Centralising this here keeps
    /// the renderers stateless about *what* an interaction means — the
    /// session resolves all priority rules (cancel ordering, snap-to-
    /// live mapping, etc.).
    pub fn dispatch(&mut self, event: Event) {
        match event {
            Event::SelectSquare(sq) => self.handle_click(sq),
            Event::ConfirmPromotion(mv) => {
                self.pending_promotion = None;
                self.apply_user_move(mv);
                self.maybe_queue_engine_search();
            }
            Event::RequestNewGame => self.open_new_game_dialog(),
            Event::Takeback => self.takeback(),
            Event::FlipBoard => self.flipped = !self.flipped,
            Event::ToggleHint => self.toggle_hint(),
            Event::OpenSettings => self.settings_open = true,
            Event::CloseSettings => self.settings_open = false,
            Event::JumpToLive => self.viewing_index = None,
            Event::ChangeDepth(d) => self.depth = d,
            Event::SetRetrospectiveDepth(d) => self.retrospective_depth = d,
            Event::SetEvalBarVisible(on) => self.show_eval_bar = on,
            Event::SetSupport(on) => self.learning.set_support(on),
            Event::SetAutoCoach(on) => self.learning.auto_coach = on,
            Event::ViewHistoryIndex(target) => {
                // Clicking the last move in the list means "back to
                // live", not "freeze on the live-equivalent index" —
                // otherwise the user can't distinguish viewing-live
                // from viewing-at-history-end.
                self.viewing_index = match target {
                    Some(i) if i + 1 == self.history.len() => None,
                    other => other,
                };
                // Clear retrospective selection when navigating to
                // a different move — annotations belong to the move
                // they describe, not whatever the user clicks next.
                self.selected_retrospective = None;
            }
            Event::SelectRetrospectiveItem(item_idx) => {
                let Some((entry_idx, _)) = self.panel_entry_with_index() else {
                    return;
                };
                // Toggle: clicking the selected card again deselects.
                self.selected_retrospective =
                    match self.selected_retrospective {
                        Some((h, i)) if h == entry_idx && i == item_idx => None,
                        _ => Some((entry_idx, item_idx)),
                    };
            }
            Event::ToggleRetrospectiveDetail => {
                self.retro_expanded = !self.retro_expanded;
            }
            Event::ToggleShowAllSignals => {
                self.show_all_signals = !self.show_all_signals;
            }
            Event::ToggleOverlay(kind) => {
                if !self.active_overlays.remove(&kind) {
                    self.active_overlays.insert(kind);
                }
            }
            Event::Cancel => self.handle_cancel(),
            Event::ConfirmNewGame => self.try_start_from_form(),
            Event::ResetBotForm => {
                if let Some(f) = self.new_game_form.as_mut() {
                    f.noise = NoiseProfile::default();
                    f.eval_mask = EvalMask::EMPTY;
                }
            }
            Event::ApplyLearningPreset(preset) => {
                // Custom is a no-op when set externally; it just means
                // "the bundle was custom-tuned, don't touch it."
                if !matches!(preset, LearningPreset::Custom) {
                    self.learning = preset.to_preferences();
                }
            }
            Event::SetRevealBestMoves(on) => {
                self.learning.reveal_best_moves = on;
            }
            Event::ContinueDespitePrompt => {
                self.pending_intervention = None;
                self.maybe_queue_engine_search();
            }
            Event::RevealMissedConcept => {
                if let Some(p) = self.pending_intervention.as_mut() {
                    p.concept_revealed = true;
                }
            }
            Event::TakeBackDuringIntervention => {
                self.pending_intervention = None;
                self.awaiting_intervention_decision = false;
                self.takeback();
            }
            Event::OpenGameReview => {
                // Only meaningful when there's at least one user move;
                // for an empty history just leave the regular surface up.
                if self.history.iter().any(|e| self.is_user_move(e)) {
                    self.game_review_open = true;
                    self.close_hint();
                }
            }
            Event::CloseGameReview => {
                self.game_review_open = false;
            }
            Event::JumpToReviewMoment(history_index) => {
                if history_index < self.history.len() {
                    self.viewing_index = Some(history_index);
                    self.selected_retrospective = None;
                    self.game_review_open = false;
                }
            }
        }
    }

    /// Resolve [`Event::Cancel`]: promotion picker > open dialog >
    /// deselect. First-launch dialog is non-cancellable (no game to
    /// fall back to), so it's skipped in the dialog branch.
    pub(crate) fn handle_cancel(&mut self) {
        if self.pending_promotion.is_some() {
            // deselect() clears pending + selection together.
            self.deselect();
            return;
        }
        if self.new_game_form.is_some() && !self.first_launch {
            self.new_game_form = None;
            return;
        }
        self.deselect();
    }
}
