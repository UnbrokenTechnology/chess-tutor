//! [`PassedPawnsOutcome`] — pre/post snapshots of the granular
//! passed-pawn [`crate::eval::PassedBreakdown`] on both sides.

use super::{post_user_move, MoveAnalysis};
use crate::eval::PassedBreakdown;
use crate::position::Position;
use crate::types::Color;

/// Pre/post snapshots of the granular passed-pawn sub-terms on both
/// sides. "Post" is the position immediately after the user's move.
/// The CLI diffs sub-term by sub-term for phrases like *"a passer
/// pushed forward"* or *"the promotion path cleared"*.
///
/// POV convention: `ours_*` refers to the user's pawns
/// (`root_stm`); `theirs_*` to the opponent's.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PassedPawnsOutcome {
    pub ours_pre: PassedBreakdown,
    pub ours_post: PassedBreakdown,
    pub theirs_pre: PassedBreakdown,
    pub theirs_post: PassedBreakdown,
}

/// Build a fresh `Evaluator` for `pos`, prime it with the standard
/// `initialize` + `pieces::evaluate` passes (the same priming
/// [`crate::eval::passed::evaluate`] expects for its attack-aware
/// free-advance bonus), and return the passed-pawn breakdown for
/// `our_color`.
fn snapshot_passed(pos: &Position, our_color: Color) -> PassedBreakdown {
    let mut e = crate::eval::Evaluator::new(pos);
    e.initialize(Color::White);
    e.initialize(Color::Black);
    crate::eval::pieces::evaluate(&mut e, Color::White);
    crate::eval::pieces::evaluate(&mut e, Color::Black);
    crate::eval::passed::evaluate(&e, our_color)
}

/// Snapshot passed-pawn state at the pre-move position and at the
/// position immediately after the user's move.
pub fn compute_passed_pawns_outcome(
    ma: &MoveAnalysis,
    pre_move_pos: &Position,
    root_stm: Color,
) -> PassedPawnsOutcome {
    let ours_pre = snapshot_passed(pre_move_pos, root_stm);
    let theirs_pre = snapshot_passed(pre_move_pos, !root_stm);

    let scratch = post_user_move(pre_move_pos, ma);

    let ours_post = snapshot_passed(&scratch, root_stm);
    let theirs_post = snapshot_passed(&scratch, !root_stm);

    PassedPawnsOutcome {
        ours_pre,
        ours_post,
        theirs_pre,
        theirs_post,
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::ma_with_pv;
    use super::*;
    use crate::types::{Move, Square};

    #[test]
    fn passed_pawns_outcome_snapshots_match_direct_eval() {
        let pre_fen = "4k3/8/3P4/8/8/8/8/4K3 w - - 0 1";
        let pre = Position::from_fen(pre_fen).unwrap();
        let mv = Move::normal(Square::D6, Square::D7);
        let ma = ma_with_pv(vec![mv], Some(0));
        let outcome = compute_passed_pawns_outcome(&ma, &pre, Color::White);

        let direct_pre = snapshot_passed(&pre, Color::White);
        assert_eq!(outcome.ours_pre, direct_pre);

        let mut post = pre.clone();
        post.do_move(mv);
        let direct_post = snapshot_passed(&post, Color::White);
        assert_eq!(outcome.ours_post, direct_post);
    }

    #[test]
    fn passed_pawns_outcome_d6_to_d7_grows_rank_bonus() {
        let pre_fen = "4k3/8/3P4/8/8/8/8/4K3 w - - 0 1";
        let pre = Position::from_fen(pre_fen).unwrap();
        let mv = Move::normal(Square::D6, Square::D7);
        let ma = ma_with_pv(vec![mv], Some(0));
        let outcome = compute_passed_pawns_outcome(&ma, &pre, Color::White);
        assert!(
            outcome.ours_post.rank_bonus.mg().0 > outcome.ours_pre.rank_bonus.mg().0,
            "rank bonus mg should grow as the passer advances, got pre={} post={}",
            outcome.ours_pre.rank_bonus.mg().0,
            outcome.ours_post.rank_bonus.mg().0,
        );
    }

    #[test]
    fn passed_pawns_outcome_startpos_is_symmetric_and_empty() {
        let pos = Position::startpos();
        let ma = ma_with_pv(Vec::new(), None);
        let outcome = compute_passed_pawns_outcome(&ma, &pos, Color::White);
        assert_eq!(outcome.ours_pre, PassedBreakdown::zero());
        assert_eq!(outcome.theirs_pre, PassedBreakdown::zero());
        assert_eq!(outcome.ours_pre, outcome.ours_post);
    }
}
