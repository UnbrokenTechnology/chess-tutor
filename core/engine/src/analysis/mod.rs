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

pub mod blocked_center_outcome;
pub mod castling_outcome;
pub mod initiative_outcome;
pub mod king_safety_outcome;
pub mod material_outcome;
pub mod mobility_outcome;
pub mod move_analysis;
pub mod passed_pawns_outcome;
pub mod pawn_structure_outcome;
pub mod pieces_positional_outcome;
pub mod space_outcome;
pub mod surprise;
pub mod term_delta;
pub mod term_id;
pub mod threats_outcome;
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
pub use mobility_outcome::{compute_mobility_outcome, MobilityOutcome};
pub use move_analysis::{analyze_position, MoveAnalysis};
pub use passed_pawns_outcome::{compute_passed_pawns_outcome, PassedPawnsOutcome};
pub use pawn_structure_outcome::{compute_pawn_structure_outcome, PawnStructureOutcome};
pub use pieces_positional_outcome::{compute_pieces_positional_outcome, PiecesPositionalOutcome};
pub use space_outcome::{compute_space_outcome, SpaceOutcome};
pub use surprise::{detect_surprise, SurpriseKind};
pub use term_delta::{compute_term_deltas, cumulative_prefix, TermDelta};
pub use term_id::{TermId, Timing};
pub use threats_outcome::{
    compute_threats_outcome, HangingPiece, PieceLocation, PressureKind, PressuredPiece,
    ThreatsOutcome,
};
pub use verdict::{classify_move, MoveVerdict};
