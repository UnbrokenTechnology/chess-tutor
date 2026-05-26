//! `do_move` / `undo_move` plus the null-move variants. The returned
//! `StateInfo` carries everything `undo_move` needs that doesn't already
//! follow from the post-move position. The mailbox/bitboard mutation
//! helpers live here too, because they are the place where all the
//! incremental invariants (key, pawn_key, psq, non_pawn_material) are
//! maintained in lockstep.

use super::Position;
use crate::bitboard::{square_bb, Bitboard};
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
    /// B3 check-info cache as it was *before* the move, restored on undo
    /// (cheaper than recomputing). See [`Position::compute_check_info`].
    pub(crate) checkers: Bitboard,
    pub(crate) king_blockers: [Bitboard; 2],
    pub(crate) king_pinners: [Bitboard; 2],
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
            checkers: self.checkers,
            king_blockers: self.king_blockers,
            king_pinners: self.king_pinners,
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

        // B3: refresh the check-info cache for the new side to move.
        self.compute_check_info();

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
        // B3: restore the pre-move check-info cache.
        self.checkers = state.checkers;
        self.king_blockers = state.king_blockers;
        self.king_pinners = state.king_pinners;

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
            checkers: self.checkers,
            king_blockers: self.king_blockers,
            king_pinners: self.king_pinners,
        };

        if let Some(ep) = self.en_passant {
            self.key ^= zobrist::ep_key(ep);
        }
        self.en_passant = None;
        self.side_to_move = !self.side_to_move;
        self.key ^= zobrist::side_to_move_key();
        self.halfmove_clock = self.halfmove_clock.saturating_add(1);

        // B3: the board is unchanged but the side to move flipped, so the
        // checkers (against the new mover's king) must be refreshed; the
        // per-color king_blockers are board-only and unchanged, but
        // recomputing both is simplest and still O(1)-per-node.
        self.compute_check_info();

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
        // B3: restore the pre-null check-info cache.
        self.checkers = state.checkers;
        self.king_blockers = state.king_blockers;
        self.king_pinners = state.king_pinners;
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
#[path = "make_move_tests.rs"]
mod tests;
