//! Phase 1 CLI harness.
//!
//!     chess-tutor analyze "<fen>"          — JSON analysis report
//!     chess-tutor explain "<fen>"          — prose (stubbed until explainer lands)
//!     chess-tutor play                     — interactive loop over stdin
//!     chess-tutor review <pgn-file>        — walk a PGN, annotating every move
//!                                            (stubbed until PGN import lands)

use std::io::{self, BufRead, Write};

use anyhow::{Context, Result};
use chess_tutor_core::{
    analyze,
    explain::Explainer,
    game::{Game, GameStatus, PlayerKind},
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
    /// `undo`, `resign`, `fen`, or `quit`.
    Play,
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
        Command::Play => play_loop()?,
        Command::Review { pgn: _ } => {
            println!("(review is stubbed — PGN import lands in Phase 1)");
        }
    }
    Ok(())
}

fn play_loop() -> Result<()> {
    let mut game = Game::new_standard(PlayerKind::Human, PlayerKind::Human);
    let stdin = io::stdin();
    let mut out = io::stdout().lock();

    writeln!(out, "chess-tutor play — UCI moves, or: undo / resign / fen / quit")?;
    loop {
        writeln!(out, "\n{:?} to move. FEN: {}", game.side_to_move(), game.fen())?;
        write!(out, "> ")?;
        out.flush()?;

        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            break; // EOF
        }
        let cmd = line.trim();
        match cmd {
            "" => continue,
            "quit" | "exit" => break,
            "fen" => writeln!(out, "{}", game.fen())?,
            "undo" => {
                match game.undo() {
                    Some(entry) => writeln!(out, "undid {}", entry.san)?,
                    None => writeln!(out, "nothing to undo")?,
                }
            }
            "resign" => {
                game.resign(game.side_to_move());
                writeln!(out, "status: {:?}", game.status())?;
            }
            uci => match game.apply(uci) {
                Ok(report) => {
                    writeln!(out, "played {} ({:?})", report.entry.san, report.class)?;
                    if !matches!(game.status(), GameStatus::Ongoing) {
                        writeln!(out, "game over: {:?}", game.status())?;
                    }
                }
                Err(e) => writeln!(out, "rejected: {e}")?,
            },
        }
    }
    Ok(())
}
