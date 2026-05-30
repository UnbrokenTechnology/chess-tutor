//! Sibling tests for [`super`] (`square_view.rs`). The key regression
//! target is the discovered-attack case-study FEN
//! ([`teaching-positions/discovered-attack-after-qxe6.md`](teaching-positions/discovered-attack-after-qxe6.md)):
//! querying square `e5` must surface
//!
//!   - the **black bishop occupant** with attackers from white,
//!   - the **e-file alignment** as a discovered-attack vehicle
//!     (Qe6 / Be5 / Re1).
//!
//! That's exactly what the agent kept reconstructing by hand and
//! getting wrong.

use super::*;
use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::Square;

const CASE_STUDY_FEN: &str = "1r4nr/p3k3/4qpp1/4b2p/2Q5/8/PPPP1PPP/R1B1R1K1 w - - 0 1";

#[test]
fn case_study_e5_reports_black_bishop_occupant() {
    let pos = Position::from_fen(CASE_STUDY_FEN).unwrap();
    let view = build(&pos, Square::E5);

    let occ = view.occupant.as_ref().unwrap();
    assert_eq!(occ.label, "be5");
    assert_eq!(occ.color, "black");
    assert_eq!(occ.piece, "bishop");
    assert_eq!(occ.classical_points, 3);
}

#[test]
fn case_study_e5_lists_defenders() {
    // Black bishop on e5 is defended by the queen on e6 and the pawn
    // on f6. Both must surface.
    let pos = Position::from_fen(CASE_STUDY_FEN).unwrap();
    let view = build(&pos, Square::E5);

    let defs = view.defenders.as_ref().unwrap();
    let labels: Vec<&str> = defs.iter().map(|d| d.label.as_str()).collect();
    assert!(labels.contains(&"qe6"), "qe6 missing: {labels:?}");
    assert!(labels.contains(&"pf6"), "pf6 missing: {labels:?}");
}

#[test]
fn case_study_e5_surfaces_discovered_attack_alignment() {
    // The critical case-study regression target. With the black bishop
    // on e5, the BLACK queen on e6 has a standing discovered attack on
    // the WHITE rook on e1 along the e-file. Moving the bishop with a
    // forcing move (...Bxh2+) fires the discovery.
    let pos = Position::from_fen(CASE_STUDY_FEN).unwrap();
    let view = build(&pos, Square::E5);

    assert!(
        !view.discovered_attacks_when_moved.is_empty(),
        "expected at least one discovery; got none. view: {view:#?}",
    );
    let d = view
        .discovered_attacks_when_moved
        .iter()
        .find(|d| d.target.square == "e1")
        .unwrap_or_else(|| {
            panic!("no discovery targeting e1 found: {view:#?}");
        });
    assert_eq!(d.discoverer.label, "qe6");
    assert_eq!(d.target.label, "Re1");
    assert_eq!(d.ray, "e-file");
}

#[test]
fn case_study_text_render_names_alignment_explicitly() {
    let pos = Position::from_fen(CASE_STUDY_FEN).unwrap();
    let view = build(&pos, Square::E5);
    let text = render_text(&view);

    assert!(text.contains("e5"));
    assert!(text.contains("black bishop"));
    assert!(text.contains("be5"));
    assert!(text.contains("discovery vehicle for"));
    assert!(text.contains("qe6"));
    assert!(text.contains("Re1"));
    assert!(text.contains("e-file"));
}

#[test]
fn startpos_e4_is_empty_with_no_attackers_or_defenders() {
    let pos = Position::startpos();
    let view = build(&pos, Square::E4);
    assert!(view.occupant.is_none());
    assert!(view.defenders.is_none());
    assert!(view.attackers.is_empty());
    assert!(view.pin.is_none());
    let text = render_text(&view);
    assert!(text.contains("empty square"));
    assert!(text.contains("no attackers"));
}

#[test]
fn startpos_f3_is_attackable_by_three_white_pieces() {
    // f3 is attacked by the e2 pawn, g2 pawn, and g1 knight from the
    // starting position. None black, none defenders (empty square).
    let pos = Position::startpos();
    let view = build(&pos, Square::F3);
    assert_eq!(
        view.attackers.len(),
        3,
        "f3 has 3 attackers in startpos: got {view:#?}",
    );
    let labels: Vec<&str> = view.attackers.iter().map(|a| a.label.as_str()).collect();
    assert!(labels.contains(&"Pe2"));
    assert!(labels.contains(&"Pg2"));
    assert!(labels.contains(&"Ng1"));
}

#[test]
fn absolute_pin_detected_against_king() {
    // Black rook e8 pins white knight e2 against white king e1.
    let pos = Position::from_fen("4rk2/8/8/8/8/8/4N3/4K3 w - - 0 1").unwrap();
    let view = build(&pos, Square::E2);
    let pin = view.pin.as_ref().expect("e2 knight should be pinned");
    assert_eq!(pin.kind, "absolute");
    assert_eq!(pin.pinner.label, "re8");
    assert_eq!(pin.pinned_to, "Ke1");
    assert_eq!(pin.ray, "e-file");
}

#[test]
fn see_verdict_marks_undefended_capture_as_win() {
    // Black queen on e7 with no defenders; white queen on e2 attacks.
    // Kings parked off the e-file so the BK doesn't defend e7.
    let pos = Position::from_fen("k7/4q3/8/8/8/8/4Q3/K7 w - - 0 1").unwrap();
    let view = build(&pos, Square::E7);
    let see = view.see_for_cheapest_capture.as_ref().unwrap();
    assert_eq!(see.cheapest_attacker.label, "Qe2");
    assert_eq!(see.verdict, "wins material");
}
