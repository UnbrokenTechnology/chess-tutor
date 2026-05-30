//! Two-step forcing-line detection: "after the mover plays a check,
//! and the opponent makes their only legal reply, does the mover have
//! another high-confidence tactic?"
//!
//! Companion to [`super::latent_threats`] for the
//! [`teaching-positions/double-fork-after-qd8`] case-study mechanism.
//! That position's tactic — `…Nd3+` then `…Nf2` forks two rooks — is
//! invisible to a single-ply detector: the static fork-shape scan
//! looks at `…Nd3+` in isolation and sees only a check, not a fork;
//! the latent-threat scanner looks at standing alignments and finds
//! nothing because Nc5's relationship to f2 is two hops away. The
//! shape only appears when you *play out the check by one ply* and
//! re-run the detector chain on the resulting position.
//!
//! This is the "look one ply past the check" discipline a 1200→1600
//! player can develop — calculation cheap enough to fit in a human
//! head (one check + one reply + look around), but two plies more
//! than what most 1200s actually calculate. Surfacing it from the
//! engine gives the teaching layer a name and a recommended defuse
//! ("play `d4` to displace the c5 knight before the check fires") for
//! eval swings that would otherwise look like silent-sequencing noise.
//!
//! Budget: in real positions, mover has 0–3 checks and opponent has
//! 1–4 legal replies to each, so the inner [`find_best_tactic_in_position`]
//! detector runs `~12` times worst-case. Cheap.

use crate::analysis::tactic_outcome::{detect_line_tactic, PriorMove, TacticHit, TacticPattern};
use crate::movegen::legal_moves_vec;
use crate::position::Position;
use crate::types::{Color, Move};

#[cfg(test)]
#[path = "check_followups_tests.rs"]
mod tests;

/// One check `mover` can play, with `opponent`'s legal replies and
/// the follow-up tactic (if any) `mover` has after each reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckFollowup {
    pub check_move: Move,
    /// All of `opponent`'s legal responses to [`Self::check_move`], in
    /// move-generation order. Typically 1–3 entries for a real check
    /// (kings have few escape squares; the rest are blocks /
    /// captures).
    pub replies: Vec<ReplyFollowup>,
}

/// One opponent reply to a check, plus the tactic `mover` then has
/// (or `None` if the reply defuses cleanly).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplyFollowup {
    pub reply: Move,
    /// `Some` when [`find_best_tactic_in_position`] fires for `mover`
    /// in the position after `check_move` then `reply`. `None` when
    /// the chain finds nothing — the reply has defused the threat for
    /// at least the next ply.
    pub followup: Option<TacticHit>,
}

/// Enumerate `mover`'s checks; for each, enumerate `!mover`'s legal
/// replies; for each reply, run the static tactic-detector chain on
/// the resulting position from `mover`'s POV. Report only checks that
/// have **at least one** reply with a follow-up tactic — a check
/// every reply defuses isn't pedagogically interesting at this layer.
///
/// `mover` may be either side. When `mover != pos.side_to_move()`, we
/// null-pivot internally so the move generator yields `mover`'s moves;
/// if the position is currently in check, the null pivot is unsound
/// and we return an empty list (the opponent's standing-threat scan
/// doesn't apply when they're already obligated to address a check).
///
/// `prior_move` is currently unused (the follow-up detection runs on a
/// position two plies past the user-supplied frame), but kept in the
/// signature so callers passing it to [`find_best_tactic_in_position`]
/// can pass it here uniformly.
pub fn find_check_followups(
    pos: &Position,
    mover: Color,
    prior_move: Option<PriorMove>,
) -> Vec<CheckFollowup> {
    let _ = prior_move;
    let mut scratch = pos.clone();
    let mut null_saved = None;
    if scratch.side_to_move() != mover {
        if scratch.in_check() {
            return Vec::new();
        }
        null_saved = Some(scratch.do_null_move());
    }

    let mover_legal = legal_moves_vec(&mut scratch);
    let mut out = Vec::new();
    for check_mv in mover_legal {
        if !scratch.gives_check(check_mv) {
            continue;
        }
        let saved = scratch.do_move(check_mv);
        let opp_legal = legal_moves_vec(&mut scratch);
        let mut replies: Vec<ReplyFollowup> = Vec::with_capacity(opp_legal.len());
        let mut any_followup = false;
        for reply_mv in opp_legal {
            let r_saved = scratch.do_move(reply_mv);
            // After opponent's reply, it's `mover`'s turn again. We
            // can't reuse [`find_best_tactic_in_position`] directly —
            // it filters to `Confidence::High`, which gates on
            // material realised inside the analysed line. Our line is
            // 1 ply (the follow-up move), so the case-study fork
            // (`Nf2+`, materialises +rook on ply 3) registers as
            // Medium and gets dropped. The *shape* is what we want at
            // this layer; the teaching surface above can decide what
            // to do with a Medium-confidence followup.
            let followup = find_followup_tactic(&scratch, mover);
            if followup.is_some() {
                any_followup = true;
            }
            replies.push(ReplyFollowup {
                reply: reply_mv,
                followup,
            });
            scratch.undo_move(reply_mv, r_saved);
        }
        scratch.undo_move(check_mv, saved);
        if any_followup {
            out.push(CheckFollowup {
                check_move: check_mv,
                replies,
            });
        }
    }

    if let Some(saved) = null_saved {
        scratch.undo_null_move(saved);
    }
    out
}

/// Confidence-relaxed equivalent of
/// [`crate::analysis::find_best_tactic_in_position`] — enumerates every
/// legal move and runs the per-pattern detector chain on each 1-ply
/// line, picking the most instructive hit (mate > pattern severity).
/// Accepts both `High` and `Medium` confidence; see the call site in
/// [`find_check_followups`] for why the standard High-only gate is the
/// wrong fit here.
fn find_followup_tactic(pos: &Position, mover: Color) -> Option<TacticHit> {
    let mut scratch = pos.clone();
    let legal = legal_moves_vec(&mut scratch);
    let mut best: Option<TacticHit> = None;
    for m in legal {
        let line = [m];
        if let Some(hit) = detect_line_tactic(pos, &line, mover, 0, None) {
            best = match best {
                None => Some(hit),
                Some(prev) => Some(if hit_outranks(&hit, &prev) { hit } else { prev }),
            };
        }
    }
    best
}

/// Tie-breaker for [`find_followup_tactic`]: mate trumps non-mate,
/// then pattern severity (Fork beats DiscoveredCheck beats … etc.).
/// Material-gain comparison is intentionally dropped — at this layer
/// the 1-ply line undercounts the realised gain of the follow-up
/// (the case-study `Nf2+` wins a rook only at ply 3), so ranking on
/// gain would push genuine forks below their material proxies.
fn hit_outranks(a: &TacticHit, b: &TacticHit) -> bool {
    let a_mate = a.pattern == TacticPattern::Checkmate;
    let b_mate = b.pattern == TacticPattern::Checkmate;
    if a_mate != b_mate {
        return a_mate;
    }
    pattern_rank(a.pattern) < pattern_rank(b.pattern)
}

fn pattern_rank(p: TacticPattern) -> u8 {
    match p {
        TacticPattern::Checkmate => 0,
        TacticPattern::Fork => 1,
        TacticPattern::RemovingDefender => 2,
        TacticPattern::HangingCapture => 3,
        TacticPattern::TrappedPiece => 4,
        TacticPattern::DoubleCheck => 5,
        TacticPattern::DiscoveredCheck => 6,
        TacticPattern::Skewer => 7,
        TacticPattern::DiscoveredAttack => 8,
        TacticPattern::RelativePin => 9,
        TacticPattern::Pin => 10,
        TacticPattern::Intermezzo => 11,
        TacticPattern::Deflection => 12,
        TacticPattern::Attraction => 13,
        TacticPattern::Interference => 14,
        TacticPattern::Clearance => 15,
        TacticPattern::XRay => 16,
        TacticPattern::AttackingF2F7 => 17,
        TacticPattern::UnderPromotion => 18,
        TacticPattern::Sacrifice => 19,
    }
}
