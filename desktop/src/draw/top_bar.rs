use eframe::egui;

use crate::session::App;

impl App {
    pub(crate) fn draw_top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("New Game").clicked() {
                self.open_new_game_dialog();
            }
            let can_takeback = !self.history.is_empty();
            if ui
                .add_enabled(can_takeback, egui::Button::new("Takeback"))
                .clicked()
            {
                self.takeback();
            }
            if ui.button("Flip Board").clicked() {
                self.flipped = !self.flipped;
            }
            // Hint is only meaningful while at the live position and
            // it's the user's turn to choose a move. Block the button
            // outside those conditions.
            let hint_enabled = self.is_viewing_live()
                && !self.engine_thinking
                && self.is_users_turn()
                && self.game_outcome().is_none();
            let hint_label = if self.hint_open { "Hide Hint" } else { "Hint" };
            if ui
                .add_enabled(hint_enabled || self.hint_open, egui::Button::new(hint_label))
                .clicked()
            {
                self.toggle_hint();
            }
            if !self.is_viewing_live() && ui.button("▶ Live").clicked() {
                self.viewing_index = None;
            }
            ui.separator();
            ui.label("Depth:");
            ui.add(egui::DragValue::new(&mut self.depth).range(1..=20));
            ui.separator();
            if self.engine_thinking {
                ui.spinner();
                ui.label("engine thinking…");
            } else if let Some(end) = self.game_outcome() {
                ui.colored_label(egui::Color32::from_rgb(0xb8, 0x55, 0x00), end);
            }
        });
    }
}
