//! `chess-tutor threats <FEN>` — unified "what's vulnerable" snapshot
//! for both sides.
//!
//! Composes every existing engine threat scanner into one report so
//! the agent doesn't have to remember which sub-command surfaces which
//! flavour of weakness:
//!
//! - **Hanging** — attacked, no defenders ([`list_hanging`]).
//! - **SEE-losing** — attacked + defended, but the exchange still
//!   loses material ([`list_see_losing`]).
//! - **Pinned** — pieces in [`Position::blockers_for_king`] (absolute)
//!   plus a relative-pin scan against each queen square.
//! - **Overloaded** — sole defenders shouldering ≥ 2 attacked duties
//!   ([`find_overloaded`]).
//! - **Trapped** — pieces whose every legal move drops material
//!   ([`trapped_cages`]).
//!
//! Maps 1:1 to the desktop coaching cards so the agent sees what the
//! human would.

use chess_tutor_engine::analysis::{
    find_overloaded, list_hanging, list_see_losing, pin_forcing_escape, trapped_cages,
    HangingPiece, OverloadedPiece, PieceLocation,
};
use chess_tutor_engine::position::Position;
use chess_tutor_engine::san;
use chess_tutor_engine::types::{Color, Move, PieceType};
use serde::Serialize;

use crate::piece_fmt::{color_name, piece_label, piece_type_name};

/// The full threats report, one [`SideThreats`] per colour, ready for
/// text or JSON rendering.
#[derive(Debug, Clone, Serialize)]
pub struct ThreatsView {
    pub white: SideThreats,
    pub black: SideThreats,
}

#[derive(Debug, Clone, Serialize)]
pub struct SideThreats {
    pub side: String,                          // "white" / "black"
    pub hanging: Vec<HangingView>,             // attacked + undefended
    pub see_losing: Vec<HangingView>,          // attacked + defended but losing the trade
    pub pinned: Vec<PinnedView>,               // absolute + relative pins
    pub overloaded: Vec<OverloadedView>,
    pub trapped: Vec<TrappedView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HangingView {
    pub piece: String,             // "Bf5"
    pub piece_kind: String,        // "bishop"
    pub square: String,            // "f5"
    pub classical_points: u8,      // 3
    pub attackers: Vec<String>,    // ["qe6", "Ne4"]
}

#[derive(Debug, Clone, Serialize)]
pub struct PinnedView {
    pub piece: String,             // "Ne2"
    pub square: String,            // "e2"
    pub kind: String,              // "absolute" / "relative"
    pub pinner: String,            // "re8"
    pub pinned_to: String,         // "Ke1" / "Qd1"
    /// SAN of a forcing (checking) move that breaks the pin, when one
    /// exists (`"Bxh2+"`). A relative pin with an escape is *nominal* —
    /// it doesn't actually restrain the piece, because the check must be
    /// answered before the pinning side could punish the departure. `None`
    /// for absolute pins and relative pins with no checking escape.
    pub escape_san: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OverloadedView {
    pub piece: String,             // "Qe2"
    pub square: String,
    pub duties: Vec<String>,       // ["Pf2", "Re1"]
}

#[derive(Debug, Clone, Serialize)]
pub struct TrappedView {
    pub piece: String,             // "Bf5"
    pub square: String,
    /// Squares the piece could move to that are all unsafe — the
    /// "cage" closing in on it.
    pub cage_squares: Vec<String>,
}

/// Build the unified threats report for both colours.
pub fn build(pos: &Position) -> ThreatsView {
    ThreatsView {
        white: build_side(pos, Color::White),
        black: build_side(pos, Color::Black),
    }
}

fn build_side(pos: &Position, side: Color) -> SideThreats {
    SideThreats {
        side: color_name(side).to_lowercase(),
        hanging: list_hanging(pos, side)
            .into_iter()
            .map(|h| build_hanging_view(pos, h))
            .collect(),
        see_losing: list_see_losing(pos, side)
            .into_iter()
            .map(|h| build_hanging_view(pos, h))
            .collect(),
        pinned: collect_pinned(pos, side),
        overloaded: find_overloaded(pos, side)
            .into_iter()
            .map(|o| build_overloaded_view(pos, o))
            .collect(),
        trapped: build_trapped(pos, side),
    }
}

fn build_hanging_view(pos: &Position, h: HangingPiece) -> HangingView {
    HangingView {
        piece: label_for_location(pos, &h.location),
        piece_kind: piece_type_name(h.location.piece).to_string(),
        square: h.location.square.to_algebraic(),
        classical_points: h.location.piece.classical_points(),
        attackers: h
            .attackers
            .iter()
            .map(|a| label_for_location(pos, a))
            .collect(),
    }
}

fn build_overloaded_view(pos: &Position, o: OverloadedPiece) -> OverloadedView {
    let piece = pos
        .piece_on(o.piece)
        .map(|p| piece_label(p, o.piece))
        .unwrap_or_else(|| o.piece.to_algebraic());
    OverloadedView {
        piece,
        square: o.piece.to_algebraic(),
        duties: o
            .duties
            .iter()
            .map(|sq| {
                pos.piece_on(*sq)
                    .map(|p| piece_label(p, *sq))
                    .unwrap_or_else(|| sq.to_algebraic())
            })
            .collect(),
    }
}

fn build_trapped(pos: &Position, side: Color) -> Vec<TrappedView> {
    trapped_cages(pos, side)
        .into_iter()
        .map(|(sq, cage_bb)| {
            let piece = pos
                .piece_on(sq)
                .map(|p| piece_label(p, sq))
                .unwrap_or_else(|| sq.to_algebraic());
            TrappedView {
                piece,
                square: sq.to_algebraic(),
                cage_squares: cage_bb
                    .into_iter()
                    .map(|s| s.to_algebraic())
                    .collect(),
            }
        })
        .collect()
}

/// Absolute + relative pins for `side`. Absolute pins come from the
/// engine's `blockers_for_king` cache; relative pins use
/// `slider_blockers` against the queen square.
fn collect_pinned(pos: &Position, side: Color) -> Vec<PinnedView> {
    use chess_tutor_engine::attacks::aligned;
    use chess_tutor_engine::bitboard::square_bb;
    use chess_tutor_engine::magics::{bishop_attacks, rook_attacks};

    let mut out = Vec::new();
    let enemy_bb = pos.pieces_by_color(!side);
    let king_sq = pos.king_square(side);

    // Absolute pins — find pinner by ray-walking from king through
    // each blocker.
    let abs_blockers = pos.blockers_for_king(side);
    for blocker_sq in abs_blockers {
        let occ_minus = pos.occupied() ^ square_bb(blocker_sq);
        let rq = pos.pieces(PieceType::Rook) | pos.pieces(PieceType::Queen);
        let bq = pos.pieces(PieceType::Bishop) | pos.pieces(PieceType::Queen);
        let pinner_candidates =
            (rook_attacks(king_sq, occ_minus) & rq | bishop_attacks(king_sq, occ_minus) & bq)
                & enemy_bb;
        for cand in pinner_candidates {
            if aligned(king_sq, blocker_sq, cand) {
                let Some(blocker_piece) = pos.piece_on(blocker_sq) else {
                    continue;
                };
                let Some(pinner_piece) = pos.piece_on(cand) else {
                    continue;
                };
                let Some(king_piece) = pos.piece_on(king_sq) else {
                    continue;
                };
                out.push(PinnedView {
                    piece: piece_label(blocker_piece, blocker_sq),
                    square: blocker_sq.to_algebraic(),
                    kind: "absolute".to_string(),
                    pinner: piece_label(pinner_piece, cand),
                    pinned_to: piece_label(king_piece, king_sq),
                    // Absolute pins almost never have a forcing escape (the
                    // piece can't leave the ray), but the engine call is
                    // the honest check — let it decide.
                    escape_san: pin_forcing_escape(pos, blocker_sq, cand, king_sq, side)
                        .map(|mv| mover_move_san(pos, mv, side)),
                });
                break;
            }
        }
    }

    // Relative pins against any of our queens.
    for queen_sq in pos.pieces_of(side, PieceType::Queen) {
        let (blockers, pinners) = pos.slider_blockers(enemy_bb, queen_sq);
        for blocker_sq in blockers {
            // Already reported as absolute? Skip.
            if abs_blockers.contains(blocker_sq) {
                continue;
            }
            let occ_minus = pos.occupied() ^ square_bb(blocker_sq);
            let rq = pos.pieces(PieceType::Rook) | pos.pieces(PieceType::Queen);
            let bq = pos.pieces(PieceType::Bishop) | pos.pieces(PieceType::Queen);
            let cands =
                (rook_attacks(queen_sq, occ_minus) & rq | bishop_attacks(queen_sq, occ_minus) & bq)
                    & pinners
                    & enemy_bb;
            for cand in cands {
                if aligned(queen_sq, blocker_sq, cand) {
                    let Some(blocker_piece) = pos.piece_on(blocker_sq) else {
                        continue;
                    };
                    let Some(pinner_piece) = pos.piece_on(cand) else {
                        continue;
                    };
                    let Some(queen_piece) = pos.piece_on(queen_sq) else {
                        continue;
                    };
                    out.push(PinnedView {
                        piece: piece_label(blocker_piece, blocker_sq),
                        square: blocker_sq.to_algebraic(),
                        kind: "relative".to_string(),
                        pinner: piece_label(pinner_piece, cand),
                        pinned_to: piece_label(queen_piece, queen_sq),
                        escape_san: pin_forcing_escape(pos, blocker_sq, cand, queen_sq, side)
                            .map(|mv| mover_move_san(pos, mv, side)),
                    });
                    break;
                }
            }
        }
    }

    out
}

/// SAN for `mv` played by `mover`, null-pivoting when it isn't already
/// `mover`'s turn (the pinned piece whose escape we name may belong to
/// either side).
fn mover_move_san(pos: &Position, mv: Move, mover: Color) -> String {
    let mut scratch = pos.clone();
    let saved = (scratch.side_to_move() != mover).then(|| scratch.do_null_move());
    let s = san::format(&scratch, mv);
    if let Some(st) = saved {
        scratch.undo_null_move(st);
    }
    s
}

fn label_for_location(pos: &Position, loc: &PieceLocation) -> String {
    pos.piece_on(loc.square)
        .map(|p| piece_label(p, loc.square))
        .unwrap_or_else(|| loc.square.to_algebraic())
}

/// Multi-line human-readable rendering.
pub fn render_text(view: &ThreatsView) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for side in [&view.white, &view.black] {
        render_side(&mut out, side);
        writeln!(out).unwrap();
    }
    out
}

fn render_side(out: &mut String, side: &SideThreats) {
    use std::fmt::Write;
    let total = side.hanging.len()
        + side.see_losing.len()
        + side.pinned.len()
        + side.overloaded.len()
        + side.trapped.len();
    if total == 0 {
        writeln!(out, "{}: (no threats found)", side.side).unwrap();
        return;
    }
    writeln!(out, "{}:", side.side).unwrap();
    if !side.hanging.is_empty() {
        writeln!(out, "  hanging ({}):", side.hanging.len()).unwrap();
        for h in &side.hanging {
            writeln!(
                out,
                "    {} ({} pts) — attacked by {}",
                h.piece,
                h.classical_points,
                h.attackers.join(", "),
            )
            .unwrap();
        }
    }
    if !side.see_losing.is_empty() {
        writeln!(out, "  SEE-losing ({}):", side.see_losing.len()).unwrap();
        for h in &side.see_losing {
            writeln!(
                out,
                "    {} ({} pts) — losing trade vs {}",
                h.piece,
                h.classical_points,
                h.attackers.join(", "),
            )
            .unwrap();
        }
    }
    if !side.pinned.is_empty() {
        writeln!(out, "  pinned ({}):", side.pinned.len()).unwrap();
        for p in &side.pinned {
            match &p.escape_san {
                // A pin the piece can break with a check is NOMINAL — it
                // reads as a restraint but doesn't actually hold. Say so
                // loudly, because the bare "pinned" label is a false-safety
                // signal: the discovered attack / departure it appears to
                // prevent is in fact live.
                Some(esc) => writeln!(
                    out,
                    "    {} — NOMINAL {} pin by {} (to {}), but ESCAPABLE via {} (a check): the pin does NOT restrain it",
                    p.piece, p.kind, p.pinner, p.pinned_to, esc,
                )
                .unwrap(),
                None => writeln!(
                    out,
                    "    {} — {} pin by {} (pinned to {})",
                    p.piece, p.kind, p.pinner, p.pinned_to,
                )
                .unwrap(),
            }
        }
    }
    if !side.overloaded.is_empty() {
        writeln!(out, "  overloaded ({}):", side.overloaded.len()).unwrap();
        for o in &side.overloaded {
            writeln!(
                out,
                "    {} — sole defender of {}",
                o.piece,
                o.duties.join(", "),
            )
            .unwrap();
        }
    }
    if !side.trapped.is_empty() {
        writeln!(out, "  trapped ({}):", side.trapped.len()).unwrap();
        for t in &side.trapped {
            writeln!(
                out,
                "    {} — no safe square (cage: {})",
                t.piece,
                t.cage_squares.join(" "),
            )
            .unwrap();
        }
    }
}

#[cfg(test)]
#[path = "threats_view_tests.rs"]
mod tests;
