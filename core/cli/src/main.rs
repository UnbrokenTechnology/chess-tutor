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
use chess_tutor_engine::types::Move;

use crate::board::{render as render_board, RenderOptions};

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
        /// Number of Lazy-SMP search threads. Default 1 for
        /// reproducible output; raise to use more cores when you
        /// don't need bit-identical results.
        #[arg(long, default_value_t = 1)]
        threads: usize,
        /// Force a move into the MultiPV result. Mirrors the
        /// retrospective's `force_include` so you can reproduce its
        /// pathological positions one-shot. Accepts SAN (`Nf3`,
        /// `Qxe6+`) or UCI (`g1f3`). Repeat the flag to force in
        /// multiple moves.
        #[arg(long = "force-include", value_name = "MOVE")]
        force_include: Vec<String>,
        /// Emit per-depth aspiration / fail-high / fail-low events to
        /// stderr. Useful for diagnosing aspiration blowups and
        /// pathological positions.
        #[arg(long)]
        verbose_progress: bool,
    },
    /// Multi-position search benchmark. Argument order and defaults
    /// mirror Stockfish 11's `bench` command: `tt_mb threads limit
    /// fen_file limit_type`, defaults `16 1 13 default depth`. Output
    /// finishes with an SF-style `Total time / Nodes searched /
    /// Nodes/second` aggregate so the numbers can be compared
    /// apples-to-apples against `stockfish bench`.
    Bench {
        /// Transposition-table size in MB. SF default is 16.
        #[arg(default_value_t = 16)]
        tt_mb: usize,
        /// Number of search threads. Only 1 is supported today (the
        /// engine is single-thread); the arg exists for SF parity.
        #[arg(default_value_t = 1)]
        threads: usize,
        /// Limit value — interpreted by `limit_type`. With the default
        /// `depth`, this is the maximum iterative-deepening depth in
        /// plies (SF default is 13).
        #[arg(default_value_t = 13)]
        limit: u64,
        /// `default` for the built-in 45-position list (mirrored from
        /// SF11), or a path to a file with one bench entry per line
        /// (same `<fen> [moves uci ...]` shape SF accepts).
        #[arg(default_value = "default")]
        fen_file: String,
        /// `depth` (default) or `nodes`. `movetime` / `perft` are not
        /// supported yet.
        #[arg(default_value = "depth")]
        limit_type: String,
        /// Call `engine.new_game()` between every position, clearing
        /// the TT, history, and pawn cache. Off by default to match
        /// SF's behaviour (one `ucinewgame` at the start of bench,
        /// TT carries across positions). Useful for isolating
        /// per-position performance from cross-position TT pollution
        /// — at large TT sizes (e.g. 128 MB), entries from earlier
        /// bench positions can displace deeper entries the later
        /// positions want, causing dramatic per-position regressions
        /// vs. the small-TT case.
        #[arg(long)]
        new_game_between_positions: bool,
        /// TEMPORARY perf-investigation: after each position completes,
        /// print selDepth and a compact per-ply node histogram. Also
        /// enables per-ID-iteration heartbeat output from the search.
        /// Doesn't affect search behaviour, just adds stderr/stdout
        /// output.
        #[arg(long)]
        verbose: bool,
        /// TEMPORARY perf-investigation: comma-separated list of
        /// 1-based position indices to run (e.g. `20,26,40,41`). When
        /// set, only those positions from the FEN list are searched;
        /// others are skipped. Useful for focusing on known-slow FENs
        /// without sitting through the rest. Indexing matches the
        /// bench-output `N/45` numbering.
        #[arg(long)]
        positions: Option<String>,
    },
    /// Measure Lazy-SMP score variance across runs. For each position,
    /// runs `analyze_position` N times with a fresh engine state and
    /// reports how much the same move's score wobbles. Used to
    /// calibrate the [`MoveVerdict`] noise buffer.
    NoiseBench {
        /// Transposition-table size in MB.
        #[arg(long, default_value_t = 16)]
        tt_mb: usize,
        /// Search depth per run. Defaults to the retrospective's
        /// `DEFAULT_DEPTH` (10) so the measurement reflects what users
        /// actually see.
        #[arg(long, default_value_t = 10)]
        depth: u32,
        /// Multi-PV breadth per run. Defaults to the retrospective's
        /// `RETROSPECTIVE_MULTI_PV` (3).
        #[arg(long, default_value_t = 3)]
        multi_pv: usize,
        /// Number of threads. Defaults to 8 — typical Lazy-SMP load on
        /// the desktop's `available_parallelism()` default.
        #[arg(long, default_value_t = 8)]
        threads: usize,
        /// Number of runs per position. Variance estimate improves
        /// with N; 5 is a reasonable starting point.
        #[arg(long, default_value_t = 5)]
        runs: usize,
        /// `default` for the built-in 45-position SF11 set, or a path
        /// to a FEN file (same format as `chess-tutor bench`).
        #[arg(long, default_value = "default")]
        fen_file: String,
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
        /// Suppress the per-term breakdown on `Best` verdicts —
        /// only the congratulatory headline prints. Default behaviour
        /// is to narrate *why* the move was best so the student who
        /// guessed right still learns the reasoning. Toggle at
        /// runtime via the REPL `explain-best` command.
        #[arg(long = "no-explain-best", action = clap::ArgAction::SetTrue)]
        no_explain_best: bool,
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
        /// Number of search threads (Lazy SMP) for **every** search:
        /// engine moves AND the auto-retrospective. Default 1 keeps
        /// every search bit-deterministic across runs and takebacks
        /// — the teaching contract is "same position, same verdict".
        /// Raise it for benchmarking. REPL `search` / `analyze`
        /// commands are always single-threaded.
        #[arg(long, default_value_t = 1)]
        threads: usize,
        /// Seed for the opponent's pseudo-randomness (opening line
        /// pick in Phase B, move sampling in later phases). Default:
        /// random per run, logged at game start. Pass a fixed value
        /// to replay an identical bot game.
        #[arg(long)]
        seed: Option<u64>,
        /// Disable the opening book for this game. Default behaviour
        /// is to pick a random line from the curated default set; pass
        /// this flag to force the engine to search from move 1.
        #[arg(long = "no-book", action = clap::ArgAction::SetTrue)]
        no_book: bool,
        /// Comma-separated list of evaluation categories the bot
        /// should be blind to for this game (e.g.
        /// `--disable-eval king-safety,pawn-structure`). Categories:
        /// pawn-structure | pieces | mobility | king-safety | threats
        /// | passed-pawns | space | initiative. The mid-game REPL
        /// `eval-mask` command can toggle individual categories.
        #[arg(long = "disable-eval", value_name = "CATEGORY[,CATEGORY...]")]
        disable_eval: Option<String>,
        /// How many top search lines the bot may sample from when
        /// softmax noise fires. Default 1 (no sampling — always #1).
        /// Pair with `--noise-temp` to actually pick from the pool;
        /// higher values cost roughly K× the per-move search time.
        #[arg(long = "noise-pool", value_name = "N", default_value_t = 1)]
        noise_pool: usize,
        /// Softmax temperature in centipawns. Default 0 (always pick
        /// #1 even when `--noise-pool > 1`). At 50 a line 50 cp behind
        /// has ~37% the weight of #1; at 200 it has ~78%. Use to dial
        /// up variety among close-scoring moves.
        #[arg(long = "noise-temp", value_name = "CP", default_value_t = 0)]
        noise_temp: i32,
        /// Per-move probability the bot drops a deliberate blunder
        /// (range 0.0–1.0). Default 0.0 (off). When > 0, the search
        /// widens to surface enough worse-than-best alternatives.
        #[arg(long = "blunder-chance", value_name = "P", default_value_t = 0.0)]
        blunder_chance: f32,
        /// Minimum loss (centipawns vs #1) for an alternative line to
        /// count as "in band" for the blunder picker. Default 100 — a
        /// clear pawn-down move the student can plausibly punish.
        #[arg(long = "blunder-min-loss", value_name = "CP", default_value_t = 100)]
        blunder_min_loss: i32,
        /// Maximum loss (centipawns vs #1) for an alternative line to
        /// count as "in band". Default 400 — caps blunders at roughly
        /// an exchange sacrifice; raise to allow more catastrophic
        /// blunders (~900 for queen hangs). When the band is empty
        /// the picker falls back to the closest-loss lines on each
        /// side of the band but excludes distant outliers.
        #[arg(long = "blunder-max-loss", value_name = "CP", default_value_t = 400)]
        blunder_max_loss: i32,
        /// Smallest mate the bot is guaranteed to convert — blunders
        /// are suppressed when `lines[0]` is a mate-in-N for
        /// `N <= guaranteed_mate_in`. Default 1 (mate-in-1 is never
        /// blundered). Set to 0 to allow blunders against any mate.
        #[arg(long = "guaranteed-mate-in", value_name = "N", default_value_t = 1)]
        guaranteed_mate_in: u32,
        /// Per-move probability the bot picks uniformly from ALL legal
        /// moves, bypassing the engine ranking entirely (range
        /// 0.0–1.0). Default 0.0 (off). This is the "beginner bot"
        /// branch — only it can pick moves the engine didn't surface
        /// (e.g. leaving a piece in a pawn's path). Same mate-guard
        /// as `--blunder-chance`.
        #[arg(long = "wild-chance", value_name = "P", default_value_t = 0.0)]
        wild_chance: f32,
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
            time_ms,
            ascii,
            flip,
            light_mode,
            no_explain_best,
            show_fens,
            reset_engine_per_move,
            search_progress,
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
                time_ms,
                ascii,
                flip,
                light_mode,
                opponent,
                explain_best: !no_explain_best,
                show_fens,
                reset_engine_per_move,
                search_progress,
                threads: threads.max(1),
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
