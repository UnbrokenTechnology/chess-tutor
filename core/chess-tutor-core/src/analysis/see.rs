//! Static Exchange Evaluation.
//!
//! Given a capture (from, to), SEE answers: "if both sides play the exchange
//! on `to` optimally, picking their least valuable attacker each time and
//! allowed to stop at any point, what is the net material swing?"
//!
//! Positive = the initiating side wins material. Zero = equal trade.
//! Negative = losing capture.
//!
//! Handles x-ray attackers (batteries, bishop-behind-queen, rook-behind-rook)
//! by re-scanning slider attackers after each capture with the capturer's
//! square cleared from the occupancy.
//!
//! Scope today: ordinary captures and en passant. Promotions are not yet
//! modelled — the promoted piece's value isn't added to the gain, and capture
//! promotions use the pawn value only. Fine for Phase 1 (the classifier cares
//! about obvious blunders, not promotion niceties); revisit when the classifier
//! actually depends on it.

use shakmaty::{attacks, Bitboard, Chess, Color, Position, Role, Square};

/// Centipawn values used by SEE. Kept local to this module because SEE's
/// material arithmetic is the only place in the core that needs a single
/// scalar per role — the positional evaluator will use richer structures.
pub fn piece_value(role: Role) -> i32 {
    match role {
        Role::Pawn => 100,
        Role::Knight => 300,
        Role::Bishop => 320,
        Role::Rook => 500,
        Role::Queen => 900,
        // Large enough that SEE never voluntarily gives up the king in the
        // capture sequence. The king can still participate as the final
        // recapturer when no opposing attackers remain.
        Role::King => 10_000,
    }
}

/// SEE for a specific capture move. Returns 0 for quiet moves.
///
/// `from` is the capturer's starting square; `to` is the target. En passant
/// is handled: when a pawn moves diagonally to an empty square matching the
/// current EP target, the victim pawn is cleared from the occupancy before
/// the exchange begins, so x-rays through the victim square resolve correctly.
pub fn see(position: &Chess, from: Square, to: Square) -> i32 {
    let board = position.board();
    let Some(attacker) = board.piece_at(from) else {
        return 0;
    };

    let (initial_gain, mut occupied) = match board.piece_at(to) {
        Some(target) => (piece_value(target.role), board.occupied()),
        None => {
            // Only en passant creates a "capture" landing on an empty square.
            if attacker.role == Role::Pawn && position.ep_square() == Some(to) {
                let victim = Square::from_coords(to.file(), from.rank());
                let mut occ = board.occupied();
                occ ^= Bitboard::from_square(victim);
                (piece_value(Role::Pawn), occ)
            } else {
                return 0;
            }
        }
    };

    // Swap list. Depth is bounded by the number of pieces that could possibly
    // attack one square (≤ 32 in any legal position); 32 slots is plenty.
    let mut gain = [0i32; 32];
    gain[0] = initial_gain;

    let mut current_from = from;
    let mut current_role = attacker.role;
    let mut current_side = attacker.color;

    let mut d: usize = 0;
    loop {
        d += 1;
        if d >= gain.len() {
            break;
        }
        gain[d] = piece_value(current_role) - gain[d - 1];

        // Pruning (CPW swap algorithm): if neither side can improve on the
        // running total, the sequence terminates. The negamax fold below
        // produces the correct answer either way.
        if gain[d].max(-gain[d - 1]) < 0 {
            break;
        }

        occupied ^= Bitboard::from_square(current_from);
        current_side = current_side.other();

        let Some((next_from, next_role)) =
            least_valuable_attacker(position, to, current_side, occupied)
        else {
            break;
        };
        current_from = next_from;
        current_role = next_role;
    }

    // Negamax fold: each side may decline to recapture, so we roll back from
    // the deepest node choosing `max(current_gain, -child_gain)` at each step.
    while d > 0 {
        d -= 1;
        gain[d] = -((-gain[d]).max(gain[d + 1]));
    }
    gain[0]
}

/// Convenience: SEE on `to` with `side`'s least valuable attacker initiating.
/// Returns `None` if the square is empty or `side` has no attacker.
pub fn see_on_square(position: &Chess, to: Square, side: Color) -> Option<i32> {
    let board = position.board();
    board.piece_at(to)?;
    let (from, _) = least_valuable_attacker(position, to, side, board.occupied())?;
    Some(see(position, from, to))
}

/// Find the least valuable attacker of `target` belonging to `side`, given the
/// current `occupied` bitboard (which the SEE loop mutates to clear used
/// attackers and reveal x-rays).
///
/// The king is only returned when the opposite side has no remaining attacker
/// on the target — otherwise the king would be moving into check.
fn least_valuable_attacker(
    position: &Chess,
    target: Square,
    side: Color,
    occupied: Bitboard,
) -> Option<(Square, Role)> {
    let board = position.board();
    let own = board.by_color(side) & occupied;

    // Pawns: invert the colour to get "squares a pawn on `target` of the
    // *other* colour would attack" — those are the squares our pawns could
    // be sitting on to attack `target`.
    let pawns = attacks::pawn_attacks(side.other(), target) & board.pawns() & own;
    if let Some(sq) = pawns.into_iter().next() {
        return Some((sq, Role::Pawn));
    }

    let knights = attacks::knight_attacks(target) & board.knights() & own;
    if let Some(sq) = knights.into_iter().next() {
        return Some((sq, Role::Knight));
    }

    // Slider attacks are re-scanned with the current `occupied` so x-rays
    // revealed by removed capturers come into view automatically.
    let diag = attacks::bishop_attacks(target, occupied);
    let bishops = diag & board.bishops() & own;
    if let Some(sq) = bishops.into_iter().next() {
        return Some((sq, Role::Bishop));
    }

    let orth = attacks::rook_attacks(target, occupied);
    let rooks = orth & board.rooks() & own;
    if let Some(sq) = rooks.into_iter().next() {
        return Some((sq, Role::Rook));
    }

    let queens = (diag | orth) & board.queens() & own;
    if let Some(sq) = queens.into_iter().next() {
        return Some((sq, Role::Queen));
    }

    // King is last: only legal when the opposite side has nothing left to
    // recapture with, else the king would walk into check.
    let kings = attacks::king_attacks(target) & board.kings() & own;
    if kings.is_empty() {
        return None;
    }
    if has_attacker(board, target, side.other(), occupied) {
        return None;
    }
    kings.into_iter().next().map(|sq| (sq, Role::King))
}

/// Fast "does `side` have any attacker on `target` under this occupancy?"
/// Used only to gate king captures; fine to recompute from scratch.
fn has_attacker(
    board: &shakmaty::Board,
    target: Square,
    side: Color,
    occupied: Bitboard,
) -> bool {
    let own = board.by_color(side) & occupied;
    if !(attacks::pawn_attacks(side.other(), target) & board.pawns() & own).is_empty() {
        return true;
    }
    if !(attacks::knight_attacks(target) & board.knights() & own).is_empty() {
        return true;
    }
    let diag = attacks::bishop_attacks(target, occupied);
    if !(diag & (board.bishops() | board.queens()) & own).is_empty() {
        return true;
    }
    let orth = attacks::rook_attacks(target, occupied);
    if !(orth & (board.rooks() | board.queens()) & own).is_empty() {
        return true;
    }
    if !(attacks::king_attacks(target) & board.kings() & own).is_empty() {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use shakmaty::fen::Fen;
    use shakmaty::CastlingMode;

    fn pos(fen: &str) -> Chess {
        fen.parse::<Fen>()
            .unwrap()
            .into_position(CastlingMode::Standard)
            .unwrap()
    }

    #[test]
    fn quiet_move_is_zero() {
        let p = pos("4k3/8/8/8/8/8/4P3/4K3 w - - 0 1");
        assert_eq!(see(&p, Square::E2, Square::E3), 0);
    }

    #[test]
    fn free_pawn_gain() {
        // c4xd5 against an undefended pawn.
        let p = pos("4k3/8/8/3p4/2P5/8/8/4K3 w - - 0 1");
        assert_eq!(see(&p, Square::C4, Square::D5), 100);
    }

    #[test]
    fn equal_pawn_trade() {
        // cxd5 recaptured by the e6 pawn — even trade.
        let p = pos("4k3/8/4p3/3p4/2P5/8/8/4K3 w - - 0 1");
        assert_eq!(see(&p, Square::C4, Square::D5), 0);
    }

    #[test]
    fn queen_takes_defended_pawn_loses() {
        // Qxd5 recaptured by e6 pawn: pawn won, queen lost.
        let p = pos("4k3/8/4p3/3p4/8/8/8/3QK3 w - - 0 1");
        assert_eq!(see(&p, Square::D1, Square::D5), 100 - 900);
    }

    #[test]
    fn knight_takes_defended_pawn_loses() {
        let p = pos("4k3/8/4p3/3p4/8/8/8/3NK3 w - - 0 1");
        assert_eq!(see(&p, Square::D1, Square::D5), 100 - 300);
    }

    #[test]
    fn rook_battery_x_ray() {
        // White Ra1 + Ra2 vs. black Ra8 + pa7.
        // Rxa7 (+P), rxa7 (−R), Rxa7 (+R). Net +P = +100.
        let p = pos("r3k3/p7/8/8/8/8/R7/R3K3 w - - 0 1");
        assert_eq!(see(&p, Square::A2, Square::A7), 100);
    }

    #[test]
    fn bishop_behind_bishop_x_ray() {
        // White Ba1 + Bb2 (battery on the a1-h8 diagonal). Black pc3
        // defended by pd4. After Bxc3, dxc3, the a1-bishop x-rays through
        // now-empty b2 to recapture. But pushing past the trade loses
        // material, so negamax stops early: SEE = 100 − 320 = −220? No —
        // let's walk it: gain[0]=100, gain[1]=220, gain[2]=−120. Negamax:
        // gain[1] = −max(−220, −120) = 120. gain[0] = −max(−100, 120) = −120.
        let p = pos("4k3/8/8/8/3p4/2p5/1B6/B3K3 w - - 0 1");
        assert_eq!(see(&p, Square::B2, Square::C3), -120);
    }

    #[test]
    fn king_cannot_recapture_into_check() {
        // Bxc3 — black king on c4 looks like a defender, but c3 is still
        // attacked by Rc1, so Kxc3 is illegal. Black has no other
        // attacker → SEE = +100.
        let p = pos("8/8/8/8/2k5/2p5/1B6/2R1K3 w - - 0 1");
        assert_eq!(see(&p, Square::B2, Square::C3), 100);
    }

    #[test]
    fn see_on_square_picks_least_valuable_attacker() {
        // Black pawn on d5 attacked by white Pc4 and Qd1. LVA is the pawn,
        // undefended, so +100.
        let p = pos("4k3/8/8/3p4/2P5/8/8/3QK3 w - - 0 1");
        assert_eq!(see_on_square(&p, Square::D5, Color::White), Some(100));
    }

    #[test]
    fn see_on_square_none_without_attackers() {
        let p = pos("4k3/8/8/3p4/8/8/8/4K3 w - - 0 1");
        assert_eq!(see_on_square(&p, Square::D5, Color::White), None);
    }

    #[test]
    fn en_passant_gain() {
        // White Pe5, black just played d7-d5; White plays exd6 e.p.,
        // capturing the d5 pawn. No defenders → +100.
        let p = pos("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1");
        assert_eq!(see(&p, Square::E5, Square::D6), 100);
    }
}
