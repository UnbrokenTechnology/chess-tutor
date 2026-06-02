use eframe::egui;

use chess_tutor_ui::event::Event;
use chess_tutor_ui::view::TopBarView;

/// Slim title bar: app title on the left, then (right-aligned) the
/// ⚙ settings and flip icon-buttons, a "Live" return-to-current button
/// while browsing history, and the depth tuner (parked here until it
/// moves fully into the Options/⚙ surface). The primary play actions —
/// including Review once the game ends — live in the bottom action bar.
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

        // Right-aligned cluster: ⚙ settings + ⤢ flip, plus the
        // interim Review / Live and depth controls.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let gear = egui::Button::new(egui::RichText::new("\u{2699}").size(18.0));
            if ui.add(gear).on_hover_text("Settings").clicked() {
                events.push(Event::OpenSettings);
            }
            let flip = egui::Button::new(egui::RichText::new("\u{21c5}").size(18.0));
            if ui.add(flip).on_hover_text("Flip board").clicked() {
                events.push(Event::FlipBoard);
            }

            ui.separator();

            // --- Interim controls (relocated in later redesign steps) ---
            // Depth: minimal tuner; its true home is the Options/⚙ surface.
            let mut depth = view.depth;
            if ui
                .add(egui::DragValue::new(&mut depth).range(1..=20))
                .on_hover_text("Search depth (moves to Options later)")
                .changed()
            {
                events.push(Event::ChangeDepth(depth));
            }
            ui.label("Depth:");

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
