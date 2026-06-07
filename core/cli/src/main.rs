//! `chess-tutor` CLI — a thin driver over `chess-tutor-engine`.
//!
//!     chess-tutor board   [FEN]   — render a position as an ANSI board
//!     chess-tutor moves   [FEN]   — list every legal move (SAN, one per line)
//!     chess-tutor eval    [FEN]   — classical-eval per-term trace
//!     chess-tutor search  [FEN]   — run a search; print PV + score + leaf trace
//!     chess-tutor opening [FEN]   — identify the opening by ECO + name
//!     chess-tutor play    [flags] — interactive game (human vs engine by default)
//!     chess-tutor bench   [args]  — multi-position perf benchmark (SF11-compatible args)

mod alignments_view;
mod analysis_report;
mod attacks_view;
mod bench;
mod bench_fens;
mod board;
mod cli_args;
mod eval_report;
mod forcing_view;
mod glossary;
mod noise_bench;
mod piece_fmt;
mod play;
mod search_report;
mod settled_audit;
mod square_view;
mod summary;
mod tactics_view;
mod threats_view;
mod uci;
mod uci_shim;
mod units;

use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;

use chess_tutor_engine::analysis::{analyze_position, find_latent_threats, find_threat_defusals};
use chess_tutor_engine::endgame::EndgameSkill;
use chess_tutor_engine::engine::{Engine, SearchParams};
use chess_tutor_engine::eval::evaluate_with_trace;
use chess_tutor_engine::movegen::legal_moves_vec;
use chess_tutor_engine::openings;
use chess_tutor_engine::position::Position;
use chess_tutor_engine::san;
use chess_tutor_engine::san::pv_to_san;
use chess_tutor_engine::types::Move;

use crate::board::{render as render_board, RenderOptions};
use crate::cli_args::{Cli, Command, EngineColor};
use crate::search_report::{render_debug_trajectory, render_multi_pv};

/// Search depth for the `tactics --latent` defusal block. Matches the
/// `explain` default so the two surfaces agree on defusal scores. Deep
/// enough to clear the horizon mis-scores that make a queen-dropping
/// decoy look like it holds (see `analysis::defusals`).
const TACTICS_DEFUSAL_DEPTH: u32 = 12;

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

/// Did a forced move give away the advantage, from the mover's own POV?
/// Both scores are side-to-move (mover) POV in raw engine-cp.
///
/// Two conditions, both required:
/// - **the move conceded more than a pawn** (`best − forced > 1.0`), and
/// - **it no longer leaves the mover clearly winning** (`forced < +1.0`).
///
/// This is deliberately broader than "crossed into a negative eval." It
/// catches two failure modes the cross-zero rule misses:
/// - *gave up a win without crossing zero* — `+2.0 → +0.2` (1.8-pawn
///   swing; you let the opponent neutralise a winning position), and
/// - *gave away the game from a neutral start* — `+0.2 → −3.0` (3.2-pawn
///   swing; the cross-zero rule wouldn't fire because you didn't start
///   ahead).
///
/// The `forced < +1.0` floor is what keeps "still clearly winning" slips
/// quiet: `+5.0 → +3.0` concedes two pawns but you're still up three, so
/// it is not the "you handed it over" lesson this banner is for.
fn gave_away_advantage(
    best: chess_tutor_engine::types::Value,
    forced: chess_tutor_engine::types::Value,
) -> bool {
    // PawnEG is exactly 1.0 conventional pawn on the engine's scale.
    let one_pawn = chess_tutor_engine::types::Value::PAWN_EG.0;
    let conceded = best.0 - forced.0;
    conceded > one_pawn && forced.0 < one_pawn
}

/// Print the "ALLOWED, not missed" reframe banner for a forced move that
/// [`gave_away_advantage`]. The whole point: when an agent reproduces a
/// move that conceded the advantage, the default instinct is "what better
/// move did I miss?" — but a large swing in the opponent's favour means
/// the move *let the opponent do something*, usually a standing threat or
/// counter that went unaccounted for. Reframe the question and point at
/// the defusal surfaces.
fn print_allowed_banner(
    pos: &Position,
    forced: Move,
    forced_pv: &[Move],
    best_score: chess_tutor_engine::types::Value,
    forced_score: chess_tutor_engine::types::Value,
    stm: chess_tutor_engine::types::Color,
) {
    use chess_tutor_engine::types::{Color, Value};
    let san = san::format(pos, forced);
    let best_p = crate::units::format_pawns(best_score);
    let forced_p = crate::units::format_pawns(forced_score);
    // Swing magnitude, mover-POV, as a positive pawn count.
    let swing = crate::units::engine_cp_to_pawns(Value(best_score.0 - forced_score.0));
    let bar = "!! ──────────────────────────────────────────────────────────────";
    println!("{bar}");
    println!(
        "!! ALLOWED, NOT MISSED — {san}: eval {best_p} → {forced_p} (your POV), a {swing:.1}-pawn"
    );
    println!("!! swing in the opponent's favour.");
    // Only show the white-POV cross-reference when the mover is Black —
    // for White it's identical to the your-POV line above and just adds
    // noise.
    if stm == Color::Black {
        let best_wp = crate::units::format_pawns(crate::units::to_white_pov(best_score, stm));
        let forced_wp = crate::units::format_pawns(crate::units::to_white_pov(forced_score, stm));
        println!("!! (white-POV / eval-bar: {best_wp} → {forced_wp}.)");
    }
    println!("!! This is not \"you missed a stronger move\" — your move ALLOWED the");
    println!("!! opponent a strong reply (a standing threat or counter you didn't address).");
    println!("!!   wrong question:  \"what better move did I have?\"");
    println!("!!   right question:  \"what did I let my opponent do?\"");
    // Show the concrete answer to that question: the opponent's punishing
    // continuation, straight from the forced move's own search PV. The
    // swing number alone says "you erred"; this line says "here is exactly
    // how it turns out", which is what stops the reader inventing a
    // different cause. `forced_pv[0]` is the forced move itself; the tail
    // is the refutation.
    let pv_san = pv_to_san(pos, forced_pv);
    if pv_san.len() > 1 {
        println!("!! how it turns out — {}", pv_san.join(" "));
        println!("!! (that line is what the search expects after {san}; read past your move to");
        println!("!!  see the reply that does the damage.)");
    }
    println!("!! Re-read the `danger:` block above; run `chess-tutor explain` or");
    println!("!! `chess-tutor tactics --latent` for the moves that hold the advantage.");
    println!("{bar}");
    println!();
}

/// One-line nudge printed at the top of `search` when no move was forced
/// in. The most common way an agent mis-diagnoses "why was my move bad?"
/// is to search the position *after* the move — which only shows that the
/// result is bad, never that the move *caused* it, and gives no swing vs.
/// the best alternative (so you don't even learn how bad it was). The fix
/// is `--force-include <move>` on the position *before* the move: it scores
/// the move against the best line and fires the [`print_allowed_banner`]
/// reframe on a winning→losing swing.
fn print_force_include_hint() {
    println!("hint:     judging a move you already played? run this on the position BEFORE");
    println!("          it with `--force-include <your move>` — you'll get its eval swing");
    println!("          vs. the best line and a flag if it gave away the advantage.");
    println!("          (Searching the position AFTER the move shows only that the result");
    println!("          is bad, not why your move was — and gives no swing to compare.)");
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

    let cli = Cli::parse();
    let json_mode = cli.json;
    // `--stm` flips score-orientation away from the white-POV default.
    // Currently honoured by `search` (single + multi-PV) so the agent
    // can ask for engine-internal side-to-move output when comparing
    // against another tool. The eval table's mg/eg columns are
    // inherently per-colour and don't need flipping.
    let stm_mode = cli.stm;

    match cli.command {
        Command::Board {
            fen,
            ascii,
            flip,
            light_mode,
        } => {
            let pos = Position::from_fen(&fen).with_context(|| format!("parsing FEN {:?}", fen))?;
            let summary_data = summary::build(&pos, summary::ScoreSource::Static, None);
            if json_mode {
                #[derive(serde::Serialize)]
                struct Out<'a> {
                    summary: &'a summary::PositionSummary,
                    board_fen: &'a str,
                }
                let payload = Out {
                    summary: &summary_data,
                    board_fen: &summary_data.fen,
                };
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                print!("{}", summary::render_text(&summary_data));
                println!();
                let view = chess_tutor_ui::view::BoardView::compose(
                    &pos,
                    flip,
                    None,
                    None,
                    &[],
                    None,
                    Vec::new(),
                );
                print!(
                    "{}",
                    render_board(&view, &RenderOptions { ascii, light_mode })
                );
            }
        }
        Command::Moves { fen } => {
            // Annotated legal-move list. Each row carries the SAN, the
            // UCI, and a short tag string covering: is this a check?
            // a capture (with the captured piece's classical-points
            // value)? a promotion? an en-passant? The agent reading
            // this never has to scan the list for `+` / `x` / `=`
            // by eye.
            use chess_tutor_engine::types::MoveKind;
            let mut pos =
                Position::from_fen(&fen).with_context(|| format!("parsing FEN {:?}", fen))?;
            let summary_data = summary::build(&pos, summary::ScoreSource::Static, None);
            let legal = legal_moves_vec(&mut pos);

            let annotate = |mv: chess_tutor_engine::types::Move| -> Vec<String> {
                let mut tags = Vec::new();
                if pos.gives_check(mv) {
                    tags.push("check".to_string());
                }
                if pos.is_capture(mv) {
                    let target_sq = if mv.kind() == MoveKind::EnPassant {
                        use chess_tutor_engine::types::Square;
                        Square::new(mv.to().file(), mv.from().rank())
                    } else {
                        mv.to()
                    };
                    if let Some(captured) = pos.piece_on(target_sq) {
                        tags.push(format!(
                            "captures {} ({} pts)",
                            piece_fmt::piece_label(captured, target_sq),
                            captured.kind().classical_points(),
                        ));
                    } else {
                        tags.push("capture".to_string());
                    }
                }
                if mv.kind() == MoveKind::Promotion {
                    tags.push(format!("promotion={:?}", mv.promoted_to()).to_lowercase());
                }
                if mv.kind() == MoveKind::EnPassant {
                    tags.push("en passant".to_string());
                }
                tags
            };

            if json_mode {
                #[derive(serde::Serialize)]
                struct Move {
                    san: String,
                    uci: String,
                    tags: Vec<String>,
                }
                #[derive(serde::Serialize)]
                struct Out<'a> {
                    summary: &'a summary::PositionSummary,
                    moves: Vec<Move>,
                }
                let payload = Out {
                    summary: &summary_data,
                    moves: legal
                        .iter()
                        .map(|mv| Move {
                            san: san::format(&pos, *mv),
                            uci: uci::format(*mv),
                            tags: annotate(*mv),
                        })
                        .collect(),
                };
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                print!("{}", summary::render_text(&summary_data));
                println!();
                if legal.is_empty() {
                    println!("(no legal moves)");
                } else {
                    for mv in &legal {
                        let tags = annotate(*mv);
                        let tag_str = if tags.is_empty() {
                            String::new()
                        } else {
                            format!("  ({})", tags.join(", "))
                        };
                        println!(
                            "{:<8}  {}{}",
                            san::format(&pos, *mv),
                            uci::format(*mv),
                            tag_str,
                        );
                    }
                }
            }
        }
        Command::Eval { fen, glossary } => {
            // `--glossary` is a standalone dump: skip the FEN entirely.
            if glossary {
                if json_mode {
                    #[derive(serde::Serialize)]
                    struct GlossRow {
                        id: String,
                        label: String,
                        pretty: String,
                        description: String,
                    }
                    let rows: Vec<GlossRow> = chess_tutor_engine::analysis::TermId::ALL
                        .iter()
                        .map(|&id| GlossRow {
                            id: format!("{:?}", id),
                            label: id.label().to_string(),
                            pretty: id.pretty_label().to_string(),
                            description: crate::glossary::description(id).to_string(),
                        })
                        .collect();
                    println!("{}", serde_json::to_string_pretty(&rows)?);
                } else {
                    print!("{}", crate::glossary::render_glossary_table());
                }
                return Ok(());
            }

            let pos = Position::from_fen(&fen).with_context(|| format!("parsing FEN {:?}", fen))?;
            let summary_data = summary::build(&pos, summary::ScoreSource::Static, None);
            let (_v, trace) = evaluate_with_trace(&pos);
            if json_mode {
                // Mirror the text output's information content as a
                // JSON-friendly shape. The full EvalTrace is engine-
                // internal; we surface the headline values + per-term
                // table as a portable schema.
                // Per-term `(mg, eg)` net values, in engine-internal
                // Score units. These are pre-taper components of the
                // classical eval; rendering them at any other scale
                // would distort the per-piece-square-table weights.
                #[derive(serde::Serialize)]
                struct TermRow {
                    id: String,
                    label: String,
                    description: String,
                    net_mg_engine: i32,
                    net_eg_engine: i32,
                }
                let term_rows: Vec<TermRow> = chess_tutor_engine::analysis::TermId::ALL
                    .iter()
                    .map(|&id| {
                        let net = id.net_score(&trace);
                        TermRow {
                            id: format!("{:?}", id),
                            label: id.label().to_string(),
                            description: crate::glossary::description(id).to_string(),
                            net_mg_engine: net.mg().0,
                            net_eg_engine: net.eg().0,
                        }
                    })
                    .collect();
                // Every cp value below this point in the eval JSON
                // payload is in engine-internal scale (PawnEG=213), not
                // conventional cp. The field names make that explicit.
                #[derive(serde::Serialize)]
                struct Out<'a> {
                    summary: &'a summary::PositionSummary,
                    phase: u32,
                    scale_factor: u32,
                    tempo_engine_cp: i32,
                    final_value_engine_cp_stm: i32,
                    terms: Vec<TermRow>,
                }
                let payload = Out {
                    summary: &summary_data,
                    phase: trace.phase as u32,
                    scale_factor: trace.scale_factor as u32,
                    tempo_engine_cp: trace.tempo.0,
                    final_value_engine_cp_stm: trace.final_value.0,
                    terms: term_rows,
                };
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                print!("{}", summary::render_text(&summary_data));
                println!();
                print!("{}", eval_report::render(&trace));
            }
        }
        Command::Opening { fen } => {
            let pos = Position::from_fen(&fen).with_context(|| format!("parsing FEN {:?}", fen))?;
            let summary_data = summary::build(&pos, summary::ScoreSource::Static, None);
            if json_mode {
                #[derive(serde::Serialize)]
                struct Out<'a> {
                    summary: &'a summary::PositionSummary,
                    opening: Option<&'a summary::OpeningBlock>,
                }
                let payload = Out {
                    summary: &summary_data,
                    opening: summary_data.opening.as_ref(),
                };
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                print!("{}", summary::render_text(&summary_data));
                println!();
                match openings::identify(&pos) {
                    Some(op) => println!("{}  {}", op.eco, op.name),
                    None => println!("(no opening matched)"),
                }
            }
        }
        Command::Square { square, fen } => {
            use chess_tutor_engine::types::Square;
            let pos = Position::from_fen(&fen).with_context(|| format!("parsing FEN {:?}", fen))?;
            let sq = Square::from_algebraic(&square)
                .with_context(|| format!("parsing square {:?} (expected e.g. 'e5')", square))?;
            let summary_data = summary::build(&pos, summary::ScoreSource::Static, None);
            let view = square_view::build(&pos, sq);
            if json_mode {
                #[derive(serde::Serialize)]
                struct Out<'a> {
                    summary: &'a summary::PositionSummary,
                    square: &'a square_view::SquareView,
                }
                let payload = Out {
                    summary: &summary_data,
                    square: &view,
                };
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                print!("{}", summary::render_text(&summary_data));
                println!();
                print!("{}", square_view::render_text(&view));
            }
        }
        Command::Threats { fen } => {
            let pos = Position::from_fen(&fen).with_context(|| format!("parsing FEN {:?}", fen))?;
            let summary_data = summary::build(&pos, summary::ScoreSource::Static, None);
            let view = threats_view::build(&pos);
            if json_mode {
                #[derive(serde::Serialize)]
                struct Out<'a> {
                    summary: &'a summary::PositionSummary,
                    threats: &'a threats_view::ThreatsView,
                }
                let payload = Out {
                    summary: &summary_data,
                    threats: &view,
                };
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                print!("{}", summary::render_text(&summary_data));
                println!();
                print!("{}", threats_view::render_text(&view));
            }
        }
        Command::Forcing { fen } => {
            let pos = Position::from_fen(&fen).with_context(|| format!("parsing FEN {:?}", fen))?;
            let summary_data = summary::build(&pos, summary::ScoreSource::Static, None);
            let view = forcing_view::build(&pos);
            if json_mode {
                #[derive(serde::Serialize)]
                struct Out<'a> {
                    summary: &'a summary::PositionSummary,
                    forcing: &'a forcing_view::ForcingView,
                }
                let payload = Out {
                    summary: &summary_data,
                    forcing: &view,
                };
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                print!("{}", summary::render_text(&summary_data));
                println!();
                print!("{}", forcing_view::render_text(&view));
            }
        }
        Command::Attacks { fen } => {
            let pos = Position::from_fen(&fen).with_context(|| format!("parsing FEN {:?}", fen))?;
            let summary_data = summary::build(&pos, summary::ScoreSource::Static, None);
            let view = attacks_view::build(&pos);
            if json_mode {
                #[derive(serde::Serialize)]
                struct Out<'a> {
                    summary: &'a summary::PositionSummary,
                    attacks: &'a attacks_view::AttacksView,
                }
                let payload = Out {
                    summary: &summary_data,
                    attacks: &view,
                };
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                print!("{}", summary::render_text(&summary_data));
                println!();
                print!("{}", attacks_view::render_text(&view));
            }
        }
        Command::Alignments { fen, all } => {
            let pos = Position::from_fen(&fen).with_context(|| format!("parsing FEN {:?}", fen))?;
            let summary_data = summary::build(&pos, summary::ScoreSource::Static, None);
            let view = alignments_view::build(&pos, all);
            if json_mode {
                #[derive(serde::Serialize)]
                struct Out<'a> {
                    summary: &'a summary::PositionSummary,
                    alignments: &'a alignments_view::AlignmentsView,
                }
                let payload = Out {
                    summary: &summary_data,
                    alignments: &view,
                };
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                print!("{}", summary::render_text(&summary_data));
                println!();
                print!("{}", alignments_view::render_text(&view));
            }
        }
        Command::Critique { fen, mv, depth } => {
            // The "I played X, why was it bad?" command. Internally this is
            // `search --force-include <mv>` on the BEFORE position — the
            // workflow the search `hint:` block points at — but packaged so
            // the move is a positional arg and the output is JUST the
            // critique (no PV-table noise, no eval dump). The load-bearing
            // output is the summary `danger:` block (what the opponent had
            // loaded) plus the swing reframe.
            let mut pos =
                Position::from_fen(&fen).with_context(|| format!("parsing FEN {:?}", fen))?;
            let played_move =
                parse_user_move(&mut pos, &mv).with_context(|| format!("parsing move {:?}", mv))?;
            let stm = pos.side_to_move();
            let orientation = crate::units::Orientation::from_stm_flag(stm_mode);

            let mut engine = Engine::default();
            let params = SearchParams {
                max_depth: depth,
                max_nodes: None,
                max_time: None,
                multi_pv: 1,
                game_history: Vec::new(),
                force_include: vec![played_move],
                verbose_progress: false,
                threads: 1,
                eval_mask: chess_tutor_engine::opponent::EvalMask::EMPTY,
                qsearch_max_plies: None,
                endgame_skill: chess_tutor_engine::endgame::EndgameSkill::Full,
                perception: None,
            };
            let lines = engine.search(&mut pos, params);
            if lines.is_empty() {
                println!("(no legal moves — terminal position; nothing to critique)");
                return Ok(());
            }
            let best = &lines[0];
            // `force_include` guarantees the played move is scored as its
            // own line; fall back to the best line if (unexpectedly) absent.
            let played = lines
                .iter()
                .find(|l| l.pv.first() == Some(&played_move))
                .unwrap_or(best);
            let is_best = best.pv.first() == Some(&played_move);
            let gave_away = gave_away_advantage(best.score, played.score);
            // Both scores are stm-POV and `stm` is the player who made the
            // move, so `best - played` is the pawns the player gave up.
            let given_up_pawns = crate::units::engine_cp_to_pawns(
                chess_tutor_engine::types::Value(best.score.0 - played.score.0),
            );

            let san = san::format(&pos, played_move);
            let best_san = pv_to_san(&pos, &best.pv);
            let played_san = pv_to_san(&pos, &played.pv);
            let summary_data = summary::build(
                &pos,
                summary::ScoreSource::Search { depth: best.depth },
                Some(best.score),
            );

            if json_mode {
                #[derive(serde::Serialize)]
                struct Out<'a> {
                    summary: &'a summary::PositionSummary,
                    depth: u32,
                    played_move_san: String,
                    played_move_uci: String,
                    played_pawns: String,
                    best_move_san: Option<String>,
                    best_pawns: String,
                    is_best: bool,
                    gave_away_advantage: bool,
                    pawns_given_up: f64,
                    best_line_san: Vec<String>,
                    played_line_san: Vec<String>,
                }
                let payload = Out {
                    summary: &summary_data,
                    depth: best.depth,
                    played_move_san: san.clone(),
                    played_move_uci: uci::format(played_move),
                    played_pawns: crate::units::format_pawns(orientation.apply(played.score, stm)),
                    best_move_san: best_san.first().cloned(),
                    best_pawns: crate::units::format_pawns(orientation.apply(best.score, stm)),
                    is_best,
                    gave_away_advantage: gave_away,
                    pawns_given_up: given_up_pawns,
                    best_line_san: best_san.clone(),
                    played_line_san: played_san.clone(),
                };
                println!("{}", serde_json::to_string_pretty(&payload)?);
                return Ok(());
            }

            print!("{}", summary::render_text(&summary_data));
            println!();

            // Verdict — three cases.
            if is_best {
                println!(
                    "verdict:  {san} is the engine's #1 move at depth {} — well played.",
                    best.depth,
                );
            } else if gave_away {
                // Strongest teaching frame: the swing is something you
                // ALLOWED (the opponent had a reply loaded), not a prettier
                // move you missed. The banner names the punishing line and
                // points at the defusal surfaces.
                print_allowed_banner(&pos, played_move, &played.pv, best.score, played.score, stm);
            } else {
                println!(
                    "verdict:  {san} is not the best move — it gives up {given_up_pawns:.1} pawn(s) vs. the top line, but it does not hand over a winning position.",
                );
            }

            // Show both lines so the reader can compare their move against
            // the engine's choice directly.
            println!();
            println!(
                "best move: {} ({} pawns {})",
                best_san.first().map(|s| s.as_str()).unwrap_or("?"),
                crate::units::format_pawns(orientation.apply(best.score, stm)),
                orientation.label(),
            );
            println!("  line:    {}", best_san.join(" "));
            if !is_best {
                println!(
                    "your move: {} ({} pawns {})",
                    san,
                    crate::units::format_pawns(orientation.apply(played.score, stm)),
                    orientation.label(),
                );
                println!("  line:    {}", played_san.join(" "));
            }

            // The ALLOWED banner already points at `explain` / `tactics`;
            // only add the pointer when it didn't fire.
            if !gave_away {
                println!();
                println!(
                    "next:     run `chess-tutor explain {fen:?}` for the full threat picture and the moves that hold the advantage.",
                );
            }
        }
        Command::Explain { fen, depth } => {
            // Aggregator: assembles one block from the same view
            // builders the dedicated subcommands use. Each section
            // is delimited by a header line so an agent (or human)
            // can grep for the cell they care about. Search is the
            // last block — it dominates wall time and is the
            // headline number the position summary at the top
            // references.
            let mut pos =
                Position::from_fen(&fen).with_context(|| format!("parsing FEN {:?}", fen))?;
            // Run a depth-N search first so the summary header can
            // carry the search score rather than a static eval.
            let mut engine = Engine::default();
            let search_params = SearchParams {
                max_depth: depth,
                max_nodes: None,
                max_time: None,
                multi_pv: 1,
                game_history: Vec::new(),
                force_include: Vec::new(),
                verbose_progress: false,
                threads: 1,
                eval_mask: chess_tutor_engine::opponent::EvalMask::EMPTY,
                qsearch_max_plies: None,
                endgame_skill: chess_tutor_engine::endgame::EndgameSkill::Full,
                perception: None,
            };
            let lines = engine.search(&mut pos, search_params);
            let (score_source, headline_score) = if lines.is_empty() {
                (summary::ScoreSource::Static, None)
            } else {
                (
                    summary::ScoreSource::Search {
                        depth: lines[0].depth,
                    },
                    Some(lines[0].score),
                )
            };
            let summary_data = summary::build(&pos, score_source, headline_score);
            let threats_data = threats_view::build(&pos);
            let mut tactics_data = tactics_view::build(
                &pos, None, /*latent*/ true, /*check_followups*/ true,
            );
            // Search-backed defusal enumeration: when the side to move
            // faces a standing threat, list the moves that actually
            // neutralise it without conceding the eval. Reuses the
            // already-constructed analytical engine; the same depth as
            // the headline search keeps the scores consistent.
            let stm_threats = find_latent_threats(&pos, pos.side_to_move());
            if !stm_threats.is_empty() {
                let report = find_threat_defusals(&mut engine, &mut pos, &stm_threats, depth);
                tactics_data.defusals =
                    Some(tactics_view::build_defusals_view(&pos, &report, depth));
            }

            if json_mode {
                #[derive(serde::Serialize)]
                struct SearchLineJson {
                    pv_san: Vec<String>,
                    pv_uci: Vec<String>,
                    engine_cp_stm: i32,
                    depth: u32,
                }
                let lines_json: Vec<SearchLineJson> = lines
                    .iter()
                    .map(|l| SearchLineJson {
                        pv_san: pv_to_san(&pos, &l.pv),
                        pv_uci: l.pv.iter().map(|m| uci::format(*m)).collect(),
                        engine_cp_stm: l.score.0,
                        depth: l.depth,
                    })
                    .collect();
                #[derive(serde::Serialize)]
                struct Out<'a> {
                    summary: &'a summary::PositionSummary,
                    threats: &'a threats_view::ThreatsView,
                    tactics: &'a tactics_view::TacticsView,
                    search: Vec<SearchLineJson>,
                }
                let payload = Out {
                    summary: &summary_data,
                    threats: &threats_data,
                    tactics: &tactics_data,
                    search: lines_json,
                };
                println!("{}", serde_json::to_string_pretty(&payload)?);
                return Ok(());
            }

            // Text output: section per data block. Position summary
            // first (already labelled with search depth + score),
            // then each named section.
            print!("{}", summary::render_text(&summary_data));
            println!();
            println!("== threats ==");
            print!("{}", threats_view::render_text(&threats_data));
            println!();
            println!("== tactics ==");
            print!("{}", tactics_view::render_text(&tactics_data));
            if !lines.is_empty() {
                println!();
                println!("== search (depth {}) ==", lines[0].depth);
                let orientation = crate::units::Orientation::from_stm_flag(stm_mode);
                let stm = pos.side_to_move();
                let oriented = orientation.apply(lines[0].score, stm);
                println!(
                    "score:    {} pawns {} (engine-cp: {} stm)",
                    crate::units::format_pawns(oriented),
                    orientation.label(),
                    crate::units::format_engine_cp(lines[0].score),
                );
                let pv_san = pv_to_san(&pos, &lines[0].pv);
                println!("pv:       {}", pv_san.join(" "));
            }
        }
        Command::Tactics {
            fen,
            prior_move,
            latent,
            check_followups,
        } => {
            use chess_tutor_engine::analysis::PriorMove;
            use chess_tutor_engine::types::{Move, PieceType, Square};
            let mut pos =
                Position::from_fen(&fen).with_context(|| format!("parsing FEN {:?}", fen))?;
            // `--prior-move` is the OPPONENT's last move — the move that
            // produced this FEN. We can't validate it against any legal
            // move list (its source square is empty in the current FEN
            // and the pre-move position isn't known), so we synthesise a
            // [`Move`] from the raw squares + optional promotion piece.
            // The recapture guard inside the detector chain only reads
            // `prior.mv.to()` and `prior.captured`, so the synthesised
            // move's [`MoveKind`] doesn't matter. `captured = None`
            // makes the guard lenient (extra HangingCapture false
            // positives possible when the prior move WAS a capture);
            // precise reconstruction would need a `--prior-fen` flag,
            // deferred per PLAN-cli.md §"Open design questions".
            let prior = match prior_move.as_deref() {
                None => None,
                Some(uci_str) => {
                    let s = uci_str.trim().to_ascii_lowercase();
                    if !(s.len() == 4 || s.len() == 5) {
                        anyhow::bail!(
                            "--prior-move must be UCI (4 or 5 chars, e.g. `g7g6` / `e7e8q`), got {:?}",
                            uci_str,
                        );
                    }
                    let from = Square::from_algebraic(&s[0..2]).ok_or_else(|| {
                        anyhow::anyhow!("--prior-move: bad from-square in {:?}", uci_str)
                    })?;
                    let to = Square::from_algebraic(&s[2..4]).ok_or_else(|| {
                        anyhow::anyhow!("--prior-move: bad to-square in {:?}", uci_str)
                    })?;
                    let mv = if s.len() == 5 {
                        let promo = match s.as_bytes()[4] as char {
                            'q' => PieceType::Queen,
                            'r' => PieceType::Rook,
                            'b' => PieceType::Bishop,
                            'n' => PieceType::Knight,
                            other => anyhow::bail!(
                                "--prior-move: bad promotion piece {other:?} in {uci_str:?}",
                            ),
                        };
                        Move::promotion(from, to, promo)
                    } else {
                        Move::normal(from, to)
                    };
                    Some(PriorMove { mv, captured: None })
                }
            };
            let summary_data = summary::build(&pos, summary::ScoreSource::Static, None);
            let mut view = tactics_view::build(&pos, prior, latent, check_followups);
            // `--latent` opts into the search-backed defusal block: when
            // the side to move faces a standing threat, enumerate the
            // moves that neutralise it AND hold the eval. This is the one
            // place `tactics` runs a search (it's otherwise static), so
            // it's gated on `latent` + an actual threat being present.
            if latent {
                let stm_threats = find_latent_threats(&pos, pos.side_to_move());
                if !stm_threats.is_empty() {
                    let mut engine = Engine::default();
                    let report = find_threat_defusals(
                        &mut engine,
                        &mut pos,
                        &stm_threats,
                        TACTICS_DEFUSAL_DEPTH,
                    );
                    view.defusals = Some(tactics_view::build_defusals_view(
                        &pos,
                        &report,
                        TACTICS_DEFUSAL_DEPTH,
                    ));
                }
            }
            if json_mode {
                #[derive(serde::Serialize)]
                struct Out<'a> {
                    summary: &'a summary::PositionSummary,
                    tactics: &'a tactics_view::TacticsView,
                }
                let payload = Out {
                    summary: &summary_data,
                    tactics: &view,
                };
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                print!("{}", summary::render_text(&summary_data));
                println!();
                print!("{}", tactics_view::render_text(&view));
            }
        }
        Command::Search {
            fen,
            depth,
            qsearch_depth,
            endgame_skill,
            perception,
            nodes,
            time_ms,
            multi_pv,
            debug,
            analyze,
            top_percent,
            threads,
            force_include,
            verbose_progress,
            annotate,
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
                .map(|s| {
                    parse_user_move(&mut pos, s)
                        .with_context(|| format!("parsing --force-include {s:?}"))
                })
                .collect::<Result<Vec<_>>>()?;
            let params = SearchParams {
                max_depth: depth,
                max_nodes: nodes,
                max_time: time_ms.map(Duration::from_millis),
                multi_pv: multi_pv.max(1),
                game_history: Vec::new(),
                force_include: force_include_moves.clone(),
                verbose_progress,
                threads: threads.max(1),
                // One-shot CLI search/analyze — analytical, no bot mask.
                eval_mask: chess_tutor_engine::opponent::EvalMask::EMPTY,
                // ...except the explicit tactical-vision dial for inspection.
                qsearch_max_plies: qsearch_depth,
                // ...and the explicit endgame-skill dial for inspection.
                endgame_skill: endgame_skill.map_or(EndgameSkill::Full, EndgameSkill::from_tier),
                // ...and the explicit perception dial for inspection.
                // Fixed seed + no attention locus: repeat runs of the
                // same command are identical.
                perception: perception.filter(|p| *p < 1.0).map(|level| {
                    chess_tutor_engine::visibility::PerceptionParams {
                        level,
                        seed: 0,
                        last_move_to: None,
                        exempt_root_checks: false,
                    }
                }),
            };

            let orientation = crate::units::Orientation::from_stm_flag(stm_mode);

            if analyze {
                // Teaching-analysis path: same search under the hood,
                // but the output surfaces per-move term deltas rather
                // than the leaf trace. JSON mode falls through to the
                // text-only behaviour today (PLAN-cli.md: `--analyze`
                // JSON shape is a follow-up; the per-term-delta
                // breakdown deserves its own schema).
                let analyses = analyze_position(&mut engine, &mut pos, params);
                if analyses.is_empty() {
                    println!("(no legal moves — terminal position)");
                    return Ok(());
                }
                let summary_data = summary::build(
                    &pos,
                    summary::ScoreSource::Search {
                        depth: analyses[0].depth,
                    },
                    Some(analyses[0].score),
                );
                print!("{}", summary::render_text(&summary_data));
                println!();
                if force_include_moves.is_empty() {
                    print_force_include_hint();
                    println!();
                }
                println!("depth: {}", analyses[0].depth);
                print!("{}", analysis_report::render(&pos, &analyses, top_percent));
                if debug {
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
                if json_mode {
                    let summary_data = summary::build(&pos, summary::ScoreSource::Static, None);
                    #[derive(serde::Serialize)]
                    struct Out<'a> {
                        summary: &'a summary::PositionSummary,
                        terminal: &'static str,
                    }
                    let payload = Out {
                        summary: &summary_data,
                        terminal: "no legal moves",
                    };
                    println!("{}", serde_json::to_string_pretty(&payload)?);
                } else {
                    println!("(no legal moves — terminal position)");
                }
                return Ok(());
            }

            let summary_data = summary::build(
                &pos,
                summary::ScoreSource::Search {
                    depth: lines[0].depth,
                },
                Some(lines[0].score),
            );

            if json_mode {
                #[derive(serde::Serialize)]
                struct JsonLine {
                    rank: usize,
                    /// White-POV (or stm-POV with `--stm`) pawns.
                    pawns: String,
                    /// Same number as [`Self::pawns`] in conv-cp
                    /// (pawn = 100 cp; chess.com / UCI scale).
                    conv_cp: String,
                    /// Engine-internal cp (PawnEG = 213) for the same
                    /// line, side-to-move-signed. Useful for comparing
                    /// against `chess_tutor_engine::search` thresholds.
                    engine_cp_stm: i32,
                    /// Delta from the top line in *engine-cp*. Connects
                    /// directly to search-code aspiration / futility
                    /// margins; conv-cp delta is `engine_cp / 2.13`.
                    delta_engine_cp_from_top: i32,
                    pv_san: Vec<String>,
                    pv_uci: Vec<String>,
                    settled_ply: Option<usize>,
                }
                let stm = pos.side_to_move();
                let top = lines[0].score.0;
                let json_lines: Vec<JsonLine> = lines
                    .iter()
                    .enumerate()
                    .map(|(i, line)| {
                        let oriented = orientation.apply(line.score, stm);
                        let pv_san = pv_to_san(&pos, &line.pv);
                        let delta_engine = line.score.0 - top;
                        JsonLine {
                            rank: i + 1,
                            pawns: crate::units::format_pawns(oriented),
                            conv_cp: crate::units::format_conventional_cp(oriented),
                            engine_cp_stm: line.score.0,
                            delta_engine_cp_from_top: delta_engine,
                            pv_san,
                            pv_uci: line.pv.iter().map(|m| uci::format(*m)).collect(),
                            settled_ply: line.settled_ply,
                        }
                    })
                    .collect();
                #[derive(serde::Serialize)]
                struct Out<'a> {
                    summary: &'a summary::PositionSummary,
                    depth: u32,
                    orientation: &'static str,
                    nodes: u64,
                    elapsed_ms: u128,
                    nps_mn: f64,
                    lines: Vec<JsonLine>,
                }
                let payload = Out {
                    summary: &summary_data,
                    depth: lines[0].depth,
                    orientation: orientation.label(),
                    nodes: engine.last_nodes(),
                    elapsed_ms: engine.last_elapsed().as_millis(),
                    nps_mn: engine.last_nps() / 1.0e6,
                    lines: json_lines,
                };
                println!("{}", serde_json::to_string_pretty(&payload)?);
                return Ok(());
            }

            // Text output.
            print!("{}", summary::render_text(&summary_data));
            println!();
            if force_include_moves.is_empty() {
                print_force_include_hint();
                println!();
            }
            let stm = pos.side_to_move();

            // "You ALLOWED, not missed" hard flag. Fires only when the
            // caller forced a move into the search (reproducing a move
            // they played) and that move flips the position from
            // winning/equal to losing. `lines` is sorted best-first, so
            // `lines[0]` is the best available; each forced move's own
            // line carries its resulting score.
            if !force_include_moves.is_empty() && !lines.is_empty() {
                let best_score = lines[0].score;
                for &fm in &force_include_moves {
                    if let Some(fl) = lines.iter().find(|l| l.pv.first() == Some(&fm)) {
                        if gave_away_advantage(best_score, fl.score) {
                            print_allowed_banner(&pos, fm, &fl.pv, best_score, fl.score, stm);
                        }
                    }
                }
            }

            if lines.len() == 1 {
                let line = &lines[0];
                let oriented = orientation.apply(line.score, stm);
                println!("depth:    {}", line.depth);
                // Pawns is the chess.com-comparable headline; engine-cp
                // is the source-code-comparable number. Both labelled.
                println!(
                    "score:    {} pawns {} (engine-cp: {} stm)",
                    crate::units::format_pawns(oriented),
                    orientation.label(),
                    crate::units::format_engine_cp(line.score),
                );
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
                // `search` answers "what's the best move here?" — the
                // per-term eval table and pawn-cache stats are search
                // *diagnostics*, not part of that answer, and a wall of
                // them is what tempts a reader to pipe through `tail` and
                // lose the `danger:` header up top. They live behind
                // `--debug` now; reach for `eval` / `explain` when you
                // actually want the term breakdown.
                if debug {
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
                    if let Some(leaf) = line.ply_traces.last() {
                        print!("{}", eval_report::render(leaf));
                    }
                }
            } else {
                println!(
                    "depth: {}  (scores: pawns {}; deltas: engine-cp)",
                    lines[0].depth,
                    orientation.label(),
                );
                print!("{}", render_multi_pv(&pos, &lines, orientation));
            }

            if debug {
                println!();
                print!("{}", render_debug_trajectory(&pos, &lines));
            }

            // Truncation guard: the load-bearing `danger:` block is at the
            // TOP, so a reader who pipes this through `tail` loses it and
            // trusts a "best move" without seeing the standing threat it
            // must answer. Echo a one-line reminder at the BOTTOM so a
            // bottom-read still lands on it. (See the agent failure mode in
            // CLAUDE.md §"Eval swing".)
            if !summary_data.danger.is_empty() {
                println!();
                println!(
                    "note:     {} standing threat(s) against the side to move — re-read the `danger:` block at the TOP before trusting the move above; run `chess-tutor critique <FEN> <your-move>` to score a move you actually played.",
                    summary_data.danger.len(),
                );
            }

            // `--annotate`: run the tactic detector on the top PV's
            // line and surface a one-line `(pattern via Move; gain
            // +N pts)` summary. No `prior_move`, so the recapture
            // guard may produce false positives in rare positions
            // (PLAN-cli.md §"Open design questions" notes this).
            if annotate && !lines[0].pv.is_empty() {
                use chess_tutor_engine::analysis::find_tactic_in_line;
                let mover = pos.side_to_move();
                if let Some(hit) = find_tactic_in_line(&pos, &lines[0].pv, mover, None) {
                    let pv_san = pv_to_san(&pos, &lines[0].pv);
                    let move_name = pv_san.first().map(|s| s.as_str()).unwrap_or("?");
                    let mate_suffix = match hit.mate_pattern {
                        Some(mp) => format!(" + {:?}", mp),
                        None => String::new(),
                    };
                    let sac = if hit.sacrifice { " (sac)" } else { "" };
                    println!();
                    let gain = match hit.material_gain {
                        Some(g) => format!("+{} pts", g),
                        None => "n/a".to_string(),
                    };
                    println!(
                        "tactic:   {:?} via {}{}{}  (gain {}, conf: {:?})",
                        hit.pattern, move_name, mate_suffix, sac, gain, hit.confidence,
                    );
                    // When the tactic fires on the first PV move, check for a
                    // forcing escape so the line doesn't read as a clean win
                    // when the opponent has a tricky out.
                    if hit.pv_ply == 0 {
                        use chess_tutor_engine::analysis::find_tactic_escape;
                        if let Some(esc) = find_tactic_escape(&pos, &hit, mover) {
                            let mut post = pos.clone();
                            if let Some(km) = hit.key_move {
                                post.do_move(km);
                            }
                            println!(
                                "escape:   opponent can break it with {} — the tactic doesn't fully cash (run `tactics` for detail)",
                                san::format(&post, esc.refutation),
                            );
                        }
                    }
                } else {
                    println!();
                    // This scans only the engine's chosen line. It does
                    // NOT mean the position is tactically quiet: the
                    // opponent may have a standing threat against the
                    // side to move (surfaced in the `danger:` header
                    // above and by `tactics --latent`). Word it so an
                    // agent doesn't read "no pattern" as "all clear".
                    println!(
                        "tactic:   (no pattern in the engine's top PV — this does NOT clear the position; \
                         see the `danger:` header + `chess-tutor tactics --latent` for the opponent's standing threats)"
                    );
                }
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
        Command::SettledAudit {
            tt_mb,
            depths,
            multi_pv,
            fen_file,
            examples,
        } => {
            settled_audit::run(settled_audit::SettledAuditArgs {
                tt_mb,
                depths,
                multi_pv,
                fen_file,
                examples,
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
            avg_move_rank,
            blunder_chance,
            blunder_min_material,
            blunder_max_material,
            miss_chance,
            guaranteed_mate_in,
            perception,
        } => {
            let mut opponent = match seed {
                Some(s) => chess_tutor_engine::opponent::OpponentProfile::with_seed(s),
                None => chess_tutor_engine::opponent::OpponentProfile::new_random(),
            };
            if !(0.0..=1.0).contains(&perception) {
                anyhow::bail!("--perception must be in [0.0, 1.0], got {perception}");
            }
            opponent.perception = perception;
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
            if !(0.0..=1.0).contains(&miss_chance) {
                anyhow::bail!("--miss-chance must be in [0.0, 1.0], got {miss_chance}");
            }
            if avg_move_rank < 1.0 {
                anyhow::bail!("--avg-move-rank must be at least 1.0, got {avg_move_rank}");
            }
            if blunder_min_material < 0.0 || blunder_max_material < blunder_min_material {
                anyhow::bail!(
                    "--blunder-min-material / --blunder-max-material must be 0 <= min <= max (got min={blunder_min_material}, max={blunder_max_material})",
                );
            }
            opponent.noise = chess_tutor_engine::opponent::NoiseProfile {
                avg_move_rank,
                blunder_chance,
                blunder_min_material_cp: (blunder_min_material * 100.0) as i32,
                blunder_max_material_cp: (blunder_max_material * 100.0) as i32,
                miss_chance,
                guaranteed_mate_in,
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
        Command::Uci {
            depth,
            threads,
            qsearch_depth,
            endgame_skill,
            seed,
            disable_eval,
            avg_move_rank,
            blunder_chance,
            blunder_min_material,
            blunder_max_material,
            miss_chance,
            guaranteed_mate_in,
            perception,
        } => {
            use chess_tutor_engine::opponent::{EvalCategory, EvalMask, NoiseProfile};
            // Same dial validation as `play` — keep the two in sync.
            if !(0.0..=1.0).contains(&perception) {
                anyhow::bail!("--perception must be in [0.0, 1.0], got {perception}");
            }
            if !(0.0..=1.0).contains(&blunder_chance) {
                anyhow::bail!("--blunder-chance must be in [0.0, 1.0], got {blunder_chance}");
            }
            if !(0.0..=1.0).contains(&miss_chance) {
                anyhow::bail!("--miss-chance must be in [0.0, 1.0], got {miss_chance}");
            }
            if avg_move_rank < 1.0 {
                anyhow::bail!("--avg-move-rank must be at least 1.0, got {avg_move_rank}");
            }
            if blunder_min_material < 0.0 || blunder_max_material < blunder_min_material {
                anyhow::bail!(
                    "--blunder-min-material / --blunder-max-material must be 0 <= min <= max (got min={blunder_min_material}, max={blunder_max_material})",
                );
            }
            let mut eval_mask = EvalMask::EMPTY;
            if let Some(list) = disable_eval {
                for token in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    let cat = EvalCategory::from_slug(token).with_context(|| {
                        format!(
                            "unknown eval category {:?} (try one of: pawn-structure, pieces, mobility, king-safety, threats, passed-pawns, space, initiative)",
                            token,
                        )
                    })?;
                    eval_mask.disable(cat);
                }
            }
            // Default to a random base seed when none is given, matching
            // OpponentProfile::new_random so unseeded runs still vary.
            let base_seed = match seed {
                Some(s) => s,
                None => chess_tutor_engine::opponent::OpponentProfile::new_random().seed,
            };
            let noise = NoiseProfile {
                avg_move_rank,
                blunder_chance,
                blunder_min_material_cp: (blunder_min_material * 100.0) as i32,
                blunder_max_material_cp: (blunder_max_material * 100.0) as i32,
                miss_chance,
                guaranteed_mate_in,
            };
            uci_shim::run(uci_shim::UciConfig {
                depth,
                threads: threads.max(1),
                base_seed,
                eval_mask,
                qsearch_max_plies: qsearch_depth,
                endgame_skill: endgame_skill.map_or(EndgameSkill::Full, EndgameSkill::from_tier),
                perception,
                noise,
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::gave_away_advantage;
    use chess_tutor_engine::types::Value;

    // 1.0 pawn = PawnEG = 213 engine-cp on our scale. The test values
    // below are written as `pawns × 213` for readability.
    const P: i32 = 213;

    #[test]
    fn fires_winning_to_losing_swing() {
        // The case-study swing: +2.0 → -1.3. Conceded 3.3 pawns, ends
        // losing — clearly an "allowed" case.
        assert!(gave_away_advantage(Value(2 * P), Value(-13 * P / 10)));
    }

    #[test]
    fn fires_when_a_won_position_is_given_up_without_crossing_zero() {
        // +2.0 → +0.2: conceded 1.8 pawns, no longer winning. The
        // cross-zero rule would miss this; the swing rule must catch it.
        assert!(gave_away_advantage(Value(2 * P), Value(P / 5)));
    }

    #[test]
    fn fires_when_a_neutral_position_is_thrown_away() {
        // +0.2 → -3.0: conceded 3.2 pawns from a roughly equal start.
        // The cross-zero rule would miss this (didn't start ahead).
        assert!(gave_away_advantage(Value(P / 5), Value(-3 * P)));
    }

    #[test]
    fn silent_when_still_clearly_winning() {
        // +5.0 → +3.0: conceded two pawns but still up three. Suboptimal,
        // not "handed it over" — the +1.0 floor keeps this quiet.
        assert!(!gave_away_advantage(Value(5 * P), Value(3 * P)));
    }

    #[test]
    fn silent_on_a_small_slip() {
        // +2.0 → +1.5: only half a pawn conceded — under the 1.0-pawn
        // swing threshold, so no banner.
        assert!(!gave_away_advantage(Value(2 * P), Value(15 * P / 10)));
    }

    #[test]
    fn silent_inside_the_equality_dead_zone() {
        // +0.28 → -0.28: a 0.56-pawn wobble around equality — below the
        // 1.0-pawn swing threshold.
        assert!(!gave_away_advantage(Value(60), Value(-60)));
    }
}
