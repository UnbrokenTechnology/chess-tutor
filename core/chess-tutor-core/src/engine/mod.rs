//! Cross-check engine abstraction.
//!
//! The analysis layer does the teaching; the engine only confirms or flags
//! disagreement. The chosen engine is **Viridithas** (Rust, MIT). The trait
//! stays generic so we can plug a second engine in later without churning
//! call sites — see `engine/README.md` at the repo root.

use serde::{Deserialize, Serialize};

/// Implementors run a bounded search on a FEN and return a top move plus
/// its evaluation. The same trait covers both the teaching-mode cross-check
/// (high depth, full strength) and the bot-opponent use case (capped
/// strength via depth / skill / contempt). Keeping it narrow makes swapping
/// engines cheap.
pub trait CrossCheckEngine {
    fn search(&mut self, fen: &str, opts: SearchOptions) -> crate::Result<EngineCheck>;
    fn name(&self) -> &'static str;
}

/// Bounds for a single search. Separate from the engine so the caller can
/// tune the same engine for "analyse this position" vs. "play a move as
/// ~1400 ELO bot" without building a wrapper per mode.
#[derive(Debug, Clone, Copy)]
pub struct SearchOptions {
    pub depth: u8,
    /// `None` means "no extra strength cap" (cross-check mode). `Some(level)`
    /// caps the engine for bot play — meaning is engine-specific; Viridithas
    /// reads this as a skill level and wires it through search pruning.
    pub skill_cap: Option<u8>,
    pub movetime_ms: Option<u32>,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            depth: 8,
            skill_cap: None,
            movetime_ms: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineCheck {
    pub engine: String,
    pub depth: u8,
    pub best_move_uci: String,
    pub eval_cp: Option<i32>,
    pub mate_in: Option<i32>,
    /// Whether the engine agrees with our top-ranked candidate. Disagreement
    /// is itself a teaching moment, so we surface it rather than hiding it.
    pub agrees_with_analysis: bool,
}
