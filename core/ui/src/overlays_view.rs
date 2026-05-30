//! Map [`OverlayKind`] toggles to [`BoardAnnotation`]s.
//!
//! All "which colors mean what" decisions live here; the renderer's
//! `annotation_square_colors` table is just a flat kind→color map.
//! When the user side ("us") differs from the engine's POV-flipping
//! convention, the per-side data on [`OverlayData`] is selected
//! accordingly so "My space" always paints the user's space
//! regardless of who's on move.

use std::collections::HashSet;

use chess_tutor_engine::analysis::OverlayData;
use chess_tutor_engine::bitboard::Bitboard;
use chess_tutor_engine::types::Color;

use crate::view::{AnnotationKind, BoardAnnotation, OverlayKind};

/// Push one `SquareHighlight` per square contributed by each active
/// overlay. Overlay annotations are emitted in [`OverlayKind::ALL`]
/// order; later overlays' highlights paint on top of earlier ones
/// when their squares overlap (the renderer iterates the list).
pub fn push_overlay_annotations(
    out: &mut Vec<BoardAnnotation>,
    data: &OverlayData,
    us: Color,
    active: &HashSet<OverlayKind>,
) {
    for kind in OverlayKind::ALL {
        if !active.contains(&kind) {
            continue;
        }
        match kind {
            OverlayKind::MySpace => push_space(out, data, us, /*ours=*/ true),
            OverlayKind::OpponentSpace => push_space(out, data, us, /*ours=*/ false),
            OverlayKind::MyMobilityArea => {
                let excluded = match us {
                    Color::White => data.white_mobility_excluded,
                    Color::Black => data.black_mobility_excluded,
                };
                push_squares(out, excluded, AnnotationKind::MobilityExcluded);
            }
            OverlayKind::KingRings => {
                push_squares(out, data.white_king_ring, AnnotationKind::KingRing);
                push_squares(out, data.black_king_ring, AnnotationKind::KingRing);
            }
            OverlayKind::Pins => {
                push_squares(out, data.white_pinned, AnnotationKind::Pin);
                push_squares(out, data.black_pinned, AnnotationKind::Pin);
            }
            OverlayKind::TrappedPieces => {
                // Both sides painted under one toggle (mirrors KingRings
                // / Pins). The piece itself reads as "weak / about to be
                // lost" → BadPiece; the surrounding cage squares paint
                // in a muted red so the box closing in is visible at a
                // glance.
                push_squares(out, data.white_trapped, AnnotationKind::BadPiece);
                push_squares(out, data.black_trapped, AnnotationKind::BadPiece);
                push_squares(out, data.white_trapped_cage, AnnotationKind::TrappedEscape);
                push_squares(out, data.black_trapped_cage, AnnotationKind::TrappedEscape);
            }
            OverlayKind::AttackHeatmap => push_attack_heat(out, data, us),
        }
    }
}

fn push_space(out: &mut Vec<BoardAnnotation>, data: &OverlayData, us: Color, ours: bool) {
    let want_white = matches!((us, ours), (Color::White, true) | (Color::Black, false));
    let (safe, reinforced, front_kind, reinforced_kind) = if ours {
        if want_white {
            (
                data.white_space_safe,
                data.white_space_reinforced,
                AnnotationKind::SpaceFront,
                AnnotationKind::SpaceReinforced,
            )
        } else {
            (
                data.black_space_safe,
                data.black_space_reinforced,
                AnnotationKind::SpaceFront,
                AnnotationKind::SpaceReinforced,
            )
        }
    } else if want_white {
        (
            data.white_space_safe,
            data.white_space_reinforced,
            AnnotationKind::OpponentSpaceFront,
            AnnotationKind::OpponentSpaceReinforced,
        )
    } else {
        (
            data.black_space_safe,
            data.black_space_reinforced,
            AnnotationKind::OpponentSpaceFront,
            AnnotationKind::OpponentSpaceReinforced,
        )
    };
    // Front-only set = safe \ reinforced so each square is painted
    // exactly once (matches the Space card's behavior).
    let front_only = safe & !reinforced;
    push_squares(out, front_only, front_kind);
    push_squares(out, reinforced, reinforced_kind);
}

fn push_attack_heat(out: &mut Vec<BoardAnnotation>, data: &OverlayData, us: Color) {
    // POV-flip so "ours" always means the user's side.
    let (ours_1, ours_2, theirs_1, theirs_2) = match us {
        Color::White => (
            data.heat_white_1,
            data.heat_white_2plus,
            data.heat_black_1,
            data.heat_black_2plus,
        ),
        Color::Black => (
            data.heat_black_1,
            data.heat_black_2plus,
            data.heat_white_1,
            data.heat_white_2plus,
        ),
    };
    push_squares(out, ours_1, AnnotationKind::HeatOurs1);
    push_squares(out, ours_2, AnnotationKind::HeatOurs2);
    push_squares(out, theirs_1, AnnotationKind::HeatTheirs1);
    push_squares(out, theirs_2, AnnotationKind::HeatTheirs2);
}

fn push_squares(out: &mut Vec<BoardAnnotation>, bb: Bitboard, kind: AnnotationKind) {
    for sq in bb {
        out.push(BoardAnnotation::SquareHighlight { square: sq, kind });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chess_tutor_engine::analysis::compute_overlays;
    use chess_tutor_engine::position::Position;
    use chess_tutor_engine::types::Square;

    use crate::view::OverlayKind;

    /// Black knight on a8, cage = {b6, c7}, white's turn. The trapped
    /// overlay must emit BadPiece on a8 and TrappedEscape on the cage.
    #[test]
    fn trapped_overlay_paints_piece_then_cage() {
        let pos = Position::from_fen("n6k/8/3P4/2PB4/8/8/8/6K1 w - - 0 1").unwrap();
        let data = compute_overlays(&pos);
        let mut out = Vec::new();
        let active: HashSet<OverlayKind> =
            std::iter::once(OverlayKind::TrappedPieces).collect();
        // `us` is irrelevant for trapped (both sides painted under one
        // toggle), so just hand it the side to move.
        push_overlay_annotations(&mut out, &data, Color::White, &active);

        let on = |sq, kind| {
            out.iter().any(|a| {
                matches!(a, &BoardAnnotation::SquareHighlight { square, kind: k }
                    if square == sq && k == kind)
            })
        };
        assert!(on(Square::A8, AnnotationKind::BadPiece));
        assert!(on(Square::B6, AnnotationKind::TrappedEscape));
        assert!(on(Square::C7, AnnotationKind::TrappedEscape));
        // Nothing for the white side at this position.
        let any_white_trap = out
            .iter()
            .any(|a| matches!(a, BoardAnnotation::SquareHighlight { square, .. }
                if *square == Square::D5 || *square == Square::C5 || *square == Square::D6));
        assert!(!any_white_trap);
    }

    #[test]
    fn trapped_overlay_off_by_default_emits_nothing_for_that_kind() {
        let pos = Position::from_fen("n6k/8/3P4/2PB4/8/8/8/6K1 w - - 0 1").unwrap();
        let data = compute_overlays(&pos);
        let mut out = Vec::new();
        let active: HashSet<OverlayKind> = HashSet::new();
        push_overlay_annotations(&mut out, &data, Color::White, &active);
        // No annotations at all when no overlay is toggled on.
        assert!(out.is_empty());
    }
}
