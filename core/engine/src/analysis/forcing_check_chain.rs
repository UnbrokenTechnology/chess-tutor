//! Self-replenishing forcing-check detection — "how many checks deep
//! can the attacker keep checking *no matter how the defender replies*?"
//!
//! Companion to [`super::check_followups`]. Where that module asks "does
//! a single check set up a follow-up tactic one ply later," this module
//! asks the [`teaching-positions/mating-net-after-ng5`] question: walk
//! the forcing line forward and see whether the checks **die out** or
//! whether another check is *always* waiting. The user's own rule of
//! thumb — "if the forcing line is at least three checks deep, stop and
//! reconsider" — is a **human-findable** signal: a player can walk three
//! checks forward by hand and notice the king is being herded into a
//! smaller and smaller box, *without* calculating the mate. That makes it
//! a legitimate detectors-only entry in the tactical-mode gate.
//!
//! This is emphatically **not** a mate solver. It does not score the
//! terminal position, does not look for mate, and does not care whether
//! the chain ends in a perpetual or a mating net. It counts one thing:
//! the depth to which forcing checks self-replenish against a given
//! king, capped so it stays sub-ms.
//!
//! ## What "self-replenishing to depth `d`" means
//!
//! From the attacker's POV, a chain reaches depth `d` when the attacker
//! has *some* checking move such that, for **every** legal reply the
//! defender has, a chain of depth `d − 1` still exists from the resulting
//! position. The "every reply" quantifier is what makes the line *forced*
//! — the defender can never step off the treadmill. Depth `0` is the base
//! case (the attacker has no check, or the cap is hit).
//!
//! ## Budget
//!
//! Checks are few (typically 0–3 per position) and a king in check has
//! few legal replies (1–4: move, block, capture). With the recursion
//! capped at [`MAX_CHAIN_DEPTH`] and a breadth cap of
//! [`MAX_CHECKS_PER_PLY`] checking moves explored per ply, the worst-case
//! node count is bounded by a small constant. No per-node allocation
//! beyond the move vectors the generator already returns.

use crate::movegen::legal_moves_vec;
use crate::position::Position;
use crate::types::{Color, Move};

#[cfg(test)]
#[path = "forcing_check_chain_tests.rs"]
mod tests;

/// Recursion cap. The user's rule fires at three; we explore a little
/// past it so the reported depth is meaningful (and so the gate can
/// distinguish "exactly three" from "much deeper"). Six plies of forcing
/// checks is already well past human calculation depth — anything beyond
/// it is reported as the cap.
pub const MAX_CHAIN_DEPTH: u8 = 6;

/// Breadth cap: at most this many of the attacker's checking moves are
/// explored per ply. Real positions rarely have more than 2–3 checks;
/// the cap is a safety net against a constructed position with a dozen
/// queen checks, keeping the scan sub-ms. Checks are tried in
/// move-generation order.
const MAX_CHECKS_PER_PLY: usize = 6;

/// The self-replenishing forcing-check chain the attacker has against
/// `defender`'s king, as computed by [`forcing_check_chain`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ForcingCheckChain {
    /// The maximum depth (in attacker checks) to which the chain
    /// self-replenishes against every defender reply. `0` means the
    /// attacker has no forcing check at all; a value equal to
    /// [`MAX_CHAIN_DEPTH`] means the chain runs at least that deep (it
    /// may be longer — we stop counting at the cap).
    pub depth: u8,
    /// The attacker's first check that begins the deepest chain found.
    /// `None` only when `depth == 0`.
    pub first_check: Option<Move>,
}

/// Compute the deepest self-replenishing forcing-check chain the
/// **attacker** (`!defender`) has against the **defender**'s king.
///
/// `defender` is the side whose king is being hunted — pass the user's
/// colour to ask "how deep a forcing-check sequence does my opponent
/// have at my king." The attacker is given the move via a null pivot
/// when it isn't already their turn (the same "free tempo" model the
/// other opponent-threat scanners use).
///
/// Returns a chain of `depth == 0` (and `first_check == None`) when the
/// attacker has no checking move, or when the defender is the side to
/// move and already in check (the standing-threat framing doesn't apply
/// — the defender has an obligation to address the current check first,
/// so the null pivot to the attacker would be unsound).
///
/// Pure with respect to `pos` (operates on a clone). Deterministic:
/// explores checks in move-generation order and reports the first check
/// that achieves the maximum depth.
pub fn forcing_check_chain(pos: &Position, defender: Color) -> ForcingCheckChain {
    let attacker = !defender;
    let mut scratch = pos.clone();
    // Pivot so it's the attacker's turn. If the defender is to move and
    // already in check, a null move would leave their king in check —
    // unsound — so bail with an empty chain.
    let null_saved = if scratch.side_to_move() != attacker {
        if scratch.in_check() {
            return ForcingCheckChain {
                depth: 0,
                first_check: None,
            };
        }
        Some(scratch.do_null_move())
    } else {
        None
    };

    let (depth, first_check) = chain_from_attacker(&mut scratch, MAX_CHAIN_DEPTH);

    if let Some(saved) = null_saved {
        scratch.undo_null_move(saved);
    }
    ForcingCheckChain { depth, first_check }
}

/// It is the attacker's turn in `pos`. Return the deepest forcing-check
/// chain (and the first check achieving it) of at most `budget` plies.
///
/// A chain of depth `k` exists via check `c` when, after `c`, **every**
/// legal defender reply admits a chain of depth `k − 1`. We want the
/// largest such `k`, so for each candidate check we take the *minimum*
/// continuation depth across the defender's replies (the defender picks
/// the line that dies out fastest), then take the *maximum* over the
/// attacker's checks (the attacker picks the most persistent check).
fn chain_from_attacker(pos: &mut Position, budget: u8) -> (u8, Option<Move>) {
    if budget == 0 {
        return (0, None);
    }
    let legal = legal_moves_vec(pos);
    let mut best_depth = 0u8;
    let mut best_check: Option<Move> = None;
    let mut checks_seen = 0usize;

    for mv in legal {
        if !pos.gives_check(mv) {
            continue;
        }
        checks_seen += 1;
        if checks_seen > MAX_CHECKS_PER_PLY {
            break;
        }
        // After this check it's the defender's turn. The chain continues
        // to depth `1 + min over replies of (their continuation depth)`.
        let saved = pos.do_move(mv);
        let cont = min_continuation_over_replies(pos, budget - 1);
        pos.undo_move(mv, saved);

        let this_depth = 1 + cont;
        if this_depth > best_depth {
            best_depth = this_depth;
            best_check = Some(mv);
            if best_depth >= budget {
                // Can't do better within this budget; stop early.
                break;
            }
        }
    }
    (best_depth, best_check)
}

/// It is the defender's turn in `pos` (they are in check). Return the
/// minimum over all legal defender replies of the attacker's subsequent
/// chain depth. If the defender has **no** legal reply the position is
/// checkmate — a *terminal* node with no further checks, so the
/// continuation depth is `0`. (A mate-in-1 must therefore report as a
/// depth-1 chain, not a saturated one: a single mating check is fully
/// human-findable as "mate," not a "long forcing sequence" — the signal
/// this detector exists to flag is the multi-check king hunt where the
/// checks *keep coming*, distinct from a one-move mate.)
fn min_continuation_over_replies(pos: &mut Position, budget: u8) -> u8 {
    let replies = legal_moves_vec(pos);
    if replies.is_empty() {
        return 0;
    }
    let mut worst = budget; // start high; take the min the defender can force.
    for reply in replies {
        let saved = pos.do_move(reply);
        let (cont, _) = chain_from_attacker(pos, budget);
        pos.undo_move(reply, saved);
        if cont < worst {
            worst = cont;
            if worst == 0 {
                // The defender has a reply after which no check exists —
                // the chain dies on this line, so it can't self-replenish
                // past this point regardless of the other replies.
                break;
            }
        }
    }
    worst
}
