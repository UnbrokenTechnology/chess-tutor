//! Quiescence / forcing-line search.
//!
//! Depth-capped walk of checks, captures, and threats from the current
//! position. Output feeds the explainer so it can describe concrete
//! consequences ("if you take, I take back and win a piece").

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ForcingLine {
    pub moves: Vec<String>,
    pub resulting_fen: String,
    /// Centipawn assessment of the leaf node, from White's perspective.
    pub leaf_eval_cp: Option<i32>,
    pub is_mate: bool,
}
