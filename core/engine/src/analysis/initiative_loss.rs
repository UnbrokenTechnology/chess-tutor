//! Loss-of-initiative detection — the *un-named* sibling of the named
//! tactics.
//!
//! When the static eval and the search disagree on a move (see
//! [`super::surprise`] / `LooksGoodButBad`), the position is governed by
//! **forcing play**, not by static features — the disagreement *magnitude*
//! is the size of the forcing sequence the one-ply static eval can't see.
//! That disagreement is explained by exactly one of three things:
//!
//! 1. a **named tactic** (fork / pin / skewer / …) — handled by
//!    [`super::tactic_outcome`];
//! 2. an **un-named forcing chain that costs us the initiative** — our
//!    pieces get chased, we retreat / defend / trade, the opponent makes a
//!    run of forcing moves we must answer while they improve — *this
//!    module*; or
//! 3. **nothing human-findable** (deep silent sequencing) — handled by
//!    [`super::silent_sequencing`] (the depth-honesty note).
//!
//! This is the missing member of that family. It is still *detectors-only*
//! in spirit: we only report a chain a human could actually have seen —
//! a short run of forcing moves at the root of the user's own line, the
//! kind a student walks forward by hand. The regression target is the
//! `e5` push in `rnbqkb1r/pp2npp1/3pp2p/8/2BQP3/5N2/PPP2PPP/RNB2RK1`:
//! after `e5`, Black plays `…d5` (hits the `Bc4`), `…Bd7` (blocks the
//! check, hits the bishop again), `…a6` (hits it a third time) — the
//! bishop is chased off the board while Black develops. Static loves `e5`
//! (space, king attack); search rates it a mistake because it hands Black
//! the initiative.
//!
//! ## What we detect
//!
//! Walk the user's PV from the move just played. For each
//! (opponent move, our reply) pair in the immediate run, count a **tempo
//! hit** when the opponent's move **newly forces one of our pieces to
//! move** (a piece that enters the hanging / SEE-losing set) **and** our
//! reply is *reactive* (we move the threatened piece, defend it, or capture
//! the attacker). Two or more such hits in a row at the root is the
//! "you're being pushed around" signature.
//!
//! Deliberately **only** counts the *attacked-and-must-move* shape — not
//! captures, not checks. A capture-and-recapture is a *trade* (the material
//! axis), and a forced trade-down sequence (`…Bxe6 Qxe6 Qxe6+ Kxe6 …`, the
//! `…Qc8` silent-sequencing case) is liquidation, not harassment — counting
//! those would mis-fire on exactly the deep-sequencing position this layer
//! must stay quiet on. Checks that herd the king are the
//! [`super::forcing_check_chain`] detector's job, not this one.
//!
//! `material_lost` distinguishes the two sub-cases for honest wording: did
//! the forcing run also net material off us (then "the line wins material
//! back"), or did we hold material but bleed tempo (then "you gave up the
//! initiative")? The `e5` case is the latter — material stays dead even.

use std::collections::HashSet;

use super::threats_outcome::{list_hanging, list_see_losing};
use crate::position::Position;
use crate::types::{Color, Move, Square};

#[cfg(test)]
#[path = "initiative_loss_tests.rs"]
mod tests;

/// One forcing-move / reactive-reply pair in the user's line: the opponent
/// attacks one of our pieces, and we have to move/defend it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TempoHit {
    /// Index of `threat` within the analysed PV (so callers can re-walk
    /// the PV to render SAN at the right board). `our_reply` is `ply + 1`.
    pub ply: usize,
    /// The opponent's move that newly attacked one of our pieces.
    pub threat: Move,
    /// Our reactive reply to it.
    pub our_reply: Move,
    /// The square of the piece that was newly forced to move/defend.
    pub target: Square,
}

/// A detected run of the opponent forcing us to react. By construction the
/// first hit is the opponent's *immediate* reply (`hits[0].ply == 1`).
///
/// Deliberately carries **no** material verdict: this card is the
/// *initiative* story (material holds, but you're on the defensive). When a
/// move genuinely drops material, that's the material / tactic cards' job —
/// not this one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InitiativeLoss {
    /// The tempo hits, in PV order. Non-empty; `hits[0].ply == 1`.
    pub hits: Vec<TempoHit>,
}

/// Only inspect the immediate forcing run — the part a human would walk
/// forward by hand. Beyond this we're into engine-depth territory that
/// [`super::silent_sequencing`] handles instead. (The fire condition keys
/// on the *first* pair regardless; this just bounds how much of the chain
/// we collect for the narration.)
pub const MAX_PV_PAIRS: usize = 4;

/// Detect a loss-of-initiative pattern in `pv` (the user's analysed line,
/// `pv[0]` = the user's move), from `root_stm`'s point of view. Returns
/// `None` when fewer than [`MIN_TEMPO_HITS`] tempo hits appear in the
/// first [`MAX_PV_PAIRS`] reply pairs.
///
/// Pure and side-effect-free (operates on a clone). The caller is expected
/// to have already established that the move is worse than it looks (a
/// mistake-ish verdict with a static-vs-search surprise) — this function
/// only answers the structural "is the opponent bullying us?" question.
pub fn detect_initiative_loss(
    pre_move_pos: &Position,
    pv: &[Move],
    root_stm: Color,
) -> Option<InitiativeLoss> {
    let first = *pv.first()?;
    let mut board = pre_move_pos.clone();
    board.do_move(first);

    let mut hits: Vec<TempoHit> = Vec::new();
    let mut pairs = 0usize;
    let mut i = 1usize;
    while i + 1 < pv.len() && pairs < MAX_PV_PAIRS {
        // The pair must be (opponent move, our reply).
        if board.side_to_move() != !root_stm {
            break;
        }
        let opp_move = pv[i];
        let our_reply = pv[i + 1];

        let before = threatened_squares(&board, root_stm);
        board.do_move(opp_move);
        let after = threatened_squares(&board, root_stm);

        // The piece newly forced into trouble — highest-value if several.
        // This is the *only* thing we count: a piece attacked and made to
        // move. Captures (trades) and checks are excluded by construction.
        let new_threat = after
            .difference(&before)
            .copied()
            .max_by_key(|sq| board.piece_on(*sq).map(|p| p.kind().classical_points()).unwrap_or(0));

        if let Some(target) = new_threat {
            if reply_is_quiet_reaction(&board, our_reply, target, root_stm) {
                hits.push(TempoHit {
                    ply: i,
                    threat: opp_move,
                    our_reply,
                    target,
                });
            }
        }

        board.do_move(our_reply);
        i += 2;
        pairs += 1;
    }

    // Fire only when the opponent's **immediate** reply (the first ply
    // after the user's move) already harasses us — "the move conceded the
    // initiative at once." Keying on ply 1 makes this robust to which exact
    // continuation the search returns: we don't depend on a deep chain
    // surviving, only on the engine's best *reply* being a piece-attack we
    // must answer. (Deeper-only harassment isn't reliably the user move's
    // fault and is left to the depth-honesty / silent-sequencing path.)
    if hits.first().map(|h| h.ply) != Some(1) {
        return None;
    }
    Some(InitiativeLoss { hits })
}

/// Whether `our_reply` (played from `after_opp`, now `root_stm` to move) is
/// a **quiet reaction** to the threat on `target` — a retreat or a defense
/// that spends a move dealing with the threat *without trading*.
///
/// Crucially a **capturing** reply does NOT count: a capture/recapture is a
/// trade (the material axis), and a forced trade-down sequence is
/// liquidation, not harassment. This is the line that keeps the `…Qc8`
/// silent-sequencing case (whose reactions are recaptures) from firing
/// while the `e5` case (whose reactions are quiet retreats/defenses) does.
fn reply_is_quiet_reaction(
    after_opp: &Position,
    our_reply: Move,
    target: Square,
    root_stm: Color,
) -> bool {
    if after_opp.is_capture(our_reply) {
        return false;
    }
    // Fled: the pressured piece itself moved (quietly) to safety.
    if our_reply.from() == target {
        return true;
    }
    // Defended: after a quiet move the target is no longer in the hanging /
    // SEE-losing set (e.g. a pawn push that now guards it, or a block).
    let mut probe = after_opp.clone();
    probe.do_move(our_reply);
    !threatened_squares(&probe, root_stm).contains(&target)
}

/// The squares of `side`'s pieces that are under real pressure — the union
/// of the hanging (undefended-and-attacked) and SEE-losing (defended but
/// loses an exchange) lists. These two lists are disjoint by construction
/// (see [`list_see_losing`]'s "don't double-report" note).
fn threatened_squares(pos: &Position, side: Color) -> HashSet<Square> {
    list_hanging(pos, side)
        .iter()
        .chain(list_see_losing(pos, side).iter())
        .map(|h| h.location.square)
        .collect()
}
