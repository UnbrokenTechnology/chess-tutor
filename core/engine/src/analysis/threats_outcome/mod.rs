//! [`ThreatsOutcome`] — hanging, SEE-losing, and Stockfish-pattern
//! pressure, for both sides, at the position immediately after the
//! user's move vs the pre-move baseline.
//!
//! Three threat categories:
//!
//! - **Hanging** — attacked by ≥ 1 enemy piece AND undefended. The
//!   simplest 400–1200 player pattern: "opponent takes for free."
//! - **SEE-losing** — attacked AND defended, but the
//!   static-exchange evaluator says the opponent still wins
//!   strictly-positive material if they initiate the exchange.
//!   Classic 1000–1400 case: our piece is defended once but
//!   attacked by two lower-value enemies (fork the defender with an
//!   overload).
//! - **Pressured** — neither hanging nor SEE-losing, but facing a
//!   Stockfish-evaluator threat pattern (minor-on-major,
//!   rook-on-queen, safe-pawn-threat) that forces the piece to
//!   move or concede positional ground.

mod guaranteed;
mod lists;
mod types;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use super::{post_user_move, MoveAnalysis};
use crate::position::Position;
use crate::types::Color;

pub use guaranteed::filter_guaranteed_targets;
pub use lists::{list_hanging, list_see_losing};
pub use types::{HangingPiece, PieceLocation, PressureKind, PressuredPiece, ThreatsOutcome};

use lists::list_pressured;

/// Compute hanging-piece + SEE-losing + Stockfish-pressure
/// comparisons against `pre_move_pos`, measured at the position
/// immediately after the user's move.
///
/// Pieces are deemed hanging if `attackers_to(sq, occupied) & enemy
/// != empty` AND `attackers_to(sq, occupied) & ours == empty`.
/// Kings excluded — "hanging king" isn't a meaningful teaching
/// concept.
pub fn compute_threats_outcome(
    ma: &MoveAnalysis,
    pre_move_pos: &Position,
    root_stm: Color,
) -> ThreatsOutcome {
    // Pre-move baseline: each category's count at the position
    // before the user moved.
    let pre_ours_hang = list_hanging(pre_move_pos, root_stm).len();
    let pre_theirs_hang = list_hanging(pre_move_pos, !root_stm).len();
    let pre_ours_see = list_see_losing(pre_move_pos, root_stm).len();
    let pre_theirs_see = list_see_losing(pre_move_pos, !root_stm).len();
    let pre_ours_pressured = list_pressured(pre_move_pos, root_stm).len();
    let pre_theirs_pressured = list_pressured(pre_move_pos, !root_stm).len();

    let scratch = post_user_move(pre_move_pos, ma);

    let ours_hanging = list_hanging(&scratch, root_stm);
    let theirs_hanging = list_hanging(&scratch, !root_stm);
    let ours_see_losing = list_see_losing(&scratch, root_stm);
    let theirs_see_losing = list_see_losing(&scratch, !root_stm);
    let ours_pressured = list_pressured(&scratch, root_stm);
    let theirs_pressured = list_pressured(&scratch, !root_stm);

    let theirs_hanging_guaranteed =
        filter_guaranteed_targets(&scratch, &theirs_hanging, root_stm);
    let theirs_see_losing_guaranteed =
        filter_guaranteed_targets(&scratch, &theirs_see_losing, root_stm);

    let ours_hanging_delta = ours_hanging.len() as i32 - pre_ours_hang as i32;
    let theirs_hanging_delta = theirs_hanging.len() as i32 - pre_theirs_hang as i32;
    let ours_see_losing_delta = ours_see_losing.len() as i32 - pre_ours_see as i32;
    let theirs_see_losing_delta = theirs_see_losing.len() as i32 - pre_theirs_see as i32;
    let ours_pressured_delta = ours_pressured.len() as i32 - pre_ours_pressured as i32;
    let theirs_pressured_delta = theirs_pressured.len() as i32 - pre_theirs_pressured as i32;

    ThreatsOutcome {
        ours_hanging,
        theirs_hanging,
        ours_see_losing,
        theirs_see_losing,
        theirs_hanging_guaranteed,
        theirs_see_losing_guaranteed,
        ours_pressured,
        theirs_pressured,
        ours_hanging_delta,
        theirs_hanging_delta,
        ours_see_losing_delta,
        theirs_see_losing_delta,
        ours_pressured_delta,
        theirs_pressured_delta,
    }
}
