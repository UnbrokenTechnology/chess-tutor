//! `chess-tutor` CLI — a thin driver over `chess-tutor-engine`.
//!
//!     chess-tutor board   [FEN]   — render a position as an ANSI board
//!     chess-tutor moves   [FEN]   — list every legal move (SAN, one per line)
//!     chess-tutor eval    [FEN]   — classical-eval per-term trace
//!     chess-tutor search  [FEN]   — run a search; print PV + score + leaf trace
//!     chess-tutor opening [FEN]   — identify the opening by ECO + name
//!     chess-tutor play    [flags] — interactive game (human vs engine by default)

mod analysis_report;
mod board;
mod eval_report;
mod play;
mod retrospective;
mod uci;

use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};

use chess_tutor_engine::analysis::analyze_position;
use chess_tutor_engine::engine::{Engine, SearchParams};
use chess_tutor_engine::eval::evaluate_with_trace;
use chess_tutor_engine::movegen::legal_moves_vec;
use chess_tutor_engine::openings;
use chess_tutor_engine::position::Position;
use chess_tutor_engine::san;

use crate::board::{render as render_board, RenderOptions};

const STARTPOS: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

#[derive(Parser)]
#[command(
    name = "chess-tutor",
    version,
    about = "Classical chess engine + teaching tool."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Render a FEN as a Unicode/ANSI chess board.
    Board {
        #[arg(default_value = STARTPOS)]
        fen: String,
        #[arg(long)]
        ascii: bool,
        #[arg(long)]
        flip: bool,
        #[arg(long)]
        light_mode: bool,
    },
    /// List every legal move in SAN, one per line.
    Moves {
        #[arg(default_value = STARTPOS)]
        fen: String,
    },
    /// Print the classical-eval per-term trace for a FEN.
    Eval {
        #[arg(default_value = STARTPOS)]
        fen: String,
    },
    /// Identify the opening (ECO code + name) of a position, if known.
    Opening {
        #[arg(default_value = STARTPOS)]
        fen: String,
    },
    /// Run an engine search; print the principal variation and the leaf
    /// [`EvalTrace`]. With `--multi-pv N > 1`, prints N ranked lines
    /// each with its score and the score delta from the top line.
    Search {
        #[arg(default_value = STARTPOS)]
        fen: String,
        /// Maximum iterative-deepening depth (plies).
        #[arg(long, default_value_t = 10)]
        depth: u32,
        /// Stop after this many nodes.
        #[arg(long)]
        nodes: Option<u64>,
        /// Stop after this wall-clock duration (milliseconds).
        #[arg(long)]
        time_ms: Option<u64>,
        /// Return up to this many ranked principal variations (default
        /// 1 = single best line). Only the top line includes the leaf
        /// [`EvalTrace`]; additional lines show PV, score, and the
        /// delta-from-top.
        #[arg(long, default_value_t = 1)]
        multi_pv: usize,
        /// Dump a per-ply trajectory table for each PV: the white-POV,
        /// tempo-free score at each ply along with the delta from the
        /// previous ply. Useful for tuning the settled-ply threshold
        /// and for understanding the ply-to-ply "sawtooth" where each
        /// side's move temporarily shifts the eval before the opponent
        /// responds.
        #[arg(long)]
        debug: bool,
        /// For each returned PV, print the teaching-pipeline term-delta
        /// attribution: what named evaluation terms shifted between the
        /// root position and the "settled" ply of the move's PV, in
        /// tapered engine-cp, sorted by the size of the swing.
        #[arg(long)]
        analyze: bool,
        /// Cumulative `|delta|` coverage percent used by `--analyze` to
        /// pick how many term rows to show per move. 75 = smallest row
        /// prefix whose absolute-delta sum is at least 75% of the
        /// total. Higher values show more detail.
        #[arg(long, default_value_t = 75.0)]
        top_percent: f32,
    },
    /// Interactive REPL. Human enters SAN or UCI; engine replies on
    /// its turn.
    Play {
        /// Seed from this FEN instead of the start position.
        #[arg(long)]
        fen: Option<String>,
        /// Which side the engine plays.
        #[arg(long, value_enum, default_value_t = EngineColor::Black)]
        engine_color: EngineColor,
        /// Max search depth for the engine (plies).
        #[arg(long, default_value_t = 10)]
        depth: u32,
        /// Engine time cap per move (milliseconds). Omit for pure
        /// depth-capped search.
        #[arg(long)]
        time_ms: Option<u64>,
        #[arg(long)]
        ascii: bool,
        #[arg(long)]
        flip: bool,
        #[arg(long)]
        light_mode: bool,
        /// When true, the automatic retrospective narrates *why* each
        /// `Best` move was best (per-term breakdown), not just the
        /// congratulatory headline. Default off — the student can
        /// flip it on at runtime via the REPL `explain-best` toggle.
        #[arg(long)]
        explain_best: bool,
        /// When true, print the current FEN before each side's turn.
        /// Useful for debugging — if the engine hangs or plays a bad
        /// move, the last-printed FEN reproduces the position exactly.
        #[arg(long)]
        show_fens: bool,
        /// Diagnostic: call `Engine::new_game()` before every engine
        /// move, clearing the transposition table and butterfly
        /// history. Isolates whether accumulated state is the cause
        /// of pathological search times in long self-play games. Not
        /// for normal use — gives up all TT / history learning.
        #[arg(long)]
        reset_engine_per_move: bool,
        /// Diagnostic: write iterative-deepening + root-move progress
        /// to stderr during every engine search. Useful for spotting
        /// search loops or depth-N iterations that never return.
        #[arg(long)]
        search_progress: bool,
    },
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum EngineColor {
    /// Engine plays white; human plays black.
    White,
    /// Engine plays black; human plays white (default).
    Black,
    /// Engine plays both sides (self-play).
    Both,
    /// Neither side is the engine — human controls both. Useful for
    /// exploring positions.
    None,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Board {
            fen,
            ascii,
            flip,
            light_mode,
        } => {
            // Validate the FEN through the engine so we fail fast on
            // bad input, but we render from the string directly.
            let _ = Position::from_fen(&fen).with_context(|| format!("parsing FEN {:?}", fen))?;
            print!(
                "{}",
                render_board(
                    &fen,
                    &RenderOptions {
                        ascii,
                        flip,
                        highlight: None,
                        light_mode,
                    },
                )
            );
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
        } => {
            let mut pos =
                Position::from_fen(&fen).with_context(|| format!("parsing FEN {:?}", fen))?;
            let mut engine = Engine::default();
            let params = SearchParams {
                max_depth: depth,
                max_nodes: nodes,
                max_time: time_ms.map(Duration::from_millis),
                multi_pv: multi_pv.max(1),
                game_history: Vec::new(),
                force_include: Vec::new(),
                verbose_progress: false,
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
        Command::Play {
            fen,
            engine_color,
            depth,
            time_ms,
            ascii,
            flip,
            light_mode,
            explain_best,
            show_fens,
            reset_engine_per_move,
            search_progress,
        } => {
            play::play_loop(play::PlayConfig {
                start_fen: fen,
                engine_color,
                depth,
                time_ms,
                ascii,
                flip,
                light_mode,
                explain_best,
                show_fens,
                reset_engine_per_move,
                search_progress,
            })?;
        }
    }
    Ok(())
}

/// Convert an engine PV (a vector of moves from the root) into a list
/// of SAN strings, playing the moves in order on a scratch position so
/// each SAN is formatted in the context where the move is actually
/// played.
fn pv_to_san(root: &Position, pv: &[chess_tutor_engine::types::Move]) -> Vec<String> {
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
fn format_score_pawns(score: chess_tutor_engine::types::Value) -> String {
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
fn render_multi_pv(root: &Position, lines: &[chess_tutor_engine::engine::SearchLine]) -> String {
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
fn format_settled_suffix(pv: &[chess_tutor_engine::types::Move], settled: Option<usize>) -> String {
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
fn render_debug_trajectory(
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
