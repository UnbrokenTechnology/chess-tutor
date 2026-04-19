//! Phase 1 CLI harness.
//!
//!     chess-tutor analyze "<fen>"        — JSON report
//!     chess-tutor explain "<fen>"        — prose (stubbed until explainer lands)

use anyhow::{Context, Result};
use chess_tutor_core::{analyze, explain::Explainer};
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
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Analyze { fen } => {
            let report = analyze(&fen).context("analysis failed")?;
            let json = serde_json::to_string_pretty(&report)?;
            println!("{json}");
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
    }
    Ok(())
}
