use chess_tutor_engine::types::Value;
use eframe::egui;

use crate::session::App;

const EVAL_BAR_SATURATION_CP: f32 = 1000.0;

impl App {
    pub(crate) fn draw_eval_bar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width() - 8.0, ui.available_height() - 32.0),
            egui::Sense::hover(),
        );
        let painter = ui.painter_at(rect);

        let white_color = egui::Color32::from_rgb(0xf0, 0xf0, 0xf0);
        let black_color = egui::Color32::from_rgb(0x30, 0x30, 0x30);
        let border = egui::Color32::from_rgb(0x80, 0x80, 0x80);

        let score = self.viewed_engine_info().map(|i| i.score_white_pov);
        let (white_ratio, label) = match score {
            Some(v) if v.abs() >= Value::MATE_IN_MAX_PLY => {
                if v.0 > 0 {
                    (1.0, format!("M{}", (Value::MATE.0 - v.0).max(1)))
                } else {
                    (0.0, format!("-M{}", (Value::MATE.0 + v.0).max(1)))
                }
            }
            Some(v) => {
                let ratio = (v.0 as f32 / EVAL_BAR_SATURATION_CP).clamp(-1.0, 1.0);
                let pawns = v.0 as f32 / Value::PAWN_MG.0 as f32;
                (0.5 + 0.5 * ratio, format!("{:+.2}", pawns))
            }
            None => (0.5, String::from("—")),
        };

        let split_y = rect.max.y - rect.height() * white_ratio;
        let top_rect = egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, split_y));
        let bot_rect = egui::Rect::from_min_max(egui::pos2(rect.min.x, split_y), rect.max);
        painter.rect_filled(top_rect, 0.0, black_color);
        painter.rect_filled(bot_rect, 0.0, white_color);
        painter.rect_stroke(rect, 0.0, egui::Stroke::new(1.0, border));

        ui.add_space(4.0);
        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
            ui.monospace(label);
        });
    }
}
