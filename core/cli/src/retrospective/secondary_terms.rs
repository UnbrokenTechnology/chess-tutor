//! Fallback "Helped" / "Hurt" lines — render the cumulative
//! top-percent prefix of the user move's term deltas as two
//! sign-grouped lists (positives first, then negatives), each sorted
//! by magnitude. Whichever terms the specialised narrators already
//! consumed are filtered out.

use std::io::{self, Write};

use chess_tutor_engine::analysis::{cumulative_prefix, MoveAnalysis, TermId};
use chess_tutor_engine::types::Color;

/// Cumulative-coverage threshold for term-based fallback explanations.
/// Lower than the `search --analyze` default (75%) because real-game
/// output showed 7–9 rows per move at 75%, most of which were noise;
/// 50% keeps the list tight enough to scan and usually lands on the
/// 2–4 terms that actually drove the swing.
const RETROSPECTIVE_TOP_PERCENT: f32 = 50.0;

/// Render the cumulative-prefix of term deltas as two grouped lists
/// (helped / hurt, from the root side-to-move's POV) sorted by
/// magnitude. When a term has already been "used" by an earlier
/// narration line (e.g. Material), pass it in `skip` so the list
/// shows only the other contributors.
pub(super) fn render_secondary_terms(
    out: &mut io::StdoutLock<'_>,
    user: &MoveAnalysis,
    root_stm: Color,
    skip: &[TermId],
) -> io::Result<()> {
    let prefix = cumulative_prefix(&user.term_deltas, RETROSPECTIVE_TOP_PERCENT);

    // Sign-flip so positives = "helped the root side-to-move" — i.e.
    // the *player's* POV, not raw white-POV.
    let sign = match root_stm {
        Color::White => 1,
        Color::Black => -1,
    };

    let rows: Vec<(TermId, i32)> = prefix
        .iter()
        .filter(|d| !skip.contains(&d.term) && d.delta_tapered != 0)
        .map(|d| (d.term, d.delta_tapered * sign))
        .collect();
    if rows.is_empty() {
        return Ok(());
    }

    let (helped, hurt): (Vec<_>, Vec<_>) = rows.into_iter().partition(|(_, cp)| *cp > 0);

    // The "used" flag becomes "Also" for both lines when specialised
    // narrators already fired, otherwise "Helped" / "Hurt" stand
    // alone — smaller visual overhead when they're the only shift
    // narration on the move.
    let helped_heading = if skip.is_empty() {
        "Helped"
    } else {
        "Also helped"
    };
    let hurt_heading = if skip.is_empty() { "Hurt" } else { "Also hurt" };

    if !helped.is_empty() {
        write_sorted_line(out, helped_heading, helped)?;
    }
    if !hurt.is_empty() {
        write_sorted_line(out, hurt_heading, hurt)?;
    }
    Ok(())
}

fn write_sorted_line(
    out: &mut io::StdoutLock<'_>,
    heading: &str,
    mut rows: Vec<(TermId, i32)>,
) -> io::Result<()> {
    rows.sort_by_key(|(_, cp)| std::cmp::Reverse(cp.abs()));
    let joined: Vec<String> = rows
        .iter()
        .map(|(term, cp)| format!("{} {:+.2}", term.pretty_label(), *cp as f32 / 100.0))
        .collect();
    writeln!(out, "                {heading}: {}.", joined.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partition_helped_and_hurt_from_white_pov() {
        // Raw deltas (white-POV): material +80, pawns connected -30.
        // Root = white, so sign=+1: material helps, connected hurts.
        let rows: Vec<(TermId, i32)> = vec![(TermId::Material, 80), (TermId::PawnsConnected, -30)];
        let (helped, hurt): (Vec<_>, Vec<_>) = rows.into_iter().partition(|(_, cp)| *cp > 0);
        assert_eq!(helped, vec![(TermId::Material, 80)]);
        assert_eq!(hurt, vec![(TermId::PawnsConnected, -30)]);
    }

    #[test]
    fn partition_flips_perspective_for_black() {
        // Raw deltas (white-POV): material +80, pawns connected -30.
        // Root = black, sign = -1: material hurts, connected helps.
        let sign = -1;
        let rows: Vec<(TermId, i32)> = vec![
            (TermId::Material, 80 * sign),
            (TermId::PawnsConnected, -30 * sign),
        ];
        let (helped, hurt): (Vec<_>, Vec<_>) = rows.into_iter().partition(|(_, cp)| *cp > 0);
        assert_eq!(helped, vec![(TermId::PawnsConnected, 30)]);
        assert_eq!(hurt, vec![(TermId::Material, -80)]);
    }

    #[test]
    fn sort_descending_by_abs_magnitude() {
        let mut rows: Vec<(TermId, i32)> = vec![
            (TermId::Material, 15),
            (TermId::KingShelter, 80),
            (TermId::MobilityKnight, 40),
        ];
        rows.sort_by_key(|(_, cp)| std::cmp::Reverse(cp.abs()));
        assert_eq!(
            rows,
            vec![
                (TermId::KingShelter, 80),
                (TermId::MobilityKnight, 40),
                (TermId::Material, 15),
            ],
        );
    }
}
