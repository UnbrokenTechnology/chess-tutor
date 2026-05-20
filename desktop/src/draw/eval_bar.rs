use eframe::egui;

use crate::view::EvalBarView;

pub(crate) fn draw(ui: &mut egui::Ui, view: &EvalBarView) {
    ui.add_space(8.0);
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width() - 8.0, ui.available_height() - 32.0),
        egui::Sense::hover(),
    );
    let painter = ui.painter_at(rect);

    let white_color = egui::Color32::from_rgb(0xf0, 0xf0, 0xf0);
    let black_color = egui::Color32::from_rgb(0x30, 0x30, 0x30);
    let border = egui::Color32::from_rgb(0x80, 0x80, 0x80);

    let split_y = rect.max.y - rect.height() * view.white_ratio;
    let top_rect = egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, split_y));
    let bot_rect = egui::Rect::from_min_max(egui::pos2(rect.min.x, split_y), rect.max);
    painter.rect_filled(top_rect, 0.0, black_color);
    painter.rect_filled(bot_rect, 0.0, white_color);
    painter.rect_stroke(rect, 0.0, egui::Stroke::new(1.0, border));

    ui.add_space(4.0);
    ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
        ui.monospace(&view.label);
    });
}
