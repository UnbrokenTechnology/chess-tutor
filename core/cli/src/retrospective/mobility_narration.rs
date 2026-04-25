//! Mobility narration — one line per side naming the piece type
//! with the largest |delta|, with phrasing tuned per sign (drop vs.
//! improve, restrict vs. opponent-improved).

use std::io::{self, Write};

use chess_tutor_engine::analysis::MobilityOutcome;
use chess_tutor_engine::eval::MobilityBreakdown;

use super::util::format_shelter_pawns;

/// Engine-cp threshold for narrating a mobility shift on a single
/// piece type. ~0.50 of a pawn — the earlier 30-cp setting fired on
/// almost every opening move (any nudge to an enemy pawn shifts the
/// mobility-area bitmap, which the term weights even when our pieces
/// haven't actually moved). 50 cp cuts the noise without hiding
/// shifts that correspond to a real change in the piece's reach.
const MOBILITY_DELTA_THRESHOLD_CP: i32 = 50;

/// Identify the piece type with the biggest |delta_mg| between
/// `pre` and `post`. Returns `None` when the biggest shift is below
/// the reporting threshold.
fn mobility_biggest_shift(
    pre: &MobilityBreakdown,
    post: &MobilityBreakdown,
) -> Option<(&'static str, i32, i32, i32)> {
    let candidates: [(&'static str, i32, i32); 4] = [
        ("knight", pre.knight.mg().0, post.knight.mg().0),
        ("bishop", pre.bishop.mg().0, post.bishop.mg().0),
        ("rook", pre.rook.mg().0, post.rook.mg().0),
        ("queen", pre.queen.mg().0, post.queen.mg().0),
    ];
    let (label, pre_mg, post_mg) = *candidates
        .iter()
        .max_by_key(|(_, pre_mg, post_mg)| (post_mg - pre_mg).abs())?;
    let delta = post_mg - pre_mg;
    if delta.abs() < MOBILITY_DELTA_THRESHOLD_CP {
        return None;
    }
    Some((label, delta, pre_mg, post_mg))
}

fn our_mobility_line(o: &MobilityOutcome) -> Option<String> {
    let (label, delta, pre, post) = mobility_biggest_shift(&o.ours_pre, &o.ours_post)?;
    let verb = if delta < 0 { "dropped" } else { "improved" };
    // "Activity" rather than "mobility": Stockfish's mobility term
    // is a weighted count of squares the piece attacks inside the
    // safe-area bitmap, not the number of legal moves the piece
    // has — which is what a student hears in "mobility."
    Some(format!(
        "Your {label} activity {verb} ({} → {}).",
        format_shelter_pawns(pre),
        format_shelter_pawns(post),
    ))
}

fn their_mobility_line(o: &MobilityOutcome) -> Option<String> {
    let (label, delta, pre, post) = mobility_biggest_shift(&o.theirs_pre, &o.theirs_post)?;
    if delta < 0 {
        Some(format!(
            "You restricted the opponent's {label} activity ({} → {}).",
            format_shelter_pawns(pre),
            format_shelter_pawns(post),
        ))
    } else {
        Some(format!(
            "The opponent's {label} activity improved ({} → {}).",
            format_shelter_pawns(pre),
            format_shelter_pawns(post),
        ))
    }
}

pub(super) fn render_mobility(
    out: &mut io::StdoutLock<'_>,
    outcome: &MobilityOutcome,
) -> io::Result<bool> {
    let mut wrote = false;
    if let Some(line) = our_mobility_line(outcome) {
        writeln!(out, "                {line}")?;
        wrote = true;
    }
    if let Some(line) = their_mobility_line(outcome) {
        writeln!(out, "                {line}")?;
        wrote = true;
    }
    Ok(wrote)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chess_tutor_engine::types::Score;

    fn mb(knight: i32, bishop: i32, rook: i32, queen: i32) -> MobilityBreakdown {
        MobilityBreakdown {
            knight: Score::new(knight, 0),
            bishop: Score::new(bishop, 0),
            rook: Score::new(rook, 0),
            queen: Score::new(queen, 0),
        }
    }

    fn mob_outcome(
        ours_pre: MobilityBreakdown,
        ours_post: MobilityBreakdown,
        theirs_pre: MobilityBreakdown,
        theirs_post: MobilityBreakdown,
    ) -> MobilityOutcome {
        MobilityOutcome {
            ours_pre,
            ours_post,
            theirs_pre,
            theirs_post,
        }
    }

    #[test]
    fn our_mobility_line_picks_biggest_piece_type_drop() {
        let out = mob_outcome(
            mb(80, 40, 80, 90),
            mb(20, 50, 75, 105),
            mb(0, 0, 0, 0),
            mb(0, 0, 0, 0),
        );
        assert_eq!(
            our_mobility_line(&out),
            Some("Your knight activity dropped (+0.80 → +0.20).".to_string()),
        );
    }

    #[test]
    fn our_mobility_line_uses_improved_verb_on_gain() {
        let out = mob_outcome(
            mb(0, 20, 0, 0),
            mb(0, 80, 0, 0),
            mb(0, 0, 0, 0),
            mb(0, 0, 0, 0),
        );
        assert_eq!(
            our_mobility_line(&out),
            Some("Your bishop activity improved (+0.20 → +0.80).".to_string()),
        );
    }

    #[test]
    fn mobility_below_threshold_does_not_fire() {
        // 40 cp would have fired under the old 30-cp threshold; the
        // new 50-cp threshold silences it.
        let out = mob_outcome(
            mb(0, 0, 0, 0),
            mb(40, 0, 0, 0),
            mb(0, 0, 0, 0),
            mb(0, 0, 0, 0),
        );
        assert_eq!(our_mobility_line(&out), None);
    }

    #[test]
    fn their_mobility_line_uses_restricted_phrasing_on_decrease() {
        let out = mob_outcome(
            mb(0, 0, 0, 0),
            mb(0, 0, 0, 0),
            mb(0, 0, 80, 0),
            mb(0, 0, 20, 0),
        );
        let line = their_mobility_line(&out).expect("should fire");
        assert!(line.starts_with("You restricted the opponent's rook activity"));
    }

    #[test]
    fn their_mobility_line_uses_neutral_phrasing_on_improvement() {
        let out = mob_outcome(
            mb(0, 0, 0, 0),
            mb(0, 0, 0, 0),
            mb(0, 0, 0, 30),
            mb(0, 0, 0, 90),
        );
        let line = their_mobility_line(&out).expect("should fire");
        assert!(line.starts_with("The opponent's queen activity improved"));
    }
}
