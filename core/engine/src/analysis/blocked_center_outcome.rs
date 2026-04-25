//! [`BlockedCenterOutcome`] — pre/post counts of own central pawns
//! that have something obstructing their advance, split into two
//! cases the retrospective narrates differently:
//!
//! - **Locked** — own central pawn blocked by an *enemy pawn*. The
//!   classical "closed centre": pawn-on-pawn structure that doesn't
//!   trade. Drives the closed-centre teaching line.
//! - **Barricaded** — own central pawn blocked by any other piece
//!   (almost always a friendly piece a knight or bishop developed in
//!   front of its own pawn). The pawn can't advance until the
//!   blocker moves first, which delays bishop development on that
//!   pawn's diagonal. Drives a separate "your piece sits in front of
//!   a central pawn" teaching line — short of "closed centre" but
//!   still real positional friction.
//!
//! Stockfish's eval-internal `blocked_centre` (the multiplier on
//! `bishop_pawns`) is the union of both cases — any piece on the
//! advance square. Splitting them lets us narrate two distinct chess
//! concepts honestly without conflating them, while still consuming
//! `TermId::PiecesBishopPawns` from the fallback line whenever
//! either count moves (because the eval's loose count moved too).

use super::{post_user_move, MoveAnalysis};
use crate::bitboard::CENTER_FILES;
use crate::position::Position;
use crate::types::{Color, Direction, PieceType};

/// Pre/post counts split by blocker kind, per side.
///
/// `locked_*` = own central pawn blocked by an enemy pawn (strict
/// pawn-on-pawn).
/// `barricaded_*` = own central pawn blocked by any other piece
/// (overwhelmingly a friendly piece sitting in front of its own
/// pawn).
///
/// `locked + barricaded` = Stockfish's loose `blocked_centre` count.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct BlockedCenterOutcome {
    pub ours_locked_pre: u32,
    pub ours_locked_post: u32,
    pub theirs_locked_pre: u32,
    pub theirs_locked_post: u32,
    pub ours_barricaded_pre: u32,
    pub ours_barricaded_post: u32,
    pub theirs_barricaded_pre: u32,
    pub theirs_barricaded_post: u32,
    /// True when our side has at least one bishop AND at least one
    /// pawn on a same-colour square (post-move). When false, the
    /// blocked-centre multiplier has nothing to amplify and the
    /// narrator should stay silent rather than describe a change
    /// with no real consequence.
    pub ours_amplifies_bishop_penalty: bool,
    pub theirs_amplifies_bishop_penalty: bool,
}

impl BlockedCenterOutcome {
    /// Combined locked-count delta across both sides. Positive means
    /// new pawn-on-pawn locks appeared this move (the canonical
    /// "closed the centre" case).
    pub fn locked_total_delta(&self) -> i32 {
        (self.ours_locked_post + self.theirs_locked_post) as i32
            - (self.ours_locked_pre + self.theirs_locked_pre) as i32
    }

    /// Combined barricade-count delta across both sides. Positive
    /// means a new "piece in front of own central pawn" relationship
    /// appeared (e.g., 2.Nf3 putting a knight in front of f2).
    pub fn barricaded_total_delta(&self) -> i32 {
        (self.ours_barricaded_post + self.theirs_barricaded_post) as i32
            - (self.ours_barricaded_pre + self.theirs_barricaded_pre) as i32
    }
}

/// Strict pawn-on-pawn count: own central pawns whose advance square
/// holds an enemy pawn.
fn locked_count(pos: &Position, side: Color) -> u32 {
    let down = Direction(-Direction::pawn_push(side).0);
    let enemy_pawns = pos.pieces_of(!side, PieceType::Pawn);
    let blocked = pos.pieces_of(side, PieceType::Pawn) & enemy_pawns.shift(down) & CENTER_FILES;
    blocked.popcount()
}

/// Loose-minus-strict: own central pawns whose advance square is
/// occupied by *any non-enemy-pawn* piece. Almost always a friendly
/// piece (own knight or bishop developed in front of its own pawn);
/// rarely an enemy non-pawn piece (outpost-style block).
fn barricaded_count(pos: &Position, side: Color) -> u32 {
    let down = Direction(-Direction::pawn_push(side).0);
    let enemy_pawns = pos.pieces_of(!side, PieceType::Pawn);
    let any_block = pos.occupied();
    let non_pawn_block = any_block & !enemy_pawns;
    let blocked =
        pos.pieces_of(side, PieceType::Pawn) & non_pawn_block.shift(down) & CENTER_FILES;
    blocked.popcount()
}

/// True when `side` has a bishop and at least one same-coloured pawn
/// for that bishop — the precondition for the `bishop_pawns` penalty
/// to be non-zero, which is what the blocked-centre multiplier
/// amplifies.
fn amplifies_bishop_penalty(pos: &Position, side: Color) -> bool {
    pos.pieces_of(side, PieceType::Bishop)
        .into_iter()
        .any(|sq| pos.pawns_on_same_color_squares(side, sq) > 0)
}

/// Snapshot locked + barricaded counts at the pre-move position and
/// at the position immediately after the user's move.
pub fn compute_blocked_center_outcome(
    ma: &MoveAnalysis,
    pre_move_pos: &Position,
    root_stm: Color,
) -> BlockedCenterOutcome {
    let ours_locked_pre = locked_count(pre_move_pos, root_stm);
    let theirs_locked_pre = locked_count(pre_move_pos, !root_stm);
    let ours_barricaded_pre = barricaded_count(pre_move_pos, root_stm);
    let theirs_barricaded_pre = barricaded_count(pre_move_pos, !root_stm);

    let scratch = post_user_move(pre_move_pos, ma);

    let ours_locked_post = locked_count(&scratch, root_stm);
    let theirs_locked_post = locked_count(&scratch, !root_stm);
    let ours_barricaded_post = barricaded_count(&scratch, root_stm);
    let theirs_barricaded_post = barricaded_count(&scratch, !root_stm);
    let ours_amplifies = amplifies_bishop_penalty(&scratch, root_stm);
    let theirs_amplifies = amplifies_bishop_penalty(&scratch, !root_stm);

    BlockedCenterOutcome {
        ours_locked_pre,
        ours_locked_post,
        theirs_locked_pre,
        theirs_locked_post,
        ours_barricaded_pre,
        ours_barricaded_post,
        theirs_barricaded_pre,
        theirs_barricaded_post,
        ours_amplifies_bishop_penalty: ours_amplifies,
        theirs_amplifies_bishop_penalty: theirs_amplifies,
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::ma_with_pv;
    use super::*;
    use crate::types::{Move, Square};

    #[test]
    fn startpos_has_zero_locked_and_barricaded() {
        let pos = Position::startpos();
        assert_eq!(locked_count(&pos, Color::White), 0);
        assert_eq!(locked_count(&pos, Color::Black), 0);
        assert_eq!(barricaded_count(&pos, Color::White), 0);
        assert_eq!(barricaded_count(&pos, Color::Black), 0);
    }

    #[test]
    fn one_e4_e5_locks_centre_on_both_sides() {
        let pre = Position::from_fen(
            "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1",
        )
        .unwrap();
        let mv = Move::normal(Square::E7, Square::E5);
        let ma = ma_with_pv(vec![mv], Some(0));
        let outcome = compute_blocked_center_outcome(&ma, &pre, Color::Black);
        assert_eq!(outcome.ours_locked_post, 1, "1...e5 locks black's own e-pawn");
        assert_eq!(
            outcome.theirs_locked_post, 1,
            "1...e5 locks white's e-pawn",
        );
        assert_eq!(outcome.locked_total_delta(), 2);
        assert_eq!(outcome.barricaded_total_delta(), 0);
    }

    #[test]
    fn nf3_after_one_e4_e5_creates_an_own_piece_barricade_not_a_lock() {
        // The motivating teaching-bug case. 2.Nf3 puts a knight in
        // front of the f2 pawn — chess-wise that's "your knight
        // barricades the f-pawn" (still positional friction worth
        // teaching), not "you closed the center."
        let pre = Position::from_fen(
            "rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq e6 0 2",
        )
        .unwrap();
        let mv = Move::normal(Square::G1, Square::F3);
        let ma = ma_with_pv(vec![mv], Some(0));
        let outcome = compute_blocked_center_outcome(&ma, &pre, Color::White);

        // Locked count unchanged — Nf3 is not a pawn move.
        assert_eq!(outcome.ours_locked_pre, 1);
        assert_eq!(outcome.ours_locked_post, 1);
        assert_eq!(outcome.locked_total_delta(), 0);

        // Barricade count goes up by 1 on white's side: f2 now has
        // its own knight in front.
        assert_eq!(outcome.ours_barricaded_pre, 0);
        assert_eq!(outcome.ours_barricaded_post, 1);
        assert_eq!(outcome.barricaded_total_delta(), 1);
    }

    #[test]
    fn central_pawn_capture_does_not_change_either_count() {
        let pre = Position::from_fen(
            "rnbqkbnr/ppp1pppp/8/3p4/4P3/8/PPPP1PPP/RNBQKBNR w KQkq d6 0 2",
        )
        .unwrap();
        let mv = Move::normal(Square::E4, Square::D5);
        let ma = ma_with_pv(vec![mv], Some(0));
        let outcome = compute_blocked_center_outcome(&ma, &pre, Color::White);
        assert_eq!(outcome.locked_total_delta(), 0);
        assert_eq!(outcome.barricaded_total_delta(), 0);
    }

    #[test]
    fn amplifies_flag_false_when_no_pawns_on_bishop_color() {
        let pos = Position::from_fen("4k3/8/8/8/8/8/P1P1P1P1/2B1K3 w - - 0 1").unwrap();
        assert!(!amplifies_bishop_penalty(&pos, Color::White));
    }
}
