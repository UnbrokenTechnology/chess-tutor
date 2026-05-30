//! Sibling tests for [`super`] (`units.rs`) — see the parent module's
//! `//!` for the scale + POV background.

use super::*;
use chess_tutor_engine::types::{Color, Value};

#[test]
fn pawn_eg_213_maps_to_one_pawn() {
    // Locked-down: an engine score of one PAWN_EG (213 engine-cp)
    // renders as `+1.00 pawns` (matching chess.com / UCI), and the
    // conventional-cp helper rounds to +100 — the chess.com cp scale.
    // `format_engine_cp` returns the raw engine number unchanged so the
    // value the CLI displays as `engine-cp: +213 stm` connects directly
    // to `Value::PAWN_EG` in the engine source.
    let v = Value(213);
    assert_eq!(format_pawns(v), "+1.00");
    assert_eq!(format_conventional_cp(v), "+100");
    assert_eq!(format_engine_cp(v), "+213");
}

#[test]
fn small_swings_format_with_two_decimals() {
    // A 50-engine-cp swing is ~0.23 pawns, not 0.50. Conv-cp matches
    // the pawns reading; engine-cp keeps the raw 50.
    assert_eq!(format_pawns(Value(50)), "+0.23");
    assert_eq!(format_pawns(Value(-50)), "-0.23");
    assert_eq!(format_conventional_cp(Value(50)), "+23");
    assert_eq!(format_engine_cp(Value(50)), "+50");
    assert_eq!(format_engine_cp(Value(-50)), "-50");
}

#[test]
fn engine_cp_and_conv_cp_are_different_scales() {
    // The whole point of this issue: engine-cp ≠ conv-cp. A pawn is
    // 213 engine-cp and 100 conv-cp. CLI output that mixed them was
    // ambiguous; this test pins the distinction so neither helper
    // silently drifts to the other scale.
    let v = Value(213);
    assert_ne!(format_engine_cp(v), format_conventional_cp(v));
    assert_eq!(format_engine_cp(v), "+213");
    assert_eq!(format_conventional_cp(v), "+100");
}

#[test]
fn to_white_pov_flips_only_for_black() {
    let v = Value(150);
    assert_eq!(to_white_pov(v, Color::White), v);
    assert_eq!(to_white_pov(v, Color::Black), Value(-150));
}

#[test]
fn mate_scores_format_as_pound_n() {
    // MATE = 32000. A mate-in-3 (5 plies of mating) is roughly
    // MATE - 5 = 31995.
    let m3 = Value(Value::MATE.0 - 5);
    assert_eq!(format_pawns(m3), "#3");
    assert_eq!(format_conventional_cp(m3), "#3");
    let neg_m3 = Value(-(Value::MATE.0 - 5));
    assert_eq!(format_pawns(neg_m3), "-#3");
}

#[test]
fn headline_triple_carries_three_units() {
    let (pawns, cp, pct) = headline_triple(Value(426));  // ≈ +2 pawns
    assert_eq!(pawns, "+2.00");
    assert_eq!(cp, "+200");
    // ~+2 pawns is solidly winning but not certain: somewhere in the
    // 75–85% band per the lila sigmoid. Just check it's > 50 and < 99.
    assert!(pct > 60 && pct < 99, "pct = {pct}");
}

#[test]
fn headline_triple_dead_equal_is_fifty_percent() {
    let (_, _, pct) = headline_triple(Value(0));
    assert_eq!(pct, 50);
}

#[test]
fn mate_score_win_pct_saturates_at_ninety_nine() {
    let m = Value(Value::MATE.0 - 3);
    let (_, _, pct) = headline_triple(m);
    assert_eq!(pct, 99);
}
