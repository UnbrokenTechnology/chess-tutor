//! Rendering helpers for the `search` / `analyze` CLI output: PV-to-SAN
//! conversion, score formatting, multi-PV tables, settled-ply suffixes, and
//! the debug eval-trajectory dump. Split out of `main.rs`; called only by the
//! driver in [`crate::main`].

use chess_tutor_engine::position::Position;
use chess_tutor_engine::san;

/// Render a score as conventional pawns (`+0.28`, `-1.05`) or mate
/// notation (`#5`, `-#3`). Routes through [`crate::units::format_pawns`]
/// so the scale conversion (engine PAWN_EG = 213 → conventional
/// pawn = 100) stays consistent across every CLI surface. The caller is
/// responsible for re-signing to white-POV when that's desired — this
/// function does not flip the sign.
pub(crate) fn format_score_pawns(score: chess_tutor_engine::types::Value) -> String {
    crate::units::format_pawns(score)
}

/// Render multiple ranked PVs as aligned rows. The first line's delta
/// column reads `(0 cp)`; subsequent lines show the delta-from-top in
/// **engine-cp** so the numbers connect to search-code thresholds
/// (aspiration widths, blunder bands, futility margins) without a
/// scale conversion. The headline score column is in pawns
/// (chess.com-comparable) and pre-oriented per [`Orientation`].
pub(crate) fn render_multi_pv(
    root: &Position,
    lines: &[chess_tutor_engine::engine::SearchLine],
    orientation: crate::units::Orientation,
) -> String {
    use std::fmt::Write;
    let stm = root.side_to_move();
    let top_score = lines[0].score.0;
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        let pv_san = san::pv_to_san(root, &line.pv);
        let oriented = orientation.apply(line.score, stm);
        let delta_engine = line.score.0 - top_score;
        let delta_str = if delta_engine == 0 {
            "(0 cp)".to_string()
        } else {
            format!("({:+} cp)", delta_engine)
        };
        let settled_str = format_settled_suffix(&line.pv, line.settled_ply);
        writeln!(
            out,
            "  {:>2}. {:>6}   {:<12}  {:<36}  {}",
            i + 1,
            format_score_pawns(oriented),
            delta_str,
            pv_san.join(" "),
            settled_str,
        )
        .unwrap();
    }
    out
}

/// Render a `[settles ply N]` / `[settles leaf]` suffix for a PV given
/// its `settled_ply`. Empty string when the PV is empty or no settled
/// index is reported.
pub(crate) fn format_settled_suffix(pv: &[chess_tutor_engine::types::Move], settled: Option<usize>) -> String {
    match settled {
        None => String::new(),
        Some(i) if pv.is_empty() => {
            let _ = i;
            String::new()
        }
        Some(i) if i + 1 == pv.len() => "[settles leaf]".to_string(),
        Some(i) => format!("[settles ply {}]", i + 1),
    }
}

/// Dump per-PV ply-by-ply trajectory: white-POV tempo-free score at each
/// ply plus the delta from the previous ply. A leading "pre" row shows
/// the root's static eval so the reader sees the baseline the PV is
/// shifting off of. The settled ply is marked with a `*`.
pub(crate) fn render_debug_trajectory(
    root: &Position,
    lines: &[chess_tutor_engine::engine::SearchLine],
) -> String {
    use chess_tutor_engine::eval::evaluate_with_trace;
    use chess_tutor_engine::search::{stm_after_ply, SETTLED_THRESHOLD_CP};
    use std::fmt::Write;

    let mut out = String::new();
    writeln!(
        out,
        "debug: per-ply trajectory (white-POV, tempo-free; threshold for settled = {} cp)",
        SETTLED_THRESHOLD_CP
    )
    .unwrap();

    let root_stm = root.side_to_move();
    // The root's own trace — captured at the pre-move position, which is
    // evaluated from root_stm's perspective. `white_pov_value` normalises
    // it for us.
    let (_, root_trace) = evaluate_with_trace(root);
    let root_white_pov = root_trace.white_pov_value(root_stm).0;

    for (i, line) in lines.iter().enumerate() {
        let pv_san = san::pv_to_san(root, &line.pv);
        writeln!(
            out,
            "  pv {} ({}):",
            i + 1,
            if pv_san.is_empty() {
                "(empty)".to_string()
            } else {
                pv_san.join(" ")
            }
        )
        .unwrap();
        writeln!(
            out,
            "     pre                {:>+6}        —",
            root_white_pov
        )
        .unwrap();

        let mut prev = root_white_pov;
        for (ply, trace) in line.ply_traces.iter().enumerate() {
            let stm = stm_after_ply(root_stm, ply);
            let cp = trace.white_pov_value(stm).0;
            let delta = cp - prev;
            let marker = if Some(ply) == line.settled_ply {
                "*"
            } else {
                " "
            };
            let san = pv_san.get(ply).map(|s| s.as_str()).unwrap_or("?");
            writeln!(
                out,
                "   {}  ply {:>2}  {:<8}  {:>+6} cp   {:>+5}",
                marker,
                ply + 1,
                san,
                cp,
                delta,
            )
            .unwrap();
            prev = cp;
        }
    }
    out
}
