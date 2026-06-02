//! The player strip drawn *below* the board — the user's own captured
//! pieces and point lead, mirroring the bot strip above. No name /
//! handicaps (no vanity user profile, decision #3): just the
//! captured-material diff, left-aligned.

use eframe::egui;

use chess_tutor_ui::view::PlayerStripView;

pub(crate) fn draw(ui: &mut egui::Ui, view: &PlayerStripView) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        crate::draw::captured::cluster(ui, &view.captured);
        if view.point_advantage > 0 {
            ui.label(
                egui::RichText::new(format!("+{}", view.point_advantage))
                    .size(14.0)
                    .strong(),
            );
        }
    });
    ui.add_space(2.0);
}
