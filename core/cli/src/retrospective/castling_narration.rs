//! Castling-loss × trapped-rook narration. Stockfish doubles the
//! [`trapped_rook`] penalty when the trapped side can no longer
//! castle — the rook has nowhere to go. We surface this as a
//! standalone teaching line whenever a side just forfeited its last
//! castling rights *and* a rook is currently boxed in by the king.
//!
//! [`trapped_rook`]: chess_tutor_engine::eval::PiecesBreakdown::trapped_rook

use std::io::{self, Write};

use chess_tutor_engine::analysis::CastlingOutcome;

/// A trapped-rook penalty smaller in magnitude than this is too small
/// to bother teaching about — the doubling-from-castling-loss adds
/// only a few cp.
const TRAPPED_ROOK_NARRATE_THRESHOLD_MG: i32 = 20;

fn ours_line(outcome: &CastlingOutcome) -> Option<&'static str> {
    if !outcome.ours_lost_castling() {
        return None;
    }
    if outcome.ours_trapped_rook_post_mg.abs() < TRAPPED_ROOK_NARRATE_THRESHOLD_MG {
        return None;
    }
    Some(
        "You forfeited castling: a rook is locked in by your king with no way to free it — \
         the trapped-rook penalty just doubled.",
    )
}

fn theirs_line(outcome: &CastlingOutcome) -> Option<&'static str> {
    if !outcome.theirs_lost_castling() {
        return None;
    }
    if outcome.theirs_trapped_rook_post_mg.abs() < TRAPPED_ROOK_NARRATE_THRESHOLD_MG {
        return None;
    }
    Some(
        "You stripped the opponent of castling: a rook of theirs is locked in by their king \
         with no way out.",
    )
}

pub(super) fn render_castling(
    out: &mut io::StdoutLock<'_>,
    outcome: &CastlingOutcome,
) -> io::Result<bool> {
    let mut wrote = false;
    if let Some(line) = ours_line(outcome) {
        writeln!(out, "                {line}")?;
        wrote = true;
    }
    if let Some(line) = theirs_line(outcome) {
        writeln!(out, "                {line}")?;
        wrote = true;
    }
    Ok(wrote)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(
        ours_pre: bool,
        ours_post: bool,
        theirs_pre: bool,
        theirs_post: bool,
        ours_trapped_mg: i32,
        theirs_trapped_mg: i32,
    ) -> CastlingOutcome {
        CastlingOutcome {
            ours_could_castle_pre: ours_pre,
            ours_could_castle_post: ours_post,
            theirs_could_castle_pre: theirs_pre,
            theirs_could_castle_post: theirs_post,
            ours_trapped_rook_post_mg: ours_trapped_mg,
            theirs_trapped_rook_post_mg: theirs_trapped_mg,
        }
    }

    #[test]
    fn ours_line_fires_when_castling_lost_and_rook_trapped() {
        let o = outcome(true, false, true, true, -90, 0);
        let line = ours_line(&o).expect("should fire");
        assert!(line.starts_with("You forfeited castling"));
    }

    #[test]
    fn ours_line_silent_when_castling_kept() {
        let o = outcome(true, true, true, true, -90, 0);
        assert_eq!(ours_line(&o), None);
    }

    #[test]
    fn ours_line_silent_when_no_trapped_rook() {
        let o = outcome(true, false, true, true, 0, 0);
        assert_eq!(ours_line(&o), None);
    }

    #[test]
    fn theirs_line_fires_when_we_strip_opponents_castling_and_their_rook_trapped() {
        let o = outcome(true, true, true, false, 0, -90);
        let line = theirs_line(&o).expect("should fire");
        assert!(line.starts_with("You stripped the opponent of castling"));
    }

    #[test]
    fn threshold_suppresses_tiny_trapped_rook_penalty() {
        let o = outcome(true, false, true, true, -10, 0);
        assert_eq!(ours_line(&o), None);
    }
}
