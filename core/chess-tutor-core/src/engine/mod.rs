//! Cross-check engine abstraction.
//!
//! The analysis layer does the teaching; the engine only confirms or flags
//! disagreement. The chosen engine is **Viridithas** (Rust, MIT). The trait
//! stays generic so we can plug a second engine in later without churning
//! call sites — see `engine/README.md` at the repo root.

use serde::{Deserialize, Serialize};

/// Implementors run a low-depth search on a FEN and return a top move plus
/// its evaluation. Kept intentionally minimal so swapping engines is cheap.
pub trait CrossCheckEngine {
    fn best_move(&mut self, fen: &str, depth: u8) -> crate::Result<EngineCheck>;
    fn name(&self) -> &'static str;
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
