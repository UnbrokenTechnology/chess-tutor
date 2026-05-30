//! `chess-tutor square <SQ> <FEN>` — full per-square dossier.
//!
//! What the agent kept reconstructing by hand (wrongly) in the
//! teaching-positions post-mortems: who attacks this square, who
//! defends it, is the piece on it pinned, is it the back rank of a
//! discovered-attack alignment. This module is the one-call answer.
//!
//! Output structure:
//!
//! ```text
//! e5: black bishop  (Be5)
//!   attacked by:    Qc4 (white queen)
//!   defended by:    qe6, Pf6
//!   pin status:     not pinned
//!   blockers/rays:
//!     blocks WHITE Qe6→Re1   (e-file; moving Be5 unblocks an attack on Re1)
//! ```
//!
//! All numeric units that appear (SEE / material gains) are in classical
//! points (Q=9, R=5, B=N=3, P=1) — matching the convention used in the
//! summary header's material block. Engine-cp piece values are an
//! internal detail of `see_ge` and don't leak here.

use chess_tutor_engine::attacks::aligned;
use chess_tutor_engine::bitboard::Bitboard;
use chess_tutor_engine::magics::{bishop_attacks, rook_attacks};
use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::{Color, Piece, PieceType, Square};
use serde::Serialize;

use crate::piece_fmt::{piece_label, piece_type_name};

/// The per-square report, ready to render as text or JSON.
#[derive(Debug, Clone, Serialize)]
pub struct SquareView {
    pub square: String,
    /// `Some` when a piece sits on the square, `None` when it's empty.
    pub occupant: Option<OccupantView>,
    /// Pieces of *either* colour that attack this square, given the
    /// current occupancy. For a square with a piece on it, this is the
    /// set of attackers (any colour). For an empty square it's "who
    /// would attack this square if a piece moved to it".
    pub attackers: Vec<AttackerView>,
    /// Pieces that defend the square — same set, filtered to the
    /// occupant's colour. `None` when the square is empty (no colour
    /// to define "defender" against).
    pub defenders: Option<Vec<AttackerView>>,
    /// Pin status of the occupant (if any).
    pub pin: Option<PinView>,
    /// Discovered-attack rays where the occupant is the moving blocker.
    /// Each entry: a friendly slider behind the occupant whose attack
    /// is currently masked, and the enemy target on the far side of the
    /// occupant.
    pub discovered_attacks_when_moved: Vec<DiscoveredAttackView>,
    /// SEE-style verdict for the cheapest hypothetical capture of the
    /// occupant by the side that doesn't own it. `None` for empty
    /// squares, or when no enemy attacker exists.
    pub see_for_cheapest_capture: Option<SeeVerdictView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OccupantView {
    pub label: String,        // "Be5"
    pub piece: String,        // "bishop"
    pub color: String,        // "black"
    pub classical_points: u8, // 3
}

#[derive(Debug, Clone, Serialize)]
pub struct AttackerView {
    pub label: String,   // "Qc4"
    pub piece: String,   // "queen"
    pub color: String,   // "white"
    pub square: String,  // "c4"
}

#[derive(Debug, Clone, Serialize)]
pub struct PinView {
    /// `"absolute"` (pinned against the king — can't move legally) or
    /// `"relative"` (pinned against a more valuable piece — can move
    /// but at material cost).
    pub kind: String,
    pub pinner: AttackerView,
    pub pinned_to: String,   // e.g. "Ke1" or "Qd7"
    pub ray: String,         // e.g. "e-file"
}

#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredAttackView {
    pub discoverer: AttackerView,   // the slider behind the occupant
    pub target: AttackerView,       // the enemy piece on the other side
    pub ray: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SeeVerdictView {
    pub cheapest_attacker: AttackerView,
    /// `"wins"` (capture sequence nets material for the attacker),
    /// `"loses"` (defender comes out ahead), `"even"`.
    pub verdict: String,
}

/// Compute the per-square dossier.
pub fn build(pos: &Position, sq: Square) -> SquareView {
    let occupant = pos.piece_on(sq);
    let occ_view = occupant.map(|p| OccupantView {
        label: piece_label(p, sq),
        piece: piece_type_name(p.kind()).to_string(),
        color: lower_color(p.color()),
        classical_points: p.kind().classical_points(),
    });

    let attackers_bb = pos.attackers_to(sq, pos.occupied());
    let mut all_attackers: Vec<AttackerView> = bb_to_attacker_views(pos, attackers_bb);

    let defenders = occupant.map(|p| {
        let mut def: Vec<AttackerView> = all_attackers
            .iter()
            .filter(|a| color_from_label(&a.color) == p.color())
            .cloned()
            .collect();
        // For a piece's own square, the piece itself shouldn't appear
        // in "attackers/defenders" — but our `attackers_to` is purely
        // bitboard-based and doesn't include the target square as one
        // of its own attackers, so no filtering needed here. Keep the
        // collected list as-is.
        def.sort_by_key(|a| a.label.clone());
        def
    });

    if let Some(_p) = occupant {
        // Attackers are the OPPOSITE colour from the occupant. For an
        // empty square we list both colours (any piece could move there
        // and be attacked). For an occupied square the "attacker" set
        // is only the enemy.
        let occ_color = occupant.unwrap().color();
        all_attackers.retain(|a| color_from_label(&a.color) != occ_color);
    }
    all_attackers.sort_by_key(|a| a.label.clone());

    let pin = occupant.and_then(|p| pin_view(pos, sq, p));
    let discovered = occupant
        .map(|p| discovered_attacks_view(pos, sq, p))
        .unwrap_or_default();

    let see = occupant.and_then(|p| see_for_cheapest_view(pos, sq, p));

    SquareView {
        square: sq.to_algebraic(),
        occupant: occ_view,
        attackers: all_attackers,
        defenders,
        pin,
        discovered_attacks_when_moved: discovered,
        see_for_cheapest_capture: see,
    }
}

/// Multi-line human-readable rendering. Self-describing so an agent
/// scanning the output never needs the JSON form for orientation —
/// labels (`occupant`, `attacked by`, `pin status`) reveal what's
/// missing as readily as what's there.
pub fn render_text(view: &SquareView) -> String {
    use std::fmt::Write;
    let mut out = String::new();

    match &view.occupant {
        Some(occ) => writeln!(
            out,
            "{}: {} {}  ({})",
            view.square, occ.color, occ.piece, occ.label
        )
        .unwrap(),
        None => writeln!(out, "{}: (empty square)", view.square).unwrap(),
    }

    if view.attackers.is_empty() {
        writeln!(out, "  attacked by:    (no attackers)").unwrap();
    } else {
        let label = if view.occupant.is_some() {
            "attacked by:    "
        } else {
            "attackable by:  "
        };
        writeln!(
            out,
            "  {}{}",
            label,
            view.attackers
                .iter()
                .map(|a| format!("{} ({} {})", a.label, a.color, a.piece))
                .collect::<Vec<_>>()
                .join(", "),
        )
        .unwrap();
    }

    if let Some(defs) = &view.defenders {
        if defs.is_empty() {
            writeln!(out, "  defended by:    (no defenders)").unwrap();
        } else {
            writeln!(
                out,
                "  defended by:    {}",
                defs.iter()
                    .map(|d| format!("{} ({} {})", d.label, d.color, d.piece))
                    .collect::<Vec<_>>()
                    .join(", "),
            )
            .unwrap();
        }
    }

    match &view.pin {
        Some(p) => writeln!(
            out,
            "  pin status:     {} pin by {} (pinned to {} along {})",
            p.kind, p.pinner.label, p.pinned_to, p.ray,
        )
        .unwrap(),
        None if view.occupant.is_some() => writeln!(out, "  pin status:     not pinned").unwrap(),
        None => {}
    }

    if !view.discovered_attacks_when_moved.is_empty() {
        writeln!(out, "  is discovery vehicle for:").unwrap();
        for d in &view.discovered_attacks_when_moved {
            writeln!(
                out,
                "    {} → {} along {}  (moving the piece on {} unblocks {})",
                d.discoverer.label,
                d.target.label,
                d.ray,
                view.square,
                d.target.label,
            )
            .unwrap();
        }
    }

    if let Some(see) = &view.see_for_cheapest_capture {
        writeln!(
            out,
            "  SEE on capture: {} captures here → {}  (cheapest attacker)",
            see.cheapest_attacker.label, see.verdict,
        )
        .unwrap();
    }

    out
}

// ---- internals -------------------------------------------------------

fn bb_to_attacker_views(pos: &Position, bb: Bitboard) -> Vec<AttackerView> {
    bb.into_iter()
        .filter_map(|sq| {
            pos.piece_on(sq).map(|p| AttackerView {
                label: piece_label(p, sq),
                piece: piece_type_name(p.kind()).to_string(),
                color: lower_color(p.color()),
                square: sq.to_algebraic(),
            })
        })
        .collect()
}

fn lower_color(c: Color) -> String {
    match c {
        Color::White => "white".to_string(),
        Color::Black => "black".to_string(),
    }
}

fn color_from_label(label: &str) -> Color {
    if label == "white" {
        Color::White
    } else {
        Color::Black
    }
}

/// Detect pin: an absolute pin (against king) reads off the engine's
/// `blockers_for_king` cache. A relative pin (against a queen, say) is
/// detected by running `slider_blockers` against the queen square.
fn pin_view(pos: &Position, sq: Square, occupant: Piece) -> Option<PinView> {
    let us = occupant.color();
    let enemy_bb = pos.pieces_by_color(!us);

    // Absolute pin: occupant is in the blockers-for-our-king set.
    if pos.blockers_for_king(us).contains(sq) {
        let king_sq = pos.king_square(us);
        // Find the specific enemy slider on the far side of `sq` from
        // the king — walk the pseudo-attack rays for both sliders.
        let pinner_sq = find_pinner_along_ray(pos, sq, king_sq, enemy_bb)?;
        let pinner_piece = pos.piece_on(pinner_sq)?;
        return Some(PinView {
            kind: "absolute".to_string(),
            pinner: AttackerView {
                label: piece_label(pinner_piece, pinner_sq),
                piece: piece_type_name(pinner_piece.kind()).to_string(),
                color: lower_color(pinner_piece.color()),
                square: pinner_sq.to_algebraic(),
            },
            pinned_to: piece_label(pos.piece_on(king_sq)?, king_sq),
            ray: ray_name(king_sq, sq),
        });
    }

    // Relative pin against our queen.
    let queens = pos.pieces_of(us, PieceType::Queen);
    for queen_sq in queens {
        if queen_sq == sq {
            continue;
        }
        let (blockers, pinners) = pos.slider_blockers(enemy_bb, queen_sq);
        if blockers.contains(sq) {
            let pinner_sq = find_pinner_along_ray(pos, sq, queen_sq, pinners)?;
            let pinner_piece = pos.piece_on(pinner_sq)?;
            return Some(PinView {
                kind: "relative".to_string(),
                pinner: AttackerView {
                    label: piece_label(pinner_piece, pinner_sq),
                    piece: piece_type_name(pinner_piece.kind()).to_string(),
                    color: lower_color(pinner_piece.color()),
                    square: pinner_sq.to_algebraic(),
                },
                pinned_to: piece_label(pos.piece_on(queen_sq)?, queen_sq),
                ray: ray_name(queen_sq, sq),
            });
        }
    }

    None
}

/// Given the blocker on `blocker_sq` and the piece it's pinned to on
/// `target_sq`, find the slider on the other side of the blocker along
/// the same ray. Filters candidates to `candidates` (caller passes the
/// enemy piece set, or the pre-computed pinner set).
fn find_pinner_along_ray(
    pos: &Position,
    blocker_sq: Square,
    target_sq: Square,
    candidates: Bitboard,
) -> Option<Square> {
    let occ = pos.occupied();
    let rq = pos.pieces(PieceType::Rook) | pos.pieces(PieceType::Queen);
    let bq = pos.pieces(PieceType::Bishop) | pos.pieces(PieceType::Queen);

    // Cast rays from `blocker_sq` *away from* the target. We do that by
    // walking attack rays from `target_sq`, removing the blocker from
    // occupancy so the slider's ray punches through to the pinner.
    let occ_minus_blocker = occ ^ chess_tutor_engine::bitboard::square_bb(blocker_sq);
    let ortho = rook_attacks(target_sq, occ_minus_blocker) & rq & candidates;
    let diag = bishop_attacks(target_sq, occ_minus_blocker) & bq & candidates;
    // The pinner is the candidate on the same line as target ↔ blocker.
    (ortho | diag)
        .into_iter()
        .find(|&cand| aligned(target_sq, blocker_sq, cand))
}

/// `discovered_attacks_when_moved`: the occupant on `sq` is shielding
/// one or more friendly sliders from enemy targets. Move the occupant,
/// the attack fires. This is the standing-discovered-attack pre-cursor
/// (PLAN-cli.md, the case-study e-file alignment) — we surface the
/// alignment itself; the latent-threat command in Phase D will judge
/// whether it actually wins material.
fn discovered_attacks_view(
    pos: &Position,
    sq: Square,
    occupant: Piece,
) -> Vec<DiscoveredAttackView> {
    let mut out = Vec::new();
    let us = occupant.color();
    let occ = pos.occupied();
    let occ_minus = occ ^ chess_tutor_engine::bitboard::square_bb(sq);

    // For each of our sliders, ask: if `sq` weren't blocking, would the
    // slider's attack ray punch through `sq` and hit a more-valuable
    // enemy piece on the far side?
    let our_rq = (pos.pieces_of(us, PieceType::Rook) | pos.pieces_of(us, PieceType::Queen))
        & !chess_tutor_engine::bitboard::square_bb(sq);
    let our_bq = (pos.pieces_of(us, PieceType::Bishop) | pos.pieces_of(us, PieceType::Queen))
        & !chess_tutor_engine::bitboard::square_bb(sq);
    let enemy_bb = pos.pieces_by_color(!us);

    for slider_sq in our_rq {
        let ray_now = rook_attacks(slider_sq, occ);
        let ray_after = rook_attacks(slider_sq, occ_minus);
        // The piece on `sq` must currently be on the slider's ray-
        // through path (i.e., the slider would see further once we
        // move). `ray_after & enemy & !ray_now` is the new enemy
        // square exposed by moving the blocker.
        let newly_seen_enemy = ray_after & enemy_bb & !ray_now;
        for target_sq in newly_seen_enemy {
            // Only report if `sq` actually lies on the line slider→target.
            if !aligned(slider_sq, sq, target_sq) {
                continue;
            }
            push_discovery(pos, &mut out, slider_sq, target_sq);
        }
    }
    for slider_sq in our_bq {
        let ray_now = bishop_attacks(slider_sq, occ);
        let ray_after = bishop_attacks(slider_sq, occ_minus);
        let newly_seen_enemy = ray_after & enemy_bb & !ray_now;
        for target_sq in newly_seen_enemy {
            if !aligned(slider_sq, sq, target_sq) {
                continue;
            }
            push_discovery(pos, &mut out, slider_sq, target_sq);
        }
    }
    // Deterministic ordering for testable output.
    out.sort_by(|a, b| a.target.label.cmp(&b.target.label));
    out
}

fn push_discovery(
    pos: &Position,
    out: &mut Vec<DiscoveredAttackView>,
    slider_sq: Square,
    target_sq: Square,
) {
    let Some(slider) = pos.piece_on(slider_sq) else {
        return;
    };
    let Some(target) = pos.piece_on(target_sq) else {
        return;
    };
    out.push(DiscoveredAttackView {
        discoverer: AttackerView {
            label: piece_label(slider, slider_sq),
            piece: piece_type_name(slider.kind()).to_string(),
            color: lower_color(slider.color()),
            square: slider_sq.to_algebraic(),
        },
        target: AttackerView {
            label: piece_label(target, target_sq),
            piece: piece_type_name(target.kind()).to_string(),
            color: lower_color(target.color()),
            square: target_sq.to_algebraic(),
        },
        ray: ray_name(slider_sq, target_sq),
    });
}

fn see_for_cheapest_view(
    pos: &Position,
    sq: Square,
    occupant: Piece,
) -> Option<SeeVerdictView> {
    let attackers_bb = pos.attackers_to(sq, pos.occupied())
        & pos.pieces_by_color(!occupant.color())
        & !pos.pieces(PieceType::King);
    if attackers_bb.is_empty() {
        return None;
    }
    // Cheapest by midgame piece value (mirrors `list_see_losing`).
    use chess_tutor_engine::types::{Move, Value};
    let mut best_from: Option<Square> = None;
    let mut best_value = i32::MAX;
    for from in attackers_bb {
        if let Some(p) = pos.piece_on(from) {
            let v = Value::mg_of_piece(p.kind()).0;
            if v < best_value {
                best_value = v;
                best_from = Some(from);
            }
        }
    }
    let from = best_from?;
    let attacker_piece = pos.piece_on(from)?;

    let capture = Move::normal(from, sq);
    let wins = pos.see_ge(capture, Value(1));
    let even = !wins && pos.see_ge(capture, Value::ZERO);
    let verdict = if wins {
        "wins material"
    } else if even {
        "even trade"
    } else {
        "loses material"
    };
    Some(SeeVerdictView {
        cheapest_attacker: AttackerView {
            label: piece_label(attacker_piece, from),
            piece: piece_type_name(attacker_piece.kind()).to_string(),
            color: lower_color(attacker_piece.color()),
            square: from.to_algebraic(),
        },
        verdict: verdict.to_string(),
    })
}

/// Human-readable name for a ray between two squares: `"e-file"`,
/// `"4th rank"`, `"a1-h8 diagonal"`, `"a8-h1 diagonal"`.
fn ray_name(a: Square, b: Square) -> String {
    if a.file() == b.file() {
        format!("{}-file", file_letter(a.file().index()))
    } else if a.rank() == b.rank() {
        format!("{}th rank", a.rank().index() + 1)
    } else if (a.rank().index() as i32 - b.rank().index() as i32).abs()
        == (a.file().index() as i32 - b.file().index() as i32).abs()
    {
        // Diagonal: name by direction.
        let going_up_right = (a.rank().index() < b.rank().index())
            == (a.file().index() < b.file().index());
        if going_up_right {
            "a1-h8 diagonal".to_string()
        } else {
            "a8-h1 diagonal".to_string()
        }
    } else {
        // Shouldn't happen for valid alignments; defensive fallback.
        format!("ray {}-{}", a.to_algebraic(), b.to_algebraic())
    }
}

fn file_letter(file_idx: usize) -> char {
    (b'a' + file_idx as u8) as char
}

#[cfg(test)]
#[path = "square_view_tests.rs"]
mod tests;
