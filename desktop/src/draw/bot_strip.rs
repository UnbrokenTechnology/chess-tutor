//! The opponent strip drawn above the board (chess.com idiom). Paints
//! the bot's name + strength, the active handicaps, and a
//! captured-material diff from a [`BotStripView`]. No user strip on the
//! opposite side (decision #3).

use eframe::egui;

use chess_tutor_ui::view::{BotHandicap, BotStripView};

pub(crate) fn draw(ui: &mut egui::Ui, view: &BotStripView) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(&view.name).size(15.0).strong());
        ui.weak("·");
        ui.label(egui::RichText::new(&view.strength_label).size(13.0).weak());

        for handicap in &view.handicaps {
            ui.weak("·");
            ui.label(egui::RichText::new(handicap_label(*handicap)).size(13.0));
        }

        // Captured-material diff pushed to the right edge. In a
        // right-to-left layout the `+N` lead is added first (so it sits
        // rightmost), then the overlapped cluster to its left; the
        // cluster paints its sprites heaviest-first left-to-right inside
        // its allocated rect.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if view.point_advantage > 0 {
                ui.label(
                    egui::RichText::new(format!("+{}", view.point_advantage))
                        .size(14.0)
                        .strong(),
                );
            }
            crate::draw::captured::cluster(ui, &view.captured);
        });
    });
    ui.add_space(2.0);
}

/// Format one handicap chip. Wording lives renderer-side (the view
/// carries only structured magnitudes) — these are status labels, not
/// teaching prose.
fn handicap_label(handicap: BotHandicap) -> String {
    match handicap {
        BotHandicap::BlunderChance(p) => format!("blunder {}%", (p * 100.0).round() as i32),
        BotHandicap::MissChance(p) => format!("miss {}%", (p * 100.0).round() as i32),
        BotHandicap::Variety(rank) => format!("variety {rank:.1}"),
        BotHandicap::EvalMask(n) => {
            if n == 1 {
                "mask 1 term".to_string()
            } else {
                format!("mask {n} terms")
            }
        }
    }
}

