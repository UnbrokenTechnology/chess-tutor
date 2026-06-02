//! The mid-game ⚙ settings surface (PLAN build-order step 5,
//! decision #2): a floating window that edits the same play options as
//! the pre-game Start screen — Eval Bar, Support, Auto-coach,
//! Reveal-best-move, Move-feedback depth, search Depth, and the board
//! overlays — but against the *live* session, so no new game is needed.
//!
//! Opponent strength is intentionally absent: changing the bot mid-game
//! needs a fresh game, so that lives only on the Start screen.
//! Engine-PV is absent everywhere (review-only, decision #9).
//!
//! Unlike the Start screen (which mutates a form committed on Play),
//! every control here emits its own intent immediately so the change
//! takes effect this turn.

use eframe::egui;

use chess_tutor_ui::event::Event;
use chess_tutor_ui::view::SettingsView;

use super::options;

pub(crate) fn draw(ctx: &egui::Context, view: &SettingsView, events: &mut Vec<Event>) {
    let mut open = true;
    egui::Window::new("\u{2699}  Settings")
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .default_width(420.0)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().max_height(560.0).show(ui, |ui| {
                ui.add_space(4.0);

                // ---- Eval bar ----
                let mut show_eval_bar = view.show_eval_bar;
                if options::toggle_row(
                    ui,
                    &mut show_eval_bar,
                    "Eval bar",
                    "Show the chess.com-style evaluation bar in the left gutter.",
                ) {
                    events.push(Event::SetEvalBarVisible(show_eval_bar));
                }

                // ---- Learning toggles ----
                let mut learning = view.learning;
                let change = options::learning_toggles(ui, &mut learning);
                if let Some(on) = change.support {
                    events.push(Event::SetSupport(on));
                }
                if let Some(on) = change.auto_coach {
                    events.push(Event::SetAutoCoach(on));
                }
                if let Some(on) = change.reveal_best_moves {
                    events.push(Event::SetRevealBestMoves(on));
                }

                ui.add_space(4.0);

                // ---- Depths ----
                let mut depth = view.depth;
                if options::depth_row(
                    ui,
                    &mut depth,
                    "Search depth",
                    "How deeply the bot searches when choosing its move.",
                ) {
                    events.push(Event::ChangeDepth(depth));
                }
                let mut retro_depth = view.retrospective_depth;
                if options::depth_row(
                    ui,
                    &mut retro_depth,
                    "Move-feedback depth",
                    "How deeply each of your moves is analysed for the after-move \
                     feedback.",
                ) {
                    events.push(Event::SetRetrospectiveDepth(retro_depth));
                }

                ui.add_space(8.0);
                egui::CollapsingHeader::new("Board overlays")
                    .default_open(false)
                    .show(ui, |ui| {
                        let mut active = view.active_overlays.clone();
                        if let Some(kind) = options::overlay_toggles(ui, &mut active) {
                            events.push(Event::ToggleOverlay(kind));
                        }
                    });

                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(
                        "Opponent strength is set per game — start a new game to \
                         change the bot.",
                    )
                    .small()
                    .weak(),
                );
                ui.add_space(8.0);
            });

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Done").clicked() {
                    events.push(Event::CloseSettings);
                }
            });
        });

    // The window's own [x] close button (or Esc) also closes settings.
    if !open {
        events.push(Event::CloseSettings);
    }
}
