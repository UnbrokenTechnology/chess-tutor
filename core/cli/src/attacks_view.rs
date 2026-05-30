//! `chess-tutor attacks <FEN>` — every (attacker, target) pair.
//!
//! For each side, every piece's attack on an enemy piece, annotated
//! with target piece type, defender count, and the cheapest-attacker
//! SEE verdict for the implied capture. This is the "have I noticed
//! every threat" enumeration — the agent doesn't have to walk all 64
//! squares looking for hits.
//!
//! Empty-square attacks (a pawn covering an empty file diagonal,
//! a knight attacking an empty board square) are deliberately
//! omitted — those have no offensive payload today. `forcing`
//! covers move-into-empty-square moves that happen to give check.

use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::{Color, Move, PieceType, Value};
use serde::Serialize;

use crate::piece_fmt::{color_name, piece_label, piece_type_name};

#[derive(Debug, Clone, Serialize)]
pub struct AttacksView {
    pub white: SideAttacks,
    pub black: SideAttacks,
}

#[derive(Debug, Clone, Serialize)]
pub struct SideAttacks {
    pub side: String,
    /// Every (attacker, target) pair where `attacker` is one of this
    /// side's pieces and `target` is an enemy piece this attacker
    /// hits, given the current occupancy.
    pub attacks: Vec<AttackRecord>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AttackRecord {
    pub attacker: String,            // "Qc4"
    pub attacker_square: String,
    pub attacker_kind: String,       // "queen"
    pub target: String,              // "qe6"
    pub target_square: String,
    pub target_kind: String,
    pub target_points: u8,           // 9 for queen
    pub defender_count: u32,         // enemy defenders of the target
    pub attacker_count: u32,         // our attackers of the target
    /// SEE verdict using `attacker` as the initiating capturer.
    pub see_verdict: String,         // "wins material" / "even trade" / "loses material"
}

pub fn build(pos: &Position) -> AttacksView {
    AttacksView {
        white: build_side(pos, Color::White),
        black: build_side(pos, Color::Black),
    }
}

fn build_side(pos: &Position, side: Color) -> SideAttacks {
    let occ = pos.occupied();
    let enemy_bb = pos.pieces_by_color(!side);
    let our_bb = pos.pieces_by_color(side);
    let mut records = Vec::new();

    for target_sq in enemy_bb {
        // The king is never SEE-captured — including it as a target
        // would produce noise (every check would surface here). Real
        // checks are surfaced by `forcing`.
        let Some(target_piece) = pos.piece_on(target_sq) else {
            continue;
        };
        if target_piece.kind() == PieceType::King {
            continue;
        }
        let all_attackers = pos.attackers_to(target_sq, occ);
        let our_attackers = all_attackers & our_bb;
        if our_attackers.is_empty() {
            continue;
        }
        let defender_bb = all_attackers & enemy_bb;
        let defender_count = defender_bb.popcount();
        let attacker_count = our_attackers.popcount();

        for attacker_sq in our_attackers {
            let Some(attacker_piece) = pos.piece_on(attacker_sq) else {
                continue;
            };
            // SEE the implied capture. Kings can't initiate SEE
            // captures of defended pieces (would move into check), so
            // skip king attackers there.
            let see_verdict = if attacker_piece.kind() == PieceType::King {
                "n/a (king)".to_string()
            } else {
                let mv = Move::normal(attacker_sq, target_sq);
                let wins = pos.see_ge(mv, Value(1));
                let even = !wins && pos.see_ge(mv, Value::ZERO);
                if wins {
                    "wins material".to_string()
                } else if even {
                    "even trade".to_string()
                } else {
                    "loses material".to_string()
                }
            };
            records.push(AttackRecord {
                attacker: piece_label(attacker_piece, attacker_sq),
                attacker_square: attacker_sq.to_algebraic(),
                attacker_kind: piece_type_name(attacker_piece.kind()).to_string(),
                target: piece_label(target_piece, target_sq),
                target_square: target_sq.to_algebraic(),
                target_kind: piece_type_name(target_piece.kind()).to_string(),
                target_points: target_piece.kind().classical_points(),
                defender_count,
                attacker_count,
                see_verdict,
            });
        }
    }

    // Deterministic ordering: highest-value targets first, then by
    // attacker square. Helps an agent scanning for "what's
    // threatening my queen" land on it instantly.
    records.sort_by(|a, b| {
        b.target_points
            .cmp(&a.target_points)
            .then_with(|| a.attacker_square.cmp(&b.attacker_square))
    });

    SideAttacks {
        side: color_name(side).to_lowercase(),
        attacks: records,
    }
}

pub fn render_text(view: &AttacksView) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for side in [&view.white, &view.black] {
        if side.attacks.is_empty() {
            writeln!(out, "{} attacks: (none)", side.side).unwrap();
            writeln!(out).unwrap();
            continue;
        }
        writeln!(out, "{} attacks on enemy pieces:", side.side).unwrap();
        writeln!(
            out,
            "  {:<6}  {:<6}  {:>3}/{:<3}  {:<16}",
            "from", "→ to", "att", "def", "SEE"
        )
        .unwrap();
        for r in &side.attacks {
            writeln!(
                out,
                "  {:<6}  → {:<4}  {:>3}/{:<3}  {:<16} (target: {} pts)",
                r.attacker, r.target, r.attacker_count, r.defender_count, r.see_verdict, r.target_points,
            )
            .unwrap();
        }
        writeln!(out).unwrap();
    }
    out
}

#[cfg(test)]
#[path = "attacks_view_tests.rs"]
mod tests;
