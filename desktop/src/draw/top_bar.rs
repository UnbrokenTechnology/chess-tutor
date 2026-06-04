use eframe::egui;

use chess_tutor_ui::event::Event;
use chess_tutor_ui::view::TopBarView;

/// Slim title bar: app title on the left, then (right-aligned) the
/// ⚙ settings and flip icon-buttons and a "Live" return-to-current
/// button while browsing history. The primary play actions — including
/// Review once the game ends — live in the bottom action bar. (Bot
/// search depth used to live here as a tuner; it's an opponent-strength
/// lever now, set per game on the Start screen.)
pub(crate) fn draw(ui: &mut egui::Ui, view: &TopBarView, events: &mut Vec<Event>) {
    ui.horizontal(|ui| {
        ui.add_space(2.0);
        ui.label(egui::RichText::new("Chess Tutor").strong().size(20.0));

        // Status slot sits next to the title so the eye finds it
        // without competing with the right-aligned icon cluster.
        ui.add_space(12.0);
        if view.engine_thinking {
            ui.spinner();
            ui.label("engine thinking…");
        } else if let Some(end) = view.game_outcome {
            ui.colored_label(crate::draw::theme::OUTCOME, end);
        }

        // Right-aligned cluster: settings gear + flip, plus the
        // interim Review / Live and depth controls.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let gear = egui::Button::new(
                crate::draw::icon::icon(egui_phosphor::regular::GEAR).size(18.0),
            );
            if ui.add(gear).on_hover_text("Settings").clicked() {
                events.push(Event::OpenSettings);
            }
            let flip = egui::Button::new(
                crate::draw::icon::icon(egui_phosphor::regular::ARROWS_DOWN_UP).size(18.0),
            );
            if ui.add(flip).on_hover_text("Flip board").clicked() {
                events.push(Event::FlipBoard);
            }

            ui.separator();

            // "Live" returns to the current position when browsing
            // history. Game review now lives on the action bar (the Hint
            // button becomes Review once the game is over).
            if !view.viewing_live && ui.button("▶ Live").clicked() {
                events.push(Event::JumpToLive);
            }
        });
    });
}
