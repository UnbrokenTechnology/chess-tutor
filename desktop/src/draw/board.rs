use chess_tutor_engine::types::{Color, Piece, PieceType, Square};
use eframe::egui;

use chess_tutor_ui::event::Event;
use chess_tutor_ui::view::{AnnotationKind, BoardAnnotation, BoardView, MoveDotKind};

/// Paint the board into `rect` — a square the caller has sized and
/// centered (alongside the eval bar) within the central area.
pub(crate) fn draw(ui: &mut egui::Ui, rect: egui::Rect, view: &BoardView, events: &mut Vec<Event>) {
    let cell = rect.width() / 8.0;
    let response = ui.interact(rect, ui.id().with("board"), egui::Sense::click_and_drag());

    // ESC -> Cancel; session resolves priority (promotion > dialog >
    // deselect). Previously this only fired with a pending promotion,
    // but emitting unconditionally is harmless: handle_cancel is a
    // no-op when nothing's selected and no dialog is open.
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        events.push(Event::Cancel);
    }

    // Map a pointer position to the logical square under it (`None` off
    // the board).
    let square_at = |pos: egui::Pos2| -> Option<Square> {
        let local = pos - rect.min;
        let col = (local.x / cell).floor() as i32;
        let row = (local.y / cell).floor() as i32;
        ((0..8).contains(&col) && (0..8).contains(&row))
            .then(|| view.rows[row as usize][col as usize].square)
    };

    // Drag-to-move — the natural gesture, and the primary one on touch.
    // A drag is translated into the *same* select-then-move intents a
    // two-click move uses: `SelectSquare(from)` on pick-up,
    // `SelectSquare(to)` on release. The in-flight source square is stashed
    // in egui memory so it survives across frames; the dragged sprite
    // follows the cursor and is suppressed on its home square below.
    let drag_id = ui.id().with("board_drag_from");
    let drag_from: Option<Square> = ui.memory(|m| m.data.get_temp(drag_id));
    if response.drag_started() {
        if let Some(sq) = response.interact_pointer_pos().and_then(square_at) {
            // Only pick up an occupied square; the session decides whether
            // it's actually the user's movable piece.
            if piece_at(view, sq).is_some() {
                ui.memory_mut(|m| m.data.insert_temp(drag_id, sq));
                events.push(Event::SelectSquare(sq));
            }
        }
    }
    if response.drag_stopped() {
        if let (Some(from), Some(to)) =
            (drag_from, response.interact_pointer_pos().and_then(square_at))
        {
            if to != from {
                events.push(Event::SelectSquare(to));
            }
        }
    }
    // The square whose piece is currently "in hand" (drawn at the cursor,
    // hidden on its home square). Only while actively dragging.
    let dragging_from = response.dragged().then_some(drag_from).flatten();

    // Click-to-move (press + release on the same square, no drag).
    let clicked_rc = response
        .clicked()
        .then(|| response.interact_pointer_pos())
        .flatten()
        .and_then(|p| {
            let local = p - rect.min;
            let col = (local.x / cell).floor() as i32;
            let row = (local.y / cell).floor() as i32;
            ((0..8).contains(&col) && (0..8).contains(&row))
                .then_some((col as usize, row as usize))
        });

    let painter = ui.painter_at(rect);

    let light = crate::draw::theme::BOARD_LIGHT;
    let dark = crate::draw::theme::BOARD_DARK;
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
                // Hide the dragged piece on its home square — it's drawn at
                // the cursor instead (below).
                if Some(cell_view.square) != dragging_from {
                    piece_image(piece).paint_at(ui, cell_rect);
                }
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

    // Annotation overlay (square highlights + arrows). Painted
    // after the cell grid but before the promotion picker so a
    // picker square can't be obscured by an annotation.
    draw_annotations(&painter, view, rect, cell);

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
            piece_image(entry.piece).paint_at(ui, cell_rect);
        }
    }

    // The dragged piece, painted last so it floats on top of everything,
    // centered on the cursor.
    if let Some(from) = dragging_from {
        if let (Some(piece), Some(pos)) =
            (piece_at(view, from), response.interact_pointer_pos())
        {
            let sprite = egui::Rect::from_center_size(pos, egui::vec2(cell, cell));
            piece_image(piece).paint_at(ui, sprite);
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

/// The piece on a logical [`Square`], if any (scans the grid; flip-state
/// agnostic).
fn piece_at(view: &BoardView, sq: Square) -> Option<Piece> {
    view.rows
        .iter()
        .flatten()
        .find(|cell| cell.square == sq)
        .and_then(|cell| cell.piece)
}

/// Locate the on-screen display (col, row) for a logical [`Square`]
/// by scanning the [`BoardView`] grid — works for any flip state
/// without needing the orientation bit exposed.
fn cell_coords(view: &BoardView, sq: Square) -> Option<(usize, usize)> {
    for (r, row) in view.rows.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            if cell.square == sq {
                return Some((c, r));
            }
        }
    }
    None
}

fn square_center(view: &BoardView, board_min: egui::Pos2, cell: f32, sq: Square) -> Option<egui::Pos2> {
    cell_coords(view, sq).map(|(c, r)| {
        board_min + egui::vec2((c as f32 + 0.5) * cell, (r as f32 + 0.5) * cell)
    })
}

fn draw_annotations(painter: &egui::Painter, view: &BoardView, rect: egui::Rect, cell: f32) {
    for ann in &view.annotations {
        match ann {
            BoardAnnotation::SquareHighlight { square, kind } => {
                if let Some((c, r)) = cell_coords(view, *square) {
                    let top_left = rect.min
                        + egui::vec2(c as f32 * cell, r as f32 * cell);
                    let cell_rect = egui::Rect::from_min_size(top_left, egui::vec2(cell, cell));
                    let (fill, border) = annotation_square_colors(*kind);
                    if let Some(fill) = fill {
                        painter.rect_filled(cell_rect, 0.0, fill);
                    }
                    if let Some(border) = border {
                        let r_inset = cell_rect.shrink(2.0);
                        painter.rect_stroke(r_inset, 0.0, egui::Stroke::new(2.5, border));
                    }
                }
            }
            BoardAnnotation::Arrow { from, to, kind } => {
                let Some(p_from) = square_center(view, rect.min, cell, *from) else { continue };
                let Some(p_to) = square_center(view, rect.min, cell, *to) else { continue };
                draw_arrow(painter, p_from, p_to, cell, *kind);
            }
        }
    }
}

fn annotation_square_colors(
    kind: AnnotationKind,
) -> (Option<egui::Color32>, Option<egui::Color32>) {
    match kind {
        AnnotationKind::Threat => (
            Some(egui::Color32::from_rgba_unmultiplied(0xff, 0x40, 0x40, 0x70)),
            Some(egui::Color32::from_rgb(0xc0, 0x20, 0x20)),
        ),
        AnnotationKind::Capture => (
            Some(egui::Color32::from_rgba_unmultiplied(0xff, 0x80, 0x40, 0x60)),
            Some(egui::Color32::from_rgb(0xc0, 0x50, 0x10)),
        ),
        AnnotationKind::KingRing => (
            Some(egui::Color32::from_rgba_unmultiplied(0xff, 0x70, 0x40, 0x50)),
            Some(egui::Color32::from_rgb(0xb0, 0x30, 0x10)),
        ),
        AnnotationKind::GoodPiece => (
            Some(egui::Color32::from_rgba_unmultiplied(0x40, 0xa0, 0x60, 0x55)),
            Some(egui::Color32::from_rgb(0x20, 0x70, 0x30)),
        ),
        AnnotationKind::BadPiece => (
            Some(egui::Color32::from_rgba_unmultiplied(0xd0, 0x80, 0x10, 0x55)),
            Some(egui::Color32::from_rgb(0xa0, 0x55, 0x00)),
        ),
        AnnotationKind::NewMobility => (
            Some(egui::Color32::from_rgba_unmultiplied(0x40, 0xa0, 0x80, 0x45)),
            None,
        ),
        AnnotationKind::LostMobility => (
            Some(egui::Color32::from_rgba_unmultiplied(0xa0, 0x40, 0x40, 0x45)),
            None,
        ),
        AnnotationKind::SpaceFront => (
            Some(egui::Color32::from_rgba_unmultiplied(0x40, 0x90, 0xc0, 0x35)),
            None,
        ),
        AnnotationKind::SpaceReinforced => (
            Some(egui::Color32::from_rgba_unmultiplied(0x20, 0x60, 0xb0, 0x60)),
            Some(egui::Color32::from_rgb(0x10, 0x40, 0x90)),
        ),
        AnnotationKind::OpponentSpaceFront => (
            // Warm amber so a "both space overlays on" board reads as
            // teal-vs-amber without the colors clashing.
            Some(egui::Color32::from_rgba_unmultiplied(0xc0, 0x70, 0x20, 0x35)),
            None,
        ),
        AnnotationKind::OpponentSpaceReinforced => (
            Some(egui::Color32::from_rgba_unmultiplied(0xb0, 0x50, 0x10, 0x60)),
            Some(egui::Color32::from_rgb(0x90, 0x40, 0x10)),
        ),
        AnnotationKind::MobilityExcluded => (
            // Muted grey — the engine considers these "dead." Subtle
            // so it doesn't fight with other overlays painted on top.
            Some(egui::Color32::from_rgba_unmultiplied(0x60, 0x60, 0x60, 0x40)),
            None,
        ),
        AnnotationKind::Pin => (
            Some(egui::Color32::from_rgba_unmultiplied(0xe0, 0x60, 0xc0, 0x55)),
            Some(egui::Color32::from_rgb(0xb0, 0x30, 0x90)),
        ),
        AnnotationKind::TrappedEscape => (
            // Muted red: the "cage" closing in on the trapped piece.
            // Lower alpha than BadPiece so the piece itself (rendered
            // under BadPiece's stronger tint) stays the focal point.
            Some(egui::Color32::from_rgba_unmultiplied(0xc0, 0x30, 0x30, 0x45)),
            None,
        ),
        AnnotationKind::HeatOurs1 => (
            Some(egui::Color32::from_rgba_unmultiplied(0x40, 0xc0, 0x60, 0x30)),
            None,
        ),
        AnnotationKind::HeatOurs2 => (
            Some(egui::Color32::from_rgba_unmultiplied(0x20, 0x90, 0x40, 0x60)),
            None,
        ),
        AnnotationKind::HeatTheirs1 => (
            Some(egui::Color32::from_rgba_unmultiplied(0xc0, 0x40, 0x40, 0x30)),
            None,
        ),
        AnnotationKind::HeatTheirs2 => (
            Some(egui::Color32::from_rgba_unmultiplied(0x90, 0x20, 0x20, 0x60)),
            None,
        ),
        AnnotationKind::Highlight => (
            Some(egui::Color32::from_rgba_unmultiplied(0xff, 0xeb, 0x3b, 0x55)),
            None,
        ),
        // Arrow-only kinds — square fallback is just a subtle tint.
        AnnotationKind::BestMove
        | AnnotationKind::Attacker
        | AnnotationKind::Defender
        | AnnotationKind::TriggerMove => (None, None),
    }
}

fn arrow_color(kind: AnnotationKind) -> egui::Color32 {
    match kind {
        AnnotationKind::BestMove => egui::Color32::from_rgba_unmultiplied(0x30, 0x80, 0xff, 0xd0),
        AnnotationKind::Capture => egui::Color32::from_rgba_unmultiplied(0xff, 0x60, 0x20, 0xd0),
        AnnotationKind::Attacker => egui::Color32::from_rgba_unmultiplied(0xff, 0x40, 0x40, 0xd0),
        AnnotationKind::Defender => egui::Color32::from_rgba_unmultiplied(0x40, 0xa0, 0x60, 0xd0),
        AnnotationKind::Threat => egui::Color32::from_rgba_unmultiplied(0xff, 0x40, 0x40, 0xd0),
        AnnotationKind::KingRing => egui::Color32::from_rgba_unmultiplied(0xb0, 0x30, 0x10, 0xd0),
        AnnotationKind::GoodPiece => egui::Color32::from_rgba_unmultiplied(0x20, 0x70, 0x30, 0xd0),
        AnnotationKind::BadPiece => egui::Color32::from_rgba_unmultiplied(0xa0, 0x55, 0x00, 0xd0),
        AnnotationKind::NewMobility => egui::Color32::from_rgba_unmultiplied(0x20, 0x90, 0x60, 0xd0),
        AnnotationKind::LostMobility => {
            egui::Color32::from_rgba_unmultiplied(0xa0, 0x40, 0x40, 0xd0)
        }
        AnnotationKind::SpaceFront => {
            egui::Color32::from_rgba_unmultiplied(0x40, 0x90, 0xc0, 0xd0)
        }
        AnnotationKind::SpaceReinforced => {
            egui::Color32::from_rgba_unmultiplied(0x20, 0x60, 0xb0, 0xd0)
        }
        AnnotationKind::OpponentSpaceFront => {
            egui::Color32::from_rgba_unmultiplied(0xc0, 0x70, 0x20, 0xd0)
        }
        AnnotationKind::OpponentSpaceReinforced => {
            egui::Color32::from_rgba_unmultiplied(0xb0, 0x50, 0x10, 0xd0)
        }
        AnnotationKind::MobilityExcluded => {
            egui::Color32::from_rgba_unmultiplied(0x60, 0x60, 0x60, 0xd0)
        }
        AnnotationKind::Pin => egui::Color32::from_rgba_unmultiplied(0xb0, 0x30, 0x90, 0xd0),
        AnnotationKind::TrappedEscape => {
            egui::Color32::from_rgba_unmultiplied(0xc0, 0x30, 0x30, 0xd0)
        }
        AnnotationKind::HeatOurs1 => {
            egui::Color32::from_rgba_unmultiplied(0x20, 0x90, 0x40, 0xd0)
        }
        AnnotationKind::HeatOurs2 => {
            egui::Color32::from_rgba_unmultiplied(0x20, 0x90, 0x40, 0xd0)
        }
        AnnotationKind::HeatTheirs1 => {
            egui::Color32::from_rgba_unmultiplied(0x90, 0x20, 0x20, 0xd0)
        }
        AnnotationKind::HeatTheirs2 => {
            egui::Color32::from_rgba_unmultiplied(0x90, 0x20, 0x20, 0xd0)
        }
        AnnotationKind::Highlight => egui::Color32::from_rgba_unmultiplied(0xff, 0xc0, 0x10, 0xd0),
        // Gold — a "heads up, this move springs it" arrow, distinct from the
        // red attacker line and the blue best-move arrow.
        AnnotationKind::TriggerMove => egui::Color32::from_rgba_unmultiplied(0xf0, 0xb0, 0x20, 0xe0),
    }
}

fn draw_arrow(
    painter: &egui::Painter,
    from: egui::Pos2,
    to: egui::Pos2,
    cell: f32,
    kind: AnnotationKind,
) {
    let color = arrow_color(kind);
    let dir = to - from;
    let len = dir.length();
    if len < 1.0 {
        return;
    }
    let unit = dir / len;
    // Inset both endpoints by ~25% of a cell so the arrow doesn't
    // visually cover the pieces it's connecting.
    let inset = cell * 0.28;
    let shaft_from = from + unit * inset;
    let head_tip = to - unit * inset;
    let stroke_w = cell * 0.10;
    painter.line_segment(
        [shaft_from, head_tip],
        egui::Stroke::new(stroke_w, color),
    );
    // Arrowhead: filled triangle.
    let head_len = cell * 0.22;
    let head_w = cell * 0.16;
    let back = head_tip - unit * head_len;
    let perp = egui::vec2(-unit.y, unit.x);
    let left = back + perp * head_w;
    let right = back - perp * head_w;
    painter.add(egui::Shape::convex_polygon(
        vec![head_tip, left, right],
        color,
        egui::Stroke::NONE,
    ));
}

/// The cburnett SVG sprite for a piece (bundled; CC-BY-SA, from lichess).
/// `egui_extras`' SVG loader rasterizes + caches each by its `uri`, so
/// `paint_at` is cheap after the first frame. `pub(crate)` so the
/// captured-material strips reuse the same sprites.
pub(crate) fn piece_image(piece: Piece) -> egui::Image<'static> {
    let (uri, bytes): (&'static str, &'static [u8]) = match (piece.color(), piece.kind()) {
        (Color::White, PieceType::King) => {
            ("bytes://cburnett/wK.svg", include_bytes!("../../assets/pieces/wK.svg"))
        }
        (Color::White, PieceType::Queen) => {
            ("bytes://cburnett/wQ.svg", include_bytes!("../../assets/pieces/wQ.svg"))
        }
        (Color::White, PieceType::Rook) => {
            ("bytes://cburnett/wR.svg", include_bytes!("../../assets/pieces/wR.svg"))
        }
        (Color::White, PieceType::Bishop) => {
            ("bytes://cburnett/wB.svg", include_bytes!("../../assets/pieces/wB.svg"))
        }
        (Color::White, PieceType::Knight) => {
            ("bytes://cburnett/wN.svg", include_bytes!("../../assets/pieces/wN.svg"))
        }
        (Color::White, PieceType::Pawn) => {
            ("bytes://cburnett/wP.svg", include_bytes!("../../assets/pieces/wP.svg"))
        }
        (Color::Black, PieceType::King) => {
            ("bytes://cburnett/bK.svg", include_bytes!("../../assets/pieces/bK.svg"))
        }
        (Color::Black, PieceType::Queen) => {
            ("bytes://cburnett/bQ.svg", include_bytes!("../../assets/pieces/bQ.svg"))
        }
        (Color::Black, PieceType::Rook) => {
            ("bytes://cburnett/bR.svg", include_bytes!("../../assets/pieces/bR.svg"))
        }
        (Color::Black, PieceType::Bishop) => {
            ("bytes://cburnett/bB.svg", include_bytes!("../../assets/pieces/bB.svg"))
        }
        (Color::Black, PieceType::Knight) => {
            ("bytes://cburnett/bN.svg", include_bytes!("../../assets/pieces/bN.svg"))
        }
        (Color::Black, PieceType::Pawn) => {
            ("bytes://cburnett/bP.svg", include_bytes!("../../assets/pieces/bP.svg"))
        }
    };
    egui::Image::from_bytes(uri, bytes)
}
