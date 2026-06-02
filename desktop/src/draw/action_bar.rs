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
        if ui
            .add_enabled(
                view.can_takeback,
                egui::Button::new(big_label("\u{27f2}", "Takeback")).min_size(size),
            )
            .clicked()
        {
            events.push(Event::Takeback);
        }

        let hint_text = if view.hint_open { "Hide Hint" } else { "Hint" };
        if ui
            .add_enabled(
                view.hint_button_enabled,
                egui::Button::new(big_label("\u{1f4a1}", hint_text)).min_size(size),
            )
            .clicked()
        {
            events.push(Event::ToggleHint);
        }

        if ui
            .add(egui::Button::new(big_label("\u{271a}", "New Game")).min_size(size))
            .clicked()
        {
            events.push(Event::RequestNewGame);
        }
    });
}

/// Glyph + label stacked into one larger, legible button face.
fn big_label(glyph: &str, text: &str) -> egui::RichText {
    egui::RichText::new(format!("{glyph}  {text}")).size(16.0).strong()
}
