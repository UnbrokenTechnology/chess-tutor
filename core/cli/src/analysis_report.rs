//! Pretty-print a [`MoveAnalysis`] — the teaching pipeline's per-move
//! output — as a human-readable per-term-delta table.
//!
//! Input is engine-internal cp (Stockfish scale: PawnEG = 213). We
//! convert to pawns at render time (`delta_cp / 100.0`, same rounding
//! the rest of the CLI uses) so the student doesn't have to do mental
//! arithmetic on weird units.
//!
//! Layout per move:
//!
//! ```text
//!   1. +0.28   e2e4  (settles ply 3)
//!        showing top 75% (3 of 25 terms)
//!        mobility           +0.40  (+62 mg,  -4 eg)
//!        king               -0.18  ( -5 mg, -26 eg)
//!        pawns.isolated     +0.08  ( +8 mg,  +8 eg)
//!        ...                                (2 more terms cover the last 25%)
//! ```

use std::fmt::Write;

use chess_tutor_engine::analysis::{cumulative_prefix, MoveAnalysis};
use chess_tutor_engine::position::Position;
use chess_tutor_engine::san;
use chess_tutor_engine::types::{Move, Value};

/// Render a batch of [`MoveAnalysis`] results as a single multi-move
/// report. `root` is the position the analyses were produced from —
/// needed to format moves in SAN.
///
/// `top_percent` is the cumulative `|delta_tapered|` coverage threshold
/// used by [`cumulative_prefix`]: e.g. 75.0 shows the smallest prefix
/// of sorted term deltas whose absolute deltas sum to ≥ 75% of the
/// total absolute movement.
pub fn render(root: &Position, analyses: &[MoveAnalysis], top_percent: f32) -> String {
    let mut out = String::new();
    for (i, a) in analyses.iter().enumerate() {
        render_one(&mut out, root, a, i + 1, top_percent);
        if i + 1 < analyses.len() {
            writeln!(out).unwrap();
        }
    }
    out
}

fn render_one(out: &mut String, root: &Position, a: &MoveAnalysis, rank: usize, top_percent: f32) {
    let pv_san = pv_to_san(root, &a.pv);
    let mv_san = pv_san.first().cloned().unwrap_or_else(|| "?".to_string());
    let settled_str = format_settled_suffix(&a.pv, a.settled_ply);
    writeln!(
        out,
        "  {:>2}. {:>6}   {:<8}  {}",
        rank,
        format_score_pawns(a.score),
        mv_san,
        settled_str,
    )
    .unwrap();

    let prefix = cumulative_prefix(&a.term_deltas, top_percent);
    let total = a.term_deltas.len();

    if prefix.is_empty() {
        // Either the deltas are all zero (root-vs-self fallback) or
        // percent was <= 0. Either way, nothing useful to say.
        writeln!(out, "        (no material term swings)").unwrap();
        return;
    }

    writeln!(
        out,
        "        showing top {:.0}% ({} of {} terms)",
        top_percent,
        prefix.len(),
        total,
    )
    .unwrap();

    for d in prefix {
        // Tapered delta is engine-internal cp (PAWN_EG=213). Convert
        // to conventional pawns via the shared `units` helper so the
        // numbers here match what the position-summary header shows.
        let pawns = crate::units::engine_cp_to_pawns(Value(d.delta_tapered)) as f32;
        writeln!(
            out,
            "        {:<26}  {:>+6.2}  ({:>+4} mg, {:>+4} eg)",
            d.term.label(),
            pawns,
            d.delta_mg,
            d.delta_eg,
        )
        .unwrap();
    }

    let remaining = total - prefix.len();
    if remaining > 0 {
        writeln!(
            out,
            "        ... {} more term{} cover the remaining {:.0}%",
            remaining,
            if remaining == 1 { "" } else { "s" },
            (100.0 - top_percent).max(0.0),
        )
        .unwrap();
    }
}

fn pv_to_san(root: &Position, pv: &[Move]) -> Vec<String> {
    let mut out = Vec::with_capacity(pv.len());
    let mut scratch = root.clone();
    for mv in pv {
        out.push(san::format_on(&mut scratch, *mv));
        scratch.do_move(*mv);
    }
    out
}

fn format_score_pawns(score: Value) -> String {
    // Route through the shared units module so the engine-cp →
    // conventional-pawn conversion (PAWN_EG=213) matches every other
    // CLI surface. Side-to-move POV is preserved here — the caller
    // re-signs to white-POV if desired.
    crate::units::format_pawns(score)
}

fn format_settled_suffix(pv: &[Move], settled: Option<usize>) -> String {
    match settled {
        None => String::new(),
        Some(_) if pv.is_empty() => String::new(),
        Some(i) if i + 1 == pv.len() => "(settles leaf)".to_string(),
        Some(i) => format!("(settles ply {})", i + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chess_tutor_engine::analysis::analyze_position;
    use chess_tutor_engine::engine::{Engine, SearchParams};

    #[test]
    fn renders_startpos_analysis_without_panic() {
        let mut pos = Position::startpos();
        let mut engine = Engine::default();
        let analyses = analyze_position(
            &mut engine,
            &mut pos,
            SearchParams {
                max_depth: 4,
                multi_pv: 2,
                ..SearchParams::default()
            },
        );
        let out = render(&pos, &analyses, 75.0);
        assert!(!out.is_empty());
        assert!(out.contains("showing top 75%"));
    }

    #[test]
    fn empty_analyses_renders_empty_string() {
        let pos = Position::startpos();
        let out = render(&pos, &[], 75.0);
        assert!(out.is_empty());
    }
}
