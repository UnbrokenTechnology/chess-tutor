//! Phase 1 CLI harness.
//!
//!     chess-tutor analyze "<fen>"          — JSON analysis report
//!     chess-tutor explain "<fen>"          — prose (stubbed until explainer lands)
//!     chess-tutor play                     — interactive loop over stdin
//!     chess-tutor review <pgn-file>        — walk a PGN, annotating every move
//!                                            (stubbed until PGN import lands)

use std::io::{self, BufRead, Write};
use std::time::Instant;

use anyhow::{Context, Result};
use chess_tutor_core::{
    analyze,
    explain::Explainer,
    game::{Game, GameStatus, PlayerKind, Side, TimeControl},
};
use clap::{Parser, Subcommand};

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
        Command::Play { time, increment } => play_loop(time, increment)?,
        Command::Review { pgn: _ } => {
            println!("(review is stubbed — PGN import lands in Phase 1)");
        }
    }
    Ok(())
}

fn play_loop(time_sec: Option<u64>, increment_sec: u64) -> Result<()> {
    let mut game = Game::new_standard(PlayerKind::Human, PlayerKind::Human);
    if let Some(sec) = time_sec {
        game = game.with_time_control(TimeControl::fischer(sec * 1_000, increment_sec * 1_000));
    }

    let stdin = io::stdin();
    let mut out = io::stdout().lock();

    writeln!(out, "chess-tutor play — UCI moves, or: undo / resign / fen / quit")?;
    if game.has_time_control() {
        writeln!(
            out,
            "clock: {}s + {}s increment",
            time_sec.unwrap(),
            increment_sec
        )?;
    }

    let mut turn_started = Instant::now();
    loop {
        let mover = game.side_to_move();
        if game.has_time_control() {
            writeln!(
                out,
                "\n{:?} to move. W {:.1}s / B {:.1}s",
                mover,
                game.remaining_ms(Side::White).unwrap() as f64 / 1000.0,
                game.remaining_ms(Side::Black).unwrap() as f64 / 1000.0,
            )?;
        } else {
            writeln!(out, "\n{:?} to move. FEN: {}", mover, game.fen())?;
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
