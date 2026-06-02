//! Shared Options widgets used by both the pre-game Start screen
//! (`draw::dialog`) and the mid-game ⚙ gear (`draw::settings`).
//!
//! These are layout/wiring helpers only (per the redesign's "layout
//! first, colours later" scope). The two surfaces differ in how they
//! source/sink state — the Start screen mutates a `NewGameForm` in
//! place, the gear emits per-option intents against the live session —
//! so the shared pieces here are the renderer-neutral *labels* and the
//! row layout, not the state plumbing. Each surface wires its own
//! booleans/values into these helpers.

use eframe::egui;

use chess_tutor_ui::view::OverlayKind;
use chess_tutor_ui::LearningPreferences;

/// A labelled on/off toggle row with a hover description. Returns the
/// (possibly mutated) value so callers can detect a change by comparing
/// or by `if changed`. Renders as a checkbox so the state reads at a
/// glance.
pub(crate) fn toggle_row(
    ui: &mut egui::Ui,
    value: &mut bool,
    label: &str,
    hover: &str,
) -> bool {
    ui.checkbox(value, egui::RichText::new(label).size(15.0))
        .on_hover_text(hover)
        .changed()
}

/// A labelled integer slider row (depth-style). Returns true on change.
pub(crate) fn depth_row(
    ui: &mut egui::Ui,
    value: &mut u32,
    label: &str,
    hover: &str,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).size(15.0))
            .on_hover_text(hover);
        changed = ui
            .add(egui::Slider::new(value, 1..=20))
            .on_hover_text(hover)
            .changed();
    });
    changed
}

/// The three learning toggles common to both surfaces: **Support**
/// (intervention pause), **Auto-coach**, and **Reveal best moves**.
/// Mutates the passed-in `LearningPreferences` in place; returns a
/// [`LearningChange`] naming which axis the user just flipped (if any)
/// so the gear can emit the matching intent. The Start screen ignores
/// the return and just reads the mutated bundle on Play.
#[derive(Default)]
pub(crate) struct LearningChange {
    pub support: Option<bool>,
    pub auto_coach: Option<bool>,
    pub reveal_best_moves: Option<bool>,
}

pub(crate) fn learning_toggles(
    ui: &mut egui::Ui,
    learning: &mut LearningPreferences,
) -> LearningChange {
    let mut change = LearningChange::default();

    let mut support = learning.support_enabled();
    if toggle_row(
        ui,
        &mut support,
        "Support — pause on a mistake",
        "Stop the game when your move trips a teaching moment or hangs \
         material, with a take-back offer. Off plays silently and saves \
         everything for the after-move feedback.",
    ) {
        learning.set_support(support);
        change.support = Some(support);
    }

    let mut auto_coach = learning.auto_coach;
    if toggle_row(
        ui,
        &mut auto_coach,
        "Auto-coach — open Hint each move",
        "Automatically pop the \"what to notice\" Hint each of your turns. \
         It names patterns and squares, never the move.",
    ) {
        learning.auto_coach = auto_coach;
        change.auto_coach = Some(auto_coach);
    }

    let mut reveal = learning.reveal_best_moves;
    if toggle_row(
        ui,
        &mut reveal,
        "Reveal best move after the fact",
        "After you commit a move, show the engine's preferred move in the \
         after-move feedback (text + board arrow). Off keeps the focus on \
         why your move fell short, not what to memorise.",
    ) {
        learning.reveal_best_moves = reveal;
        change.reveal_best_moves = Some(reveal);
    }

    change
}

/// The board-overlay checkbox block, collapsed under a heading. Mutates
/// the passed `active` set in place; returns the [`OverlayKind`] the
/// user just toggled (if any) so the gear can emit `ToggleOverlay`. The
/// Start screen ignores the return and reads the mutated set on Play.
pub(crate) fn overlay_toggles(
    ui: &mut egui::Ui,
    active: &mut std::collections::HashSet<OverlayKind>,
) -> Option<OverlayKind> {
    let mut toggled = None;
    for kind in OverlayKind::ALL {
        let mut on = active.contains(&kind);
        if ui
            .checkbox(&mut on, kind.label())
            .on_hover_text(kind.description())
            .changed()
        {
            if on {
                active.insert(kind);
            } else {
                active.remove(&kind);
            }
            toggled = Some(kind);
        }
    }
    toggled
}
