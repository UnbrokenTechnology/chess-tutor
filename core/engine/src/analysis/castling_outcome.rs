//! [`CastlingOutcome`] — pre/post castling-rights status for both
//! sides, paired with the post-move trapped-rook penalty.
//!
//! Stockfish's classical evaluation doubles the
//! [`crate::eval::PiecesBreakdown::trapped_rook`] penalty when the
//! trapped side has no remaining castling rights — the rook has
//! literally no way to escape, so the positional cost is permanent.
//! This outcome captures the raw signals that drive the multiplier;
//! the CLI narrator turns them into a teaching line when the user's
//! move is the one that flipped castling rights from available to
//! gone *and* a rook is currently boxed in by the king.

use super::{post_user_move, MoveAnalysis};
use crate::position::Position;
use crate::types::{CastlingRights, Color};

/// Pre/post castling-rights availability per side, plus the
/// post-move trapped-rook penalty (mg cp) so the narrator can gate
/// on "is there actually a rook to amplify the penalty for?".
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CastlingOutcome {
    pub ours_could_castle_pre: bool,
    pub ours_could_castle_post: bool,
    pub theirs_could_castle_pre: bool,
    pub theirs_could_castle_post: bool,
    /// Post-move trapped-rook penalty for our side, in mg cp.
    /// Negative = an active penalty. Zero = no rook is trapped.
    pub ours_trapped_rook_post_mg: i32,
    pub theirs_trapped_rook_post_mg: i32,
}

impl CastlingOutcome {
    /// True when our side just lost the last of its castling rights
    /// on this move (had at least one side available pre, has none
    /// post).
    pub fn ours_lost_castling(&self) -> bool {
        self.ours_could_castle_pre && !self.ours_could_castle_post
    }
    pub fn theirs_lost_castling(&self) -> bool {
        self.theirs_could_castle_pre && !self.theirs_could_castle_post
    }
}

fn could_castle(pos: &Position, side: Color) -> bool {
    pos.castling_rights()
        .intersects(CastlingRights::for_color(side))
}

fn trapped_rook_mg(pos: &Position, side: Color) -> i32 {
    let mut e = crate::eval::Evaluator::new(pos);
    e.initialize(Color::White);
    e.initialize(Color::Black);
    let w = crate::eval::pieces::evaluate(&mut e, Color::White);
    let b = crate::eval::pieces::evaluate(&mut e, Color::Black);
    match side {
        Color::White => w.trapped_rook.mg().0,
        Color::Black => b.trapped_rook.mg().0,
    }
}

/// Snapshot castling-rights status pre and post the user's move, plus
/// the post-move trapped-rook penalty for both sides.
pub fn compute_castling_outcome(
    ma: &MoveAnalysis,
    pre_move_pos: &Position,
    root_stm: Color,
) -> CastlingOutcome {
    let ours_could_castle_pre = could_castle(pre_move_pos, root_stm);
    let theirs_could_castle_pre = could_castle(pre_move_pos, !root_stm);

    let scratch = post_user_move(pre_move_pos, ma);

    let ours_could_castle_post = could_castle(&scratch, root_stm);
    let theirs_could_castle_post = could_castle(&scratch, !root_stm);
    let ours_trapped_rook_post_mg = trapped_rook_mg(&scratch, root_stm);
    let theirs_trapped_rook_post_mg = trapped_rook_mg(&scratch, !root_stm);

    CastlingOutcome {
        ours_could_castle_pre,
        ours_could_castle_post,
        theirs_could_castle_pre,
        theirs_could_castle_post,
        ours_trapped_rook_post_mg,
        theirs_trapped_rook_post_mg,
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::ma_with_pv;
    use super::*;
    use crate::types::{Move, Square};

    #[test]
    fn startpos_both_sides_can_castle() {
        let pos = Position::startpos();
        assert!(could_castle(&pos, Color::White));
        assert!(could_castle(&pos, Color::Black));
    }

    #[test]
    fn king_move_loses_castling_rights() {
        // White king on e1 with both rooks; pawns clear away from
        // king/rook ranks so the king move is legal. Black king on
        // e8 stays put.
        let pre =
            Position::from_fen("4k3/8/8/8/8/8/PPPPPPPP/R3K2R w KQ - 0 1").unwrap();
        let mv = Move::normal(Square::E1, Square::E2);
        let ma = ma_with_pv(vec![mv], Some(0));
        let outcome = compute_castling_outcome(&ma, &pre, Color::White);
        assert!(outcome.ours_could_castle_pre);
        assert!(!outcome.ours_could_castle_post);
        assert!(outcome.ours_lost_castling());
    }

    #[test]
    fn rook_capture_strips_opponent_castling_right() {
        // White rook on a1, black king on g8 with king-side rook on
        // h8 and queen-side rook on a8. White's Rxa8+ removes black's
        // queen-side castling right while leaving king-side intact —
        // black still "could castle" overall, so theirs_lost_castling
        // is false. (Rights flip from "both" to "only kingside",
        // meaning could_castle stays true.)
        let pre =
            Position::from_fen("r5k1/8/8/8/8/8/8/R3K3 w Qq - 0 1").unwrap();
        let mv = Move::normal(Square::A1, Square::A8);
        let ma = ma_with_pv(vec![mv], Some(0));
        let outcome = compute_castling_outcome(&ma, &pre, Color::White);
        assert!(outcome.theirs_could_castle_pre);
        // Black's queen-side right was the only one in this FEN — so
        // post-capture black can no longer castle on either side.
        assert!(!outcome.theirs_could_castle_post);
        assert!(outcome.theirs_lost_castling());
    }

    #[test]
    fn trapped_rook_penalty_present_when_king_blocks_rook() {
        // White king on g1, white rook on h1 — king is "kingside"
        // (file > E) AND the rook is to its right (NOT left of king).
        // The pieces.rs heuristic fires `king_is_queenside ==
        // rook_is_left_of_king` (false == false → trapped). With
        // pawns on rank 2 the rook also has `mob <= 3` (h-file
        // blocked by h2 pawn). With castling rights gone the
        // penalty doubles.
        let pos = Position::from_fen("4k3/8/8/8/8/8/PPPPPPPP/6KR w - - 0 1").unwrap();
        let mg = trapped_rook_mg(&pos, Color::White);
        assert!(
            mg < 0,
            "expected an active trapped-rook penalty (negative mg), got {mg}",
        );
    }
}
