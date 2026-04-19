//! Tactical motif detection.
//!
//! Phase 1 targets: fork, pin (absolute + relative), skewer, discovered
//! attack, double check. Phase 6 expands to deflection, interference,
//! overloading, trapped pieces, x-ray, etc.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TacticsReport {
    pub motifs: Vec<Motif>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Motif {
    pub kind: MotifKind,
    /// Squares involved (attacker, victims, pivot piece, etc.), for the UI to
    /// highlight.
    pub squares: Vec<String>,
    /// Optional free-form detail — not user-facing prose, just enough data for
    /// the explainer to pick the right template.
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MotifKind {
    Fork,
    AbsolutePin,
    RelativePin,
    Skewer,
    DiscoveredAttack,
    DoubleCheck,
    // Phase 6 additions:
    Deflection,
    Interference,
    Overloading,
    BackRankWeakness,
    SmotheredMatePattern,
    TrappedPiece,
    XRay,
}
