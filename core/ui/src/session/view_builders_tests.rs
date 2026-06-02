use super::*;
use chess_tutor_engine::types::Value;

/// The eval bar labels the *viewed* position, but its score is the
/// analysis of the move that reached it — rooted one ply earlier. So a
/// position that is mate-in-1 is reached by a move whose line is
/// `mate_in(2)`; the bar must drop that played ply and read "M1", in
/// moves, not the old raw-plies "M2".
#[test]
fn eval_bar_mate_drops_the_played_ply_and_renders_moves() {
    // White-POV mate score for the move that *reached* a mate-in-1
    // position (line = played move + the mate = 2 plies from the root).
    let (ratio, label) = eval_bar_fill_and_label(Some(Value::mate_in(2)));
    assert_eq!(label, "M1");
    assert_eq!(ratio, 1.0);
}

#[test]
fn eval_bar_mate_on_board_shows_bare_hash() {
    // The reaching move itself delivered mate (line = 1 ply): the viewed
    // position is checkmate, so there's no countdown to show.
    let (_, label) = eval_bar_fill_and_label(Some(Value::mate_in(1)));
    assert_eq!(label, "#");
}

#[test]
fn eval_bar_longer_mate_renders_moves_after_dropping_a_ply() {
    // 7 plies from the analysis root -> 6 from the viewed position ->
    // mate in 3 moves.
    let (_, label) = eval_bar_fill_and_label(Some(Value::mate_in(7)));
    assert_eq!(label, "M3");
}

#[test]
fn eval_bar_black_mate_is_signed() {
    // Black winning: mated_in(3) is 3 plies from the root -> 2 from the
    // viewed position -> mate in 1 move, on the black side.
    let (ratio, label) = eval_bar_fill_and_label(Some(Value::mated_in(3)));
    assert_eq!(label, "-M1");
    assert_eq!(ratio, 0.0);
}

#[test]
fn eval_bar_cp_score_uses_pawn_eg_scale() {
    // One endgame pawn reads as +1.00 (chess.com-aligned), not raw cp.
    let (_, label) = eval_bar_fill_and_label(Some(Value(Value::PAWN_EG.0)));
    assert_eq!(label, "+1.00");
}

#[test]
fn eval_bar_none_is_neutral_dash() {
    let (ratio, label) = eval_bar_fill_and_label(None);
    assert_eq!(label, "—");
    assert_eq!(ratio, 0.5);
}

// ---- Game-review verdict → summary-tier mapping (step 6) ----------

#[test]
fn verdict_tier_folds_best_available_into_best() {
    use chess_tutor_engine::analysis::MoveVerdict;
    use crate::view::ReviewVerdictTier;
    // Both "as good as it gets" verdicts collapse to one summary bucket.
    assert_eq!(verdict_tier(MoveVerdict::Best), ReviewVerdictTier::Best);
    assert_eq!(
        verdict_tier(MoveVerdict::BestAvailable),
        ReviewVerdictTier::Best
    );
}

#[test]
fn verdict_tier_maps_remaining_verdicts_one_to_one() {
    use chess_tutor_engine::analysis::MoveVerdict;
    use crate::view::ReviewVerdictTier;
    assert_eq!(verdict_tier(MoveVerdict::Good), ReviewVerdictTier::Good);
    assert_eq!(
        verdict_tier(MoveVerdict::Inaccuracy),
        ReviewVerdictTier::Inaccuracy
    );
    assert_eq!(
        verdict_tier(MoveVerdict::Mistake),
        ReviewVerdictTier::Mistake
    );
    assert_eq!(verdict_tier(MoveVerdict::Miss), ReviewVerdictTier::Miss);
    assert_eq!(
        verdict_tier(MoveVerdict::Blunder),
        ReviewVerdictTier::Blunder
    );
}

#[test]
fn review_verdict_tier_all_covers_every_tier_in_display_order() {
    use crate::view::ReviewVerdictTier;
    // The renderer iterates ALL to lay out the tally rows; the order is
    // the chess.com display order (Best → Blunder).
    assert_eq!(
        ReviewVerdictTier::ALL,
        [
            ReviewVerdictTier::Best,
            ReviewVerdictTier::Good,
            ReviewVerdictTier::Inaccuracy,
            ReviewVerdictTier::Mistake,
            ReviewVerdictTier::Miss,
            ReviewVerdictTier::Blunder,
        ]
    );
}
