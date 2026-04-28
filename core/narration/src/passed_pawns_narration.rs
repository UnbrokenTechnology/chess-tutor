//! Passed-pawn narration — per-sub-term worsening / improving per
//! side, mirroring the pawn-structure and king-safety precedence
//! (worsening wins over improving on the same side).

use std::io;

use chess_tutor_engine::analysis::PassedPawnsOutcome;
use chess_tutor_engine::eval::PassedBreakdown;

/// Engine-cp threshold per passed-pawn sub-term for narrating a
/// shift. Passed-pawn swings scale hard with rank — a rank-5 passer
/// alone puts ~170 cp of MG rank-bonus on the board — so a ~25 cp
/// floor suppresses noise while catching meaningful per-passer
/// events.
const PASSED_DELTA_THRESHOLD_CP: i32 = 25;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum PassedSubTerm {
    RankBonus,
    KingProximity,
    FreeAdvance,
    StopperPenalty,
}

impl PassedSubTerm {
    const ALL: [PassedSubTerm; 4] = [
        PassedSubTerm::RankBonus,
        PassedSubTerm::KingProximity,
        PassedSubTerm::FreeAdvance,
        PassedSubTerm::StopperPenalty,
    ];

    fn delta_mg(self, pre: &PassedBreakdown, post: &PassedBreakdown) -> i32 {
        match self {
            PassedSubTerm::RankBonus => post.rank_bonus.mg().0 - pre.rank_bonus.mg().0,
            PassedSubTerm::KingProximity => post.king_proximity.mg().0 - pre.king_proximity.mg().0,
            PassedSubTerm::FreeAdvance => post.free_advance.mg().0 - pre.free_advance.mg().0,
            PassedSubTerm::StopperPenalty => {
                post.stopper_penalty.mg().0 - pre.stopper_penalty.mg().0
            }
        }
    }

    fn worsened_phrase(self) -> &'static str {
        match self {
            PassedSubTerm::RankBonus => "a passer fell back",
            PassedSubTerm::KingProximity => "king race worsened",
            PassedSubTerm::FreeAdvance => "the promotion path got crowded",
            PassedSubTerm::StopperPenalty => "a passer drifted to a harder file",
        }
    }

    fn improved_phrase(self) -> &'static str {
        match self {
            PassedSubTerm::RankBonus => "a passer pushed forward",
            PassedSubTerm::KingProximity => "king race improved",
            PassedSubTerm::FreeAdvance => "the promotion path cleared",
            PassedSubTerm::StopperPenalty => "a passer reached an easier file",
        }
    }
}

fn worsened_passed_categories(pre: &PassedBreakdown, post: &PassedBreakdown) -> Vec<&'static str> {
    PassedSubTerm::ALL
        .iter()
        .filter(|st| st.delta_mg(pre, post) <= -PASSED_DELTA_THRESHOLD_CP)
        .map(|st| st.worsened_phrase())
        .collect()
}

fn improved_passed_categories(pre: &PassedBreakdown, post: &PassedBreakdown) -> Vec<&'static str> {
    PassedSubTerm::ALL
        .iter()
        .filter(|st| st.delta_mg(pre, post) >= PASSED_DELTA_THRESHOLD_CP)
        .map(|st| st.improved_phrase())
        .collect()
}

fn our_passed_worsened_line(o: &PassedPawnsOutcome) -> Option<String> {
    let clauses = worsened_passed_categories(&o.ours_pre, &o.ours_post);
    if clauses.is_empty() {
        return None;
    }
    Some(format!(
        "Your passed pawns weakened: {}.",
        clauses.join(", ")
    ))
}

fn our_passed_improved_line(o: &PassedPawnsOutcome) -> Option<String> {
    let clauses = improved_passed_categories(&o.ours_pre, &o.ours_post);
    if clauses.is_empty() {
        return None;
    }
    Some(format!(
        "Your passed pawns improved: {}.",
        clauses.join(", ")
    ))
}

fn their_passed_worsened_line(o: &PassedPawnsOutcome) -> Option<String> {
    let clauses = worsened_passed_categories(&o.theirs_pre, &o.theirs_post);
    if clauses.is_empty() {
        return None;
    }
    Some(format!(
        "You weakened the opponent's passed pawns: {}.",
        clauses.join(", "),
    ))
}

fn their_passed_improved_line(o: &PassedPawnsOutcome) -> Option<String> {
    let clauses = improved_passed_categories(&o.theirs_pre, &o.theirs_post);
    if clauses.is_empty() {
        return None;
    }
    Some(format!(
        "The opponent's passed pawns improved: {}.",
        clauses.join(", "),
    ))
}

pub(crate) fn render_passed_pawns(
    out: &mut dyn io::Write,
    outcome: &PassedPawnsOutcome,
) -> io::Result<bool> {
    let mut wrote = false;

    let ours = our_passed_worsened_line(outcome).or_else(|| our_passed_improved_line(outcome));
    if let Some(line) = ours {
        writeln!(out, "                {line}")?;
        wrote = true;
    }

    let theirs =
        their_passed_worsened_line(outcome).or_else(|| their_passed_improved_line(outcome));
    if let Some(line) = theirs {
        writeln!(out, "                {line}")?;
        wrote = true;
    }

    Ok(wrote)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chess_tutor_engine::types::Score;

    fn pa(rank: i32, king_prox: i32, free_adv: i32, stopper: i32) -> PassedBreakdown {
        PassedBreakdown {
            rank_bonus: Score::new(rank, 0),
            king_proximity: Score::new(king_prox, 0),
            free_advance: Score::new(free_adv, 0),
            stopper_penalty: Score::new(stopper, 0),
        }
    }

    fn pass_outcome(
        ours_pre: PassedBreakdown,
        ours_post: PassedBreakdown,
        theirs_pre: PassedBreakdown,
        theirs_post: PassedBreakdown,
    ) -> PassedPawnsOutcome {
        PassedPawnsOutcome {
            ours_pre,
            ours_post,
            theirs_pre,
            theirs_post,
        }
    }

    #[test]
    fn our_passed_improved_line_fires_when_rank_bonus_grows() {
        let out = pass_outcome(
            pa(50, 0, 0, 0),
            pa(80, 0, 0, 0),
            pa(0, 0, 0, 0),
            pa(0, 0, 0, 0),
        );
        let line = our_passed_improved_line(&out).expect("should fire");
        assert!(line.starts_with("Your passed pawns improved:"));
        assert!(line.contains("a passer pushed forward"));
    }

    #[test]
    fn our_passed_worsened_line_fires_when_free_advance_shrinks() {
        let out = pass_outcome(
            pa(0, 0, 60, 0),
            pa(0, 0, 20, 0),
            pa(0, 0, 0, 0),
            pa(0, 0, 0, 0),
        );
        let line = our_passed_worsened_line(&out).expect("should fire");
        assert!(line.contains("the promotion path got crowded"));
    }

    #[test]
    fn passed_delta_below_threshold_does_not_fire() {
        let out = pass_outcome(
            pa(0, 0, 0, 0),
            pa(20, 0, 0, 0),
            pa(0, 0, 0, 0),
            pa(0, 0, 0, 0),
        );
        assert_eq!(our_passed_worsened_line(&out), None);
        assert_eq!(our_passed_improved_line(&out), None);
    }

    #[test]
    fn their_passed_worsened_line_uses_active_voice() {
        let out = pass_outcome(
            pa(0, 0, 0, 0),
            pa(0, 0, 0, 0),
            pa(0, 0, 60, 0),
            pa(0, 0, 20, 0),
        );
        let line = their_passed_worsened_line(&out).expect("should fire");
        assert!(line.starts_with("You weakened the opponent's passed pawns:"));
    }

    #[test]
    fn their_passed_improved_line_uses_third_person() {
        let out = pass_outcome(
            pa(0, 0, 0, 0),
            pa(0, 0, 0, 0),
            pa(50, 0, 0, 0),
            pa(80, 0, 0, 0),
        );
        let line = their_passed_improved_line(&out).expect("should fire");
        assert!(line.starts_with("The opponent's passed pawns improved:"));
    }
}
