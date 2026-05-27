//! Teaching-analysis pipeline: turn a [`crate::engine::SearchLine`]
//! into a [`MoveAnalysis`] that attributes the move's effect to
//! named classical-evaluation terms, plus a family of structured
//! outcome types that drive the CLI retrospective narrators.
//!
//! For each root move the search produced, we:
//!
//! 1. Walk the PV to the "settled" ply (the point past which the
//!    white-POV score stops shifting materially — see
//!    [`crate::search::compute_settled_ply`]).
//! 2. Diff that settled [`crate::eval::EvalTrace`] against the
//!    trace of the root position (the `pre_move_trace` baseline)
//!    term-by-term.
//! 3. Taper each term's `(mg, eg)` delta with the same phase and
//!    endgame-scale-factor the main evaluator used for the settled
//!    position, so the resulting `delta_tapered` is in the same
//!    engine-internal cp the search scores are in.
//! 4. Sort by absolute tapered delta so the biggest strategic
//!    swing is first.
//!
//! Units everywhere inside [`MoveAnalysis`] are engine-internal
//! Stockfish-scale cp (PawnEG = 213). UI layers convert to pawns at
//! render time.
//!
//! ## Module layout
//!
//! - [`term_id`] / [`term_delta`]: the enum labelling each granular
//!   sub-term and the sorted tapered-cp diff between two traces.
//! - [`move_analysis`]: [`MoveAnalysis`] + [`analyze_position`] —
//!   the orchestrator that wraps search output.
//! - [`verdict`]: [`MoveVerdict`] + `classify_move` thresholds.
//! - [`surprise`]: [`SurpriseKind`] + `detect_surprise` shallow-vs-deep
//!   disagreement detector.
//! - `*_outcome` modules: structured pre/post snapshots of
//!   material, threats, king safety, pawn structure, mobility,
//!   passed pawns, and piece placement. Material alone walks the
//!   whole PV to the settled ply (it's narrating the expected
//!   capture sequence); every other positional outcome diffs
//!   pre-move against the position immediately after the user's
//!   single move, so narration attributes board-state changes to
//!   that move and not to the opponent's subsequent replies.
//!
//! ## Design principles
//!
//! Everything the UI says must trace back to concrete engine data
//! — never pattern-matched templates against aggregate scores. This
//! is the explicit anti-goal vs. consumer chess sites that
//! "narrate" by guessing what a swing-of-X-cp probably means.
//!
//! **Cumulative-threshold term selection**, not top-N. Narration
//! shows the smallest prefix of `term_deltas` that accounts for
//! ≥ 75 % of total |delta|. A one-term blunder produces a one-term
//! list; a subtle positional combo produces 4–5.
//!
//! **Two analysis triggers, no others.** On-demand (user hits a
//! hint button — no latency budget) and retrospective (after the
//! most-recently-played move). Every-move pre-commit analysis is
//! explicitly NOT a mode — too expensive, too hand-holdy.
//!
//! ## Out of scope for this pipeline
//!
//! - **Traps.** Memorisation of named refutation patterns. Lives
//!   in [`crate::traps`] with its own structured outputs.
//! - **Opening identification.** Lives in [`crate::openings`];
//!   may be referenced as *context* for narration but doesn't
//!   drive verdict classification.
//! - **Game-level post-game commentary** (blunder summary, ELO
//!   estimate, etc.). Separate product surface, not built.
//!
//! ## Deferred work
//!
//! - **Cheap-pass + surprise detection (Phase 2).** Depth-1
//!   qsearch + SEE for every legal move; compare cheap ranking
//!   against full-depth MultiPV ranking to flag
//!   `LooksGoodButBad` / `LooksBadButGood`. Today's workaround:
//!   `multi_pv = legal_count` gives a real deep score for every
//!   root move. Phase 2 is a latency optimisation, not a
//!   correctness prerequisite.
//! - **Signal-mask (Phase 4).** Zero each [`crate::eval::EvalTrace`]
//!   term in turn and re-rank moves. If zeroing term X changes
//!   the top move, attach a `MaskedHint` saying "you'd prefer
//!   M' if you undervalued X — but X is what makes M the best."
//! - **Tactic library (Phase 5).** Parallel to [`crate::traps`]
//!   but for general patterns: absolute pin, relative pin, fork,
//!   skewer, double attack, then discovered attack, deflection,
//!   overloading, x-ray, interference. Each is a detector
//!   `fn(&Position, Move) -> Option<TacticHit>`. Run on demand
//!   only — the prior-attempt repo ran tactics inside search and
//!   killed perf.
//!
//! ## Open questions (revisit when real output lands)
//!
//! - **Settled-ply threshold**: currently 25 cp. Real positions
//!   may want higher, or a different metric (largest single jump?
//!   variance-based?).
//! - **Piece attribution**: Material/PSQ are trivial. Threats /
//!   King Safety / Mobility aggregate over many pieces; richer
//!   attribution may need scratch state on `Evaluator` or
//!   pattern-matching at template time.

pub mod blocked_center_outcome;
pub mod castling_outcome;
pub mod initiative_outcome;
pub mod king_safety_outcome;
pub mod material_outcome;
pub mod mobility_outcome;
pub mod move_analysis;
pub mod move_assessment;
pub mod overlays;
pub mod overloading;
pub mod passed_pawns_outcome;
pub mod pawn_structure_outcome;
pub mod pieces_positional_outcome;
pub mod space_outcome;
pub mod surprise;
pub mod tactic_outcome;
pub mod tactic_util;
pub mod term_delta;
pub mod term_id;
pub mod threats_outcome;
pub mod win_chances;
pub mod verdict;

#[cfg(test)]
mod test_support;

/// Apply the user's move (ply 0 of the analysed line) to a clone of
/// `pre_move_pos` and return the resulting position. Used by every
/// non-material positional outcome to diff pre-move state against
/// the state immediately after the user's move. An empty PV returns
/// `pre_move_pos` unchanged.
fn post_user_move(
    pre_move_pos: &crate::position::Position,
    ma: &MoveAnalysis,
) -> crate::position::Position {
    let mut scratch = pre_move_pos.clone();
    if let Some(&mv) = ma.pv.first() {
        scratch.do_move(mv);
    }
    scratch
}

// Re-exports: the `analysis::Foo` path is the documented public API.
// Sub-modules keep implementation and tests close together but
// callers should use `chess_tutor_engine::analysis::Foo` everywhere.
pub use blocked_center_outcome::{compute_blocked_center_outcome, BlockedCenterOutcome};
pub use castling_outcome::{compute_castling_outcome, CastlingOutcome};
pub use initiative_outcome::{compute_initiative_outcome, InitiativeOutcome};
pub use king_safety_outcome::{compute_king_safety_outcome, KingSafetyOutcome, KingSafetySnapshot};
pub use material_outcome::{compute_material_outcome, CaptureEvent, MaterialOutcome};
pub use mobility_outcome::{compute_mobility_outcome, MobilityOutcome, PieceMobility};
pub use move_analysis::{analyze_position, MoveAnalysis};
pub use move_assessment::{
    classify_user_move, BlunderInfo, GatingConfig, MoveAssessment, TeachingInfo, TermContribution,
    TermFamily,
};
pub use overlays::{compute_overlays, trapped_cages, OverlayData};
pub use overloading::{find_overloaded, OverloadedPiece};
pub use passed_pawns_outcome::{compute_passed_pawns_outcome, PassedPawnsOutcome};
pub use pawn_structure_outcome::{compute_pawn_structure_outcome, PawnStructureOutcome};
pub use pieces_positional_outcome::{compute_pieces_positional_outcome, PiecesPositionalOutcome};
pub use space_outcome::{compute_space_outcome, SpaceOutcome};
pub use surprise::{detect_surprise, SurpriseKind};
pub use tactic_outcome::{
    compute_tactic_outcome, Confidence, MatePattern, PriorMove, TacticHit, TacticPattern,
    TacticsOutcome,
};
pub use term_delta::{compute_term_deltas, cumulative_prefix, TermDelta};
pub use term_id::TermId;
pub use threats_outcome::{
    compute_threats_outcome, filter_guaranteed_targets, list_hanging, list_see_losing,
    HangingPiece, PieceLocation, PressureKind, PressuredPiece, ThreatsOutcome,
};
pub use verdict::{classify_move, MoveVerdict};
pub use win_chances::win_chances;
