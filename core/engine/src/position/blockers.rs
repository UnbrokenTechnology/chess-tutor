//! Blocker and pinner detection: which pieces, if removed, would expose
//! a target square to a slider attack, and which of those sliders is
//! actually pinning them.

use super::Position;
use crate::attacks::{between_bb, bishop_pseudo_attacks, rook_pseudo_attacks};
use crate::bitboard::Bitboard;
use crate::types::{Color, PieceType, Square};

impl Position {
    /// Blockers and pinners for `target`, given `candidate_attackers` as the
    /// set of potential enemy sliders to consider.
    ///
    /// A **blocker** is a piece (of either colour) that, if removed, would
    /// expose `target` to a slider attack from `candidate_attackers`. A
    /// **pinner** is the attacker on the far side of such a line whose
    /// blocker happens to be the same colour as the piece standing on
    /// `target` — i.e., the pinner pins the blocker to `target`.
    ///
    /// The "candidate_attackers" filter makes the routine dual-purpose:
    /// passing the enemy's full piece set finds the pieces pinned against
    /// our king (our blockers, their pinners); passing only the enemy's
    /// sliders against a queen square detects a pin on the queen.
    pub fn slider_blockers(
        &self,
        candidate_attackers: Bitboard,
        target: Square,
    ) -> (Bitboard, Bitboard) {
        let mut blockers = Bitboard::EMPTY;
        let mut pinners = Bitboard::EMPTY;

        // Snipers are candidate attackers that, if the board were empty
        // between them and `target`, would attack `target`. Testing only
        // empty-board line membership is a cheap first filter.
        let rq = self.pieces(PieceType::Rook) | self.pieces(PieceType::Queen);
        let bq = self.pieces(PieceType::Bishop) | self.pieces(PieceType::Queen);
        let snipers = ((rook_pseudo_attacks(target) & rq) | (bishop_pseudo_attacks(target) & bq))
            & candidate_attackers;

        if snipers.is_empty() {
            return (blockers, pinners);
        }

        // Remove all snipers from occupancy when checking who stands in the
        // line. Otherwise two aligned snipers could each mask the other
        // out of the line-of-attack calculation.
        let occupancy = self.occupied() ^ snipers;
        let target_color = self.piece_on(target).map(|p| p.color());

        for sniper_sq in snipers {
            let in_between = between_bb(target, sniper_sq) & occupancy;
            // Exactly one blocker between target and sniper: that's a
            // blocker. If it matches the colour of the piece on `target`,
            // the sniper is also a pinner (the blocker is pinned).
            if in_between.any() && !in_between.more_than_one() {
                blockers |= in_between;
                if let Some(c) = target_color {
                    if (in_between & self.pieces_by_color(c)).any() {
                        pinners = pinners | sniper_sq;
                    }
                }
            }
        }

        (blockers, pinners)
    }

    /// Our pieces pinned against our own king by any enemy slider.
    /// Reads the B3 cache (maintained by [`compute_check_info`]); falls
    /// back to nothing only in the transient kingless positions the
    /// move-generation legality filter can create.
    pub fn blockers_for_king(&self, us: Color) -> Bitboard {
        self.king_blockers[us.index()]
    }

    /// Recompute the cached check info (B3): the side-to-move's checkers
    /// and each king's blockers + pinners. Called once per `do_move` /
    /// `do_null_move` / `from_fen`, so the per-move `checkers()` /
    /// `blockers_for_king()` / SEE / `legal()` / `gives_check()` reads are
    /// O(1) lookups instead of repeated `attackers_to` / `slider_blockers`.
    ///
    /// Guards against a missing king: the move-generation do/undo legality
    /// filter can make a pseudo-legal move that captures a king, leaving a
    /// transient kingless side. Such a position is never searched (it is
    /// immediately undone), so an empty cache for it is harmless.
    pub(crate) fn compute_check_info(&mut self) {
        let stm = self.side_to_move;
        // Probe the king *bitboard* first: an empty one means a transient
        // kingless side, and `king_square` would `lsb()` an empty board
        // (index 64, which `Square::from_index` debug-asserts against).
        let stm_kings = self.pieces_of(stm, PieceType::King);
        self.checkers = if stm_kings.is_empty() {
            Bitboard::EMPTY
        } else {
            self.attackers_to(stm_kings.lsb(), self.occupied()) & self.pieces_by_color(!stm)
        };
        for color in Color::both() {
            let kings = self.pieces_of(color, PieceType::King);
            let (blockers, pinners) = if kings.is_empty() {
                (Bitboard::EMPTY, Bitboard::EMPTY)
            } else {
                self.slider_blockers(self.pieces_by_color(!color), kings.lsb())
            };
            self.king_blockers[color.index()] = blockers;
            self.king_pinners[color.index()] = pinners;
        }
    }
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startpos_has_no_blockers_for_either_king() {
        // In the starting position no slider reaches a king through a
        // single blocker, so blockers_for_king is empty for both sides.
        let p = Position::startpos();
        assert!(p.blockers_for_king(Color::White).is_empty());
        assert!(p.blockers_for_king(Color::Black).is_empty());
    }

    #[test]
    fn simple_pin_registers_both_blocker_and_pinner() {
        // Black rook on e8, white knight on e2, white king on e1, black
        // king parked off-file. The knight is pinned against the king by
        // the rook.
        let p = Position::from_fen("4rk2/8/8/8/8/8/4N3/4K3 w - - 0 1").unwrap();
        let blockers = p.blockers_for_king(Color::White);
        assert_eq!(blockers.popcount(), 1);
        assert!(blockers.contains(Square::E2));

        let enemy = p.pieces_by_color(Color::Black);
        let (_b, pinners) = p.slider_blockers(enemy, Square::E1);
        assert_eq!(pinners.popcount(), 1);
        assert!(pinners.contains(Square::E8));
    }

    #[test]
    fn two_blockers_on_a_line_produce_no_pin() {
        // Two white pawns between white king (e1) and black rook (e8):
        // neither pawn is pinned because removing one leaves another in
        // the way.
        let p = Position::from_fen("4rk2/8/8/8/8/4P3/4P3/4K3 w - - 0 1").unwrap();
        let blockers = p.blockers_for_king(Color::White);
        assert!(
            blockers.is_empty(),
            "two-piece line must not count as a blocker"
        );
    }

    #[test]
    fn diagonal_pin_is_detected() {
        // White king on e1, white knight on d2, black bishop on a5. The
        // knight blocks the a5→e1 diagonal.
        let p = Position::from_fen("4k3/8/8/b7/8/8/3N4/4K3 w - - 0 1").unwrap();
        let blockers = p.blockers_for_king(Color::White);
        assert!(blockers.contains(Square::D2));
    }
}
