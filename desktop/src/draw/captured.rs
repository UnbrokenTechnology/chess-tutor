//! Shared captured-material rendering: overlapping cburnett sprites so a
//! pile of pawns reads as a tight cluster rather than a wide row of
//! glyphs. Used by both the bot strip (above the board) and the player
//! strip (below it).

use chess_tutor_engine::types::{Color, Piece, PieceType};
use eframe::egui;

/// Sprite size for captured pieces (px).
const GLYPH: f32 = 22.0;
/// Horizontal step between two pieces of the *same* type — heavy overlap.
const STEP_SAME: f32 = GLYPH * 0.42;
/// Horizontal step when the type changes — a small gap so groups read as
/// distinct, while still far tighter than a full sprite width.
const STEP_NEW: f32 = GLYPH * 0.78;

/// Paint `pieces` (heaviest-first) as an overlapped cluster, allocating
/// exactly the cluster's footprint so it composes inside any layout. The
/// caller renders the adjacent `+N` lead label (placement differs by
/// strip). No-op for an empty list.
pub(crate) fn cluster(ui: &mut egui::Ui, pieces: &[Piece]) {
    if pieces.is_empty() {
        return;
    }

    // Pre-compute each sprite's x-offset: same-type pieces overlap
    // tightly, a new type opens a small gap.
    let mut xs = Vec::with_capacity(pieces.len());
    let mut x = 0.0;
    let mut prev: Option<(Color, PieceType)> = None;
    for piece in pieces {
        let key = (piece.color(), piece.kind());
        if let Some(previous) = prev {
            x += if previous == key { STEP_SAME } else { STEP_NEW };
        }
        xs.push(x);
        prev = Some(key);
    }
    let width = x + GLYPH;

    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, GLYPH), egui::Sense::hover());
    for (piece, &x) in pieces.iter().zip(&xs) {
        let sprite = egui::Rect::from_min_size(
            rect.min + egui::vec2(x, 0.0),
            egui::vec2(GLYPH, GLYPH),
        );
        crate::draw::board::piece_image(*piece).paint_at(ui, sprite);
    }
}
