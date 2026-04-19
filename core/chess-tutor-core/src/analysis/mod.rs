//! Square-level analysis: attacker/defender maps, Static Exchange Evaluation,
//! candidate move annotation.

use serde::{Deserialize, Serialize};

/// Per-square attacker/defender data for the current position.
///
/// Phase 1 stub. Filled in once the attacker-map pass lands.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SquareData {
    /// Placeholder until bitboards land. 64 entries, a1..h8.
    pub squares: Vec<SquareReport>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SquareReport {
    pub white_attackers: u8,
    pub black_attackers: u8,
    /// SEE value for capturing on this square with the side to move.
    pub see: Option<i32>,
}

/// A legal move annotated with the analysis hooks the explainer consumes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CandidateMove {
    pub uci: String,
    pub san: String,
    pub material_change: i32,
    pub see: Option<i32>,
    pub gives_check: bool,
    pub is_capture: bool,
    /// Names of tactical motifs this move creates or executes, e.g. "fork".
    pub tactics: Vec<String>,
    /// Names of positional features this move changes, e.g. "opens-d-file".
    pub positional: Vec<String>,
    pub rank: u32,
}
