//! Chess Tutor core.
//!
//! Pure analysis and explanation. No I/O, no UI, no platform APIs. Everything
//! is driven from a FEN string and produces a fully-structured
//! [`PositionAnalysis`], which the [`explain`] module walks to render prose.
//!
//! Module layout mirrors the pipeline:
//!
//! - [`game`]     — live game state (history, apply/undo, status, per-move reports)
//! - [`analysis`] — attacker/defender maps, SEE, threat detection
//! - [`tactics`]  — motif detection (forks, pins, skewers, ...)
//! - [`positional`] — pawn structure, king safety, piece activity
//! - [`book`]     — Polyglot opening book reader
//! - [`search`]   — quiescence / forcing-line walker
//! - [`explain`]  — template-based prose generator
//! - [`engine`]   — pluggable cross-check engine + bot-opponent trait (Viridithas)

pub mod analysis;
pub mod book;
pub mod engine;
pub mod error;
pub mod explain;
pub mod game;
pub mod positional;
pub mod search;
pub mod tactics;

pub use error::{Error, Result};

use serde::{Deserialize, Serialize};

/// Version of the analysis schema. Bump whenever [`PositionAnalysis`] changes
/// in a way that platform shells or persisted reports would notice.
pub const ANALYSIS_SCHEMA_VERSION: u32 = 1;

/// Top-level output of the analysis pipeline for a single position.
///
/// This is the source of truth the [`explain::Explainer`] walks to produce
/// prose. It is serialisable so platform shells can cache it or round-trip it
/// through JSON.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PositionAnalysis {
    pub schema_version: u32,
    pub fen: String,
    pub square_data: analysis::SquareData,
    pub candidates: Vec<analysis::CandidateMove>,
    pub tactics: tactics::TacticsReport,
    pub positional: positional::PositionalReport,
    pub opening: Option<book::OpeningHit>,
    pub forcing_lines: Vec<search::ForcingLine>,
    pub engine_check: Option<engine::EngineCheck>,
}

impl PositionAnalysis {
    pub fn empty(fen: impl Into<String>) -> Self {
        Self {
            schema_version: ANALYSIS_SCHEMA_VERSION,
            fen: fen.into(),
            ..Default::default()
        }
    }
}

/// Analyse a position given its FEN.
///
/// Phase 1 — currently populates `square_data` from the attacker/defender
/// map. Remaining fields fill in as subsequent Phase 1 tasks land (SEE,
/// tactics, positional, candidates, explainer).
pub fn analyze(fen: &str) -> Result<PositionAnalysis> {
    use shakmaty::fen::Fen;
    use shakmaty::{CastlingMode, Chess};

    let parsed: Fen = fen.parse().map_err(|e| Error::InvalidFen(format!("{e}")))?;
    let pos: Chess = parsed
        .into_position(CastlingMode::Standard)
        .map_err(|e| Error::InvalidFen(format!("{e}")))?;

    let attack_map = analysis::AttackMap::from_position(&pos);

    let mut report = PositionAnalysis::empty(fen);
    report.square_data = attack_map.to_square_data();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyze_accepts_startpos() {
        let fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        let report = analyze(fen).expect("startpos should parse");
        assert_eq!(report.fen, fen);
        assert_eq!(report.schema_version, ANALYSIS_SCHEMA_VERSION);
    }

    #[test]
    fn analyze_rejects_garbage_fen() {
        assert!(analyze("not a fen").is_err());
    }
}
