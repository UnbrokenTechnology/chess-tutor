//! `chess-tutor noise-bench` — measure Lazy SMP score variance to
//! calibrate the [`MoveVerdict`] threshold buffer.
//!
//! For each position in the supplied list, run `analyze_position` N
//! times (with `engine.new_game()` between runs so the TT can't carry
//! the answer from one run into the next) using the same params the
//! desktop retrospective uses by default. For every move that appears
//! in at least two of those runs, compute the (max − min) score range
//! across the runs it appeared in. Print per-position stats plus an
//! aggregate percentile distribution so we can pick a noise buffer
//! from data instead of guessing.
//!
//! Why this matters: the [`crate::analysis::verdict`] classifier
//! compares `best_score − user_score` against a `BEST_LOSS_MAX`
//! constant. If Lazy SMP can wobble that delta by more than the
//! constant, the same move at the same position gets different
//! verdicts across runs and after takebacks — a major teaching-tool
//! disconnect. Setting `BEST_LOSS_MAX` above the measured 95th
//! percentile of variance absorbs the noise.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};

use chess_tutor_engine::analysis::analyze_position;
use chess_tutor_engine::engine::{Engine, SearchParams};
use chess_tutor_engine::movegen::legal_moves_vec;
use chess_tutor_engine::types::{Move, Value};

use crate::bench_fens::{parse_bench_entry, DEFAULT_BENCH_FENS};

pub struct NoiseBenchArgs {
    pub tt_mb: usize,
    pub depth: u32,
    pub multi_pv: usize,
    pub threads: usize,
    pub runs: usize,
    /// `default` for the 45-position SF11 bench list, or a path to a
    /// FEN file (same format as `chess-tutor bench`).
    pub fen_file: String,
}

pub fn run(args: NoiseBenchArgs) -> Result<()> {
    if args.runs < 2 {
        return Err(anyhow!(
            "noise-bench: --runs must be >= 2 to measure variance"
        ));
    }
    if args.threads < 1 {
        return Err(anyhow!("noise-bench: --threads must be >= 1"));
    }
    if args.multi_pv < 1 {
        return Err(anyhow!("noise-bench: --multi-pv must be >= 1"));
    }

    let positions = load_positions(&args.fen_file)?;
    if positions.is_empty() {
        return Err(anyhow!("noise-bench: no positions to search"));
    }

    println!(
        "noise-bench: {} positions, TT = {} MB, depth = {}, multi_pv = {}, threads = {}, runs = {}",
        positions.len(),
        args.tt_mb,
        args.depth,
        args.multi_pv,
        args.threads,
        args.runs,
    );
    println!();

    // Per-position worst variance (max move-score range across the
    // move's appearances). Aggregated at the end into percentiles.
    let mut per_position_max_range: Vec<i32> = Vec::with_capacity(positions.len());

    let mut engine = Engine::new(args.tt_mb);

    for (i, entry) in positions.iter().enumerate() {
        let mut pos =
            parse_bench_entry(entry).with_context(|| format!("parsing bench entry {}", i + 1))?;

        if legal_moves_vec(&mut pos).is_empty() {
            let label = if pos.in_check() {
                "checkmate"
            } else {
                "stalemate"
            };
            println!("  {:>2}/{}  (terminal — {})", i + 1, positions.len(), label);
            continue;
        }

        // move → Vec<score across runs>. We only record a score when
        // the move appeared in that run's top-`multi_pv`.
        let mut scores_by_move: HashMap<Move, Vec<i32>> = HashMap::new();
        // Per-run gap between PV[0] and PV[1] — the natural "how much
        // better is the best move than the runner-up" gap. Useful as a
        // chess-truth reference: in the opening this is often tiny
        // (many moves tied), in the endgame it's usually larger.
        let mut per_run_top_two_gaps: Vec<i32> = Vec::with_capacity(args.runs);

        for _ in 0..args.runs {
            engine.new_game();
            let params = SearchParams {
                max_depth: args.depth,
                max_nodes: None,
                max_time: None,
                multi_pv: args.multi_pv,
                game_history: Vec::new(),
                force_include: Vec::new(),
                verbose_progress: false,
                threads: args.threads,
                eval_mask: chess_tutor_engine::opponent::EvalMask::EMPTY,
                qsearch_max_plies: None,
                endgame_skill: chess_tutor_engine::endgame::EndgameSkill::Full,
                perception: None,
            };
            let analyses = analyze_position(&mut engine, &mut pos.clone(), params);
            for a in &analyses {
                if let Some(&mv) = a.pv.first() {
                    scores_by_move.entry(mv).or_default().push(a.score.0);
                }
            }
            // Top-two gap: PV[0].score − PV[1].score, side-to-move POV.
            // Clamp at 0 because of the documented MultiPV quirk where
            // per-slot scores aren't strictly monotonic across slots.
            if analyses.len() >= 2 {
                let gap = (analyses[0].score.0 - analyses[1].score.0).max(0);
                per_run_top_two_gaps.push(gap);
            }
        }

        // For each move that appeared in 2+ runs, compute its score
        // range. Worst-case for the position is the max of these.
        let mut max_range: i32 = 0;
        for scores in scores_by_move.values() {
            if scores.len() < 2 {
                continue;
            }
            let lo = *scores.iter().min().unwrap();
            let hi = *scores.iter().max().unwrap();
            let range = hi - lo;
            if range > max_range {
                max_range = range;
            }
        }
        per_position_max_range.push(max_range);

        // Top-two gap stats for this position. We report min, median,
        // max across runs since the gap itself wobbles run-to-run.
        let (gap_min, gap_med, gap_max) = if per_run_top_two_gaps.is_empty() {
            (0, 0, 0)
        } else {
            let mut sorted = per_run_top_two_gaps.clone();
            sorted.sort_unstable();
            let med_idx = sorted.len() / 2;
            (sorted[0], sorted[med_idx], sorted[sorted.len() - 1])
        };

        println!(
            "  {:>2}/{}  same-move range: {:>4} cp  | #1→#2 gap (min/med/max): {:>4}/{:>4}/{:>4} cp",
            i + 1,
            positions.len(),
            max_range,
            gap_min,
            gap_med,
            gap_max,
        );
    }

    println!();
    println!("aggregate (max move-score range per position, in engine-internal cp):");
    let mut sorted = per_position_max_range.clone();
    sorted.sort_unstable();
    if sorted.is_empty() {
        println!("  (no data — every position was terminal)");
        return Ok(());
    }
    let pct = |p: f32| -> i32 {
        let idx = ((sorted.len() as f32 - 1.0) * p).round() as usize;
        sorted[idx]
    };
    println!("  positions:   {}", sorted.len());
    println!(
        "  min:        {:>4} cp",
        sorted.first().copied().unwrap_or(0)
    );
    println!("  p50:        {:>4} cp", pct(0.50));
    println!("  p75:        {:>4} cp", pct(0.75));
    println!("  p90:        {:>4} cp", pct(0.90));
    println!("  p95:        {:>4} cp", pct(0.95));
    println!("  p99:        {:>4} cp", pct(0.99));
    println!(
        "  max:        {:>4} cp",
        sorted.last().copied().unwrap_or(0)
    );
    let mean = sorted.iter().sum::<i32>() as f32 / sorted.len() as f32;
    println!("  mean:       {:>6.1} cp", mean);
    println!();
    println!(
        "Suggested BEST_LOSS_MAX: {} cp (p95 of per-position max range, rounded up to a clean number).",
        round_up(pct(0.95))
    );
    println!(
        "  — anything within that window of #1 will classify as Excellent, absorbing Lazy SMP noise."
    );

    let _ = Duration::ZERO; // (used by sibling bench; kept import shape consistent)
    Ok(())
}

/// Round `n` up to the next multiple of 10 so the suggested threshold
/// reads as a clean tuning number.
fn round_up(n: i32) -> i32 {
    if n <= 0 {
        0
    } else {
        ((n + 9) / 10) * 10
    }
}

fn load_positions(fen_file: &str) -> Result<Vec<String>> {
    if fen_file == "default" {
        return Ok(DEFAULT_BENCH_FENS.iter().map(|s| s.to_string()).collect());
    }
    let contents = std::fs::read_to_string(fen_file)
        .with_context(|| format!("reading FEN file {fen_file:?}"))?;
    Ok(contents
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|s| s.to_string())
        .collect())
}

// Keep one Value reference so a future caller that wants to print
// scores directly doesn't need to re-import it.
const _: fn() = || {
    let _ = Value::ZERO;
};
