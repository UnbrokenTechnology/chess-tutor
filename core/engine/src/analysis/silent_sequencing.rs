//! Silent-sequencing diagnostic — the teaching layer's *humility* gate.
//!
//! See `PLAN-teaching-gui.md` §4.3 and
//! [`teaching-positions/silent-sequencing-after-qc8`]. Some moves the
//! engine hates have **no teachable mechanism**: the verdict only emerges
//! below human calculation depth and no named pattern fires. The `…Qc8`
//! case is the canonical one — at depth 6 (a competent human's tactical
//! horizon) `…Qc8` and the engine's pick `…Be5` are functionally tied;
//! the gap only opens at depth 8+, and our full detector chain finds
//! nothing wrong with the move. Calling that a "blunder" implies the
//! student could have done better with available cognitive resources, and
//! the depth evidence says they couldn't.
//!
//! The diagnostic, straight from the case study's pseudocode:
//!
//! 1. **Gap small at shallow depth (~6).** Both the played move and the
//!    best move evaluate similarly — the difference is invisible at human
//!    depth.
//! 2. **Gap large at full depth.** The blunder verdict is real, just deep.
//! 3. **No tactic / overload detector fires, and the move didn't walk into
//!    a standing alignment.** There is a *name-able* pattern only if one of
//!    [`find_best_tactic_in_position`] (for the mover now, or the opponent
//!    after the move), [`find_overloaded`], or a surviving latent threat
//!    against the mover fires — and then the move is handled by the normal
//!    tactic / latent surfaces, not here.
//!
//! When all three hold, the honest description is "the engine sees this
//! getting difficult over the next several moves, but the reason is beyond
//! practical calculation depth — there isn't a shorter lesson here." No
//! "blunder" stamp, no fabricated mechanism (PLAN §4.3, the case study's
//! "what the teaching layer should *not* do").
//!
//! ## Cost / determinism
//!
//! The deep gap is already computed by the retrospective's full-depth
//! search (passed in), so the only added work is one **shallow** search
//! (`SHALLOW_DEPTH` plies) that force-includes the two candidate moves.
//! That runs on a fresh, isolated single-thread [`Engine`] so it never
//! touches the play engine's TT and is bit-deterministic (depth-budget,
//! no time budget). It is only invoked on a move that already tripped the
//! bad-eval pipeline, so the cost is bounded to those rare moves.

use crate::analysis::overloading::find_overloaded;
use crate::analysis::tactic_outcome::{find_best_tactic_in_position, PriorMove};
use crate::engine::{Engine, SearchParams};
use crate::position::Position;
use crate::types::{Color, Move, Value};

#[cfg(test)]
#[path = "silent_sequencing_tests.rs"]
mod tests;

/// The shallow depth that stands in for "a competent human's tactical
/// calculation horizon" (the case study's depth-6 row, where `…Qc8` and
/// `…Be5` are functionally tied). The diagnostic asks: is the gap
/// invisible *here* yet large at full depth?
pub const SHALLOW_DEPTH: u32 = 6;

/// Max |gap| at [`SHALLOW_DEPTH`], in engine-internal cp, for the moves
/// to count as "functionally tied at human depth."
///
/// Calibration note (2026-05-31, against the current engine): the case
/// study's depth-6 numbers (~74 cp gap) were measured with MultiPV-4, but
/// our MultiPV shallow score is wildly unstable with `multi_pv` (74 cp at
/// mpv-4, 544 cp at mpv-8) because of the aspiration-window interaction
/// (memory `project_multipv_mate_pathology`). So [`shallow_gap_cp`]
/// measures the gap with **two independent single-PV searches** instead —
/// deterministic and ordering-free. On that stable measurement the `…Qc8`
/// case study's depth-6 gap is ~203 cp, while the genuinely-findable
/// `Qc5+` blunder's depth-6 gap is ~480 cp; 300 cp separates them with
/// headroom on both sides. A gap a human would see at shallow depth means
/// the lesson *is* findable — not silent sequencing.
pub const SHALLOW_GAP_MAX_CP: i32 = 300;

/// Min |gap| at full depth, in engine-internal cp, for the deep verdict
/// to be "real." The case study's deep gap was ~567 cp; below ~1 pawn the
/// two picks aren't far enough apart to call either wrong, so the move
/// isn't a hidden blunder and there is nothing to be humble about.
pub const DEEP_GAP_MIN_CP: i32 = 213;

/// Does the move `candidate` (vs the engine's `best`) qualify as **silent
/// sequencing** from `pre_pos`?
///
/// - `pre_pos` — the position the move was played from (`root_stm` to
///   move).
/// - `candidate` / `best` — the played move and the engine's preferred
///   move (both legal at `pre_pos`).
/// - `deep_gap_cp` — `best_score − candidate_score` at full search depth,
///   in engine-internal cp, both root-STM POV. Already available from the
///   retrospective's `MoveAnalysis` slice; pass it in rather than
///   re-searching.
/// - `prior_move` — the opponent's move into `pre_pos`, for the recapture
///   guard in the detector chain. Pass `None` when unknown.
///
/// Returns `true` only when all three diagnostic conditions hold. Runs one
/// bounded shallow search on a fresh isolated engine; pure with respect to
/// any caller state.
pub fn is_silent_sequencing(
    pre_pos: &Position,
    candidate: Move,
    best: Move,
    deep_gap_cp: i32,
    prior_move: Option<PriorMove>,
) -> bool {
    // Same move → no gap to explain.
    if candidate == best {
        return false;
    }
    // Condition 2 first — it's free (the gap is passed in). A small deep
    // gap means neither pick is clearly wrong; nothing to suppress.
    if deep_gap_cp.abs() < DEEP_GAP_MIN_CP {
        return false;
    }
    // Condition 3 — a name-able pattern means the move is handled by the
    // tactic / latent / overload surfaces, not here. Cheap static scans;
    // run them before the (more expensive) shallow search.
    if has_nameable_pattern(pre_pos, candidate, prior_move) {
        return false;
    }
    // Condition 1 — the shallow search. Only reached for moves that
    // already passed the deep-gap and detector gates.
    let root_stm = pre_pos.side_to_move();
    let Some(shallow_gap) = shallow_gap_cp(pre_pos, candidate, best, root_stm) else {
        return false;
    };
    shallow_gap.abs() <= SHALLOW_GAP_MAX_CP
}

/// Whether any named tactic / overload / surviving-latent pattern fires
/// for the move — in which case it's a *teachable* mistake, not silent
/// sequencing. Mirrors the case study's condition (3) detector chain,
/// extended with the latent-threat survival check that PLAN §4.1 added to
/// the walked-into slot (a discovered attack the move failed to disrupt is
/// name-able, so it disqualifies silent sequencing).
fn has_nameable_pattern(pre_pos: &Position, candidate: Move, prior_move: Option<PriorMove>) -> bool {
    let root_stm = pre_pos.side_to_move();
    // (a) A tactic the mover can play now.
    if find_best_tactic_in_position(pre_pos, root_stm, prior_move).is_some() {
        return true;
    }
    // (b) A tactic the opponent gets to play after the candidate move.
    let mut after = pre_pos.clone();
    after.do_move(candidate);
    let sub_prior = PriorMove::new(pre_pos, candidate);
    if find_best_tactic_in_position(&after, !root_stm, Some(sub_prior)).is_some() {
        return true;
    }
    // (c) The mover overloaded — a sole defender doing two jobs.
    if !find_overloaded(pre_pos, root_stm).is_empty() {
        return true;
    }
    // (d) A *material-winning* standing alignment the candidate failed to
    //     disrupt — the same surviving-alignment signal PLAN §4.1 wired
    //     into the walked-into slot. We only count patterns whose payoff is
    //     an actual capture (DiscoveredAttack / RemovingDefender / Skewer),
    //     NOT the structural Pin / RelativePin: an absolute pin "always
    //     lights" geometrically and survives any move that leaves the king
    //     and pinned piece in line — including `…Qc8`, which *defends* the
    //     pinned piece (2:2) so the pin wins nothing. Counting the bare pin
    //     would wrongly disqualify the canonical silent-sequencing case,
    //     whose whole point is that no detector finds a real problem.
    use crate::analysis::tactic_outcome::TacticPattern;
    crate::analysis::latent_threats::find_latent_threats(&after, root_stm)
        .into_iter()
        .any(|t| {
            matches!(
                t.pattern,
                TacticPattern::DiscoveredAttack
                    | TacticPattern::RemovingDefender
                    | TacticPattern::Skewer
            )
        })
}

/// `best_score − candidate_score` at the shallow horizon, root-STM POV, in
/// engine-internal cp.
///
/// **Measured as two independent single-PV searches**, one per move, on
/// the position *after* each move (so each move's own subtree is scored in
/// isolation). This sidesteps the MultiPV-aspiration instability that
/// makes a force-include / MultiPV shallow gap meaningless (see
/// [`SHALLOW_GAP_MAX_CP`]). Each post-move search runs to
/// `SHALLOW_DEPTH - 1` plies (the move itself is the first ply), so the
/// move's effective horizon is `SHALLOW_DEPTH`. Fresh isolated
/// single-thread engines keep it deterministic and off the play engine's
/// TT.
fn shallow_gap_cp(pre_pos: &Position, candidate: Move, best: Move, root_stm: Color) -> Option<i32> {
    let cand = move_score_shallow(pre_pos, candidate, root_stm)?;
    let bst = move_score_shallow(pre_pos, best, root_stm)?;
    Some(bst.0 - cand.0)
}

/// Score `mv` (played from `pre_pos` by `root_stm`) at the shallow horizon,
/// in root-STM POV cp, via a single-PV search of the position after it. The
/// post-move search reports its score from the *opponent's* POV (they are
/// to move), so we negate to get root-STM POV.
fn move_score_shallow(pre_pos: &Position, mv: Move, root_stm: Color) -> Option<Value> {
    let mut after = pre_pos.clone();
    after.do_move(mv);
    // A move that ends the game (checkmate/stalemate) leaves no line to
    // search — treat as no measurable gap (the diagnostic bails).
    let depth = SHALLOW_DEPTH.saturating_sub(1).max(1);
    let mut engine = Engine::new(1);
    let params = SearchParams {
        max_depth: depth,
        multi_pv: 1,
        threads: 1,
        ..SearchParams::default()
    };
    let lines = engine.search(&mut after, params);
    let opp_pov = lines.first()?.score;
    let _ = root_stm; // POV flip is purely the negation below.
    Some(Value(-opp_pov.0))
}
