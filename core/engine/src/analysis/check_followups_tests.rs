//! Sibling tests for [`super`] (`check_followups.rs`). The Phase E
//! regression target — the `double-fork-after-qd8` FEN — lives here:
//! Black has a `…Nd3+` (check) followed by `…Nf2` (Fork) sequence
//! that's invisible to the single-ply detector but light up cleanly
//! once we play the check forward by one ply.

use super::*;
use crate::analysis::tactic_outcome::TacticPattern;
use crate::position::Position;
use crate::types::{Color, Square};

#[test]
fn startpos_has_no_check_followups() {
    let pos = Position::startpos();
    assert!(find_check_followups(&pos, Color::White, None).is_empty());
    assert!(find_check_followups(&pos, Color::Black, None).is_empty());
}

#[test]
fn case_study_double_fork_after_qd8() {
    // Phase E done-criterion: on the pre-castle FEN, Black has a
    // standing `…Nd3+ → …Nf2` Fork sequence. Black is the side to
    // move; we ask for `mover = Black` directly (no null pivot).
    let pos =
        Position::from_fen("r1b1kbnr/pp2qp1p/2p3p1/2n1p3/2P1P3/1P3P2/PBQPN1PP/R3KBNR b KQkq - 0 10")
            .unwrap();
    let followups = find_check_followups(&pos, Color::Black, None);

    // At least one of Black's checks must be Nc5xd3+ (n c5 → d3 is the
    // killer one). Other checks are unlikely on this board but the
    // assertion is structural: there must be a CheckFollowup whose
    // check_move lands on d3 and at least one reply leads to a Fork.
    let nd3 = followups
        .iter()
        .find(|cf| cf.check_move.from() == Square::C5 && cf.check_move.to() == Square::D3)
        .unwrap_or_else(|| panic!("expected Nc5-d3+ check followup; got {followups:#?}"));

    // At least one reply (the Kd1 forced king move) must have a
    // followup with TacticPattern::Fork — Nf2 forks Kd1 + Rh1
    // pre-castle. The detector at this layer doesn't guarantee Fork
    // specifically (could be DiscoveredCheck, DoubleCheck, etc.) but
    // for this position the documented mechanism is Fork.
    let fork_reply = nd3
        .replies
        .iter()
        .find(|r| {
            r.followup
                .as_ref()
                .is_some_and(|h| h.pattern == TacticPattern::Fork)
        })
        .unwrap_or_else(|| panic!("expected at least one reply leading to a Fork followup; got {:#?}", nd3.replies));

    // The follow-up's primary square must be f2 — that's the key
    // weak square the Nf2 hop lands on, per the case-study writeup.
    let hit = fork_reply.followup.as_ref().unwrap();
    assert_eq!(
        hit.primary_piece,
        Square::F2,
        "Nf2 should land on f2; got {hit:#?}",
    );
}

#[test]
fn null_pivot_skipped_when_side_to_move_is_in_check() {
    // White king in check from black queen on e8 — null-pivot to
    // ask "what checks could Black play if granted a free tempo"
    // is unsound. find_check_followups must return empty rather
    // than crash or produce bogus output.
    let pos = Position::from_fen("4q2k/8/8/8/8/8/8/4K3 w - - 0 1").unwrap();
    assert!(pos.in_check());
    let followups = find_check_followups(&pos, Color::Black, None);
    assert!(
        followups.is_empty(),
        "must skip cleanly when stm is in check; got {followups:#?}"
    );
}

#[test]
fn null_pivot_for_side_not_to_move_works_when_clean() {
    // Same double-fork FEN but ask for `mover = White` — White is
    // the side NOT to move on this FEN. We expect either an empty
    // result (no White check leads to a fork) or whatever the
    // engine finds; the assertion here is just that the call
    // doesn't panic and returns something sensible.
    let pos =
        Position::from_fen("r1b1kbnr/pp2qp1p/2p3p1/2n1p3/2P1P3/1P3P2/PBQPN1PP/R3KBNR b KQkq - 0 10")
            .unwrap();
    let _ = find_check_followups(&pos, Color::White, None);
}
