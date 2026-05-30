//! Escape-hatch detection for a detected tactic.
//!
//! The [`super::tactic_outcome`] detectors are static: they confirm a
//! pattern's *geometry* exists, but can't see a forcing defensive
//! resource that resolves a move or two later. This module adds that
//! check, structurally and without eval thresholds.
//!
//! ## Model (see `PLAN-tactic-escape.md`)
//!
//! A tactic asserts a **specific expected capture** — a concrete
//! board-state change, not a centipawn delta. After the tactic's key
//! move, the owner threatens to capture an expected target on the
//! following move. The opponent gets one reply. If some reply leaves the
//! owner unable to profitably capture **any** expected target, the
//! opponent **escaped**, and that reply is the refutation. This sidesteps
//! the eval-threshold tar pit: "won a rook but dropped 150 cp of
//! position" / "swapped down to a minor" are never threshold calls — we
//! ask only the boolean "did the *specific* expected capture survive?".
//!
//! "Profitably capturable" reuses [`super::tactic_util::is_in_bad_spot`]
//! — the exact predicate the detectors use for "the owner can win this
//! piece" — so escape detection and detection agree on what "winnable"
//! means.
//!
//! Only the patterns whose payoff is "I take X next move" are analysed
//! (`Fork`, `RemovingDefender`, `Pin`, `Skewer`, `DiscoveredAttack`).
//! Patterns whose material is already collected by the key move
//! (`HangingCapture`, …) or that aren't material (`Checkmate`, the
//! checks) are left alone.

use super::tactic_outcome::{TacticHit, TacticPattern};
use super::tactic_util::{is_in_bad_spot, king_value};
use crate::movegen::legal_moves_vec;
use crate::position::Position;
use crate::types::{Color, Move, Square};

#[cfg(test)]
#[path = "tactic_escape_tests.rs"]
mod tests;

/// Why the opponent's refutation prevents the expected capture. Derived
/// purely from the refuting move's shape, so it's deterministic and
/// explainable.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EscapeKind {
    /// The refutation gives check — the owner must answer it instead of
    /// collecting the target (e.g. a pinned piece leaving with check).
    ForcingCheck,
    /// The refutation is a capture (often the threatened piece taking the
    /// attacker, or a forcing in-between capture).
    Zwischenzug,
    /// A quiet move that neutralises a multi-target (fork) threat at once.
    DefendsBothTargets,
    /// The threatened piece simply moves itself to safety.
    AdequateRetreat,
    /// A quiet move that defends the target or makes a counter-threat.
    CounterAttack,
}

/// A clean defensive resource against a detected tactic.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TacticEscape {
    /// The opponent's refuting move (their first reply to the tactic).
    pub refutation: Move,
    pub kind: EscapeKind,
    /// The square the owner expected to capture but no longer can.
    pub expected_target: Square,
}

/// Whether `hit` is a pattern this module reasons about — one whose
/// payoff is "the owner captures a target on the move after the key
/// move."
fn analysable(p: TacticPattern) -> bool {
    matches!(
        p,
        TacticPattern::Fork
            | TacticPattern::RemovingDefender
            | TacticPattern::Pin
            | TacticPattern::Skewer
            | TacticPattern::DiscoveredAttack
    )
}

/// Whether the owner can profitably win the piece on `sq` right now: an
/// enemy piece sits there and it's in a bad spot (attacked by the owner
/// and either hanging or takeable by something cheaper).
fn target_winnable(pos: &Position, sq: Square, owner: Color) -> bool {
    matches!(pos.piece_on(sq), Some(p) if p.color() != owner) && is_in_bad_spot(pos, sq)
}

/// Find a clean escape from `hit`, owned by `owner`, in the pre-move
/// position `pos` (the position `hit.key_move` is played from).
///
/// Returns `None` when the pattern isn't analysable, the key move /
/// targets are missing, the tactic's premise doesn't hold (no target is
/// winnable to begin with), or no opponent reply defuses it. Otherwise
/// returns the opponent's best (most forcing, then lowest-indexed)
/// refuting reply.
pub fn find_tactic_escape(pos: &Position, hit: &TacticHit, owner: Color) -> Option<TacticEscape> {
    if !analysable(hit.pattern) || hit.targets.is_empty() {
        return None;
    }
    let key_move = hit.key_move?;

    let mut post_m = pos.clone();
    post_m.do_move(key_move);
    // After the owner's key move it is the opponent's turn.
    let targets = &hit.targets;

    // Premise: at least one target must be winnable right now, or there's
    // no "expected capture" for an escape to deny — stay silent.
    let expected_target = targets
        .iter()
        .copied()
        .filter(|&t| target_winnable(&post_m, t, owner))
        .max_by_key(|&t| post_m.piece_on(t).map(|p| king_value(p.kind())).unwrap_or(0))?;

    let mut scratch = post_m.clone();
    let mut best: Option<(EscapeKind, Move)> = None;
    for reply in legal_moves_vec(&mut scratch) {
        if !reply_defuses(&post_m, targets, owner, reply) {
            continue;
        }
        let kind = classify(&post_m, targets, reply);
        best = Some(match best {
            None => (kind, reply),
            Some(prev) => better_escape(prev, (kind, reply)),
        });
    }

    best.map(|(kind, refutation)| TacticEscape {
        refutation,
        kind,
        expected_target,
    })
}

/// Whether `reply` (the opponent's move in `post_m`) leaves the owner
/// unable to profitably capture any expected target.
fn reply_defuses(post_m: &Position, targets: &[Square], owner: Color, reply: Move) -> bool {
    let gives_check = post_m.gives_check(reply);
    let mut post_r = post_m.clone();
    post_r.do_move(reply);

    if gives_check {
        // The owner is in check and must answer it. The tactic survives
        // only if the owner can answer by capturing a target outright;
        // otherwise the check pulls the owner off the target — an escape.
        // (Per the plan we report the first forcing reply and don't trace
        // deeper than this.)
        let mut s = post_r.clone();
        let can_capture_target = legal_moves_vec(&mut s)
            .iter()
            .any(|m| targets.contains(&m.to()) && post_r.is_capture(*m));
        !can_capture_target
    } else {
        // No check: the tactic survives iff some target is still winnable.
        !targets.iter().any(|&t| target_winnable(&post_r, t, owner))
    }
}

/// Classify a refuting reply by its shape. Order matters — a checking
/// reply that also captures is still reported as `ForcingCheck`.
fn classify(post_m: &Position, targets: &[Square], reply: Move) -> EscapeKind {
    if post_m.gives_check(reply) {
        EscapeKind::ForcingCheck
    } else if post_m.is_capture(reply) {
        EscapeKind::Zwischenzug
    } else if targets.len() >= 2 {
        EscapeKind::DefendsBothTargets
    } else if targets.contains(&reply.from()) {
        EscapeKind::AdequateRetreat
    } else {
        EscapeKind::CounterAttack
    }
}

/// Rank for choosing among multiple escapes — lower is "more forcing",
/// the move the opponent would actually reach for.
fn kind_rank(k: EscapeKind) -> u8 {
    match k {
        EscapeKind::ForcingCheck => 0,
        EscapeKind::Zwischenzug => 1,
        EscapeKind::DefendsBothTargets => 2,
        EscapeKind::AdequateRetreat => 3,
        EscapeKind::CounterAttack => 4,
    }
}

/// Pick the better of two escape candidates: most forcing first, then
/// lowest (from, to) square indices for a deterministic tiebreak.
fn better_escape(a: (EscapeKind, Move), b: (EscapeKind, Move)) -> (EscapeKind, Move) {
    let key = |c: &(EscapeKind, Move)| (kind_rank(c.0), c.1.from().index(), c.1.to().index());
    if key(&b) < key(&a) {
        b
    } else {
        a
    }
}
