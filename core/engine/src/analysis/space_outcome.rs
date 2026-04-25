//! [`SpaceOutcome`] — pre/post snapshots of the
//! [`crate::eval::space::evaluate`] term and each side's piece count,
//! captured at the position immediately after the user's move.
//!
//! Like the other state-based positional outcomes (king safety,
//! mobility, etc.), this diffs against ply 1 rather than the settled
//! ply: claiming "pieces traded" when the trades only happen later
//! in the PV would attribute opponent's-future-replies-and-our-future-
//! responses to the one move the user actually made. The narrator
//! therefore only fires when the user's move itself was a capture
//! (the opponent's piece count drops by 1) and the opponent had a
//! meaningful space advantage to dilute.

use super::{post_user_move, MoveAnalysis};
use crate::position::Position;
use crate::types::Color;

/// Pre/post `space` Score (mg cp) and piece counts for both sides at
/// the settled ply.
///
/// POV convention: `ours_*` = user's side (`root_stm`); `theirs_*` =
/// opponent.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SpaceOutcome {
    pub ours_space_pre_mg: i32,
    pub ours_space_post_mg: i32,
    pub theirs_space_pre_mg: i32,
    pub theirs_space_post_mg: i32,
    /// Total piece count for our side (any piece type) — drives the
    /// quadratic `(piece_count − 1)²` weight in the space term.
    pub ours_piece_count_pre: u32,
    pub ours_piece_count_post: u32,
    pub theirs_piece_count_pre: u32,
    pub theirs_piece_count_post: u32,
}

impl SpaceOutcome {
    pub fn ours_space_delta_mg(&self) -> i32 {
        self.ours_space_post_mg - self.ours_space_pre_mg
    }
    pub fn theirs_space_delta_mg(&self) -> i32 {
        self.theirs_space_post_mg - self.theirs_space_pre_mg
    }
    pub fn ours_piece_count_dropped(&self) -> bool {
        self.ours_piece_count_post < self.ours_piece_count_pre
    }
    pub fn theirs_piece_count_dropped(&self) -> bool {
        self.theirs_piece_count_post < self.theirs_piece_count_pre
    }
}

fn space_mg(pos: &Position, side: Color) -> i32 {
    let mut e = crate::eval::Evaluator::new(pos);
    e.initialize(Color::White);
    e.initialize(Color::Black);
    crate::eval::pieces::evaluate(&mut e, Color::White);
    crate::eval::pieces::evaluate(&mut e, Color::Black);
    crate::eval::space::evaluate(&e, side).mg().0
}

fn piece_count(pos: &Position, side: Color) -> u32 {
    pos.pieces_by_color(side).popcount()
}

/// Snapshot space score + piece counts at the pre-move position and
/// at the position immediately after the user's move. At ply 1 the
/// user's piece count is invariant (their move can't remove their
/// own pieces), so `ours_piece_count_pre == ours_piece_count_post` —
/// the field is kept for symmetry but the narrator only consults
/// `theirs_*`.
pub fn compute_space_outcome(
    ma: &MoveAnalysis,
    pre_move_pos: &Position,
    root_stm: Color,
) -> SpaceOutcome {
    let ours_space_pre_mg = space_mg(pre_move_pos, root_stm);
    let theirs_space_pre_mg = space_mg(pre_move_pos, !root_stm);
    let ours_piece_count_pre = piece_count(pre_move_pos, root_stm);
    let theirs_piece_count_pre = piece_count(pre_move_pos, !root_stm);

    let scratch = post_user_move(pre_move_pos, ma);

    let ours_space_post_mg = space_mg(&scratch, root_stm);
    let theirs_space_post_mg = space_mg(&scratch, !root_stm);
    let ours_piece_count_post = piece_count(&scratch, root_stm);
    let theirs_piece_count_post = piece_count(&scratch, !root_stm);

    SpaceOutcome {
        ours_space_pre_mg,
        ours_space_post_mg,
        theirs_space_pre_mg,
        theirs_space_post_mg,
        ours_piece_count_pre,
        ours_piece_count_post,
        theirs_piece_count_pre,
        theirs_piece_count_post,
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::ma_with_pv;
    use super::*;
    use crate::types::{Move, Square};

    #[test]
    fn space_outcome_records_startpos_symmetry() {
        let pos = Position::startpos();
        let ma = ma_with_pv(Vec::new(), None);
        let outcome = compute_space_outcome(&ma, &pos, Color::White);
        assert_eq!(outcome.ours_space_pre_mg, outcome.theirs_space_pre_mg);
        assert_eq!(outcome.ours_piece_count_pre, 16);
        assert_eq!(outcome.theirs_piece_count_pre, 16);
    }

    #[test]
    fn space_outcome_drops_piece_count_after_capture_pv() {
        // Minimal capture position: black knight on f6, white knight
        // on e4. White's Nxf6 is a single-ply PV that drops black's
        // piece count by one.
        let pre = Position::from_fen("4k3/8/5n2/8/4N3/8/8/4K3 w - - 0 1").unwrap();
        let ma = ma_with_pv(vec![Move::normal(Square::E4, Square::F6)], Some(0));
        let outcome = compute_space_outcome(&ma, &pre, Color::White);
        assert_eq!(outcome.theirs_piece_count_pre, 2);
        assert_eq!(outcome.theirs_piece_count_post, 1);
        assert!(outcome.theirs_piece_count_dropped());
    }
}
