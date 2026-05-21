//! [`MobilityOutcome`] — pre/post snapshots of per-piece-type
//! mobility (knight / bishop / rook / queen) on both sides.

use super::{post_user_move, MoveAnalysis};
use crate::eval::MobilityBreakdown;
use crate::position::Position;
use crate::types::{Color, PieceType, Square};

/// One piece's mobility contribution at a single position snapshot.
/// Score is the engine-cp midgame mobility bonus for that one piece —
/// what [`crate::eval::MobilityBreakdown`] would aggregate per
/// piece-type, but here disaggregated to the specific square the
/// piece is on.
///
/// Surfaced so the retrospective UI can answer *"which bishop's
/// activity actually improved?"* — the per-piece-type
/// [`MobilityBreakdown`] alone can't distinguish them.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PieceMobility {
    pub square: Square,
    pub piece: PieceType,
    /// Midgame mobility bonus in engine-cp.
    pub mg: i32,
}

/// Pre/post snapshots of per-piece-type mobility on both sides plus
/// the per-piece disaggregation that lets renderers identify the
/// specific piece(s) whose mobility changed.
/// "Post" is the position immediately after the user's move.
///
/// POV convention: `ours_*` refers to the user's pieces
/// (`root_stm`); `theirs_*` to the opponent's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MobilityOutcome {
    pub ours_pre: MobilityBreakdown,
    pub ours_post: MobilityBreakdown,
    pub theirs_pre: MobilityBreakdown,
    pub theirs_post: MobilityBreakdown,
    /// Per-piece mobility for our side at the pre-move position.
    pub ours_per_piece_pre: Vec<PieceMobility>,
    /// Per-piece mobility for our side at the post-move position.
    pub ours_per_piece_post: Vec<PieceMobility>,
    pub theirs_per_piece_pre: Vec<PieceMobility>,
    pub theirs_per_piece_post: Vec<PieceMobility>,
}

/// Run the standard `initialize` + `pieces::evaluate` priming with
/// per-piece bookkeeping enabled. Returns the per-color breakdowns
/// and the per-piece records.
fn snapshot_mobility_both(
    pos: &Position,
) -> (
    MobilityBreakdown,
    MobilityBreakdown,
    Vec<PieceMobility>,
    Vec<PieceMobility>,
) {
    let mut e = crate::eval::Evaluator::new(pos);
    e.per_piece_mobility = Some(Vec::new());
    e.initialize(Color::White);
    e.initialize(Color::Black);
    crate::eval::pieces::evaluate(&mut e, Color::White);
    crate::eval::pieces::evaluate(&mut e, Color::Black);
    let mut white = Vec::new();
    let mut black = Vec::new();
    if let Some(vec) = e.per_piece_mobility.take() {
        for (sq, col, pt, score) in vec {
            let pm = PieceMobility {
                square: sq,
                piece: pt,
                mg: score.mg().0,
            };
            match col {
                Color::White => white.push(pm),
                Color::Black => black.push(pm),
            }
        }
    }
    (
        e.mobility[Color::White.index()],
        e.mobility[Color::Black.index()],
        white,
        black,
    )
}

/// Snapshot mobility at the pre-move position and at the position
/// immediately after the user's move.
pub fn compute_mobility_outcome(
    ma: &MoveAnalysis,
    pre_move_pos: &Position,
    root_stm: Color,
) -> MobilityOutcome {
    let (pre_w, pre_b, pre_w_pieces, pre_b_pieces) = snapshot_mobility_both(pre_move_pos);

    let scratch = post_user_move(pre_move_pos, ma);
    let (post_w, post_b, post_w_pieces, post_b_pieces) = snapshot_mobility_both(&scratch);

    let (ours_pre, theirs_pre, ours_per_piece_pre, theirs_per_piece_pre) = match root_stm {
        Color::White => (pre_w, pre_b, pre_w_pieces, pre_b_pieces),
        Color::Black => (pre_b, pre_w, pre_b_pieces, pre_w_pieces),
    };
    let (ours_post, theirs_post, ours_per_piece_post, theirs_per_piece_post) = match root_stm {
        Color::White => (post_w, post_b, post_w_pieces, post_b_pieces),
        Color::Black => (post_b, post_w, post_b_pieces, post_w_pieces),
    };

    MobilityOutcome {
        ours_pre,
        ours_post,
        theirs_pre,
        theirs_post,
        ours_per_piece_pre,
        ours_per_piece_post,
        theirs_per_piece_pre,
        theirs_per_piece_post,
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

        let (direct_pre_w, _, _, _) = snapshot_mobility_both(&pre);
        assert_eq!(outcome.ours_pre, direct_pre_w);

        let mut post = pre.clone();
        post.do_move(mv);
        let (direct_post_w, _, _, _) = snapshot_mobility_both(&post);
        assert_eq!(outcome.ours_post, direct_post_w);
    }

    #[test]
    fn mobility_outcome_per_piece_sums_to_per_type_breakdown() {
        // Per-piece tracker is the disaggregation of MobilityBreakdown
        // — summing the per-piece scores per piece type must match the
        // aggregate.
        let pos = Position::startpos();
        let ma = ma_with_pv(Vec::new(), None);
        let outcome = compute_mobility_outcome(&ma, &pos, Color::White);
        let mut knight_sum = 0;
        let mut bishop_sum = 0;
        for pm in &outcome.ours_per_piece_pre {
            match pm.piece {
                PieceType::Knight => knight_sum += pm.mg,
                PieceType::Bishop => bishop_sum += pm.mg,
                _ => {}
            }
        }
        assert_eq!(knight_sum, outcome.ours_pre.knight.mg().0);
        assert_eq!(bishop_sum, outcome.ours_pre.bishop.mg().0);
    }

    #[test]
    fn mobility_outcome_per_piece_only_lists_minors_and_majors() {
        // The mobility loop in pieces::evaluate iterates KNIGHT, BISHOP,
        // ROOK, QUEEN — pawns and kings shouldn't appear in the
        // per-piece tracker.
        let pos = Position::startpos();
        let ma = ma_with_pv(Vec::new(), None);
        let outcome = compute_mobility_outcome(&ma, &pos, Color::White);
        for pm in &outcome.ours_per_piece_pre {
            assert!(
                matches!(
                    pm.piece,
                    PieceType::Knight | PieceType::Bishop | PieceType::Rook | PieceType::Queen
                ),
                "unexpected piece type in mobility tracker: {:?}",
                pm.piece,
            );
        }
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
