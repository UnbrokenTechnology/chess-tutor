//! Static exchange evaluation: `see_ge` answers "is the material balance
//! of this capture sequence at least `threshold`?" without materialising
//! the capture chain.

use super::Position;
use crate::bitboard::{square_bb, Bitboard};
use crate::magics::{bishop_attacks, rook_attacks};
use crate::types::{Color, Move, MoveKind, PieceType, Square, Value};

impl Position {
    /// Tests whether the static exchange evaluation of `mv` is greater
    /// than or equal to `threshold`.
    ///
    /// SEE resolves a sequence of captures on the destination square,
    /// assuming both sides bring in their least-valuable attacker each
    /// turn. A positive value means the side making the first capture
    /// comes out ahead by that much material; a negative value means
    /// they lose material; zero means even trade. Only normal moves
    /// are resolved by the full algorithm — castling, en-passant, and
    /// promotion short-circuit to "is zero ≥ threshold?" which is
    /// conservative but cheap.
    ///
    /// The implementation follows Stockfish's null-window style: a
    /// running `swap` balance drives an early-exit decision at each
    /// recapture step, so we never need to materialise the full chain.
    /// X-ray attackers behind a removed slider are re-revealed via
    /// `attacks_bb`. Pinned pieces (other than kings) are excluded
    /// from the attacker set whenever at least one pinner is still on
    /// the board.
    pub fn see_ge(&self, mv: Move, threshold: Value) -> bool {
        // Non-normal moves fall out of the algorithm's assumptions
        // (special captures, piece transformation on arrival, etc.).
        // Treat them as SEE == 0, which matches the reference.
        if mv.kind() != MoveKind::Normal {
            return Value::ZERO >= threshold;
        }

        let from = mv.from();
        let to = mv.to();

        // `swap` tracks the material deficit the side to move must
        // make up. `piece_value_at(to)` is the value of the first
        // captured piece; subtract the threshold to see whether that
        // alone clears the bar.
        let mut swap = piece_value_at(self, to).0 - threshold.0;
        if swap < 0 {
            return false;
        }

        // After our capture, if the opponent recaptures and takes our
        // piece, we need our piece's value to still match the deficit.
        // If the net is already ≤ 0, threshold is met no matter what
        // follows.
        swap = piece_value_at(self, from).0 - swap;
        if swap <= 0 {
            return true;
        }

        let mut occupancy = self.occupied() ^ from ^ to;
        let mover = self.piece_on(from).expect("see_ge: no piece at from");
        let mut stm = mover.color();
        let mut attackers = self.attackers_to(to, occupancy);
        // Running result bit: flipped each time the stm switches.
        // `res = 1` corresponds to the "we just lost our material at
        // this depth" state in the reference.
        let mut res = 1i32;

        // Pre-compute pinners/blockers for both sides. During SEE the
        // position evolves (attackers leave the board), but pinners
        // don't leave via the `to` square, so snapshotting once and
        // checking "does this pinner still sit in `occupancy`?" each
        // iteration is sufficient.
        let mut pinners = [Bitboard::EMPTY; 2];
        let mut blockers = [Bitboard::EMPTY; 2];
        for color in Color::both() {
            let (b, p) =
                self.slider_blockers(self.pieces_by_color(!color), self.king_square(color));
            blockers[color.index()] = b;
            pinners[color.index()] = p;
        }

        loop {
            stm = !stm;
            attackers &= occupancy;

            // Current side's attackers on the target square.
            let mut stm_attackers = attackers & self.pieces_by_color(stm);
            if stm_attackers.is_empty() {
                break;
            }

            // Exclude pinned pieces from the attacker set while at
            // least one pinner is still on the board. Kings are never
            // in the blockers set so they're automatically allowed
            // through this filter.
            if (pinners[stm.index()] & occupancy).any() {
                stm_attackers &= !blockers[stm.index()];
            }
            if stm_attackers.is_empty() {
                break;
            }

            res ^= 1;

            // Strip the least-valuable attacker. Each branch advances
            // the swap balance by that piece's value and early-exits
            // when it drops below the `res` threshold. Sliders reveal
            // x-ray attackers behind them; knights and pawns don't
            // contribute to x-rays.
            if let Some(bb) = take_least(stm_attackers, self.pieces(PieceType::Pawn)) {
                swap = Value::PAWN_MG.0 - swap;
                if swap < res {
                    break;
                }
                occupancy ^= square_bb(bb.lsb());
                attackers |= bishop_attacks(to, occupancy)
                    & (self.pieces(PieceType::Bishop) | self.pieces(PieceType::Queen));
            } else if let Some(bb) = take_least(stm_attackers, self.pieces(PieceType::Knight)) {
                swap = Value::KNIGHT_MG.0 - swap;
                if swap < res {
                    break;
                }
                occupancy ^= square_bb(bb.lsb());
                // Knights don't create x-rays.
            } else if let Some(bb) = take_least(stm_attackers, self.pieces(PieceType::Bishop)) {
                swap = Value::BISHOP_MG.0 - swap;
                if swap < res {
                    break;
                }
                occupancy ^= square_bb(bb.lsb());
                attackers |= bishop_attacks(to, occupancy)
                    & (self.pieces(PieceType::Bishop) | self.pieces(PieceType::Queen));
            } else if let Some(bb) = take_least(stm_attackers, self.pieces(PieceType::Rook)) {
                swap = Value::ROOK_MG.0 - swap;
                if swap < res {
                    break;
                }
                occupancy ^= square_bb(bb.lsb());
                attackers |= rook_attacks(to, occupancy)
                    & (self.pieces(PieceType::Rook) | self.pieces(PieceType::Queen));
            } else if let Some(bb) = take_least(stm_attackers, self.pieces(PieceType::Queen)) {
                swap = Value::QUEEN_MG.0 - swap;
                if swap < res {
                    break;
                }
                occupancy ^= square_bb(bb.lsb());
                // Queen generates both diagonal and orthogonal x-rays.
                attackers |= (bishop_attacks(to, occupancy)
                    & (self.pieces(PieceType::Bishop) | self.pieces(PieceType::Queen)))
                    | (rook_attacks(to, occupancy)
                        & (self.pieces(PieceType::Rook) | self.pieces(PieceType::Queen)));
            } else {
                // King is the last resort. If any enemy attacker
                // remains, the king can't legally capture (it would
                // move into check), so reverse the result.
                let enemy_still_attacks = (attackers & !self.pieces_by_color(stm)).any();
                return if enemy_still_attacks {
                    (res ^ 1) != 0
                } else {
                    res != 0
                };
            }
        }

        res != 0
    }
}

/// Piece mg value at a square, zero when empty.
fn piece_value_at(pos: &Position, sq: Square) -> Value {
    match pos.piece_on(sq) {
        Some(p) => Value::mg_of_piece(p.kind()),
        None => Value::ZERO,
    }
}

/// If `attackers` contains any piece of the given kind-mask, return
/// that subset; otherwise `None`. Used to walk stm's attackers from
/// least- to most-valuable.
fn take_least(attackers: Bitboard, kind_mask: Bitboard) -> Option<Bitboard> {
    let intersection = attackers & kind_mask;
    if intersection.is_empty() {
        None
    } else {
        Some(intersection)
    }
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn see_ge_free_capture_passes_threshold_zero() {
        // White knight on c3 captures an undefended black bishop on d5.
        // SEE = +bishop, easily ≥ 0.
        let p = Position::from_fen("4k3/8/8/3b4/8/2N5/8/4K3 w - - 0 1").unwrap();
        let mv = Move::normal(Square::C3, Square::D5);
        assert!(p.see_ge(mv, Value::ZERO));
    }

    #[test]
    fn see_ge_equal_trade_clears_zero_but_not_positive() {
        // White knight on c3 captures a black knight on d5 defended by
        // a black bishop on g8 (no wait — let me place it with a clear
        // single defender). Use: Nxd5 where d5 holds a black knight and
        // is defended by a black pawn on c6. Exchange value: +knight
        // (captured) − knight (recaptured) = 0.
        let p = Position::from_fen("4k3/8/2p5/3n4/8/2N5/8/4K3 w - - 0 1").unwrap();
        let mv = Move::normal(Square::C3, Square::D5);
        assert!(p.see_ge(mv, Value::ZERO));
        assert!(!p.see_ge(mv, Value(1)));
    }

    #[test]
    fn see_ge_bad_capture_fails_zero_threshold() {
        // White queen captures a black pawn defended by a black knight
        // — queen for pawn is a catastrophic trade. SEE ≈ +pawn −
        // queen, deeply negative.
        let p = Position::from_fen("4k3/8/8/3p4/1n6/8/3Q4/4K3 w - - 0 1").unwrap();
        let mv = Move::normal(Square::D2, Square::D5);
        assert!(!p.see_ge(mv, Value::ZERO));
    }

    #[test]
    fn see_ge_rook_xray_through_rook_doubles_support() {
        // Two stacked white rooks on a1/a2 capture a black rook on a8
        // which is defended by a black queen behind. Without x-ray
        // detection the attacker count looks wrong. With x-ray, the
        // exchange resolves correctly.
        //
        // Concrete exchange from white's POV: we capture a rook with a
        // rook, trade rook-for-queen, and recover with the back rook.
        // Net: +rook − rook + queen − rook ≈ +queen. Definitely ≥ 0.
        let p = Position::from_fen("r3k3/q7/8/8/8/8/R7/R3K3 w - - 0 1").unwrap();
        let mv = Move::normal(Square::A2, Square::A8);
        assert!(p.see_ge(mv, Value::ZERO));
    }

    #[test]
    fn see_ge_non_normal_move_returns_threshold_vs_zero() {
        // Castling: SEE short-circuits to VALUE_ZERO ≥ threshold. So
        // threshold 0 passes, any positive threshold fails.
        let p = Position::from_fen("4k3/8/8/8/8/8/8/R3K2R w KQ - 0 1").unwrap();
        let mv = Move::castling(Square::E1, Square::G1);
        assert!(p.see_ge(mv, Value::ZERO));
        assert!(!p.see_ge(mv, Value(1)));
    }

    #[test]
    fn see_ge_hanging_queen_returns_queen_value() {
        // An undefended black queen captured by a white pawn. SEE =
        // +queen.
        let p = Position::from_fen("4k3/8/8/4q3/3P4/8/8/4K3 w - - 0 1").unwrap();
        let mv = Move::normal(Square::D4, Square::E5);
        assert!(p.see_ge(mv, Value::ZERO));
        // Should also clear a threshold close to a queen's value.
        assert!(p.see_ge(mv, Value(2000)));
    }
}
