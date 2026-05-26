//! Rendering helpers for the `search` / `analyze` CLI output: PV-to-SAN
//! conversion, score formatting, multi-PV tables, settled-ply suffixes, and
//! the debug eval-trajectory dump. Split out of `main.rs`; called only by the
//! driver in [`crate::main`].

use chess_tutor_engine::position::Position;
use chess_tutor_engine::san;

/// Convert an engine PV (a vector of moves from the root) into a list
/// of SAN strings, playing the moves in order on a scratch position so
/// each SAN is formatted in the context where the move is actually
/// played.
pub(crate) fn pv_to_san(root: &Position, pv: &[chess_tutor_engine::types::Move]) -> Vec<String> {
    let mut out = Vec::with_capacity(pv.len());
    let mut scratch = root.clone();
    for mv in pv {
        out.push(san::format_on(&mut scratch, *mv));
        scratch.do_move(*mv);
    }
    out
}

/// Render a score as pawns (`+0.28`, `-1.05`) or mate notation
/// (`#5`, `-#3`) from the root side-to-move's point of view. Matches the
/// convention the REPL uses.
pub(crate) fn format_score_pawns(score: chess_tutor_engine::types::Value) -> String {
    use chess_tutor_engine::types::Value;
    let abs = score.0.abs();
    let mate_threshold = Value::MATE.0 - Value::MAX_PLY;
    if abs >= mate_threshold {
        // Plies-to-mate = MATE - abs_score. Moves = (plies + 1) / 2.
        let plies = Value::MATE.0 - abs;
        let moves = (plies + 1) / 2;
        if score.0 >= 0 {
            format!("#{}", moves)
        } else {
            format!("-#{}", moves)
        }
    } else {
        format!("{:+.2}", score.0 as f32 / 100.0)
    }
}

/// Render multiple ranked PVs as aligned rows. The first line's delta
/// column reads `(0 cp)` (since it's the leader); subsequent lines show
/// delta-from-top. Column widths are chosen so every PV starts in the
/// same output column.
pub(crate) fn render_multi_pv(root: &Position, lines: &[chess_tutor_engine::engine::SearchLine]) -> String {
    use std::fmt::Write;
    let top_score = lines[0].score.0;
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        let pv_san = pv_to_san(root, &line.pv);
        let delta = line.score.0 - top_score;
        let delta_str = if delta == 0 {
            "(0 cp)".to_string()
        } else {
            format!("({:+} cp)", delta)
        };
        let settled_str = format_settled_suffix(&line.pv, line.settled_ply);
        writeln!(
            out,
            "  {:>2}. {:>6}   {:<10}  {:<36}  {}",
            i + 1,
            format_score_pawns(line.score),
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
        let pv_san = pv_to_san(root, &line.pv);
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
