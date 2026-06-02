use eframe::egui;

use chess_tutor_ui::view::EvalBarView;

/// Paint the eval bar into `rect` — a fixed-width gutter the caller sizes
/// and positions flush against the board's left edge. The numeric label
/// renders *inside* the bar (chess.com idiom), so no separate number row
/// is reserved.
pub(crate) fn draw(ui: &mut egui::Ui, rect: egui::Rect, view: &EvalBarView) {
    let painter = ui.painter_at(rect);

    let white_color = crate::draw::theme::EVAL_WHITE;
    let black_color = crate::draw::theme::EVAL_BLACK;
    let border = crate::draw::theme::EVAL_BORDER;

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
            crate::draw::theme::EVAL_TEXT_ON_LIGHT,
        )
    } else {
        (
            egui::pos2(rect.center().x, rect.min.y + 10.0),
            crate::draw::theme::EVAL_TEXT_ON_DARK,
        )
    };
    painter.text(
        anchor,
        egui::Align2::CENTER_CENTER,
        &view.label,
        // 9pt fits a 5-char score (e.g. "-1.31") inside the ~30px-wide bar.
        egui::FontId::monospace(9.0),
        text_color,
    );
}
