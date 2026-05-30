//! `chess-tutor forcing <FEN>` — every forcing move available for
//! both sides.
//!
//! "Forcing" = check, capture, or promotion. The "look at every
//! forcing move first" discipline written about in the
//! [`double-fork-after-qd8`](../../teaching-positions/double-fork-after-qd8.md)
//! case study, exposed as a standing query. The agent reading this
//! never has to scan the legal-move list for `+` / `x` / `=` markers
//! by eye.
//!
//! ## Why both sides
//!
//! For the side to move, "forcing moves I can play" is direct: walk
//! the legal-move list. For the side **not** to move ("forcing moves
//! the opponent will have next") we use a null-move trick: flip side-
//! to-move, generate legal moves, classify each. The opponent's
//! forcing-move list is what the discipline cares about — *what
//! threats are loaded against me*.

use chess_tutor_engine::movegen::legal_moves_vec;
use chess_tutor_engine::position::Position;
use chess_tutor_engine::san;
use chess_tutor_engine::types::{Color, Move, MoveKind, PieceType, Square, Value};
use serde::Serialize;

use crate::piece_fmt::{color_name, piece_label, piece_type_name};

/// The full forcing-moves report, one [`SideForcing`] per colour.
#[derive(Debug, Clone, Serialize)]
pub struct ForcingView {
    pub white: SideForcing,
    pub black: SideForcing,
}

#[derive(Debug, Clone, Serialize)]
pub struct SideForcing {
    pub side: String,
    /// `true` when this is the side currently to move. The opposite
    /// side's list is computed via a null-move so the agent can see
    /// the opponent's standing forcing options.
    pub is_to_move: bool,
    pub checks: Vec<ForcingMoveView>,
    pub captures: Vec<ForcingMoveView>,
    pub promotions: Vec<ForcingMoveView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ForcingMoveView {
    pub san: String,
    pub uci: String,
    /// The piece doing the moving, in `Pf3` form (case = colour).
    pub mover: String,
    /// For captures: the piece captured. `None` for non-captures (a
    /// check that isn't a capture, a promotion that isn't a capture).
    pub captures: Option<CaptureTarget>,
    /// `true` if the move gives check to the enemy king. Captures
    /// that also give check appear in both `captures` and `checks`
    /// lists — agents auditing for "every forcing move" don't have to
    /// de-duplicate twice.
    pub gives_check: bool,
    /// Square promoted to (`q` / `r` / `b` / `n`), if any.
    pub promotion_piece: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CaptureTarget {
    pub piece: String,        // "qe6"
    pub square: String,       // "e6"
    pub piece_kind: String,   // "queen"
    pub classical_points: u8, // 9
    /// SEE verdict for the cheapest-attacker version of this capture
    /// (`"wins material"` / `"even trade"` / `"loses material"`).
    pub see_verdict: String,
}

/// Build the forcing-move report for both colours.
pub fn build(pos: &Position) -> ForcingView {
    let stm = pos.side_to_move();
    let mover = build_side(pos, stm, true);
    let other = build_opponent_side(pos);
    if stm == Color::White {
        ForcingView {
            white: mover,
            black: other,
        }
    } else {
        ForcingView {
            white: other,
            black: mover,
        }
    }
}

/// Compute forcing moves for the side currently to move. Uses the
/// legal-move generator directly.
fn build_side(pos: &Position, side: Color, is_to_move: bool) -> SideForcing {
    let mut scratch = pos.clone();
    let legal = legal_moves_vec(&mut scratch);

    let mut checks = Vec::new();
    let mut captures = Vec::new();
    let mut promotions = Vec::new();

    for mv in legal {
        let view = classify_move(pos, mv);
        if view.gives_check {
            checks.push(view.clone());
        }
        if view.captures.is_some() {
            captures.push(view.clone());
        }
        if view.promotion_piece.is_some() {
            promotions.push(view.clone());
        }
    }

    SideForcing {
        side: color_name(side).to_lowercase(),
        is_to_move,
        checks,
        captures,
        promotions,
    }
}

/// Compute forcing moves for the side **not** to move, by doing a
/// null-move (flip stm, regenerate). The opponent's standing forcing
/// options are the "what's loaded against me" surface.
fn build_opponent_side(pos: &Position) -> SideForcing {
    let opp = !pos.side_to_move();
    // Null-move flip: if the side to move is in check, this is
    // physically not a legal position (you can't pass a check). We
    // handle that gracefully by reporting an empty forcing list with
    // a note in the side label.
    if pos.in_check() {
        return SideForcing {
            side: color_name(opp).to_lowercase(),
            is_to_move: false,
            checks: Vec::new(),
            captures: Vec::new(),
            promotions: Vec::new(),
        };
    }
    let mut scratch = pos.clone();
    let st = scratch.do_null_move();
    let legal = legal_moves_vec(&mut scratch);
    let view = legal
        .into_iter()
        .map(|mv| classify_move(&scratch, mv))
        .collect::<Vec<_>>();
    scratch.undo_null_move(st);

    let mut checks = Vec::new();
    let mut captures = Vec::new();
    let mut promotions = Vec::new();
    for v in view {
        if v.gives_check {
            checks.push(v.clone());
        }
        if v.captures.is_some() {
            captures.push(v.clone());
        }
        if v.promotion_piece.is_some() {
            promotions.push(v.clone());
        }
    }

    SideForcing {
        side: color_name(opp).to_lowercase(),
        is_to_move: false,
        checks,
        captures,
        promotions,
    }
}

fn classify_move(pos: &Position, mv: Move) -> ForcingMoveView {
    let mover_piece = pos.moved_piece(mv);
    let san = san::format(pos, mv);

    let captures = if pos.is_capture(mv) {
        let target_sq = if mv.kind() == MoveKind::EnPassant {
            // En passant captures the pawn behind the destination
            // (same file as the destination, same rank as the from).
            Square::new(mv.to().file(), mv.from().rank())
        } else {
            mv.to()
        };
        pos.piece_on(target_sq).map(|tp| {
            // Cheapest-attacker SEE on this destination — same logic
            // as `square_view::see_for_cheapest_view`.
            let wins = pos.see_ge(mv, Value(1));
            let even = !wins && pos.see_ge(mv, Value::ZERO);
            let verdict = if wins {
                "wins material"
            } else if even {
                "even trade"
            } else {
                "loses material"
            };
            CaptureTarget {
                piece: piece_label(tp, target_sq),
                square: target_sq.to_algebraic(),
                piece_kind: piece_type_name(tp.kind()).to_string(),
                classical_points: tp.kind().classical_points(),
                see_verdict: verdict.to_string(),
            }
        })
    } else {
        None
    };

    let promotion_piece = if mv.kind() == MoveKind::Promotion {
        Some(promoted_to_letter(mv.promoted_to()))
    } else {
        None
    };

    ForcingMoveView {
        san,
        uci: crate::uci::format(mv),
        mover: piece_label(mover_piece, mv.from()),
        captures,
        gives_check: pos.gives_check(mv),
        promotion_piece,
    }
}

fn promoted_to_letter(pt: PieceType) -> String {
    match pt {
        PieceType::Queen => "q",
        PieceType::Rook => "r",
        PieceType::Bishop => "b",
        PieceType::Knight => "n",
        _ => "?",
    }
    .to_string()
}

pub fn render_text(view: &ForcingView) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for side in [&view.white, &view.black] {
        render_side(&mut out, side);
        writeln!(out).unwrap();
    }
    out
}

fn render_side(out: &mut String, side: &SideForcing) {
    use std::fmt::Write;
    let total = side.checks.len() + side.captures.len() + side.promotions.len();
    let header = if side.is_to_move {
        format!("{} (to move):", side.side)
    } else {
        format!("{} (opponent, via null-move):", side.side)
    };
    if total == 0 {
        writeln!(out, "{} (no forcing moves)", header).unwrap();
        return;
    }
    writeln!(out, "{}", header).unwrap();
    if !side.checks.is_empty() {
        writeln!(out, "  checks ({}):", side.checks.len()).unwrap();
        for m in &side.checks {
            writeln!(out, "    {}{}", m.san, capture_suffix(m)).unwrap();
        }
    }
    if !side.captures.is_empty() {
        writeln!(out, "  captures ({}):", side.captures.len()).unwrap();
        for m in &side.captures {
            writeln!(out, "    {}{}", m.san, capture_suffix(m)).unwrap();
        }
    }
    if !side.promotions.is_empty() {
        writeln!(out, "  promotions ({}):", side.promotions.len()).unwrap();
        for m in &side.promotions {
            let p = m.promotion_piece.as_deref().unwrap_or("?");
            writeln!(out, "    {} (promotes to {})", m.san, p).unwrap();
        }
    }
}

fn capture_suffix(m: &ForcingMoveView) -> String {
    match &m.captures {
        Some(c) => format!(
            "  — captures {} ({} pts; SEE: {})",
            c.piece, c.classical_points, c.see_verdict
        ),
        None => String::new(),
    }
}

#[cfg(test)]
#[path = "forcing_view_tests.rs"]
mod tests;
