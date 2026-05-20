use chess_tutor_engine::types::{Color, Piece, PieceType};
use eframe::egui;

use crate::event::Event;
use crate::view::{BoardView, MoveDotKind};

pub(crate) fn draw(ui: &mut egui::Ui, view: &BoardView, events: &mut Vec<Event>) {
    let avail = ui.available_size();
    let board_size = avail.x.min(avail.y);
    let cell = board_size / 8.0;
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(board_size, board_size), egui::Sense::click());

    // ESC -> Cancel; session resolves priority (promotion > dialog >
    // deselect). Previously this only fired with a pending promotion,
    // but emitting unconditionally is harmless: handle_cancel is a
    // no-op when nothing's selected and no dialog is open.
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        events.push(Event::Cancel);
    }

    let clicked_local = response.clicked().then(|| {
        response
            .interact_pointer_pos()
            .map(|p| p - rect.min)
    }).flatten();
    let clicked_rc = clicked_local.and_then(|local| {
        let col = (local.x / cell).floor() as i32;
        let row = (local.y / cell).floor() as i32;
        if (0..8).contains(&col) && (0..8).contains(&row) {
            Some((col as usize, row as usize))
        } else {
            None
        }
    });

    let painter = ui.painter_at(rect);

    let light = egui::Color32::from_rgb(0xf0, 0xd9, 0xb5);
    let dark = egui::Color32::from_rgb(0xb5, 0x88, 0x63);
    let last_move_tint = egui::Color32::from_rgba_unmultiplied(0xff, 0xeb, 0x3b, 0x66);
    let selected_tint = egui::Color32::from_rgba_unmultiplied(0xff, 0xb3, 0x00, 0xaa);
    let check_tint = egui::Color32::from_rgba_unmultiplied(0xff, 0x40, 0x40, 0xaa);
    let dot_color = egui::Color32::from_rgba_unmultiplied(0x10, 0x10, 0x10, 0x66);

    for (display_row, row_cells) in view.rows.iter().enumerate() {
        for (display_col, cell_view) in row_cells.iter().enumerate() {
            let top_left = rect.min
                + egui::vec2(display_col as f32 * cell, display_row as f32 * cell);
            let cell_rect = egui::Rect::from_min_size(top_left, egui::vec2(cell, cell));
            let square_color = if cell_view.is_light { light } else { dark };
            painter.rect_filled(cell_rect, 0.0, square_color);
            if cell_view.last_move {
                painter.rect_filled(cell_rect, 0.0, last_move_tint);
            }
            if cell_view.selected {
                painter.rect_filled(cell_rect, 0.0, selected_tint);
            }
            if cell_view.check_tint {
                painter.rect_filled(cell_rect, 0.0, check_tint);
            }
            if let Some(piece) = cell_view.piece {
                painter.text(
                    cell_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    piece_glyph(piece),
                    egui::FontId::proportional(cell * 0.7),
                    egui::Color32::BLACK,
                );
            }
            match cell_view.move_dot {
                Some(MoveDotKind::Capture) => {
                    painter.circle_stroke(
                        cell_rect.center(),
                        cell * 0.42,
                        egui::Stroke::new(cell * 0.06, dot_color),
                    );
                }
                Some(MoveDotKind::Move) => {
                    painter.circle_filled(cell_rect.center(), cell * 0.16, dot_color);
                }
                None => {}
            }
        }
    }

    // Promotion picker overlay — paint after the regular board so it
    // overdraws any piece on the squares it covers.
    if let Some(picker) = &view.pending_promotion {
        let picker_bg = egui::Color32::from_rgb(0xff, 0xff, 0xff);
        let picker_stroke = egui::Stroke::new(2.0, egui::Color32::BLACK);
        for entry in &picker.entries {
            let top_left = rect.min
                + egui::vec2(entry.display_col as f32 * cell, entry.display_row as f32 * cell);
            let cell_rect = egui::Rect::from_min_size(top_left, egui::vec2(cell, cell));
            painter.rect_filled(cell_rect, 0.0, picker_bg);
            painter.rect_stroke(cell_rect, 0.0, picker_stroke);
            painter.text(
                cell_rect.center(),
                egui::Align2::CENTER_CENTER,
                piece_glyph(entry.piece),
                egui::FontId::proportional(cell * 0.7),
                egui::Color32::BLACK,
            );
        }
    }

    if let Some((col, row)) = clicked_rc {
        // Promotion picker takes precedence: clicking a picker square
        // confirms; clicking anywhere else cancels (session's
        // handle_cancel clears the pending promotion).
        if let Some(picker) = &view.pending_promotion {
            if let Some(entry) = picker.entries.iter().find(|e| {
                e.display_col as usize == col && e.display_row as usize == row
            }) {
                events.push(Event::ConfirmPromotion(entry.move_));
            } else {
                events.push(Event::Cancel);
            }
        } else {
            let clicked_cell = &view.rows[row][col];
            events.push(Event::SelectSquare(clicked_cell.square));
        }
    }
}

fn piece_glyph(piece: Piece) -> &'static str {
    match (piece.color(), piece.kind()) {
        (Color::White, PieceType::King) => "\u{2654}",
        (Color::White, PieceType::Queen) => "\u{2655}",
        (Color::White, PieceType::Rook) => "\u{2656}",
        (Color::White, PieceType::Bishop) => "\u{2657}",
        (Color::White, PieceType::Knight) => "\u{2658}",
        (Color::White, PieceType::Pawn) => "\u{2659}",
        (Color::Black, PieceType::King) => "\u{265A}",
        (Color::Black, PieceType::Queen) => "\u{265B}",
        (Color::Black, PieceType::Rook) => "\u{265C}",
        (Color::Black, PieceType::Bishop) => "\u{265D}",
        (Color::Black, PieceType::Knight) => "\u{265E}",
        (Color::Black, PieceType::Pawn) => "\u{265F}",
    }
}
