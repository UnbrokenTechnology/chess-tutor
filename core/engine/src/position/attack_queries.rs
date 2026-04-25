//! Attack-related queries on a position: who attacks a square, who is
//! currently giving check, whether a move captures, and the piece at a
//! move's origin.

use super::Position;
use crate::attacks::{king_attacks, knight_attacks, pawn_attacks_from};
use crate::bitboard::Bitboard;
use crate::magics::{bishop_attacks, rook_attacks};
use crate::types::{Color, Move, MoveKind, Piece, PieceType, Square};

impl Position {
    /// Bitboard of every piece (both colors) that attacks `square` given the
    /// hypothetical `occupancy`. Pass `self.occupied()` for the current
    /// occupancy; other values are useful for static exchange evaluation,
    /// where attackers are replayed after removing captured pieces.
    ///
    /// Pawn attackers are found by flipping color: "squares from which a
    /// white pawn attacks `t`" is identical to "squares a black pawn on `t`
    /// attacks", because pawn attack directions mirror each other.
    pub fn attackers_to(&self, square: Square, occupancy: Bitboard) -> Bitboard {
        let diag_sliders = self.pieces(PieceType::Bishop) | self.pieces(PieceType::Queen);
        let ortho_sliders = self.pieces(PieceType::Rook) | self.pieces(PieceType::Queen);

        (pawn_attacks_from(Color::Black, square) & self.pieces_of(Color::White, PieceType::Pawn))
            | (pawn_attacks_from(Color::White, square)
                & self.pieces_of(Color::Black, PieceType::Pawn))
            | (knight_attacks(square) & self.pieces(PieceType::Knight))
            | (bishop_attacks(square, occupancy) & diag_sliders)
            | (rook_attacks(square, occupancy) & ortho_sliders)
            | (king_attacks(square) & self.pieces(PieceType::King))
    }

    /// Bitboard of every enemy piece that is currently checking the
    /// side-to-move's king. Empty when the side to move is not in check.
    pub fn checkers(&self) -> Bitboard {
        let us = self.side_to_move;
        self.attackers_to(self.king_square(us), self.occupied()) & self.pieces_by_color(!us)
    }

    /// True when the side to move is currently in check.
    pub fn in_check(&self) -> bool {
        self.checkers().any()
    }

    /// True when `mv` captures an enemy piece. Normal moves and promotions
    /// count as captures when the destination is occupied; en-passant is
    /// always a capture; castling is never.
    pub fn is_capture(&self, mv: Move) -> bool {
        match mv.kind() {
            MoveKind::Normal | MoveKind::Promotion => self.piece_on(mv.to()).is_some(),
            MoveKind::EnPassant => true,
            MoveKind::Castling => false,
        }
    }

    /// The piece standing on `mv`'s origin square. Panics if the origin is
    /// empty — a well-formed move is always made from an occupied square.
    pub fn moved_piece(&self, mv: Move) -> Piece {
        self.piece_on(mv.from())
            .expect("moved_piece: no piece on from-square")
    }
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bitboard::square_bb;

    // ---- attackers_to -----------------------------------------------

    #[test]
    fn startpos_f3_is_attacked_by_three_white_pieces() {
        // From the starting position, f3 is attacked by the e2 pawn,
        // the g2 pawn, and the g1 knight — three white pieces, no black.
        let p = Position::startpos();
        let attackers = p.attackers_to(Square::F3, p.occupied());
        let white_attackers = attackers & p.pieces_by_color(Color::White);
        let black_attackers = attackers & p.pieces_by_color(Color::Black);
        assert_eq!(white_attackers.popcount(), 3);
        assert!(white_attackers.contains(Square::E2));
        assert!(white_attackers.contains(Square::G2));
        assert!(white_attackers.contains(Square::G1));
        assert!(black_attackers.is_empty());
    }

    #[test]
    fn startpos_central_squares_have_no_attackers() {
        let p = Position::startpos();
        for sq in &[Square::D4, Square::E4, Square::D5, Square::E5] {
            assert!(
                p.attackers_to(*sq, p.occupied()).is_empty(),
                "startpos {} should have no attackers",
                sq.to_algebraic()
            );
        }
    }

    #[test]
    fn attackers_to_finds_slider_along_a_clear_ray() {
        // White rook on a1, kings parked off rank 1. The rook attacks h1
        // with no blocker.
        let p = Position::from_fen("4k3/8/8/4K3/8/8/8/R7 w - - 0 1").unwrap();
        let attackers = p.attackers_to(Square::H1, p.occupied());
        assert!(attackers.contains(Square::A1));
    }

    #[test]
    fn attackers_to_respects_a_blocker() {
        // White rook on a1, white pawn on d1. The pawn blocks the rank-1
        // ray, so the rook does NOT attack h1.
        let p = Position::from_fen("4k3/8/8/4K3/8/8/8/R2P4 w - - 0 1").unwrap();
        let attackers_to_h1 = p.attackers_to(Square::H1, p.occupied());
        assert!(
            !attackers_to_h1.contains(Square::A1),
            "d1 pawn should block a1→h1"
        );
    }

    #[test]
    fn attackers_to_with_reduced_occupancy_sees_through_blocker() {
        // Same position, but if we pretend the d1 pawn isn't there (as SEE
        // does after a hypothetical capture), the rook now attacks h1.
        let p = Position::from_fen("4k3/8/8/4K3/8/8/8/R2P4 w - - 0 1").unwrap();
        let reduced = p.occupied() & !square_bb(Square::D1);
        let attackers_to_h1 = p.attackers_to(Square::H1, reduced);
        assert!(attackers_to_h1.contains(Square::A1));
    }

    #[test]
    fn attackers_to_finds_both_colors() {
        // Black knight on c6 attacks e5 from the starting-italian setup.
        let p =
            Position::from_fen("r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3")
                .unwrap();
        let attackers = p.attackers_to(Square::E5, p.occupied());
        // Black knight on c6 and black pawn on e5 itself is the target (not
        // an attacker). Expected black attackers of e5: Nc6.
        assert!(attackers.contains(Square::C6));
        // White's knight on f3 attacks e5 too.
        assert!(attackers.contains(Square::F3));
    }

    // ---- in_check / checkers ----------------------------------------

    #[test]
    fn in_check_is_false_from_startpos() {
        let p = Position::startpos();
        assert!(!p.in_check());
        assert!(p.checkers().is_empty());
    }

    #[test]
    fn in_check_detects_rook_along_open_file() {
        // Black rook on e7 gives check to the white king on e1 along the
        // open e-file. (Black king parked on a8 to satisfy the validator.)
        let p = Position::from_fen("k7/4r3/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        assert!(p.in_check());
        let checkers = p.checkers();
        assert_eq!(checkers.popcount(), 1);
        assert!(checkers.contains(Square::E7));
    }

    #[test]
    fn in_check_is_false_when_only_own_pieces_attack_own_king() {
        // Our own rook on the same file as our king is never a check —
        // `checkers` filters to enemy attackers only.
        let p = Position::from_fen("4k3/8/8/8/8/8/4R3/4K3 w - - 0 1").unwrap();
        assert!(!p.in_check());
    }

    // ---- is_capture / moved_piece -----------------------------------

    #[test]
    fn is_capture_distinguishes_capture_and_quiet_move() {
        // White pawn on e4, black pawn on d5. e4-e5 is quiet; e4xd5 is a
        // capture.
        let p = Position::from_fen("4k3/8/8/3p4/4P3/8/8/4K3 w - - 0 1").unwrap();
        let quiet = Move::normal(Square::E4, Square::E5);
        let capture = Move::normal(Square::E4, Square::D5);
        assert!(!p.is_capture(quiet));
        assert!(p.is_capture(capture));
    }

    #[test]
    fn is_capture_true_for_en_passant_and_false_for_castling() {
        // En-passant target on d6; white's e5 pawn can capture ep.
        let ep_pos = Position::from_fen("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 3").unwrap();
        assert!(ep_pos.is_capture(Move::en_passant(Square::E5, Square::D6)));
        // Castling: destination square is empty, but even if it weren't,
        // castling is never a capture.
        let castle_pos = Position::from_fen("4k3/8/8/8/8/8/8/R3K2R w KQ - 0 1").unwrap();
        assert!(!castle_pos.is_capture(Move::castling(Square::E1, Square::G1)));
    }

    #[test]
    fn moved_piece_returns_piece_on_from_square() {
        let p = Position::startpos();
        let m = Move::normal(Square::G1, Square::F3);
        assert_eq!(p.moved_piece(m), Piece::WhiteKnight);
    }
}
