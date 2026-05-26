//! Attack-related queries on a position: who attacks a square, who is
//! currently giving check, whether a move captures, and the piece at a
//! move's origin.

use super::Position;
use crate::attacks::{aligned, king_attacks, knight_attacks, pawn_attacks_from, rook_pseudo_attacks};
use crate::bitboard::{square_bb, Bitboard};
use crate::magics::{bishop_attacks, queen_attacks, rook_attacks};
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

    /// Tests whether the pseudo-legal move `mv` gives check to the
    /// opponent king **without making the move**. Mirrors Stockfish 11's
    /// `Position::gives_check` (position.cpp:627-678): direct check, then
    /// discovered check, then the promotion / en-passant / castling
    /// special cases. Used by qsearch futility pruning, which must skip
    /// checking moves before pruning them. Computed on demand (no cached
    /// `checkSquares`/`blockersForKing` — that caching is a possible NPS
    /// follow-up).
    pub fn gives_check(&self, mv: Move) -> bool {
        let us = self.side_to_move;
        let them = !us;
        let from = mv.from();
        let to = mv.to();
        let ksq = self.king_square(them);
        let from_bb = square_bb(from);
        let occ = self.occupied();

        // Direct check: the moved piece, arriving on `to`, attacks the
        // enemy king. Sliders use the post-move occupancy (the from-square
        // is vacated; the piece's own to-square never blocks its outgoing
        // rays). Non-sliders are occupancy-independent.
        let direct = match self.moved_piece(mv).kind() {
            PieceType::Pawn => pawn_attacks_from(us, to).contains(ksq),
            PieceType::Knight => knight_attacks(to).contains(ksq),
            PieceType::King => false,
            PieceType::Bishop => bishop_attacks(to, occ ^ from_bb).contains(ksq),
            PieceType::Rook => rook_attacks(to, occ ^ from_bb).contains(ksq),
            PieceType::Queen => queen_attacks(to, occ ^ from_bb).contains(ksq),
        };
        if direct {
            return true;
        }

        // Discovered check: `from` blocks one of OUR sliders from the
        // enemy king, and the move leaves that line. `blockers_for_king`
        // for the enemy is exactly the set of pieces blocking our sliders
        // from their king (snipers = our pieces), so a blocker on `from`
        // means moving it uncovers our slider's check — unless `to` stays
        // collinear with the slider's ray (`aligned`).
        if self.blockers_for_king(them).contains(from) && !aligned(from, to, ksq) {
            return true;
        }

        match mv.kind() {
            MoveKind::Normal => false,
            MoveKind::Promotion => {
                // The promoted piece, on `to` with `from` vacated, attacks ksq.
                let occ_after = occ ^ from_bb;
                match mv.promoted_to() {
                    PieceType::Knight => knight_attacks(to).contains(ksq),
                    PieceType::Bishop => bishop_attacks(to, occ_after).contains(ksq),
                    PieceType::Rook => rook_attacks(to, occ_after).contains(ksq),
                    PieceType::Queen => queen_attacks(to, occ_after).contains(ksq),
                    _ => false,
                }
            }
            MoveKind::EnPassant => {
                // Direct and ordinary discovered checks are handled above;
                // the only extra case is a discovered check opened by
                // removing the captured pawn (on the to-file, from-rank).
                let capsq = Square::new(to.file(), from.rank());
                let b = (occ ^ from_bb ^ square_bb(capsq)) | square_bb(to);
                let rq = self.pieces_of(us, PieceType::Rook) | self.pieces_of(us, PieceType::Queen);
                let bq =
                    self.pieces_of(us, PieceType::Bishop) | self.pieces_of(us, PieceType::Queen);
                (rook_attacks(ksq, b) & rq).any() || (bishop_attacks(ksq, b) & bq).any()
            }
            MoveKind::Castling => {
                // Only the rook can give check after castling (a king never
                // does). In our encoding `to` is the king's destination;
                // the rook hops `rfrom`→`rto`.
                let (rfrom, rto) = super::make_move::castling_rook_squares(us, to);
                let occ_after = (occ ^ from_bb ^ square_bb(rfrom)) | square_bb(to) | square_bb(rto);
                rook_pseudo_attacks(rto).contains(ksq) && rook_attacks(rto, occ_after).contains(ksq)
            }
        }
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

    // ---- gives_check ------------------------------------------------

    /// The authoritative check for `gives_check`: for every legal move in
    /// a diverse set of positions, the no-make prediction must equal the
    /// make/unmake ground truth (`do_move; in_check; undo`). Covers direct
    /// checks, discovered checks, promotions, en passant, and castling.
    #[test]
    fn gives_check_matches_make_unmake_oracle() {
        use crate::movegen::{generate_legal_moves, MoveList};
        let fens = [
            // Start position.
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            // Kiwipete: castling both sides, pins, many captures/checks.
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            // Same, black to move.
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R b KQkq - 0 1",
            // En passant capturable (white e5 x d6) — exercises the ep
            // discovered-check branch.
            "4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1",
            // Pawn one step from promotion; g7-g8=Q/R checks the e8 king
            // along the back rank (B/N do not). Legal: the g7 pawn attacks
            // f8/h8, not e8, so black isn't already in check.
            "4k3/6P1/8/8/8/8/8/4K3 w - - 0 1",
            // Position 3 (Roycroft) — sliders, discovered-check geometry.
            "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
        ];
        for fen in fens {
            let mut p = Position::from_fen(fen).unwrap();
            let mut moves = MoveList::new();
            generate_legal_moves(&mut p, &mut moves);
            for &m in &moves {
                let predicted = p.gives_check(m);
                let st = p.do_move(m);
                let actual = p.in_check();
                p.undo_move(m, st);
                assert_eq!(
                    predicted, actual,
                    "gives_check({m:?}) = {predicted} but make/unmake oracle = {actual} in FEN {fen}"
                );
            }
        }
    }
}
