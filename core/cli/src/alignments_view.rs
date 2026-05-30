//! `chess-tutor alignments <FEN>` — pure geometric ray scan.
//!
//! For each slider on the board (queen, rook, bishop), walk the four
//! relevant rays and report every alignment where the slider's ray
//! passes through exactly one blocker before reaching another piece.
//! Two flavours:
//!
//! - **Discovered-attack candidate**: same-colour blocker between our
//!   slider and an enemy piece on the far side. Moving the blocker
//!   exposes the slider's attack.
//! - **Pin / skewer candidate**: enemy-colour blocker between our
//!   slider and the next piece on the line.
//!
//! This is a **purely static geometric primitive** — no SEE, no
//! "is this actually winning material" judgement. PLAN-cli.md
//! Phase D's `tactics --latent` (the latent-threat detector) is
//! the SEE-filtered, judgement-applied version of this primitive.
//! `alignments` is exposed separately because the agent kept
//! reconstructing this geometry by hand and getting it wrong.
//!
//! ## Filtering
//!
//! Pure output is noisy — every long-diagonal bishop has many
//! endpoints. Default-filter to "target is higher value than blocker"
//! (the only alignments that win material when fired). `--all`
//! reverses that for the rare case the agent needs the unfiltered
//! geometric view.

use chess_tutor_engine::bitboard::square_bb;
use chess_tutor_engine::magics::{bishop_attacks, rook_attacks};
use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::{Color, PieceType, Square};
use serde::Serialize;

use crate::piece_fmt::{color_name, piece_label, piece_type_name};

#[derive(Debug, Clone, Serialize)]
pub struct AlignmentsView {
    pub white: SideAlignments,
    pub black: SideAlignments,
}

#[derive(Debug, Clone, Serialize)]
pub struct SideAlignments {
    pub side: String,
    /// Ray candidates where moving the blocker (a same-colour piece)
    /// reveals an attack on an enemy target. The "discoverer" is the
    /// slider; the "vehicle" is the blocker.
    pub discovered_attack_candidates: Vec<RayAlignment>,
    /// Ray candidates where the blocker is an enemy piece — the
    /// shape of pin / skewer geometries.
    pub pin_skewer_candidates: Vec<RayAlignment>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RayAlignment {
    pub slider: String,         // "Re1"
    pub slider_square: String,
    pub slider_kind: String,    // "rook"
    pub blocker: String,        // "be5"
    pub blocker_square: String,
    pub blocker_kind: String,
    pub blocker_color: String,  // "black"
    pub target: String,         // "qe6"
    pub target_square: String,
    pub target_kind: String,
    pub target_color: String,
    pub ray: String,            // "e-file"
    /// `true` when the target is more valuable than the blocker —
    /// the alignments worth reporting by default. Below the bar?
    /// shown only with `include_low_value: true`.
    pub target_more_valuable: bool,
}

pub fn build(pos: &Position, include_low_value: bool) -> AlignmentsView {
    AlignmentsView {
        white: build_side(pos, Color::White, include_low_value),
        black: build_side(pos, Color::Black, include_low_value),
    }
}

fn build_side(pos: &Position, side: Color, include_low_value: bool) -> SideAlignments {
    let mut disc = Vec::new();
    let mut pin = Vec::new();

    let our_bb = pos.pieces_by_color(side);
    let enemy_bb = pos.pieces_by_color(!side);
    let occ = pos.occupied();

    // For every slider of `side`, run the discovery-style scan
    // (own-colour blocker → enemy target).
    let our_rq = pos.pieces_of(side, PieceType::Rook) | pos.pieces_of(side, PieceType::Queen);
    let our_bq = pos.pieces_of(side, PieceType::Bishop) | pos.pieces_of(side, PieceType::Queen);

    for slider_sq in our_rq {
        for_each_ray_with_one_blocker(
            pos,
            slider_sq,
            true,
            occ,
            |blocker_sq, target_sq, blocker_in_our_set, target_in_enemy_set| {
                if !blocker_in_our_set || !target_in_enemy_set {
                    return;
                }
                if let Some(rec) = build_alignment(
                    pos,
                    slider_sq,
                    blocker_sq,
                    target_sq,
                    include_low_value,
                ) {
                    disc.push(rec);
                }
            },
            our_bb,
            enemy_bb,
        );
    }
    for slider_sq in our_bq {
        for_each_ray_with_one_blocker(
            pos,
            slider_sq,
            false,
            occ,
            |blocker_sq, target_sq, blocker_in_our_set, target_in_enemy_set| {
                if !blocker_in_our_set || !target_in_enemy_set {
                    return;
                }
                if let Some(rec) = build_alignment(
                    pos,
                    slider_sq,
                    blocker_sq,
                    target_sq,
                    include_low_value,
                ) {
                    disc.push(rec);
                }
            },
            our_bb,
            enemy_bb,
        );
    }

    // Pin / skewer: enemy blocker, any-coloured target. We scan our
    // own sliders the same way, but flip the blocker test.
    for slider_sq in our_rq {
        for_each_ray_with_one_blocker(
            pos,
            slider_sq,
            true,
            occ,
            |blocker_sq, target_sq, blocker_in_our_set, _target_in_enemy_set| {
                if blocker_in_our_set {
                    return;
                }
                if let Some(rec) = build_alignment(
                    pos,
                    slider_sq,
                    blocker_sq,
                    target_sq,
                    include_low_value,
                ) {
                    pin.push(rec);
                }
            },
            our_bb,
            enemy_bb,
        );
    }
    for slider_sq in our_bq {
        for_each_ray_with_one_blocker(
            pos,
            slider_sq,
            false,
            occ,
            |blocker_sq, target_sq, blocker_in_our_set, _target_in_enemy_set| {
                if blocker_in_our_set {
                    return;
                }
                if let Some(rec) = build_alignment(
                    pos,
                    slider_sq,
                    blocker_sq,
                    target_sq,
                    include_low_value,
                ) {
                    pin.push(rec);
                }
            },
            our_bb,
            enemy_bb,
        );
    }

    // Deterministic order: by slider square, then blocker square.
    disc.sort_by(|a, b| {
        a.slider_square
            .cmp(&b.slider_square)
            .then(a.blocker_square.cmp(&b.blocker_square))
    });
    pin.sort_by(|a, b| {
        a.slider_square
            .cmp(&b.slider_square)
            .then(a.blocker_square.cmp(&b.blocker_square))
    });

    SideAlignments {
        side: color_name(side).to_lowercase(),
        discovered_attack_candidates: disc,
        pin_skewer_candidates: pin,
    }
}

/// Walk each of the slider's rays. For each ray, if the first piece
/// hit is a candidate blocker and there's another piece on the line
/// past it, call `f(blocker_sq, target_sq, blocker_is_ours,
/// target_is_enemy)`. The "one blocker between slider and target"
/// invariant is enforced before calling `f`.
fn for_each_ray_with_one_blocker<F: FnMut(Square, Square, bool, bool)>(
    _pos: &Position,
    slider_sq: Square,
    orthogonal: bool,
    occ: chess_tutor_engine::bitboard::Bitboard,
    mut f: F,
    our_bb: chess_tutor_engine::bitboard::Bitboard,
    enemy_bb: chess_tutor_engine::bitboard::Bitboard,
) {
    // Find each blocker on a ray (the first hit per direction). Then
    // ray-walk past it (occ minus blocker) to find the next piece.
    let normal_ray = if orthogonal {
        rook_attacks(slider_sq, occ)
    } else {
        bishop_attacks(slider_sq, occ)
    };
    for blocker_sq in normal_ray {
        let bb = square_bb(blocker_sq);
        if (occ & bb).is_empty() {
            continue;
        }
        let blocker_is_ours = (our_bb & bb).any();
        if !blocker_is_ours && (enemy_bb & bb).is_empty() {
            continue;
        }
        let occ_minus = occ ^ bb;
        let extended = if orthogonal {
            rook_attacks(slider_sq, occ_minus)
        } else {
            bishop_attacks(slider_sq, occ_minus)
        };
        // The new pieces visible after removing the blocker — exclude
        // the original ray's reach.
        let newly_visible = extended & !normal_ray;
        for target_sq in newly_visible {
            let tbb = square_bb(target_sq);
            if (occ & tbb).is_empty() {
                continue;
            }
            // Must lie along the line slider → blocker.
            if !chess_tutor_engine::attacks::aligned(slider_sq, blocker_sq, target_sq) {
                continue;
            }
            let target_is_enemy = (enemy_bb & tbb).any();
            f(blocker_sq, target_sq, blocker_is_ours, target_is_enemy);
        }
    }
}

fn build_alignment(
    pos: &Position,
    slider_sq: Square,
    blocker_sq: Square,
    target_sq: Square,
    include_low_value: bool,
) -> Option<RayAlignment> {
    let slider = pos.piece_on(slider_sq)?;
    let blocker = pos.piece_on(blocker_sq)?;
    let target = pos.piece_on(target_sq)?;
    let target_more_valuable =
        target.kind().classical_points() > blocker.kind().classical_points();
    if !include_low_value && !target_more_valuable {
        return None;
    }
    Some(RayAlignment {
        slider: piece_label(slider, slider_sq),
        slider_square: slider_sq.to_algebraic(),
        slider_kind: piece_type_name(slider.kind()).to_string(),
        blocker: piece_label(blocker, blocker_sq),
        blocker_square: blocker_sq.to_algebraic(),
        blocker_kind: piece_type_name(blocker.kind()).to_string(),
        blocker_color: color_name(blocker.color()).to_lowercase(),
        target: piece_label(target, target_sq),
        target_square: target_sq.to_algebraic(),
        target_kind: piece_type_name(target.kind()).to_string(),
        target_color: color_name(target.color()).to_lowercase(),
        ray: ray_name(slider_sq, target_sq),
        target_more_valuable,
    })
}

fn ray_name(a: Square, b: Square) -> String {
    if a.file() == b.file() {
        format!("{}-file", file_letter(a.file().index()))
    } else if a.rank() == b.rank() {
        format!("{}th rank", a.rank().index() + 1)
    } else {
        let going_up_right = (a.rank().index() < b.rank().index())
            == (a.file().index() < b.file().index());
        if going_up_right {
            "a1-h8 diagonal".to_string()
        } else {
            "a8-h1 diagonal".to_string()
        }
    }
}

fn file_letter(file_idx: usize) -> char {
    (b'a' + file_idx as u8) as char
}

pub fn render_text(view: &AlignmentsView) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for side in [&view.white, &view.black] {
        let total =
            side.discovered_attack_candidates.len() + side.pin_skewer_candidates.len();
        if total == 0 {
            writeln!(out, "{} alignments: (none)", side.side).unwrap();
            writeln!(out).unwrap();
            continue;
        }
        writeln!(out, "{} alignments:", side.side).unwrap();
        if !side.discovered_attack_candidates.is_empty() {
            writeln!(
                out,
                "  discovered-attack candidates ({}):",
                side.discovered_attack_candidates.len(),
            )
            .unwrap();
            for r in &side.discovered_attack_candidates {
                writeln!(
                    out,
                    "    {} → (vehicle {}) → {} along {}",
                    r.slider, r.blocker, r.target, r.ray,
                )
                .unwrap();
            }
        }
        if !side.pin_skewer_candidates.is_empty() {
            writeln!(
                out,
                "  pin / skewer candidates ({}):",
                side.pin_skewer_candidates.len(),
            )
            .unwrap();
            for r in &side.pin_skewer_candidates {
                writeln!(
                    out,
                    "    {} → (enemy blocker {}) → {} along {}",
                    r.slider, r.blocker, r.target, r.ray,
                )
                .unwrap();
            }
        }
        writeln!(out).unwrap();
    }
    out
}

#[cfg(test)]
#[path = "alignments_view_tests.rs"]
mod tests;
