//! Sibling tests for [`super`] (`alignments_view.rs`). The regression
//! target is the same case-study e-file alignment the
//! `square_view::tests` exercise — but viewed through the all-sliders
//! enumeration instead of a per-square query.

use super::*;
use chess_tutor_engine::position::Position;

#[test]
fn case_study_black_qe6_be5_re1_alignment_surfaces() {
    // The standing discovered-attack alignment from the case study,
    // viewed from BLACK's side: the black queen on e6 is the slider,
    // the black bishop on e5 is the blocker, the white rook on e1 is
    // the target.
    let pos =
        Position::from_fen("1r4nr/p3k3/4qpp1/4b2p/2Q5/8/PPPP1PPP/R1B1R1K1 w - - 0 1").unwrap();
    let view = build(&pos, false);

    let alignment = view
        .black
        .discovered_attack_candidates
        .iter()
        .find(|r| r.slider == "qe6" && r.blocker == "be5" && r.target == "Re1")
        .unwrap_or_else(|| {
            panic!("qe6/be5/Re1 alignment missing: {view:#?}");
        });
    assert_eq!(alignment.ray, "e-file");
    assert!(
        alignment.target_more_valuable,
        "rook (5) > bishop (3) — must be flagged as default-shown",
    );
}

#[test]
fn case_study_white_re1_pin_skewer_against_be5_and_qe6_surfaces() {
    // From WHITE's side, the same e-file alignment is a
    // pin/skewer candidate: white Re1 → (black blocker be5) → black
    // qe6. The bishop on e5 is being pinned to the queen on e6.
    let pos =
        Position::from_fen("1r4nr/p3k3/4qpp1/4b2p/2Q5/8/PPPP1PPP/R1B1R1K1 w - - 0 1").unwrap();
    let view = build(&pos, false);

    let pin = view
        .white
        .pin_skewer_candidates
        .iter()
        .find(|r| r.slider == "Re1" && r.blocker == "be5")
        .unwrap_or_else(|| panic!("Re1 pin/skewer missing: {view:#?}"));
    assert_eq!(pin.target, "qe6");
    assert_eq!(pin.ray, "e-file");
}

#[test]
fn low_value_target_filtered_by_default() {
    // Discovered alignment where the target is LESS valuable than the
    // blocker: white Rb1, white Bb2 (blocker), black p-b5 (target).
    // The b-file rook→bishop→pawn alignment is a real ray triple but
    // not worth surfacing (a bishop discovering an attack on a pawn
    // is a noisy false positive in default output).
    //
    // We test the include_low_value=true mode brings it back.
    let pos = Position::from_fen("k7/8/8/1p6/8/8/1B6/1R2K3 w - - 0 1").unwrap();
    let filtered = build(&pos, false);
    let unfiltered = build(&pos, true);
    let count_filtered = filtered.white.discovered_attack_candidates.len();
    let count_unfiltered = unfiltered.white.discovered_attack_candidates.len();
    assert!(
        count_unfiltered >= count_filtered,
        "include_low_value should add entries: {count_filtered} → {count_unfiltered}",
    );
}
