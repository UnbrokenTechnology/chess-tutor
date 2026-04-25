//! [`PawnStructureOutcome`] — pre/post snapshots of the granular
//! pawn-structure [`crate::eval::PawnsBreakdown`] on both sides.
//!
//! The CLI diffs sub-term by sub-term to turn this into sentences
//! like *"Your pawn structure weakened: doubled a pawn, exposed a
//! weak pawn."*

use super::{post_user_move, MoveAnalysis};
use crate::eval::PawnsBreakdown;
use crate::position::Position;
use crate::types::Color;

/// Pre/post snapshots of the granular pawn-structure sub-terms on
/// both sides. "Post" is the position immediately after the user's
/// move — opponent replies are intentionally excluded so narration
/// attributes pawn-structure shifts to the one move that caused them.
///
/// POV convention: `ours_*` refers to the user's pawn structure
/// (`root_stm`); `theirs_*` to the opponent's.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PawnStructureOutcome {
    pub ours_pre: PawnsBreakdown,
    pub ours_post: PawnsBreakdown,
    pub theirs_pre: PawnsBreakdown,
    pub theirs_post: PawnsBreakdown,
}

/// Snapshot pawn structure at the pre-move position and at the
/// position immediately after the user's move. Purely passes through
/// [`crate::pawns::evaluate`] — no [`crate::eval::Evaluator`]
/// priming needed since pawn-structure scoring doesn't depend on
/// piece attack tables.
pub fn compute_pawn_structure_outcome(
    ma: &MoveAnalysis,
    pre_move_pos: &Position,
    root_stm: Color,
) -> PawnStructureOutcome {
    let pre_eval = crate::pawns::evaluate(pre_move_pos);
    let ours_pre = pre_eval.breakdowns[root_stm.index()];
    let theirs_pre = pre_eval.breakdowns[(!root_stm).index()];

    let scratch = post_user_move(pre_move_pos, ma);

    let post_eval = crate::pawns::evaluate(&scratch);
    let ours_post = post_eval.breakdowns[root_stm.index()];
    let theirs_post = post_eval.breakdowns[(!root_stm).index()];

    PawnStructureOutcome {
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
    fn pawn_structure_outcome_snapshots_match_direct_eval() {
        // A capture that creates doubled + isolated white pawns.
        let pre_fen = "4k3/8/8/8/3pP3/3P4/8/4K3 w - - 0 1";
        let pre = Position::from_fen(pre_fen).unwrap();
        let mv = Move::normal(Square::E4, Square::D4);
        let ma = ma_with_pv(vec![mv], Some(0));
        let outcome = compute_pawn_structure_outcome(&ma, &pre, Color::White);

        let pre_direct = crate::pawns::evaluate(&pre).breakdowns[Color::White.index()];
        assert_eq!(outcome.ours_pre, pre_direct);

        let mut post = pre.clone();
        post.do_move(mv);
        let post_direct = crate::pawns::evaluate(&post).breakdowns[Color::White.index()];
        assert_eq!(outcome.ours_post, post_direct);

        assert!(
            outcome.ours_post.doubled.mg().0 < outcome.ours_pre.doubled.mg().0,
            "doubled mg should decrease post-capture, got pre={:?} post={:?}",
            outcome.ours_pre.doubled,
            outcome.ours_post.doubled,
        );
    }

    #[test]
    fn pawn_structure_outcome_startpos_is_symmetric() {
        let pos = Position::startpos();
        let ma = ma_with_pv(Vec::new(), None);
        let outcome = compute_pawn_structure_outcome(&ma, &pos, Color::White);
        assert_eq!(outcome.ours_pre, outcome.ours_post);
        assert_eq!(outcome.theirs_pre, outcome.theirs_post);
        assert_eq!(outcome.ours_pre, outcome.theirs_pre);
    }
}
