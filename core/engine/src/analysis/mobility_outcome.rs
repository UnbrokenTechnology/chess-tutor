//! [`MobilityOutcome`] — pre/post snapshots of per-piece-type
//! mobility (knight / bishop / rook / queen) on both sides.

use super::{post_user_move, MoveAnalysis};
use crate::eval::MobilityBreakdown;
use crate::position::Position;
use crate::types::Color;

/// Pre/post snapshots of per-piece-type mobility on both sides.
/// "Post" is the position immediately after the user's move. The CLI
/// picks the piece type with the largest |delta| per side and renders
/// a single line like *"Your knight mobility dropped (+0.60 →
/// +0.30)."*.
///
/// POV convention: `ours_*` refers to the user's pieces
/// (`root_stm`); `theirs_*` to the opponent's.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct MobilityOutcome {
    pub ours_pre: MobilityBreakdown,
    pub ours_post: MobilityBreakdown,
    pub theirs_pre: MobilityBreakdown,
    pub theirs_post: MobilityBreakdown,
}

/// Build a fresh `Evaluator` for `pos`, prime it with the standard
/// `initialize` + `pieces::evaluate` passes (the same priming that
/// populates mobility accumulation), and return the per-piece-type
/// mobility breakdown for `our_color`.
fn snapshot_mobility(pos: &Position, our_color: Color) -> MobilityBreakdown {
    let mut e = crate::eval::Evaluator::new(pos);
    e.initialize(Color::White);
    e.initialize(Color::Black);
    crate::eval::pieces::evaluate(&mut e, Color::White);
    crate::eval::pieces::evaluate(&mut e, Color::Black);
    e.mobility[our_color.index()]
}

/// Snapshot mobility at the pre-move position and at the position
/// immediately after the user's move.
pub fn compute_mobility_outcome(
    ma: &MoveAnalysis,
    pre_move_pos: &Position,
    root_stm: Color,
) -> MobilityOutcome {
    let ours_pre = snapshot_mobility(pre_move_pos, root_stm);
    let theirs_pre = snapshot_mobility(pre_move_pos, !root_stm);

    let scratch = post_user_move(pre_move_pos, ma);

    let ours_post = snapshot_mobility(&scratch, root_stm);
    let theirs_post = snapshot_mobility(&scratch, !root_stm);

    MobilityOutcome {
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
    fn mobility_outcome_snapshots_match_direct_eval() {
        let pre = Position::startpos();
        let mv = Move::normal(Square::G1, Square::F3);
        let ma = ma_with_pv(vec![mv], Some(0));
        let outcome = compute_mobility_outcome(&ma, &pre, Color::White);

        let direct_pre = snapshot_mobility(&pre, Color::White);
        assert_eq!(outcome.ours_pre, direct_pre);

        let mut post = pre.clone();
        post.do_move(mv);
        let direct_post = snapshot_mobility(&post, Color::White);
        assert_eq!(outcome.ours_post, direct_post);
    }

    #[test]
    fn mobility_outcome_nf3_increases_knight_mobility() {
        let pre = Position::startpos();
        let mv = Move::normal(Square::G1, Square::F3);
        let ma = ma_with_pv(vec![mv], Some(0));
        let outcome = compute_mobility_outcome(&ma, &pre, Color::White);
        assert!(
            outcome.ours_post.knight.mg().0 > outcome.ours_pre.knight.mg().0,
            "knight mobility should increase after Nf3, got pre={} post={}",
            outcome.ours_pre.knight.mg().0,
            outcome.ours_post.knight.mg().0,
        );
    }
}
