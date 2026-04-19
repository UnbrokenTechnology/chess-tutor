//! Phase 1 CLI harness.
//!
//!     chess-tutor analyze "<fen>"          — JSON analysis report
//!     chess-tutor explain "<fen>"          — prose (stubbed until explainer lands)
//!     chess-tutor board "<fen>"            — render a FEN as a Unicode board
//!     chess-tutor play                     — interactive loop with live board
//!     chess-tutor review <pgn-file>        — walk a PGN, annotating every move
//!                                            (stubbed until PGN import lands)

mod board;

use std::io::{self, BufRead, Write};
use std::time::Instant;

use anyhow::{Context, Result};
use chess_tutor_core::{
    analyze,
    explain::Explainer,
    game::{Game, GameStatus, PlayerKind, Side, TimeControl},
};
use clap::{Parser, Subcommand};

use crate::board::{render as render_board, RenderOptions};

#[derive(Parser)]
#[command(name = "chess-tutor", version, about = "Deterministic chess analysis + explanation.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the analysis pipeline on a FEN and print the JSON report.
    Analyze { fen: String },
    /// Run the explainer on a FEN and print user-facing prose.
    Explain { fen: String },
    /// Play an interactive game over stdin. Type UCI moves (e.g. e2e4); type
    /// `undo`, `resign`, `fen`, or `quit`. Pass `--time` (seconds) and
    /// optionally `--increment` (seconds) for Fischer-clock play.
    Play {
        /// Initial clock per side, in seconds. Omit for untimed play.
        #[arg(long)]
        time: Option<u64>,
        /// Fischer increment in seconds. Requires --time. Defaults to 0.
        #[arg(long, default_value_t = 0)]
        increment: u64,
        /// Use plain ASCII pieces (K Q R B N P) instead of Unicode.
        #[arg(long)]
        ascii: bool,
        /// Flip the board to always show the current mover at the bottom.
        #[arg(long)]
        auto_flip: bool,
        /// Hide the live board and fall back to text-only status updates.
        #[arg(long)]
        no_board: bool,
    },
    /// Render a position's board in the terminal.
    Board {
        /// FEN. Omit to render the standard start position.
        #[arg(default_value = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1")]
        fen: String,
        /// Use plain ASCII pieces.
        #[arg(long)]
        ascii: bool,
        /// Render from Black's perspective.
        #[arg(long)]
        flip: bool,
    },
    /// Walk a PGN file and annotate every move. Stubbed until PGN import lands.
    Review { pgn: String },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Analyze { fen } => {
            let report = analyze(&fen).context("analysis failed")?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Explain { fen } => {
            let report = analyze(&fen).context("analysis failed")?;
            let phrases = Explainer::new().explain(&report);
            if phrases.is_empty() {
                println!("(no phrases yet — explainer templates land in Phase 1)");
            } else {
                for p in phrases {
                    println!("{}", p.text);
                }
            }
        }
        Command::Play {
            time,
            increment,
            ascii,
            auto_flip,
            no_board,
        } => play_loop(time, increment, ascii, auto_flip, !no_board)?,
        Command::Board { fen, ascii, flip } => {
            let out = render_board(
                &fen,
                &RenderOptions {
                    ascii,
                    flip,
                    highlight: None,
                },
            );
            print!("{out}");
        }
        Command::Review { pgn: _ } => {
            println!("(review is stubbed — PGN import lands in Phase 1)");
        }
    }
    Ok(())
}

fn play_loop(
    time_sec: Option<u64>,
    increment_sec: u64,
    ascii: bool,
    auto_flip: bool,
    show_board: bool,
) -> Result<()> {
    let mut game = Game::new_standard(PlayerKind::Human, PlayerKind::Human);
    if let Some(sec) = time_sec {
        game = game.with_time_control(TimeControl::fischer(sec * 1_000, increment_sec * 1_000));
    }

    let stdin = io::stdin();
    let mut out = io::stdout().lock();

    writeln!(
        out,
        "chess-tutor play — UCI moves, or: undo / resign / fen / flip / quit"
    )?;
    if game.has_time_control() {
        writeln!(
            out,
            "clock: {}s + {}s increment",
            time_sec.unwrap(),
            increment_sec
        )?;
    }

    let mut manual_flip = false;
    let mut turn_started = Instant::now();

    loop {
        let mover = game.side_to_move();

        if show_board {
            let flip = manual_flip || (auto_flip && mover == Side::Black);
            let highlight = last_move_squares(&game);
            writeln!(out)?;
            write!(
                out,
                "{}",
                render_board(
                    &game.fen(),
                    &RenderOptions {
                        ascii,
                        flip,
                        highlight,
                    },
                )
            )?;
        }

        if game.has_time_control() {
            writeln!(
                out,
                "{:?} to move. W {:.1}s / B {:.1}s",
                mover,
                game.remaining_ms(Side::White).unwrap() as f64 / 1000.0,
                game.remaining_ms(Side::Black).unwrap() as f64 / 1000.0,
            )?;
        } else {
            writeln!(out, "{:?} to move.", mover)?;
        }
        write!(out, "> ")?;
        out.flush()?;

        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            break; // EOF
        }
        let elapsed_ms = turn_started.elapsed().as_millis() as u64;
        let cmd = line.trim();
        match cmd {
            "" => continue,
            "quit" | "exit" => break,
            "flip" => {
                manual_flip = !manual_flip;
            }
            "fen" => {
                writeln!(out, "{}", game.fen())?;
            }
            "undo" => match game.undo() {
                Some(entry) => writeln!(out, "undid {}", entry.san)?,
                None => writeln!(out, "nothing to undo")?,
            },
            "resign" => {
                game.resign(mover);
                writeln!(out, "status: {:?}", game.status())?;
            }
            uci => {
                let result = if game.has_time_control() {
                    game.apply_timed(uci, elapsed_ms)
                } else {
                    game.apply(uci)
                };
                match result {
                    Ok(report) => {
                        writeln!(out, "played {} ({:?})", report.entry.san, report.class)?;
                        if !matches!(game.status(), GameStatus::Ongoing) {
                            // Show the final position before breaking.
                            if show_board {
                                let flip = manual_flip || (auto_flip && mover == Side::Black);
                                let highlight = last_move_squares(&game);
                                writeln!(out)?;
                                write!(
                                    out,
                                    "{}",
                                    render_board(
                                        &game.fen(),
                                        &RenderOptions {
                                            ascii,
                                            flip,
                                            highlight,
                                        },
                                    )
                                )?;
                            }
                            writeln!(out, "game over: {:?}", game.status())?;
                            break;
                        }
                        turn_started = Instant::now();
                    }
                    Err(e) => writeln!(out, "rejected: {e}")?,
                }
            }
        }
    }
    Ok(())
}

fn last_move_squares(game: &Game) -> Option<(String, String)> {
    let last = game.history().last()?;
    // UCI moves are 4 chars (e2e4) or 5 with promotion (e7e8q).
    let uci = &last.uci;
    if uci.len() < 4 {
        return None;
    }
    Some((uci[0..2].to_string(), uci[2..4].to_string()))
}
