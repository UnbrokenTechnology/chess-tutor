//! Closed-centre + own-piece-barricade narration. Both stories ride
//! the same Stockfish `bishop_pawns` multiplier — any piece in front
//! of an own central pawn amplifies the bishop-vs-own-pawns penalty —
//! but they're chess-distinct teaching concepts, so the narrator
//! emits them as separate lines:
//!
//! - **Closed centre** — own central pawn meets enemy pawn directly
//!   in front. Pawn structure is locked, favours knights and slow
//!   maneuvering, cramps bishops behind their own pawns.
//! - **Barricaded pawn** — own central pawn now has another piece
//!   (almost always one of your own) sitting in front of it. The
//!   pawn can't push until the blocker moves first, so the bishop
//!   diagonals that pawn would clear stay constrained.

use std::io;

use chess_tutor_engine::analysis::BlockedCenterOutcome;

fn locked_line(outcome: &BlockedCenterOutcome) -> Option<&'static str> {
    let delta = outcome.locked_total_delta();
    if delta == 0 {
        return None;
    }
    if !outcome.ours_amplifies_bishop_penalty && !outcome.theirs_amplifies_bishop_penalty {
        return None;
    }
    if delta > 0 {
        Some(
            "You closed the center: pawn play is locked, cramping bishops behind their own pawns.",
        )
    } else {
        Some("The center opened: bishops and rooks gain scope.")
    }
}

fn barricade_line(outcome: &BlockedCenterOutcome) -> Option<&'static str> {
    let delta = outcome.barricaded_total_delta();
    if delta == 0 {
        return None;
    }
    if !outcome.ours_amplifies_bishop_penalty && !outcome.theirs_amplifies_bishop_penalty {
        return None;
    }
    if delta > 0 {
        Some(
            "A piece now sits in front of a central pawn: the pawn can't advance, \
             so the bishop diagonals it would clear stay constrained until the blocker moves.",
        )
    } else {
        Some(
            "A central pawn's path cleared: the pawn can advance now, \
             freeing the bishop diagonals it had been holding back.",
        )
    }
}

pub(crate) fn render_blocked_center(
    out: &mut dyn io::Write,
    outcome: &BlockedCenterOutcome,
) -> io::Result<bool> {
    let mut wrote = false;
    if let Some(line) = locked_line(outcome) {
        writeln!(out, "                {line}")?;
        wrote = true;
    }
    if let Some(line) = barricade_line(outcome) {
        writeln!(out, "                {line}")?;
        wrote = true;
    }
    Ok(wrote)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::too_many_arguments)]
    fn outcome(
        ours_locked_pre: u32,
        ours_locked_post: u32,
        theirs_locked_pre: u32,
        theirs_locked_post: u32,
        ours_barricaded_pre: u32,
        ours_barricaded_post: u32,
        theirs_barricaded_pre: u32,
        theirs_barricaded_post: u32,
        ours_amp: bool,
        theirs_amp: bool,
    ) -> BlockedCenterOutcome {
        BlockedCenterOutcome {
            ours_locked_pre,
            ours_locked_post,
            theirs_locked_pre,
            theirs_locked_post,
            ours_barricaded_pre,
            ours_barricaded_post,
            theirs_barricaded_pre,
            theirs_barricaded_post,
            ours_amplifies_bishop_penalty: ours_amp,
            theirs_amplifies_bishop_penalty: theirs_amp,
        }
    }

    #[test]
    fn locked_line_fires_closed_when_pawn_lock_appears() {
        let o = outcome(0, 1, 0, 1, 0, 0, 0, 0, true, true);
        let line = locked_line(&o).expect("should fire");
        assert!(line.starts_with("You closed the center"));
        assert_eq!(barricade_line(&o), None);
    }

    #[test]
    fn locked_line_fires_opened_when_pawn_lock_dissolves() {
        let o = outcome(2, 1, 1, 0, 0, 0, 0, 0, true, true);
        let line = locked_line(&o).expect("should fire");
        assert!(line.starts_with("The center opened"));
    }

    #[test]
    fn barricade_line_fires_when_own_piece_lands_in_front_of_central_pawn() {
        // 2.Nf3 case: locked unchanged, barricade goes up by 1 on
        // our side.
        let o = outcome(1, 1, 1, 1, 0, 1, 0, 0, true, true);
        assert_eq!(locked_line(&o), None);
        let line = barricade_line(&o).expect("should fire");
        assert!(line.starts_with("A piece now sits in front"));
    }

    #[test]
    fn barricade_line_fires_cleared_when_blocker_moves_off() {
        let o = outcome(0, 0, 0, 0, 1, 0, 0, 0, true, true);
        let line = barricade_line(&o).expect("should fire");
        assert!(line.starts_with("A central pawn's path cleared"));
    }

    #[test]
    fn both_lines_silent_when_neither_side_amplifies() {
        let o = outcome(0, 1, 0, 1, 0, 1, 0, 0, false, false);
        assert_eq!(locked_line(&o), None);
        assert_eq!(barricade_line(&o), None);
    }

    #[test]
    fn lines_silent_when_no_change() {
        let o = outcome(1, 1, 0, 0, 0, 0, 0, 0, true, true);
        assert_eq!(locked_line(&o), None);
        assert_eq!(barricade_line(&o), None);
    }
}
