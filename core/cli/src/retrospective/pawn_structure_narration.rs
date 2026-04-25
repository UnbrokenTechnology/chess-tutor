//! Pawn-structure narration — per-sub-term worsening / improving
//! lines per side, mirroring the king-safety precedence (worsening
//! wins over improving on the same side).

use std::io::{self, Write};

use chess_tutor_engine::analysis::PawnStructureOutcome;
use chess_tutor_engine::eval::PawnsBreakdown;

/// Engine-cp threshold per sub-term for calling a pawn-structure
/// shift worth narrating. ~0.15 of a pawn — big enough to skip the
/// 1-2 cp wobble from tapered rescoring but small enough to catch
/// single-pawn-scale events like a new doubled pawn.
const PAWN_STRUCTURE_DELTA_THRESHOLD_CP: i32 = 15;

/// The six pawn sub-terms, enumerated so we can iterate them in a
/// fixed order and pair each with phrasing for the two directions.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum PawnSubTerm {
    Connected,
    Isolated,
    Backward,
    Doubled,
    WeakUnopposed,
    WeakLever,
}

impl PawnSubTerm {
    const ALL: [PawnSubTerm; 6] = [
        PawnSubTerm::Connected,
        PawnSubTerm::Isolated,
        PawnSubTerm::Backward,
        PawnSubTerm::Doubled,
        PawnSubTerm::WeakUnopposed,
        PawnSubTerm::WeakLever,
    ];

    /// `post.mg() - pre.mg()` for this sub-term. Positive =
    /// improved (bonus grew or penalty shrank); negative = worsened.
    fn delta_mg(self, pre: &PawnsBreakdown, post: &PawnsBreakdown) -> i32 {
        match self {
            PawnSubTerm::Connected => post.connected.mg().0 - pre.connected.mg().0,
            PawnSubTerm::Isolated => post.isolated.mg().0 - pre.isolated.mg().0,
            PawnSubTerm::Backward => post.backward.mg().0 - pre.backward.mg().0,
            PawnSubTerm::Doubled => post.doubled.mg().0 - pre.doubled.mg().0,
            PawnSubTerm::WeakUnopposed => post.weak_unopposed.mg().0 - pre.weak_unopposed.mg().0,
            PawnSubTerm::WeakLever => post.weak_lever.mg().0 - pre.weak_lever.mg().0,
        }
    }

    fn worsened_phrase(self) -> &'static str {
        match self {
            PawnSubTerm::Connected => "broke pawn connections",
            PawnSubTerm::Isolated => "isolated a pawn",
            PawnSubTerm::Backward => "created a backward pawn",
            PawnSubTerm::Doubled => "doubled a pawn",
            PawnSubTerm::WeakUnopposed => "exposed a weak pawn",
            PawnSubTerm::WeakLever => "walked into a pawn lever",
        }
    }

    fn improved_phrase(self) -> &'static str {
        match self {
            PawnSubTerm::Connected => "connected pawns",
            PawnSubTerm::Isolated => "reconnected an isolated pawn",
            PawnSubTerm::Backward => "freed a backward pawn",
            PawnSubTerm::Doubled => "resolved a doubled pawn",
            PawnSubTerm::WeakUnopposed => "covered a weak pawn",
            PawnSubTerm::WeakLever => "resolved a pawn lever",
        }
    }
}

fn worsened_pawn_categories(pre: &PawnsBreakdown, post: &PawnsBreakdown) -> Vec<&'static str> {
    PawnSubTerm::ALL
        .iter()
        .filter(|st| st.delta_mg(pre, post) <= -PAWN_STRUCTURE_DELTA_THRESHOLD_CP)
        .map(|st| st.worsened_phrase())
        .collect()
}

fn improved_pawn_categories(pre: &PawnsBreakdown, post: &PawnsBreakdown) -> Vec<&'static str> {
    PawnSubTerm::ALL
        .iter()
        .filter(|st| st.delta_mg(pre, post) >= PAWN_STRUCTURE_DELTA_THRESHOLD_CP)
        .map(|st| st.improved_phrase())
        .collect()
}

fn our_pawns_worsened_line(o: &PawnStructureOutcome) -> Option<String> {
    let clauses = worsened_pawn_categories(&o.ours_pre, &o.ours_post);
    if clauses.is_empty() {
        return None;
    }
    Some(format!(
        "Your pawn structure weakened: {}.",
        clauses.join(", ")
    ))
}

fn our_pawns_improved_line(o: &PawnStructureOutcome) -> Option<String> {
    let clauses = improved_pawn_categories(&o.ours_pre, &o.ours_post);
    if clauses.is_empty() {
        return None;
    }
    Some(format!(
        "Your pawn structure improved: {}.",
        clauses.join(", ")
    ))
}

fn their_pawns_worsened_line(o: &PawnStructureOutcome) -> Option<String> {
    let clauses = worsened_pawn_categories(&o.theirs_pre, &o.theirs_post);
    if clauses.is_empty() {
        return None;
    }
    Some(format!(
        "You weakened the opponent's pawn structure: {}.",
        clauses.join(", "),
    ))
}

fn their_pawns_improved_line(o: &PawnStructureOutcome) -> Option<String> {
    let clauses = improved_pawn_categories(&o.theirs_pre, &o.theirs_post);
    if clauses.is_empty() {
        return None;
    }
    Some(format!(
        "The opponent's pawn structure improved: {}.",
        clauses.join(", "),
    ))
}

/// Render the pawn-structure lines (ours, theirs). Per side,
/// worsened takes precedence over improved if both fire — worsening
/// is more urgent teaching, mirroring the king-safety precedence.
pub(super) fn render_pawn_structure(
    out: &mut io::StdoutLock<'_>,
    outcome: &PawnStructureOutcome,
) -> io::Result<bool> {
    let mut wrote = false;

    let ours = our_pawns_worsened_line(outcome).or_else(|| our_pawns_improved_line(outcome));
    if let Some(line) = ours {
        writeln!(out, "                {line}")?;
        wrote = true;
    }

    let theirs = their_pawns_worsened_line(outcome).or_else(|| their_pawns_improved_line(outcome));
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

    fn pb(
        connected: i32,
        isolated: i32,
        backward: i32,
        doubled: i32,
        weak_unopposed: i32,
        weak_lever: i32,
    ) -> PawnsBreakdown {
        PawnsBreakdown {
            connected: Score::new(connected, 0),
            isolated: Score::new(isolated, 0),
            backward: Score::new(backward, 0),
            doubled: Score::new(doubled, 0),
            weak_unopposed: Score::new(weak_unopposed, 0),
            weak_lever: Score::new(weak_lever, 0),
        }
    }

    fn ps_outcome(
        ours_pre: PawnsBreakdown,
        ours_post: PawnsBreakdown,
        theirs_pre: PawnsBreakdown,
        theirs_post: PawnsBreakdown,
    ) -> PawnStructureOutcome {
        PawnStructureOutcome {
            ours_pre,
            ours_post,
            theirs_pre,
            theirs_post,
        }
    }

    #[test]
    fn our_pawns_worsened_line_fires_on_new_doubled() {
        let out = ps_outcome(
            pb(0, 0, 0, 0, 0, 0),
            pb(0, 0, 0, -20, 0, 0),
            pb(0, 0, 0, 0, 0, 0),
            pb(0, 0, 0, 0, 0, 0),
        );
        assert_eq!(
            our_pawns_worsened_line(&out),
            Some("Your pawn structure weakened: doubled a pawn.".to_string()),
        );
    }

    #[test]
    fn our_pawns_worsened_line_combines_multiple_categories() {
        let out = ps_outcome(
            pb(0, 0, 0, 0, 0, 0),
            pb(0, 0, 0, -20, -25, 0),
            pb(0, 0, 0, 0, 0, 0),
            pb(0, 0, 0, 0, 0, 0),
        );
        let line = our_pawns_worsened_line(&out).expect("should fire");
        assert!(line.contains("doubled a pawn"));
        assert!(line.contains("exposed a weak pawn"));
    }

    #[test]
    fn our_pawns_improved_line_fires_when_doubled_resolved() {
        let out = ps_outcome(
            pb(0, 0, 0, -20, 0, 0),
            pb(0, 0, 0, 0, 0, 0),
            pb(0, 0, 0, 0, 0, 0),
            pb(0, 0, 0, 0, 0, 0),
        );
        assert_eq!(
            our_pawns_improved_line(&out),
            Some("Your pawn structure improved: resolved a doubled pawn.".to_string()),
        );
    }

    #[test]
    fn pawn_delta_below_threshold_does_not_fire() {
        let out = ps_outcome(
            pb(0, 0, 0, 0, 0, 0),
            pb(0, 0, 0, -10, 0, 0),
            pb(0, 0, 0, 0, 0, 0),
            pb(0, 0, 0, 0, 0, 0),
        );
        assert_eq!(our_pawns_worsened_line(&out), None);
        assert_eq!(our_pawns_improved_line(&out), None);
    }

    #[test]
    fn their_pawns_worsened_line_uses_active_voice() {
        let out = ps_outcome(
            pb(0, 0, 0, 0, 0, 0),
            pb(0, 0, 0, 0, 0, 0),
            pb(0, 0, 0, 0, 0, 0),
            pb(0, 0, 0, -20, 0, 0),
        );
        let line = their_pawns_worsened_line(&out).expect("should fire");
        assert!(line.starts_with("You weakened the opponent's pawn structure:"));
        assert!(line.contains("doubled a pawn"));
    }

    #[test]
    fn their_pawns_improved_line_uses_third_person() {
        let out = ps_outcome(
            pb(0, 0, 0, 0, 0, 0),
            pb(0, 0, 0, 0, 0, 0),
            pb(0, 0, 0, -20, 0, 0),
            pb(0, 0, 0, 0, 0, 0),
        );
        let line = their_pawns_improved_line(&out).expect("should fire");
        assert!(line.starts_with("The opponent's pawn structure improved:"));
    }
}
