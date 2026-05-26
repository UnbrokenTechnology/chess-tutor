//! Space-dilution narration. Stockfish's space term scales as
//! `bonus × (piece_count − 1)²` per side, so removing a piece shrinks
//! that side's space advantage quadratically. Fires when the user's
//! move captured a piece *and* the opponent had a meaningful
//! pre-move space advantage that the capture diluted. (At ply 1 the
//! user's own piece count is invariant — their move can't remove
//! their own pieces — so an "ours" line never fires.)

use std::io;

use chess_tutor_engine::analysis::SpaceOutcome;
#[cfg(test)]
use chess_tutor_engine::bitboard::Bitboard;

/// Minimum |delta_mg| to bother narrating. Space is a small term to
/// begin with — most shifts are <20 cp — so we threshold modestly.
const SPACE_DELTA_THRESHOLD_MG: i32 = 15;

/// Minimum pre-move space score for the side whose advantage is
/// being diluted. Below this, the side didn't have a meaningful
/// space advantage to lose in the first place, so the narration
/// would be misleading ("their space advantage matters less" when
/// there was no advantage).
const SPACE_PRE_THRESHOLD_MG: i32 = 25;

fn theirs_line(outcome: &SpaceOutcome) -> Option<String> {
    if !outcome.theirs_piece_count_dropped() {
        return None;
    }
    if outcome.theirs_space_pre_mg < SPACE_PRE_THRESHOLD_MG {
        return None;
    }
    let delta = outcome.theirs_space_delta_mg();
    if delta >= -SPACE_DELTA_THRESHOLD_MG {
        return None;
    }
    Some(format!(
        "You captured a piece, diluting the opponent's space advantage ({} → {}).",
        format_pawns(outcome.theirs_space_pre_mg),
        format_pawns(outcome.theirs_space_post_mg),
    ))
}

fn format_pawns(mg: i32) -> String {
    format!("{:+.2}", mg as f32 / 100.0)
}

pub(crate) fn render_space(
    out: &mut dyn io::Write,
    outcome: &SpaceOutcome,
) -> io::Result<bool> {
    if let Some(line) = theirs_line(outcome) {
        writeln!(out, "                {line}")?;
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::too_many_arguments)]
    fn outcome(
        ours_pre: i32,
        ours_post: i32,
        theirs_pre: i32,
        theirs_post: i32,
        ours_count_pre: u32,
        ours_count_post: u32,
        theirs_count_pre: u32,
        theirs_count_post: u32,
    ) -> SpaceOutcome {
        SpaceOutcome {
            ours_space_pre_mg: ours_pre,
            ours_space_post_mg: ours_post,
            theirs_space_pre_mg: theirs_pre,
            theirs_space_post_mg: theirs_post,
            ours_piece_count_pre: ours_count_pre,
            ours_piece_count_post: ours_count_post,
            theirs_piece_count_pre: theirs_count_pre,
            theirs_piece_count_post: theirs_count_post,
            ours_safe_post: Bitboard::EMPTY,
            ours_reinforced_post: Bitboard::EMPTY,
            theirs_safe_post: Bitboard::EMPTY,
            theirs_reinforced_post: Bitboard::EMPTY,
        }
    }

    #[test]
    fn theirs_line_fires_when_user_captures_a_piece_with_meaningful_space() {
        // User captured (theirs piece count dropped 16→15); pre-space
        // 100, post 60 — delta −40 cp, above threshold.
        let o = outcome(0, 0, 100, 60, 16, 16, 16, 15);
        let line = theirs_line(&o).expect("should fire");
        assert!(line.starts_with("You captured a piece"));
        assert!(line.contains("opponent's space advantage"));
    }

    #[test]
    fn theirs_line_silent_when_no_capture_happened() {
        // Non-capture move — opponent piece count unchanged.
        let o = outcome(0, 0, 100, 90, 16, 16, 16, 16);
        assert_eq!(theirs_line(&o), None);
    }

    #[test]
    fn theirs_line_silent_when_no_meaningful_pre_advantage() {
        // Capture happened but opponent had no real space advantage.
        let o = outcome(0, 0, 10, 0, 16, 16, 16, 15);
        assert_eq!(theirs_line(&o), None);
    }

    #[test]
    fn theirs_line_silent_when_delta_too_small() {
        // Capture happened, pre-space meaningful, but delta too small.
        let o = outcome(0, 0, 100, 90, 16, 16, 16, 15);
        assert_eq!(theirs_line(&o), None);
    }
}
