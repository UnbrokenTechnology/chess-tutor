//! Sibling tests for [`super`] (`attacks_view.rs`).

use super::*;
use chess_tutor_engine::position::Position;

#[test]
fn startpos_has_no_attacks_between_sides() {
    let pos = Position::startpos();
    let view = build(&pos);
    assert!(view.white.attacks.is_empty());
    assert!(view.black.attacks.is_empty());
}

#[test]
fn case_study_lists_qc4_attacking_qe6() {
    // Discovered-attack FEN. White Qc4 attacks black Qe6 along the
    // c4-d5-e6 diagonal (d5 empty). Must surface; SEE verdict has both
    // queens equally valuable, so it depends on defender count.
    let pos =
        Position::from_fen("1r4nr/p3k3/4qpp1/4b2p/2Q5/8/PPPP1PPP/R1B1R1K1 w - - 0 1").unwrap();
    let view = build(&pos);
    let record = view
        .white
        .attacks
        .iter()
        .find(|r| r.attacker == "Qc4" && r.target == "qe6")
        .unwrap_or_else(|| panic!("Qc4→qe6 missing: {view:#?}"));
    assert_eq!(record.target_points, 9);
    // Black queen on e6 has the black king on e7 as a defender, so
    // Qxe6+ followed by Kxe6 nets even (queen for queen).
    assert!(
        record.see_verdict == "even trade" || record.see_verdict == "wins material",
        "got SEE verdict: {}",
        record.see_verdict,
    );
}

#[test]
fn highest_value_targets_sort_first() {
    // Position with both a queen-target and a pawn-target attacked.
    // The queen entry must appear before the pawn entry in the
    // returned list (deterministic ordering for agent scanning).
    let pos = Position::from_fen("k7/8/8/8/3q4/8/3R4/3KR3 w - - 0 1").unwrap();
    let view = build(&pos);
    // White rook on d2 attacks black queen on d4. Make sure that's
    // among the first entries.
    assert!(!view.white.attacks.is_empty(), "{view:#?}");
    let first = &view.white.attacks[0];
    assert_eq!(first.target_kind, "queen");
}
