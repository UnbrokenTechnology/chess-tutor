//! `do_move` / `undo_move` plus the null-move variants. The returned
//! `StateInfo` carries everything `undo_move` needs that doesn't already
//! follow from the post-move position. The mailbox/bitboard mutation
//! helpers live here too, because they are the place where all the
//! incremental invariants (key, pawn_key, psq, non_pawn_material) are
//! maintained in lockstep.

use super::Position;
use crate::bitboard::square_bb;
use crate::psqt::psq_score;
use crate::types::{
    CastlingRights, Color, Direction, Move, MoveKind, Piece, PieceType, Rank, Square, Value,
};
use crate::zobrist;

/// State needed to undo a move: the fields that don't follow from the
/// post-move position itself. Returned by `do_move`, consumed by `undo_move`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateInfo {
    /// The piece this move captured, if any. For an en-passant capture,
    /// this is the pawn removed from the square `to ± 8`, not `to`.
    pub captured: Option<Piece>,
    /// Castling rights as they were *before* the move.
    pub castling_rights: CastlingRights,
    /// En-passant target as it was *before* the move.
    pub en_passant: Option<Square>,
    /// Halfmove clock as it was *before* the move.
    pub halfmove_clock: u16,
    /// Zobrist key as it was *before* the move.
    pub key: u64,
    /// Pawn-only Zobrist key as it was *before* the move.
    pub pawn_key: u64,
}

/// Per-square mask of castling rights that are cleared when any piece moves
/// onto or off of this square. A king move clears both of its colour's
/// rights; a rook leaving its original square clears that side's right;
/// an enemy capturing on a rook's square has the same effect. Computed
/// once at compile time.
const CASTLING_CLEAR: [CastlingRights; 64] = {
    let mut table = [CastlingRights::NONE; 64];
    table[Square::A1.index()] = CastlingRights::WHITE_QUEEN;
    table[Square::E1.index()] = CastlingRights::WHITE;
    table[Square::H1.index()] = CastlingRights::WHITE_KING;
    table[Square::A8.index()] = CastlingRights::BLACK_QUEEN;
    table[Square::E8.index()] = CastlingRights::BLACK;
    table[Square::H8.index()] = CastlingRights::BLACK_KING;
    table
};

impl Position {
    /// Apply a (pseudo-legal) move to this position, returning the state
    /// needed to undo it. The caller is responsible for legality; this
    /// method blindly performs the requested piece movements.
    pub fn do_move(&mut self, m: Move) -> StateInfo {
        let us = self.side_to_move;
        let them = !us;
        let from = m.from();
        let to = m.to();
        let kind = m.kind();
        let moving = self.board[from.index()].expect("do_move: no piece on from square");
        let moving_kind = moving.kind();

        let saved = StateInfo {
            captured: None, // filled in below
            castling_rights: self.castling_rights,
            en_passant: self.en_passant,
            halfmove_clock: self.halfmove_clock,
            key: self.key,
            pawn_key: self.pawn_key,
        };

        // Start updating the key by XOR-ing out the pre-move state pieces
        // that are about to change: side-to-move always flips, the ep and
        // castling contributions both get refreshed at the end.
        let mut key = self.key;
        key ^= zobrist::side_to_move_key();
        if let Some(ep) = self.en_passant {
            key ^= zobrist::ep_key(ep);
        }
        key ^= zobrist::castling_key(self.castling_rights);

        // Any stale ep target is gone; the only way it gets re-set is a
        // pawn double push below.
        self.en_passant = None;

        // --- Captured piece (if any) -----------------------------------
        let captured: Option<Piece>;
        if kind == MoveKind::EnPassant {
            // En-passant: the captured pawn is one rank behind `to`, not
            // on `to`. "Behind" means opposite of the moving pawn's push.
            let cap_square = to - Direction::pawn_push(us);
            let cap_piece = self.board[cap_square.index()].expect("en passant without victim");
            self.remove_piece_mailbox_and_bitboards(cap_square, cap_piece);
            key ^= zobrist::piece_square_key(cap_piece, cap_square);
            captured = Some(cap_piece);
        } else if let Some(cap_piece) = self.board[to.index()] {
            self.remove_piece_mailbox_and_bitboards(to, cap_piece);
            key ^= zobrist::piece_square_key(cap_piece, to);
            captured = Some(cap_piece);
        } else {
            captured = None;
        }

        // --- The moving piece itself -----------------------------------
        self.remove_piece_mailbox_and_bitboards(from, moving);
        key ^= zobrist::piece_square_key(moving, from);

        if kind == MoveKind::Promotion {
            let promoted = Piece::new(us, m.promoted_to());
            self.put_piece_mailbox_and_bitboards(to, promoted);
            key ^= zobrist::piece_square_key(promoted, to);
        } else {
            self.put_piece_mailbox_and_bitboards(to, moving);
            key ^= zobrist::piece_square_key(moving, to);
        }

        // --- Rook hop for castling -------------------------------------
        if kind == MoveKind::Castling {
            let (rook_from, rook_to) = castling_rook_squares(us, to);
            let rook = Piece::new(us, PieceType::Rook);
            self.remove_piece_mailbox_and_bitboards(rook_from, rook);
            self.put_piece_mailbox_and_bitboards(rook_to, rook);
            key ^= zobrist::piece_square_key(rook, rook_from);
            key ^= zobrist::piece_square_key(rook, rook_to);
        }

        // --- Castling-rights bookkeeping -------------------------------
        // A king move, a rook leaving its home square, or a piece capturing
        // on a rook's home square all clear rights. The per-square mask
        // table collapses that logic to one AND-NOT.
        self.castling_rights =
            self.castling_rights & !(CASTLING_CLEAR[from.index()] | CASTLING_CLEAR[to.index()]);

        // --- New en-passant target (pawn double push only) -------------
        // SF11 (position.cpp:792-794) records the ep square — and folds
        // it into the Zobrist key — ONLY when an enemy pawn can actually
        // capture en passant. Recording it unconditionally diverges
        // `key()` from SF for every double push with no capturer, and
        // because both the transposition table and repetition detection
        // key off `key()`, two genuinely-identical positions reached via
        // different move orders get different keys — suppressing legit
        // TT/repetition hits. A `us` pawn standing on `ep_sq` attacks
        // exactly the squares an enemy pawn would occupy to capture it,
        // so intersect that set with their pawns.
        if moving_kind == PieceType::Pawn {
            let rank_from = from.rank();
            let rank_to = to.rank();
            let is_double_push = match us {
                Color::White => rank_from == Rank::R2 && rank_to == Rank::R4,
                Color::Black => rank_from == Rank::R7 && rank_to == Rank::R5,
            };
            if is_double_push {
                let ep_sq = from + Direction::pawn_push(us);
                let their_pawns = self.pieces_of(them, PieceType::Pawn);
                if (crate::attacks::pawn_attacks_from(us, ep_sq) & their_pawns).any() {
                    self.en_passant = Some(ep_sq);
                }
            }
        }

        // --- Clocks ----------------------------------------------------
        if moving_kind == PieceType::Pawn || captured.is_some() {
            self.halfmove_clock = 0;
        } else {
            self.halfmove_clock += 1;
        }
        if us == Color::Black {
            self.fullmove_number += 1;
        }

        self.side_to_move = them;

        // --- Finish key update with new castling + new ep --------------
        key ^= zobrist::castling_key(self.castling_rights);
        if let Some(ep) = self.en_passant {
            key ^= zobrist::ep_key(ep);
        }
        self.key = key;

        StateInfo { captured, ..saved }
    }

    /// Reverse a previously applied move using the `StateInfo` returned by
    /// `do_move`. The move argument must be the same move that was passed
    /// to `do_move`.
    pub fn undo_move(&mut self, m: Move, state: StateInfo) {
        let from = m.from();
        let to = m.to();
        let kind = m.kind();

        // Whoever moved is now the not-side-to-move, because do_move flipped.
        let them = self.side_to_move;
        let us = !them;
        self.side_to_move = us;

        // Put the moving piece back. For a promotion, the piece at `to` is
        // the promoted form and must be removed, and a pawn restored at
        // `from`. For anything else, just move `to` → `from`.
        let restored_mover = if kind == MoveKind::Promotion {
            let promoted = self.board[to.index()].expect("undo_move: missing promoted piece");
            self.remove_piece_mailbox_and_bitboards(to, promoted);
            let pawn = Piece::new(us, PieceType::Pawn);
            self.put_piece_mailbox_and_bitboards(from, pawn);
            pawn
        } else {
            let mover = self.board[to.index()].expect("undo_move: missing moved piece");
            self.remove_piece_mailbox_and_bitboards(to, mover);
            self.put_piece_mailbox_and_bitboards(from, mover);
            mover
        };
        let _ = restored_mover; // clarity: we just put it back at `from`

        // Reverse the rook hop for castling.
        if kind == MoveKind::Castling {
            let (rook_from, rook_to) = castling_rook_squares(us, to);
            let rook = Piece::new(us, PieceType::Rook);
            self.remove_piece_mailbox_and_bitboards(rook_to, rook);
            self.put_piece_mailbox_and_bitboards(rook_from, rook);
        }

        // Put the captured piece back.
        if let Some(cap_piece) = state.captured {
            let cap_square = if kind == MoveKind::EnPassant {
                to - Direction::pawn_push(us)
            } else {
                to
            };
            self.put_piece_mailbox_and_bitboards(cap_square, cap_piece);
        }

        // Restore the scalar state that do_move saved.
        self.castling_rights = state.castling_rights;
        self.en_passant = state.en_passant;
        self.halfmove_clock = state.halfmove_clock;
        self.key = state.key;
        self.pawn_key = state.pawn_key;

        // Roll back the fullmove number (incremented only after a black move).
        if us == Color::Black {
            self.fullmove_number -= 1;
        }
    }

    /// Play a null (pass) move: flip the side to move, clear the
    /// en-passant target, and bump the halfmove clock. Returns the state
    /// needed to undo. Used by search for null-move pruning; the caller
    /// is responsible for the usual null-move preconditions (not in
    /// check, some non-pawn material for the moving side, etc.).
    pub fn do_null_move(&mut self) -> StateInfo {
        let saved = StateInfo {
            captured: None,
            castling_rights: self.castling_rights,
            en_passant: self.en_passant,
            halfmove_clock: self.halfmove_clock,
            key: self.key,
            pawn_key: self.pawn_key,
        };

        if let Some(ep) = self.en_passant {
            self.key ^= zobrist::ep_key(ep);
        }
        self.en_passant = None;
        self.side_to_move = !self.side_to_move;
        self.key ^= zobrist::side_to_move_key();
        self.halfmove_clock = self.halfmove_clock.saturating_add(1);

        saved
    }

    /// Reverse a previous [`do_null_move`]. Pass the `StateInfo` it
    /// returned. Restores side-to-move, en-passant, halfmove clock, and
    /// Zobrist key to their pre-null values.
    pub fn undo_null_move(&mut self, state: StateInfo) {
        self.side_to_move = !self.side_to_move;
        self.en_passant = state.en_passant;
        self.halfmove_clock = state.halfmove_clock;
        self.key = state.key;
    }

    // ----- private bitboard/mailbox mutation helpers -----------------------

    fn remove_piece_mailbox_and_bitboards(&mut self, sq: Square, piece: Piece) {
        debug_assert_eq!(
            self.board[sq.index()],
            Some(piece),
            "remove: wrong piece at {}",
            sq.to_algebraic()
        );
        self.board[sq.index()] = None;
        self.by_kind[piece.kind().index()] ^= square_bb(sq);
        self.by_color[piece.color().index()] ^= square_bb(sq);
        self.psq -= psq_score(piece, sq);
        let kind = piece.kind();
        if kind == PieceType::Pawn {
            self.pawn_key ^= zobrist::piece_square_key(piece, sq);
        } else if kind != PieceType::King {
            self.non_pawn_material[piece.color().index()] -= Value::mg_of_piece(kind);
        }
    }

    fn put_piece_mailbox_and_bitboards(&mut self, sq: Square, piece: Piece) {
        debug_assert!(
            self.board[sq.index()].is_none(),
            "put: {} is already occupied by {:?}",
            sq.to_algebraic(),
            self.board[sq.index()]
        );
        self.board[sq.index()] = Some(piece);
        self.by_kind[piece.kind().index()] ^= square_bb(sq);
        self.by_color[piece.color().index()] ^= square_bb(sq);
        self.psq += psq_score(piece, sq);
        let kind = piece.kind();
        if kind == PieceType::Pawn {
            self.pawn_key ^= zobrist::piece_square_key(piece, sq);
        } else if kind != PieceType::King {
            self.non_pawn_material[piece.color().index()] += Value::mg_of_piece(kind);
        }
    }
}

/// Given the castling king's destination square, return the rook's
/// (from, to) pair. Assumes standard chess, not Chess960.
pub(crate) fn castling_rook_squares(color: Color, king_to: Square) -> (Square, Square) {
    match (color, king_to) {
        (Color::White, Square::G1) => (Square::H1, Square::F1),
        (Color::White, Square::C1) => (Square::A1, Square::D1),
        (Color::Black, Square::G8) => (Square::H8, Square::F8),
        (Color::Black, Square::C8) => (Square::A8, Square::D8),
        _ => panic!(
            "bad castling destination for {:?}: {}",
            color,
            king_to.to_algebraic()
        ),
    }
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Score;

    // ---- do_move basics ---------------------------------------------

    #[test]
    fn do_move_e2_e4_produces_expected_position() {
        let mut p = Position::startpos();
        p.do_move(Move::normal(Square::E2, Square::E4));
        // No black pawn can capture on e3, so (matching SF11) the ep
        // square is NOT recorded — the canonical FEN has "-" in the ep
        // field, not "e3". Recording a non-capturable ep would diverge
        // key() from SF and break TT/repetition matching.
        assert_eq!(
            p.to_fen(),
            "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1"
        );
        assert_eq!(p.en_passant(), None);
        // The key must be the same as computing from scratch.
        assert_eq!(p.key(), p.compute_key_from_scratch());
    }

    #[test]
    fn do_move_double_push_sets_ep_only_when_capturable() {
        // Black pawn on d4; white plays e2-e4. A black pawn on d4 attacks
        // e3, so en passant (exd... dxe3) is possible → e3 IS recorded.
        let mut p =
            Position::from_fen("4k3/8/8/8/3p4/8/4P3/4K3 w - - 0 1").unwrap();
        p.do_move(Move::normal(Square::E2, Square::E4));
        assert_eq!(p.en_passant(), Some(Square::E3));
        assert_eq!(p.key(), p.compute_key_from_scratch());
    }

    #[test]
    fn do_move_nf3_bumps_halfmove_clock_and_flips_side() {
        let mut p = Position::startpos();
        p.do_move(Move::normal(Square::G1, Square::F3));
        assert_eq!(p.side_to_move(), Color::Black);
        assert_eq!(p.halfmove_clock(), 1);
        assert_eq!(p.piece_on(Square::F3), Some(Piece::WhiteKnight));
        assert_eq!(p.piece_on(Square::G1), None);
        assert_eq!(p.en_passant(), None);
        assert_eq!(p.key(), p.compute_key_from_scratch());
    }

    #[test]
    fn do_move_black_move_increments_fullmove_number() {
        let mut p = Position::startpos();
        p.do_move(Move::normal(Square::E2, Square::E4));
        assert_eq!(p.fullmove_number(), 1);
        p.do_move(Move::normal(Square::E7, Square::E5));
        assert_eq!(p.fullmove_number(), 2);
    }

    #[test]
    fn do_move_capture_zeroes_halfmove_clock_and_removes_victim() {
        // A white pawn on e4 captures a black pawn on d5. The halfmove
        // clock is set to 5 in the FEN so we can watch it reset to 0 when
        // the capture lands.
        let mut p =
            Position::from_fen("rnbqkbnr/ppp1pppp/8/3p4/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 5 3")
                .unwrap();
        assert_eq!(p.halfmove_clock(), 5);
        p.do_move(Move::normal(Square::E4, Square::D5));
        assert_eq!(p.piece_on(Square::D5), Some(Piece::WhitePawn));
        assert_eq!(p.piece_on(Square::E4), None);
        assert_eq!(p.halfmove_clock(), 0, "capture must reset halfmove clock");
        assert_eq!(p.side_to_move(), Color::Black);
        assert_eq!(p.key(), p.compute_key_from_scratch());
    }

    // ---- Do / undo roundtrip ----------------------------------------

    fn roundtrip(fen: &str, m: Move) {
        let p0 = Position::from_fen(fen).unwrap();
        let mut p = p0.clone();
        let st = p.do_move(m);
        p.undo_move(m, st);
        assert_eq!(p, p0, "do/undo should restore position for move {:?}", m);
        assert_eq!(
            p.key(),
            p.compute_key_from_scratch(),
            "do/undo key must match scratch computation"
        );
    }

    #[test]
    fn roundtrip_normal_pawn_move() {
        roundtrip(
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            Move::normal(Square::E2, Square::E4),
        );
    }

    #[test]
    fn roundtrip_knight_move() {
        roundtrip(
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            Move::normal(Square::G1, Square::F3),
        );
    }

    #[test]
    fn roundtrip_capture() {
        roundtrip(
            "rnbqkbnr/ppp1pppp/8/3p4/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2",
            Move::normal(Square::E4, Square::D5),
        );
    }

    #[test]
    fn roundtrip_promotion_to_queen() {
        // White pawn on a7 promotes by pushing to a8 (no capture).
        roundtrip(
            "4k3/P7/8/8/8/8/8/4K3 w - - 0 1",
            Move::promotion(Square::A7, Square::A8, PieceType::Queen),
        );
    }

    #[test]
    fn roundtrip_promotion_with_capture_to_knight() {
        // White pawn on a7 captures a rook on b8 and promotes to knight.
        roundtrip(
            "1r2k3/P7/8/8/8/8/8/4K3 w - - 0 1",
            Move::promotion(Square::A7, Square::B8, PieceType::Knight),
        );
    }

    #[test]
    fn roundtrip_en_passant_capture() {
        // White pawn on e5 captures en passant on d6, removing the black
        // pawn on d5.
        roundtrip(
            "4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 3",
            Move::en_passant(Square::E5, Square::D6),
        );
    }

    #[test]
    fn roundtrip_white_kingside_castling() {
        roundtrip(
            "4k3/8/8/8/8/8/8/4K2R w K - 0 1",
            Move::castling(Square::E1, Square::G1),
        );
    }

    #[test]
    fn roundtrip_white_queenside_castling() {
        roundtrip(
            "4k3/8/8/8/8/8/8/R3K3 w Q - 0 1",
            Move::castling(Square::E1, Square::C1),
        );
    }

    #[test]
    fn roundtrip_black_kingside_castling() {
        roundtrip(
            "4k2r/8/8/8/8/8/8/4K3 b k - 0 1",
            Move::castling(Square::E8, Square::G8),
        );
    }

    #[test]
    fn roundtrip_black_queenside_castling() {
        roundtrip(
            "r3k3/8/8/8/8/8/8/4K3 b q - 0 1",
            Move::castling(Square::E8, Square::C8),
        );
    }

    // ---- Effects of specific special moves --------------------------

    #[test]
    fn en_passant_capture_removes_the_passed_pawn() {
        let mut p = Position::from_fen("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 3").unwrap();
        p.do_move(Move::en_passant(Square::E5, Square::D6));
        assert_eq!(p.piece_on(Square::D6), Some(Piece::WhitePawn));
        assert_eq!(p.piece_on(Square::E5), None);
        assert_eq!(
            p.piece_on(Square::D5),
            None,
            "en-passant victim should be gone from d5"
        );
    }

    #[test]
    fn kingside_castling_moves_both_king_and_rook() {
        let mut p = Position::from_fen("4k3/8/8/8/8/8/8/4K2R w K - 0 1").unwrap();
        p.do_move(Move::castling(Square::E1, Square::G1));
        assert_eq!(p.piece_on(Square::G1), Some(Piece::WhiteKing));
        assert_eq!(p.piece_on(Square::F1), Some(Piece::WhiteRook));
        assert_eq!(p.piece_on(Square::E1), None);
        assert_eq!(p.piece_on(Square::H1), None);
    }

    #[test]
    fn queenside_castling_moves_both_king_and_rook() {
        let mut p = Position::from_fen("4k3/8/8/8/8/8/8/R3K3 w Q - 0 1").unwrap();
        p.do_move(Move::castling(Square::E1, Square::C1));
        assert_eq!(p.piece_on(Square::C1), Some(Piece::WhiteKing));
        assert_eq!(p.piece_on(Square::D1), Some(Piece::WhiteRook));
        assert_eq!(p.piece_on(Square::E1), None);
        assert_eq!(p.piece_on(Square::A1), None);
    }

    #[test]
    fn promotion_replaces_pawn_with_chosen_piece() {
        let mut p = Position::from_fen("4k3/P7/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        p.do_move(Move::promotion(Square::A7, Square::A8, PieceType::Queen));
        assert_eq!(p.piece_on(Square::A8), Some(Piece::WhiteQueen));
        assert_eq!(p.piece_on(Square::A7), None);
        assert_eq!(p.count(Color::White, PieceType::Pawn), 0);
        assert_eq!(p.count(Color::White, PieceType::Queen), 1);
    }

    // ---- Castling-rights bookkeeping --------------------------------

    #[test]
    fn king_move_clears_both_castling_rights_for_that_color() {
        let mut p = Position::from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").unwrap();
        p.do_move(Move::normal(Square::E1, Square::E2));
        let rights = p.castling_rights();
        assert!(!rights.contains(CastlingRights::WHITE_KING));
        assert!(!rights.contains(CastlingRights::WHITE_QUEEN));
        // Black's rights aren't affected.
        assert!(rights.contains(CastlingRights::BLACK_KING));
        assert!(rights.contains(CastlingRights::BLACK_QUEEN));
    }

    #[test]
    fn rook_leaving_home_clears_that_side() {
        let mut p = Position::from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").unwrap();
        p.do_move(Move::normal(Square::H1, Square::H2));
        let rights = p.castling_rights();
        assert!(!rights.contains(CastlingRights::WHITE_KING));
        assert!(rights.contains(CastlingRights::WHITE_QUEEN));
    }

    #[test]
    fn capturing_a_rook_on_its_home_clears_that_side() {
        // White rook on a1 captures a black rook on a8 along the open
        // a-file. Both the captured rook (its home square) and the moving
        // rook (leaving its own home) lose queenside rights.
        let mut p = Position::from_fen("r3k3/8/8/8/8/8/8/R3K3 w Qq - 0 1").unwrap();
        p.do_move(Move::normal(Square::A1, Square::A8));
        let rights = p.castling_rights();
        assert!(
            !rights.contains(CastlingRights::BLACK_QUEEN),
            "black queenside right must be gone after its rook is captured"
        );
        assert!(!rights.contains(CastlingRights::WHITE_QUEEN));
    }

    #[test]
    fn castling_itself_clears_that_color_both_rights() {
        let mut p = Position::from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").unwrap();
        p.do_move(Move::castling(Square::E1, Square::G1));
        let rights = p.castling_rights();
        assert!(!rights.contains(CastlingRights::WHITE_KING));
        assert!(!rights.contains(CastlingRights::WHITE_QUEEN));
        assert!(rights.contains(CastlingRights::BLACK_KING));
        assert!(rights.contains(CastlingRights::BLACK_QUEEN));
    }

    // ---- PSQ score ---------------------------------------------------

    #[test]
    fn startpos_psq_is_zero() {
        let p = Position::startpos();
        assert_eq!(p.psq_score(), Score::ZERO);
    }

    #[test]
    fn psq_is_incrementally_maintained_through_a_sequence() {
        // At every step of a game the stored psq must equal a from-scratch
        // recomputation. Drift here would mean remove/put is out of sync
        // with what from_fen would compute.
        let mut p = Position::startpos();
        let moves = [
            Move::normal(Square::E2, Square::E4),
            Move::normal(Square::E7, Square::E5),
            Move::normal(Square::G1, Square::F3),
            Move::normal(Square::B8, Square::C6),
            Move::normal(Square::F1, Square::C4),
            Move::normal(Square::G8, Square::F6),
        ];
        for m in moves {
            p.do_move(m);
            assert_eq!(
                p.psq_score(),
                p.compute_psq_from_scratch(),
                "psq drift after {:?}",
                m
            );
        }
    }

    #[test]
    fn psq_is_restored_by_undo() {
        let original = Position::startpos();
        let mut p = original.clone();
        let m = Move::normal(Square::E2, Square::E4);
        let st = p.do_move(m);
        p.undo_move(m, st);
        assert_eq!(p.psq_score(), original.psq_score());
    }

    #[test]
    fn capture_leaves_psq_consistent() {
        // A piece is captured: the captured piece's psq contribution must
        // vanish, and the scratch recomputation must agree.
        let mut p =
            Position::from_fen("rnbqkbnr/ppp1pppp/8/3p4/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2")
                .unwrap();
        p.do_move(Move::normal(Square::E4, Square::D5));
        assert_eq!(p.psq_score(), p.compute_psq_from_scratch());
    }

    #[test]
    fn promotion_leaves_psq_consistent() {
        let mut p = Position::from_fen("4k3/P7/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        p.do_move(Move::promotion(Square::A7, Square::A8, PieceType::Queen));
        assert_eq!(p.psq_score(), p.compute_psq_from_scratch());
    }

    #[test]
    fn castling_leaves_psq_consistent() {
        let mut p = Position::from_fen("4k3/8/8/8/8/8/8/R3K2R w KQ - 0 1").unwrap();
        p.do_move(Move::castling(Square::E1, Square::G1));
        assert_eq!(p.psq_score(), p.compute_psq_from_scratch());
    }

    #[test]
    fn en_passant_leaves_psq_consistent() {
        let mut p = Position::from_fen("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 3").unwrap();
        p.do_move(Move::en_passant(Square::E5, Square::D6));
        assert_eq!(p.psq_score(), p.compute_psq_from_scratch());
    }

    // ---- Pawn-key incremental maintenance across moves --------------

    #[test]
    fn pawn_key_is_stable_through_long_sequence() {
        let mut p = Position::startpos();
        let moves = [
            Move::normal(Square::E2, Square::E4),
            Move::normal(Square::E7, Square::E5),
            Move::normal(Square::G1, Square::F3),
            Move::normal(Square::B8, Square::C6),
            Move::normal(Square::F1, Square::C4),
            Move::normal(Square::G8, Square::F6),
        ];
        for m in moves {
            p.do_move(m);
            assert_eq!(
                p.pawn_key(),
                p.compute_pawn_key_from_scratch(),
                "pawn_key drift after {:?}",
                m
            );
        }
    }

    #[test]
    fn pawn_key_is_consistent_after_promotion() {
        // A pawn promoting disappears from the pawn structure; the promoted
        // piece doesn't enter the pawn_key.
        let mut p = Position::from_fen("4k3/P7/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        p.do_move(Move::promotion(Square::A7, Square::A8, PieceType::Queen));
        assert_eq!(p.pawn_key(), p.compute_pawn_key_from_scratch());
        // All pawns gone => just the noPawns base.
        assert_eq!(p.pawn_key(), crate::zobrist::no_pawns_key());
    }

    #[test]
    fn pawn_key_is_consistent_after_en_passant_capture() {
        let mut p = Position::from_fen("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 3").unwrap();
        p.do_move(Move::en_passant(Square::E5, Square::D6));
        assert_eq!(p.pawn_key(), p.compute_pawn_key_from_scratch());
    }

    #[test]
    fn pawn_key_is_restored_by_undo() {
        let original = Position::startpos();
        let mut p = original.clone();
        let m = Move::normal(Square::E2, Square::E4);
        let st = p.do_move(m);
        p.undo_move(m, st);
        assert_eq!(p.pawn_key(), original.pawn_key());
    }

    // ---- Long-sequence incremental maintenance ----------------------

    #[test]
    fn zobrist_key_is_stable_through_long_sequence() {
        // Play a handful of moves, and at every step verify that the
        // incrementally-maintained key equals the from-scratch key.
        let mut p = Position::startpos();
        let moves = [
            Move::normal(Square::E2, Square::E4),
            Move::normal(Square::E7, Square::E5),
            Move::normal(Square::G1, Square::F3),
            Move::normal(Square::B8, Square::C6),
            Move::normal(Square::F1, Square::C4),
            Move::normal(Square::G8, Square::F6),
        ];
        for m in moves {
            p.do_move(m);
            assert_eq!(
                p.key(),
                p.compute_key_from_scratch(),
                "key drift after {:?}",
                m
            );
        }
    }

    #[test]
    fn do_undo_across_long_sequence_restores_original() {
        let original = Position::startpos();
        let moves: [Move; 6] = [
            Move::normal(Square::E2, Square::E4),
            Move::normal(Square::E7, Square::E5),
            Move::normal(Square::G1, Square::F3),
            Move::normal(Square::B8, Square::C6),
            Move::normal(Square::F1, Square::C4),
            Move::normal(Square::G8, Square::F6),
        ];
        let mut p = original.clone();
        let mut states: Vec<StateInfo> = Vec::with_capacity(moves.len());
        for m in moves {
            states.push(p.do_move(m));
        }
        // Undo in reverse.
        for (m, st) in moves.iter().rev().zip(states.iter().rev()) {
            p.undo_move(*m, *st);
        }
        assert_eq!(p, original);
    }
}
