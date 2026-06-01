//! Sibling tests for [`super`] (`forcing_check_chain.rs`).

use super::*;
use crate::position::Position;
use crate::types::Color;

#[test]
fn startpos_has_no_forcing_check_chain() {
    let pos = Position::startpos();
    // Neither side has any check at all from the opening position.
    assert_eq!(forcing_check_chain(&pos, Color::White).depth, 0);
    assert_eq!(forcing_check_chain(&pos, Color::Black).depth, 0);
    assert!(forcing_check_chain(&pos, Color::White).first_check.is_none());
}

#[test]
fn lone_kings_have_no_chain() {
    let pos = Position::from_fen("4k3/8/8/8/8/8/8/4K3 w - - 0 1").unwrap();
    assert_eq!(forcing_check_chain(&pos, Color::White).depth, 0);
    assert_eq!(forcing_check_chain(&pos, Color::Black).depth, 0);
}

#[test]
fn skips_cleanly_when_defender_is_stm_and_in_check() {
    // White king in check from the black queen on e8: pivoting to give
    // Black (the attacker) a free tempo would be unsound, so the scan
    // bails with depth 0 rather than producing bogus output.
    let pos = Position::from_fen("4q2k/8/8/8/8/8/8/4K3 w - - 0 1").unwrap();
    assert!(pos.in_check());
    let chain = forcing_check_chain(&pos, Color::White);
    assert_eq!(chain.depth, 0);
    assert!(chain.first_check.is_none());
}

#[test]
fn case_study_mating_net_after_ng5() {
    // Position AFTER White plays the blundering `Ng5` (Black to move).
    // Per `mating-net-after-ng5.md`, Black has a forced mating sequence
    // of checks against the white king:
    //   …Qa3+ Kd2 …Qxc3+ Kc1 …Qa3+ Kb1 …Bxa2+ Ka1 [mate]
    // i.e. a self-replenishing forcing-check chain at least three deep
    // against White's king. The detector must report depth >= the gate
    // threshold so the soft king-hunt warning fires.
    let pos =
        Position::from_fen("5rk1/ppp1qp2/4b1nQ/4p1N1/3p4/2P5/P1P3PP/2KR1B1R b - - 1 1").unwrap();

    // Black (the attacker) hunts the WHITE king, so we ask about
    // `defender = White`.
    let chain = forcing_check_chain(&pos, Color::White);
    assert!(
        chain.depth >= 3,
        "expected a forcing-check chain >= 3 deep at the white king; got {chain:?}"
    );
    assert!(
        chain.first_check.is_some(),
        "a non-zero chain must name the first check"
    );

    // Symmetrically, the BLACK king is not under a comparable forcing
    // sequence here (White's checks die out — every white check has a
    // black reply that leaves no further check). Black's king should
    // report a shallower chain than White's.
    let black_chain = forcing_check_chain(&pos, Color::Black);
    assert!(
        black_chain.depth < chain.depth,
        "black king should face a shorter chain than the hunted white king; \
         black={black_chain:?} white={chain:?}"
    );
}

#[test]
fn reported_depth_never_exceeds_cap() {
    let pos =
        Position::from_fen("5rk1/ppp1qp2/4b1nQ/4p1N1/3p4/2P5/P1P3PP/2KR1B1R b - - 1 1").unwrap();
    let chain = forcing_check_chain(&pos, Color::White);
    assert!(chain.depth <= MAX_CHAIN_DEPTH);
}
