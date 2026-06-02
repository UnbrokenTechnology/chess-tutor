use eframe::egui;

use chess_tutor_ui::view::EvalBarView;

pub(crate) fn draw(ui: &mut egui::Ui, view: &EvalBarView) {
    ui.add_space(8.0);
    // The bar now fills the full available height — the numeric label
    // renders *inside* it (chess.com idiom) rather than below, so no
    // height is reserved for a separate number row.
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width() - 8.0, ui.available_height() - 8.0),
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

    // Number renders inside the bar, pinned to the end held by whoever
    // is ahead (chess.com idiom): a white-favoured score sits at the
    // bottom (white band), a black-favoured score at the top (black
    // band). The label is drawn in the contrasting colour to the band
    // it sits on so it stays legible.
    let white_ahead = view.white_ratio >= 0.5;
    let (anchor, text_color) = if white_ahead {
        (
            egui::pos2(rect.center().x, rect.max.y - 10.0),
            egui::Color32::from_rgb(0x20, 0x20, 0x20),
        )
    } else {
        (
            egui::pos2(rect.center().x, rect.min.y + 10.0),
            egui::Color32::from_rgb(0xf0, 0xf0, 0xf0),
        )
    };
    painter.text(
        anchor,
        egui::Align2::CENTER_CENTER,
        &view.label,
        egui::FontId::monospace(13.0),
        text_color,
    );
}
