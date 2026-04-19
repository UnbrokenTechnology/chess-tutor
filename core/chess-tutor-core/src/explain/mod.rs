//! Template-based prose generator.
//!
//! No LLMs, ever. Every sentence the user reads comes from a registered
//! template with typed slots. Templates are prioritised by significance so
//! the app can show the most important insight first and fall back to
//! progressive disclosure for detail.

use crate::PositionAnalysis;

/// A single piece of user-facing commentary, paired with the significance
/// score that determines ordering in the UI.
#[derive(Debug, Clone)]
pub struct Phrase {
    pub text: String,
    pub significance: u32,
}

/// Walks a [`PositionAnalysis`] and emits ranked [`Phrase`]s.
#[derive(Debug, Default)]
pub struct Explainer;

impl Explainer {
    pub fn new() -> Self {
        Self
    }

    /// Produce an ordered set of phrases for the given analysis.
    ///
    /// Phase 1 stub. Filled in as templates land.
    pub fn explain(&self, _analysis: &PositionAnalysis) -> Vec<Phrase> {
        Vec::new()
    }

    /// Produce the paired "your move vs. best move" commentary that is the
    /// app's signature pedagogical feature.
    pub fn compare(
        &self,
        _your_move: &PositionAnalysis,
        _best_move: &PositionAnalysis,
    ) -> Vec<Phrase> {
        Vec::new()
    }
}
