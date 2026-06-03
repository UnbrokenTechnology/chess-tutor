//! The big bottom-of-the-right-column action bar (chess.com idiom):
//! Takeback / Hint / New Game. Sized large and obvious — these are the
//! primary play controls, relocated out of the old cramped top bar.

use eframe::egui;

use chess_tutor_ui::event::Event;
use chess_tutor_ui::view::ActionBarView;

/// Height of each big action button. Sizing (not colour) is in scope
/// for this step — chess.com-style legibility.
const BUTTON_HEIGHT: f32 = 44.0;

pub(crate) fn draw(ui: &mut egui::Ui, view: &ActionBarView, events: &mut Vec<Event>) {
    // Three equal-width buttons spanning the column width.
    let spacing = ui.spacing().item_spacing.x;
    let total_w = ui.available_width();
    let button_w = ((total_w - spacing * 2.0) / 3.0).max(0.0);
    let size = egui::vec2(button_w, BUTTON_HEIGHT);

    ui.horizontal(|ui| {
        // Left slot: Takeback during play; while reviewing, Takeback is
        // meaningless (the game is over), so this becomes the Summary
        // button that opens the review-summary popover.
        if view.review_open {
            if ui
                .add(egui::Button::new(big_label("Summary")).min_size(size))
                .clicked()
            {
                events.push(Event::OpenGameReview);
            }
        } else if ui
            .add_enabled(
                view.can_takeback,
                egui::Button::new(big_label("Takeback")).min_size(size),
            )
            .clicked()
        {
            events.push(Event::Takeback);
        }

        // Middle button: Hint during play, Review once the game is over
        // (a hint is useless with no move to make; you don't review
        // mid-game). The Review form is itself a toggle.
        if view.game_over {
            let label = if view.review_open { "Close Review" } else { "Review" };
            if ui
                .add_enabled(
                    view.review_button_enabled || view.review_open,
                    egui::Button::new(big_label(label)).min_size(size),
                )
                .clicked()
            {
                events.push(if view.review_open {
                    Event::CloseGameReview
                } else {
                    // "Review" enters step-through review immediately (no
                    // summary gate); the summary is on-demand via the
                    // left "Summary" button (OpenGameReview).
                    Event::StartReview
                });
            }
        } else {
            let hint_text = if view.hint_open { "Hide Hint" } else { "Hint" };
            if ui
                .add_enabled(
                    view.hint_button_enabled,
                    egui::Button::new(big_label(hint_text)).min_size(size),
                )
                .clicked()
            {
                events.push(Event::ToggleHint);
            }
        }

        if ui
            .add(egui::Button::new(big_label("New Game")).min_size(size))
            .clicked()
        {
            events.push(Event::RequestNewGame);
        }
    });
}

/// Big, legible button face. Text-only for now — leading icons return in
/// the later styling pass with a bundled icon font (the emoji used before
/// rendered as tofu and crowded "New Game" off the button).
fn big_label(text: &str) -> egui::RichText {
    egui::RichText::new(text).size(15.0).strong()
}
