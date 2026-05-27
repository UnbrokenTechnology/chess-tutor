//! Overloaded-defender detection — a pre-move analytical scan.
//!
//! **Built from scratch, not transliterated.** lichess-puzzler's
//! `cook.py:overloading` is a `return False` stub — lichess never implemented
//! it — so there is no reference predicate to port. This is our own design,
//! aimed at chess.com-parity ("this piece is overloaded"): a *pre-move*
//! observation, like the trapped-piece scan, not a pattern in the
//! [`super::tactic_outcome`] played/missed/walked-into chain. Overloading is a
//! static property of a position ("that piece is doing two jobs"), not a move
//! you "play", so it lives on its own analytical surface (a future coaching
//! card / overlay), keeping it out of the move-by-move tactic slots — where
//! the *exploitation* of an overload already surfaces as RemovingDefender or
//! Deflection.
//!
//! The predicate is deliberately **strict** to keep misfires low: an enemy
//! piece is overloaded only when it is the *sole* defender of two or more of
//! its own pieces that the attacker is already hitting. Material precision
//! (does winning the duty actually gain material) is a documented follow-up;
//! the structural "sole defender of ≥ 2 attacked pieces" signal is the v1.

use crate::position::Position;
use crate::types::{Color, PieceType, Square};

#[cfg(test)]
#[path = "overloading_tests.rs"]
mod tests;

/// An overloaded defender: a single piece that is the only thing holding up
/// two or more of its own pieces, each under attack — so it cannot save them
/// all once the attacker forces the issue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverloadedPiece {
    /// The overloaded defender's square.
    pub piece: Square,
    /// The friendly pieces it is the *sole* defender of, each attacked by the
    /// opponent. Always ≥ 2, in ascending square order.
    pub duties: Vec<Square>,
}

/// Scan `pos` for pieces of colour `victim` that are overloaded — the sole
/// defender of ≥ 2 of their own pieces that `!victim` (the attacker) is
/// hitting. A caller wanting "which of the opponent's pieces are overloaded so
/// I can exploit one" passes `victim = !side_to_move`; passing
/// `victim = side_to_move` surfaces the player's own overloaded pieces as a
/// warning. Pure and side-effect-free; deterministic ordering (defenders and
/// their duties both ascending by square).
pub fn find_overloaded(pos: &Position, victim: Color) -> Vec<OverloadedPiece> {
    let attacker = !victim;
    let occ = pos.occupied();
    let victim_bb = pos.pieces_by_color(victim);
    let attacker_bb = pos.pieces_by_color(attacker);

    // (defender, target) for every victim piece that is attacked by the
    // opponent and held up by exactly one friendly defender.
    let mut pairs: Vec<(Square, Square)> = Vec::new();
    for target in victim_bb {
        // The king is never won as material (attacking it is check — a
        // different matter), so it isn't an overload duty.
        if pos.piece_on(target).map(|p| p.kind()) == Some(PieceType::King) {
            continue;
        }
        // Only a piece the attacker is actually hitting is a duty worth
        // contesting.
        let attackers = pos.attackers_to(target, occ) & attacker_bb;
        if attackers.is_empty() {
            continue;
        }
        // Sole friendly defender? (A piece never attacks its own square, so
        // the target itself is never in this set.)
        let defenders = pos.attackers_to(target, occ) & victim_bb;
        if defenders.popcount() == 1 {
            let d = defenders.into_iter().next().expect("popcount == 1");
            pairs.push((d, target));
        }
    }

    // A defender shouldering ≥ 2 such duties can't hold them all → overloaded.
    pairs.sort_unstable();
    let mut out: Vec<OverloadedPiece> = Vec::new();
    let mut i = 0;
    while i < pairs.len() {
        let defender = pairs[i].0;
        let mut duties = Vec::new();
        while i < pairs.len() && pairs[i].0 == defender {
            duties.push(pairs[i].1);
            i += 1;
        }
        if duties.len() >= 2 {
            out.push(OverloadedPiece {
                piece: defender,
                duties,
            });
        }
    }
    out
}
