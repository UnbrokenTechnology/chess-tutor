//! `chess-tutor` CLI — a thin driver over `chess-tutor-engine`.
//!
//!     chess-tutor board   [FEN]   — render a position as an ANSI board
//!     chess-tutor moves   [FEN]   — list every legal move (SAN, one per line)
//!     chess-tutor eval    [FEN]   — classical-eval per-term trace
//!     chess-tutor search  [FEN]   — run a search; print PV + score + leaf trace
//!     chess-tutor opening [FEN]   — identify the opening by ECO + name
//!     chess-tutor play    [flags] — interactive game (human vs engine by default)
//!     chess-tutor bench   [args]  — multi-position perf benchmark (SF11-compatible args)

mod analysis_report;
mod bench;
mod bench_fens;
mod board;
mod eval_report;
mod noise_bench;
mod play;
mod uci;
mod cli_args;
mod search_report;

use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;

use chess_tutor_engine::analysis::analyze_position;
use chess_tutor_engine::engine::{Engine, SearchParams};
use chess_tutor_engine::eval::evaluate_with_trace;
use chess_tutor_engine::movegen::legal_moves_vec;
use chess_tutor_engine::openings;
use chess_tutor_engine::position::Position;
use chess_tutor_engine::san;
use chess_tutor_engine::types::Move;

use crate::board::{render as render_board, RenderOptions};
use crate::cli_args::{Cli, Command, EngineColor};
use crate::search_report::{
    format_score_pawns, pv_to_san, render_debug_trajectory, render_multi_pv,
};

/// Resolve a user-supplied move string to a `Move` legal in `pos`.
/// Accepts SAN (`Nf3`, `Rxe6+`) or UCI (`g1f3`); tries SAN first.
fn parse_user_move(pos: &mut Position, input: &str) -> Result<Move> {
    match san::parse(pos, input) {
        Ok(mv) => Ok(mv),
        Err(san_err) => match uci::parse(pos, input) {
            Ok(mv) => Ok(mv),
            Err(uci_err) => Err(anyhow::anyhow!(
                "{input:?} parsed as neither SAN ({san_err}) nor UCI ({uci_err})",
            )),
        },
    }
}

// Heap profiler. When the `dhat-heap` feature is enabled, every
// allocation goes through dhat's wrapping allocator, which records the
// source line of every alloc/dealloc and writes a `dhat-heap.json`
// next to the binary on exit. View the result at
// https://nnethercote.github.io/dh_view/dh_view.html (runs locally in
// the browser; doesn't upload). Adds ~30 % runtime overhead.
#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn main() -> Result<()> {
    // Hold the dhat profiler for the lifetime of `main` so it writes
    // its report on Drop after the actual work is done.
    #[cfg(feature = "dhat-heap")]
    let _profiler = dhat::Profiler::new_heap();

    match Cli::parse().command {
        Command::Board {
            fen,
            ascii,
            flip,
            light_mode,
        } => {
            let pos = Position::from_fen(&fen).with_context(|| format!("parsing FEN {:?}", fen))?;
            let view = chess_tutor_ui::view::BoardView::compose(
                &pos,
                flip,
                None,
                None,
                &[],
                None,
                Vec::new(),
            );
            print!("{}", render_board(&view, &RenderOptions { ascii, light_mode }));
        }
        Command::Moves { fen } => {
            let mut pos =
                Position::from_fen(&fen).with_context(|| format!("parsing FEN {:?}", fen))?;
            let legal = legal_moves_vec(&mut pos);
            if legal.is_empty() {
                println!("(no legal moves)");
            } else {
                for mv in &legal {
                    println!("{:<8}  {}", san::format(&pos, *mv), uci::format(*mv));
                }
            }
        }
        Command::Eval { fen } => {
            let pos = Position::from_fen(&fen).with_context(|| format!("parsing FEN {:?}", fen))?;
            let (_v, trace) = evaluate_with_trace(&pos);
            print!("{}", eval_report::render(&trace));
        }
        Command::Opening { fen } => {
            let pos = Position::from_fen(&fen).with_context(|| format!("parsing FEN {:?}", fen))?;
            match openings::identify(&pos) {
                Some(op) => println!("{}  {}", op.eco, op.name),
                None => println!("(no opening matched)"),
            }
        }
        Command::Search {
            fen,
            depth,
            nodes,
            time_ms,
            multi_pv,
            debug,
            analyze,
            top_percent,
            threads,
            force_include,
            verbose_progress,
        } => {
            let mut pos =
                Position::from_fen(&fen).with_context(|| format!("parsing FEN {:?}", fen))?;
            let mut engine = Engine::default();
            // Resolve `--force-include` strings against the live
            // position so the user can write `--force-include Rc8+`
            // or `--force-include f1c8`. SAN parser is lenient on
            // disambiguation / check / capture markers; falls back
            // to UCI on failure.
            let force_include_moves = force_include
                .iter()
                .map(|s| parse_user_move(&mut pos, s)
                    .with_context(|| format!("parsing --force-include {s:?}")))
                .collect::<Result<Vec<_>>>()?;
            let params = SearchParams {
                max_depth: depth,
                max_nodes: nodes,
                max_time: time_ms.map(Duration::from_millis),
                multi_pv: multi_pv.max(1),
                game_history: Vec::new(),
                force_include: force_include_moves,
                verbose_progress,
                threads: threads.max(1),
                // One-shot CLI search/analyze — analytical, no bot mask.
                eval_mask: chess_tutor_engine::opponent::EvalMask::EMPTY,
            };

            if analyze {
                // Teaching-analysis path: same search under the hood,
                // but the output surfaces per-move term deltas rather
                // than the leaf trace.
                let analyses = analyze_position(&mut engine, &mut pos, params);
                if analyses.is_empty() {
                    println!("(no legal moves — terminal position)");
                    return Ok(());
                }
                println!("depth: {}", analyses[0].depth);
                print!("{}", analysis_report::render(&pos, &analyses, top_percent));
                if debug {
                    // Rebuild a Vec<SearchLine>-shaped view for the
                    // debug trajectory renderer. Cheap: we clone the
                    // fields it needs.
                    let lines: Vec<chess_tutor_engine::engine::SearchLine> = analyses
                        .iter()
                        .map(|a| chess_tutor_engine::engine::SearchLine {
                            pv: a.pv.clone(),
                            score: a.score,
                            depth: a.depth,
                            ply_traces: a.ply_traces.clone(),
                            settled_ply: a.settled_ply,
                        })
                        .collect();
                    println!();
                    print!("{}", render_debug_trajectory(&pos, &lines));
                }
                return Ok(());
            }

            let lines = engine.search(&mut pos, params);
            if lines.is_empty() {
                println!("(no legal moves — terminal position)");
                return Ok(());
            }

            if lines.len() == 1 {
                let line = &lines[0];
                println!("depth:    {}", line.depth);
                println!("score:    {}", format_score_pawns(line.score));
                let pv_san = pv_to_san(&pos, &line.pv);
                println!("pv:       {}", pv_san.join(" "));
                if let Some(settled) = line.settled_ply {
                    println!(
                        "settled:  ply {} of {} ({})",
                        settled + 1,
                        line.pv.len(),
                        pv_san.get(settled).map(|s| s.as_str()).unwrap_or("?"),
                    );
                }
                println!(
                    "nodes:    {} in {} ms ({:.2} Mnps)",
                    engine.last_nodes(),
                    engine.last_elapsed().as_millis(),
                    engine.last_nps() / 1.0e6,
                );
                // TEMPORARY: pawn-cache hit rate for perf investigation.
                let (hits, misses) = engine.pawn_cache_stats();
                let total = hits + misses;
                if total > 0 {
                    println!(
                        "pawn$:    {} probes, {} hits, {} misses ({:.2}% hit rate)",
                        total,
                        hits,
                        misses,
                        100.0 * hits as f64 / total as f64,
                    );
                }
                println!();
                // The leaf trace is the last entry in ply_traces; that's
                // what the existing renderer expects.
                if let Some(leaf) = line.ply_traces.last() {
                    print!("{}", eval_report::render(leaf));
                }
            } else {
                println!("depth: {}", lines[0].depth);
                print!("{}", render_multi_pv(&pos, &lines));
            }

            if debug {
                println!();
                print!("{}", render_debug_trajectory(&pos, &lines));
            }
        }
        Command::Bench {
            tt_mb,
            threads,
            limit,
            fen_file,
            limit_type,
            new_game_between_positions,
            verbose,
            positions,
        } => {
            bench::run(bench::BenchArgs {
                tt_mb,
                threads,
                limit,
                fen_file,
                limit_type,
                new_game_between_positions,
                verbose,
                positions,
            })?;
        }
        Command::NoiseBench {
            tt_mb,
            depth,
            multi_pv,
            threads,
            runs,
            fen_file,
        } => {
            noise_bench::run(noise_bench::NoiseBenchArgs {
                tt_mb,
                depth,
                multi_pv,
                threads,
                runs,
                fen_file,
            })?;
        }
        Command::Play {
            fen,
            engine_color,
            depth,
            retrospective_depth,
            time_ms,
            ascii,
            flip,
            light_mode,
            no_explain_best,
            show_fens,
            threads,
            seed,
            no_book,
            disable_eval,
            noise_pool,
            noise_temp,
            blunder_chance,
            blunder_min_loss,
            blunder_max_loss,
            guaranteed_mate_in,
            wild_chance,
        } => {
            let mut opponent = match seed {
                Some(s) => chess_tutor_engine::opponent::OpponentProfile::with_seed(s),
                None => chess_tutor_engine::opponent::OpponentProfile::new_random(),
            };
            if no_book {
                opponent.book = chess_tutor_engine::opponent::BookSelection::None;
            }
            if let Some(list) = disable_eval {
                for token in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    let cat = chess_tutor_engine::opponent::EvalCategory::from_slug(token)
                        .with_context(|| {
                            format!(
                                "unknown eval category {:?} (try one of: pawn-structure, pieces, mobility, king-safety, threats, passed-pawns, space, initiative)",
                                token,
                            )
                        })?;
                    opponent.eval_mask.disable(cat);
                }
            }
            if !(0.0..=1.0).contains(&blunder_chance) {
                anyhow::bail!("--blunder-chance must be in [0.0, 1.0], got {blunder_chance}");
            }
            if !(0.0..=1.0).contains(&wild_chance) {
                anyhow::bail!("--wild-chance must be in [0.0, 1.0], got {wild_chance}");
            }
            if noise_pool == 0 {
                anyhow::bail!("--noise-pool must be at least 1");
            }
            if blunder_min_loss < 0 || blunder_max_loss < blunder_min_loss {
                anyhow::bail!(
                    "--blunder-min-loss / --blunder-max-loss must be 0 <= min <= max (got min={blunder_min_loss}, max={blunder_max_loss})",
                );
            }
            opponent.noise = chess_tutor_engine::opponent::NoiseProfile {
                candidate_pool: noise_pool,
                temperature_cp: noise_temp,
                blunder_chance,
                blunder_min_loss_cp: blunder_min_loss,
                blunder_max_loss_cp: blunder_max_loss,
                guaranteed_mate_in,
                wild_chance,
            };
            play::play_loop(play::PlayConfig {
                start_fen: fen,
                engine_color,
                depth,
                retrospective_depth,
                time_ms,
                ascii,
                flip,
                light_mode,
                opponent,
                explain_best: !no_explain_best,
                show_fens,
                threads: threads.max(1),
            })?;
        }
    }
    Ok(())
}
