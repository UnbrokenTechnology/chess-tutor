//! Sibling tests for [`super`] (`threats_view.rs`). Covers each
//! category independently plus the "everything quiet" case so an
//! agent-facing regression in the unified rollup is caught.

use super::*;
use chess_tutor_engine::position::Position;

#[test]
fn startpos_has_no_threats_for_either_side() {
    let pos = Position::startpos();
    let view = build(&pos);
    assert!(view.white.hanging.is_empty());
    assert!(view.white.see_losing.is_empty());
    assert!(view.white.pinned.is_empty());
    assert!(view.white.overloaded.is_empty());
    assert!(view.white.trapped.is_empty());
    assert!(view.black.hanging.is_empty());
    let text = render_text(&view);
    assert!(text.contains("white: (no threats found)"));
    assert!(text.contains("black: (no threats found)"));
}

#[test]
fn hanging_piece_surfaces_with_attackers() {
    // Black rook on h8 attacked by white queen on h1; no defenders.
    // Black king parked on a1-side so the position is legal but the
    // BK isn't a defender of h8.
    let pos = Position::from_fen("k6r/8/8/8/8/8/8/4K2Q w - - 0 1").unwrap();
    let view = build(&pos);
    assert_eq!(view.black.hanging.len(), 1, "{view:#?}");
    let h = &view.black.hanging[0];
    assert_eq!(h.piece, "rh8");
    assert_eq!(h.classical_points, 5);
    assert!(h.attackers.iter().any(|a| a == "Qh1"));
}

#[test]
fn absolute_pin_surfaces_in_pinned_list() {
    // Black rook e8 pins white knight e2 against white king e1.
    let pos = Position::from_fen("4rk2/8/8/8/8/8/4N3/4K3 w - - 0 1").unwrap();
    let view = build(&pos);
    assert_eq!(view.white.pinned.len(), 1, "{view:#?}");
    let p = &view.white.pinned[0];
    assert_eq!(p.piece, "Ne2");
    assert_eq!(p.kind, "absolute");
    assert_eq!(p.pinner, "re8");
    assert_eq!(p.pinned_to, "Ke1");
}

#[test]
fn case_study_e5_bishop_appears_as_pinned_for_black() {
    // The discovered-attack case study: Be5 is relatively-pinned by
    // Re1 against qe6 (moving the bishop unblocks the rook). The pin
    // is from BLACK's POV (the pinned piece is black).
    let pos =
        Position::from_fen("1r4nr/p3k3/4qpp1/4b2p/2Q5/8/PPPP1PPP/R1B1R1K1 w - - 0 1").unwrap();
    let view = build(&pos);
    let be5 = view
        .black
        .pinned
        .iter()
        .find(|p| p.piece == "be5")
        .unwrap_or_else(|| panic!("Be5 not in black.pinned: {view:#?}"));
    assert_eq!(be5.kind, "relative");
    assert_eq!(be5.pinner, "Re1");
    assert_eq!(be5.pinned_to, "qe6");
}

#[test]
fn json_round_trips_threats() {
    let pos = Position::startpos();
    let view = build(&pos);
    let json = serde_json::to_string(&view).unwrap();
    assert!(json.contains("\"white\""));
    assert!(json.contains("\"black\""));
    assert!(json.contains("\"hanging\":[]"));
    assert!(json.contains("\"pinned\":[]"));
}
