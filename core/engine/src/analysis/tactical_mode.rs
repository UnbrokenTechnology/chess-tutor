//! The tactical-mode gate — the shared spine every teaching surface
//! consults to decide "is this position tactically live, and if so, why?"
//!
//! See `PLAN-teaching-gui.md` §1 and "The core principle." The decision
//! the user signed off on, and the load-bearing reason this module exists:
//!
//! > The gate is **detectors-only.** A position is tactically live for
//! > teaching purposes iff a *named, human-findable* pattern fires —
//! > in-check, an opponent's loaded threat, an opponent's check-followup,
//! > a self-replenishing check chain at our king, a tactic we can play,
//! > or a loose piece. We deliberately **do not** use a
//! > static-vs-quiescence eval delta as a gate.
//!
//! The rationale is teaching-honesty, not implementation convenience: if
//! the only signal is "search disagrees with static eval," a human could
//! never have *seen* the position was tactical — only an engine notices,
//! so there is nothing teachable there. This single decision unifies the
//! design (it defines the gate every renderer reads) and **subsumes
//! silent-sequencing for free**: the `…Qc8` case fires no detector, so it
//! reads as quiet, so no tactic card appears — exactly what the
//! `silent-sequencing-after-qc8` case study demands, with no separate
//! suppressor for the coaching surface.
//!
//! ## What this module is and isn't
//!
//! It is **pure composition** of the existing analytical detectors. It
//! runs **no search** — every scan it calls is a static, sub-ms
//! bitboard/SEE pass, so the whole gate is cheap enough to run every
//! frame alongside the rest of coaching. It allocates exactly one `Vec`
//! (the `reasons`) per call; no per-node heap traffic.
//!
//! It does **not** decide *what to render* — it only reports the ordered
//! list of nameable causes. The three UI renderers (coached / supported /
//! practicing) consume [`TacticalState::reasons`] in vec order; the order
//! here (see [`classify_tactical_mode`]) is the card-priority order from
//! PLAN §2.

use crate::analysis::check_followups::{find_check_followups, CheckFollowup};
use crate::analysis::forcing_check_chain::forcing_check_chain;
use crate::analysis::latent_threats::{find_latent_threats, LatentThreat};
use crate::analysis::tactic_outcome::{find_best_tactic_in_position, PriorMove, TacticHit};
use crate::analysis::threats_outcome::{list_hanging, list_see_losing};
use crate::position::Position;
use crate::types::{Color, Square};

#[cfg(test)]
#[path = "tactical_mode_tests.rs"]
mod tests;

/// The minimum self-replenishing forcing-check depth (in attacker
/// checks) at which we surface a [`TacticalReason::ForcingCheckChain`].
/// The `mating-net-after-ng5` case study's user-articulated rule is
/// "three checks deep → stop and reconsider." See PLAN §7 (open
/// question: confirm 3 in real play; may want tuning per king exposure).
pub const FORCING_CHECK_CHAIN_MIN_DEPTH: u8 = 3;

/// A single nameable reason a position is tactically live. Card builders
/// render these directly; see [`TacticalState`] for the ordering
/// contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TacticalReason {
    /// The user is in check — they must respond now. Highest priority.
    InCheck,
    /// The opponent has a tactic *loaded* against the user (a discovered
    /// attack / pin / skewer / removing-the-defender they fire if the
    /// user's move doesn't address it). From
    /// [`find_latent_threats`]`(pos, user_color)`.
    OpponentLatentThreat(LatentThreat),
    /// The opponent has a check whose forced reply leaves them a
    /// follow-up tactic (a two-step fork — the `double-fork-after-qd8`
    /// mechanism). From
    /// [`find_check_followups`]`(pos, !user_color, prior)`.
    OpponentCheckFollowup(CheckFollowup),
    /// The opponent has a forcing-check sequence at the user's king that
    /// self-replenishes at least [`FORCING_CHECK_CHAIN_MIN_DEPTH`] deep
    /// (the `mating-net-after-ng5` soft-warning signal). Carries only the
    /// depth — the UI must stay **mechanism-free** here and never name a
    /// mate or a line.
    ForcingCheckChain { depth: u8 },
    /// The user has a combination available now. From
    /// [`find_best_tactic_in_position`]`(pos, user_color, prior)`.
    OurTactic(TacticHit),
    /// A piece is hanging or SEE-losing — either side. Lowest priority.
    LoosePiece {
        /// The colour of the loose piece (the side that stands to lose
        /// it). `side == user_color` is a risk to the user; the opposite
        /// is an opportunity for the user.
        side: Color,
        /// The square the loose piece sits on.
        square: Square,
    },
}

/// The result of the tactical-mode gate: whether the position is live and
/// the ordered list of nameable causes.
///
/// Ordering contract: [`reasons`](Self::reasons) is sorted by the
/// card-priority order in PLAN §2 — `InCheck` first, then
/// `OpponentLatentThreat`, `OpponentCheckFollowup`, `ForcingCheckChain`,
/// `OurTactic`, and `LoosePiece` last. UI surfaces render in vec order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TacticalState {
    /// `true` iff [`reasons`](Self::reasons) is non-empty.
    pub live: bool,
    /// The nameable causes, highest-priority first.
    pub reasons: Vec<TacticalReason>,
}

/// Classify whether `pos` is tactically live for `user_color` and why.
///
/// `user_color` is the side the student plays. It is used both as the
/// live side-to-move framing (coaching is always the user's turn) and as
/// the `defender_color` for the opponent-threat scans. `prior_move` feeds
/// the recapture guard in [`find_best_tactic_in_position`] /
/// [`find_check_followups`] — pass `None` when there is no move history.
///
/// Detectors-only: **no quiescence delta, no search.** Cost is the sum of
/// the static scans (all sub-ms in release). Pure with respect to `pos`.
///
/// `live == !reasons.is_empty()`. Reasons are returned in the
/// card-priority order documented on [`TacticalState`].
pub fn classify_tactical_mode(
    pos: &Position,
    user_color: Color,
    prior_move: Option<PriorMove>,
) -> TacticalState {
    let mut reasons: Vec<TacticalReason> = Vec::new();

    // 1. InCheck — the user must respond right now. Only meaningful when
    //    the user is the side to move (coaching is always the user's
    //    turn); we report it whenever the position is in check and the
    //    side to move is the user.
    if pos.side_to_move() == user_color && pos.in_check() {
        reasons.push(TacticalReason::InCheck);
    }

    // 2. OpponentLatentThreat — the opponent's loaded tactics against us.
    for threat in find_latent_threats(pos, user_color) {
        reasons.push(TacticalReason::OpponentLatentThreat(threat));
    }

    // 3. OpponentCheckFollowup — a check by the opponent that sets up a
    //    follow-up tactic one ply past our forced reply.
    for cf in find_check_followups(pos, !user_color, prior_move) {
        reasons.push(TacticalReason::OpponentCheckFollowup(cf));
    }

    // 4. ForcingCheckChain — a self-replenishing forcing-check sequence
    //    at the user's king, reported only at/above the minimum depth.
    let chain = forcing_check_chain(pos, user_color);
    if chain.depth >= FORCING_CHECK_CHAIN_MIN_DEPTH {
        reasons.push(TacticalReason::ForcingCheckChain { depth: chain.depth });
    }

    // 5. OurTactic — a high-confidence combination the user can play now.
    if let Some(hit) = find_best_tactic_in_position(pos, user_color, prior_move) {
        reasons.push(TacticalReason::OurTactic(hit));
    }

    // 6. LoosePiece — hanging / SEE-losing pieces on either side, lowest
    //    priority. Scanned for both colours; deduplicated per square so a
    //    piece that is both hanging and SEE-losing reports once. The
    //    user's own loose pieces (a risk) are listed before the
    //    opponent's (an opportunity), matching the "address your own
    //    danger first" reading discipline.
    push_loose_pieces(pos, user_color, &mut reasons);
    push_loose_pieces(pos, !user_color, &mut reasons);

    TacticalState {
        live: !reasons.is_empty(),
        reasons,
    }
}

/// Append a [`TacticalReason::LoosePiece`] for each distinct square of
/// `side` that is hanging or SEE-losing, in ascending square order with
/// no duplicate squares.
fn push_loose_pieces(pos: &Position, side: Color, reasons: &mut Vec<TacticalReason>) {
    let mut squares: Vec<Square> = list_hanging(pos, side)
        .into_iter()
        .map(|h| h.location.square)
        .chain(list_see_losing(pos, side).into_iter().map(|h| h.location.square))
        .collect();
    squares.sort_unstable_by_key(|s| s.index());
    squares.dedup();
    for square in squares {
        reasons.push(TacticalReason::LoosePiece { side, square });
    }
}
