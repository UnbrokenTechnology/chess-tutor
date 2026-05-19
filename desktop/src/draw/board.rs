use chess_tutor_engine::types::{Color, File, Piece, PieceType, Rank, Square};
use eframe::egui;

use crate::session::App;

impl App {
    pub(crate) fn draw_board(&mut self, ui: &mut egui::Ui) {
        let avail = ui.available_size();
        let board_size = avail.x.min(avail.y);
        let cell = board_size / 8.0;
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(board_size, board_size), egui::Sense::click());

        // Escape cancels a pending promotion. Treat like an off-picker
        // click — drop both the promotion state and the selection so
        // the user starts the move from scratch.
        if self.pending_promotion.is_some()
            && ui.input(|i| i.key_pressed(egui::Key::Escape))
        {
            self.deselect();
        }

        let clicked_square = response
            .clicked()
            .then(|| {
                response
                    .interact_pointer_pos()
                    .and_then(|p| pixel_to_square(p - rect.min, cell, self.flipped))
            })
            .flatten();

        let painter = ui.painter_at(rect);

        let light = egui::Color32::from_rgb(0xf0, 0xd9, 0xb5);
        let dark = egui::Color32::from_rgb(0xb5, 0x88, 0x63);
        let last_move_tint = egui::Color32::from_rgba_unmultiplied(0xff, 0xeb, 0x3b, 0x66);
        let selected_tint = egui::Color32::from_rgba_unmultiplied(0xff, 0xb3, 0x00, 0xaa);
        let check_tint = egui::Color32::from_rgba_unmultiplied(0xff, 0x40, 0x40, 0xaa);
        let dot_color = egui::Color32::from_rgba_unmultiplied(0x10, 0x10, 0x10, 0x66);

        let viewed_pos = self.viewed_position().clone();
        let viewed_mv = self.viewed_entry().map(|e| e.mv);
        let king_in_check = viewed_pos
            .in_check()
            .then(|| viewed_pos.king_square(viewed_pos.side_to_move()));
        let live = self.is_viewing_live();

        for display_row in 0..8u8 {
            for display_col in 0..8u8 {
                let (file_idx, rank_idx) = if self.flipped {
                    (7 - display_col, display_row)
                } else {
                    (display_col, 7 - display_row)
                };
                let is_light = (rank_idx + file_idx) % 2 != 0;
                let square_color = if is_light { light } else { dark };
                let top_left = rect.min
                    + egui::vec2(display_col as f32 * cell, display_row as f32 * cell);
                let cell_rect = egui::Rect::from_min_size(top_left, egui::vec2(cell, cell));
                painter.rect_filled(cell_rect, 0.0, square_color);

                let sq = Square::new(
                    File::from_index(file_idx).unwrap(),
                    Rank::from_index(rank_idx).unwrap(),
                );

                if let Some(mv) = viewed_mv {
                    if mv.from() == sq || mv.to() == sq {
                        painter.rect_filled(cell_rect, 0.0, last_move_tint);
                    }
                }
                if live && Some(sq) == self.selected {
                    painter.rect_filled(cell_rect, 0.0, selected_tint);
                }
                if Some(sq) == king_in_check {
                    painter.rect_filled(cell_rect, 0.0, check_tint);
                }

                if let Some(piece) = viewed_pos.piece_on(sq) {
                    painter.text(
                        cell_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        piece_glyph(piece),
                        egui::FontId::proportional(cell * 0.7),
                        egui::Color32::BLACK,
                    );
                }

                if live {
                    if let Some(legal_mv) =
                        self.legal_from_selected.iter().find(|m| m.to() == sq).copied()
                    {
                        if self.position.is_capture(legal_mv) {
                            painter.circle_stroke(
                                cell_rect.center(),
                                cell * 0.42,
                                egui::Stroke::new(cell * 0.06, dot_color),
                            );
                        } else {
                            painter.circle_filled(cell_rect.center(), cell * 0.16, dot_color);
                        }
                    }
                }
            }
        }

        // Promotion picker overlay: a vertical stack of [Q, R, B, N]
        // anchored at the promotion target, paint *after* the regular
        // board so it overdraws any piece on the squares it covers.
        if let Some(pending) = self.pending_promotion.as_ref() {
            let picker_bg = egui::Color32::from_rgb(0xff, 0xff, 0xff);
            let picker_stroke = egui::Stroke::new(2.0, egui::Color32::BLACK);
            let promoter_color = self.position.side_to_move();
            for (i, mv) in pending.candidates.iter().enumerate() {
                let pt = mv.promoted_to();
                let sq = picker_square_at(pending.to, i);
                let (dc, dr) = square_to_display_coords(sq, self.flipped);
                let top_left =
                    rect.min + egui::vec2(dc as f32 * cell, dr as f32 * cell);
                let cell_rect = egui::Rect::from_min_size(top_left, egui::vec2(cell, cell));
                painter.rect_filled(cell_rect, 0.0, picker_bg);
                painter.rect_stroke(cell_rect, 0.0, picker_stroke);
                painter.text(
                    cell_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    piece_glyph(Piece::new(promoter_color, pt)),
                    egui::FontId::proportional(cell * 0.7),
                    egui::Color32::BLACK,
                );
            }
        }

        if let Some(sq) = clicked_square {
            // Promotion picker takes precedence: a click on one of the
            // four picker squares applies that promotion; anything else
            // cancels (deselect drops the pending state too).
            if let Some(pending) = self.pending_promotion.take() {
                let picker_squares: [Square; 4] =
                    std::array::from_fn(|i| picker_square_at(pending.to, i));
                if let Some(idx) = picker_squares.iter().position(|&s| s == sq) {
                    let chosen = pending.candidates[idx];
                    self.apply_user_move(chosen);
                    self.maybe_queue_engine_search();
                } else {
                    // Click landed outside the picker — cancel. We
                    // already `take()`d the pending state, so deselect
                    // just clears the lingering pawn selection.
                    self.deselect();
                }
            } else {
                self.handle_click(sq);
            }
        }
    }
}

fn pixel_to_square(local: egui::Vec2, cell: f32, flipped: bool) -> Option<Square> {
    let col = (local.x / cell).floor() as i32;
    let row = (local.y / cell).floor() as i32;
    if !(0..8).contains(&col) || !(0..8).contains(&row) {
        return None;
    }
    let (file_idx, rank_idx) = if flipped {
        (7 - col as u8, row as u8)
    } else {
        (col as u8, 7 - row as u8)
    };
    Some(Square::new(
        File::from_index(file_idx).unwrap(),
        Rank::from_index(rank_idx).unwrap(),
    ))
}

/// Display (column, row) for `sq` given board orientation. Mirrors
/// the inverse of [`pixel_to_square`].
fn square_to_display_coords(sq: Square, flipped: bool) -> (u8, u8) {
    let file_idx = sq.file().index() as u8;
    let rank_idx = sq.rank().index() as u8;
    if flipped {
        (7 - file_idx, rank_idx)
    } else {
        (file_idx, 7 - rank_idx)
    }
}

/// The `i`-th square in the promotion picker stack: index 0 = the
/// promotion target itself, then walking back along the file toward
/// the centre of the board. Always returns a valid square because
/// promotions land on rank 0 or rank 7, leaving four ranks of headroom
/// in the relevant direction.
fn picker_square_at(target: Square, i: usize) -> Square {
    let file = target.file();
    let target_rank = target.rank().index() as i8;
    // Promotion target is on rank 8 (idx 7, white promoting) or rank 1
    // (idx 0, black promoting). Walk inward.
    let direction: i8 = if target_rank == 7 { -1 } else { 1 };
    let rank_idx = (target_rank + direction * i as i8) as u8;
    Square::new(file, Rank::from_index(rank_idx).unwrap())
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
