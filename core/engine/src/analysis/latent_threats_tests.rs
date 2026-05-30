//! Sibling tests for [`super`] (`latent_threats.rs`). The case-study
//! regression targets — Phase D done-criteria — live here:
//! `case_study_qxe6_finds_standing_discovered_attack` for the
//! `Qe6 / Be5 / Re1` alignment (PLAN.md done #2 verbatim);
//! `case_study_desperado_finds_standing_removing_defender` for the
//! `Nf6 attacks Pe4, sole defender of Nf5` shape (PLAN.md done #1's
//! pre-move framing).

use super::*;
use crate::position::Position;

fn classify(threats: &[LatentThreat], pattern: TacticPattern) -> Vec<&LatentThreat> {
    threats.iter().filter(|t| t.pattern == pattern).collect()
}

#[test]
fn startpos_has_no_latent_threats_for_either_side() {
    let pos = Position::startpos();
    assert!(find_latent_threats(&pos, Color::White).is_empty());
    assert!(find_latent_threats(&pos, Color::Black).is_empty());
}

#[test]
fn case_study_qxe6_finds_standing_discovered_attack() {
    // Discovered-attack-after-qxe6 FEN. White to move. The
    // documented standing threat is Black's Qe6 + Be5 → Re1
    // alignment: any forcing move by Be5 (Bxh2+ is the killer)
    // discovers the queen's attack on the rook.
    let pos =
        Position::from_fen("1r4nr/p3k3/4qpp1/4b2p/2Q5/8/PPPP1PPP/R1B1R1K1 w - - 0 1").unwrap();
    let threats = find_latent_threats(&pos, Color::White);
    let discoveries = classify(&threats, TacticPattern::DiscoveredAttack);
    assert!(
        discoveries.iter().any(|t| {
            t.discoverer == Square::E6
                && t.vehicle == Some(Square::E5)
                && t.target == Square::E1
        }),
        "expected Qe6 + Be5 → Re1 alignment; got {threats:#?}"
    );
}

#[test]
fn case_study_qxe6_discovery_fires_via_forcing_check_despite_counter_pin() {
    // Same FEN. The discovered attack on Re1 is genuinely live because
    // the vehicle (Be5) has a *forcing* move — `…Bxh2+` (e5→h2) — that
    // springs it. The bishop is nominally counter-pinned by that same Re1
    // to the queen on e6, but a check overrides the pin: White must answer
    // it before it could ever play Rxe6. This is the synthesis the danger
    // block needs.
    let pos =
        Position::from_fen("1r4nr/p3k3/4qpp1/4b2p/2Q5/8/PPPP1PPP/R1B1R1K1 w - - 0 1").unwrap();
    let threats = find_latent_threats(&pos, Color::White);
    let da = threats
        .iter()
        .find(|t| t.pattern == TacticPattern::DiscoveredAttack && t.target == Square::E1)
        .expect("discovered attack on Re1");

    let firing = describe_discovery_firing(&pos, Color::White, da)
        .expect("the discovery should have a concrete firing move");
    assert_eq!(firing.firing_move.from(), Square::E5, "vehicle is the e5 bishop");
    assert_eq!(firing.firing_move.to(), Square::H2, "Bxh2+ is the forcing spring");
    assert!(firing.gives_check, "Bxh2+ is a check");
    assert!(firing.is_capture, "Bxh2+ grabs the h2 pawn");
    assert!(
        firing.vehicle_counter_pinned,
        "Re1 counter-pins the bishop to the queen — that's why the check matters"
    );
}

#[test]
fn case_study_qxe6_pin_on_bishop_is_escapable_via_check() {
    // The counterpart, read from Black's side: `find_latent_threats(.., Black)`
    // reports Re1's relative pin of Be5 against qe6. That pin does NOT
    // restrain the bishop, because `…Bxh2+` is a checking escape that
    // exposes the queen anyway. pin_forcing_escape names that move.
    let pos =
        Position::from_fen("1r4nr/p3k3/4qpp1/4b2p/2Q5/8/PPPP1PPP/R1B1R1K1 w - - 0 1").unwrap();
    let escape = pin_forcing_escape(&pos, Square::E5, Square::E1, Square::E6, Color::Black)
        .expect("the relatively-pinned bishop has a checking escape");
    assert_eq!(escape.from(), Square::E5);
    assert_eq!(escape.to(), Square::H2);
}

#[test]
fn absolute_pin_has_no_forcing_escape() {
    // A knight pinned to its own king on the same file cannot leave the
    // ray at all, so there is no escape — the pin genuinely holds.
    // White Re1 pins Black Ne7 to Ke8; Black knight to (hypothetically) move.
    let pos = Position::from_fen("4k3/4n3/8/8/8/8/8/4R1K1 b - - 0 1").unwrap();
    assert!(
        pin_forcing_escape(&pos, Square::E7, Square::E1, Square::E8, Color::Black).is_none(),
        "an absolutely-pinned knight has no legal escape, forcing or otherwise"
    );
}

#[test]
fn case_study_desperado_finds_standing_removing_defender() {
    // Missed-desperado-after-qe6 FEN. White to move. Standing
    // threat against White: Black's Nf6 attacks Pe4, which is the
    // sole defender of Nf5. If Black plays Nxe4, the f5-knight is
    // unhooked.
    let pos =
        Position::from_fen("r1b1kb1r/1p3ppp/p2pqn2/4pNB1/4P3/2N5/PPP2PPP/R2QK2R w KQkq - 1 9")
            .unwrap();
    let threats = find_latent_threats(&pos, Color::White);
    let rd = classify(&threats, TacticPattern::RemovingDefender);
    let hit = rd
        .iter()
        .find(|t| t.target == Square::F5 && t.discoverer == Square::F6)
        .expect("expected RemovingDefender hit on Nf5 via Nxe4");
    match hit.trigger_shape {
        TriggerShape::DefenderRemoved { defender } => {
            assert_eq!(defender, Square::E4, "defender should be Pe4");
        }
        other => panic!("expected DefenderRemoved trigger, got {other:?}"),
    }
    assert!(hit.min_gain >= 3, "min_gain should clear the minor-piece bar");
}

#[test]
fn absolute_pin_lights_as_latent_pin_against_blocker_side() {
    // White rook e1 pins Black knight e7 against Black king e8.
    // (Empty board + minimal pieces so no other shapes fire.)
    let pos = Position::from_fen("4k3/4n3/8/8/8/8/8/4R2K b - - 0 1").unwrap();
    let threats = find_latent_threats(&pos, Color::Black);
    let pins = classify(&threats, TacticPattern::Pin);
    let hit = pins
        .iter()
        .find(|t| {
            t.discoverer == Square::E1 && t.vehicle == Some(Square::E7) && t.target == Square::E8
        })
        .unwrap_or_else(|| panic!("expected Re1 + Ne7 → Ke8 pin; got {threats:#?}"));
    assert_eq!(hit.trigger_shape, TriggerShape::VehicleConstrained);
    // King target → gain saturates at the king sentinel.
    assert!(hit.min_gain >= 9);
}

#[test]
fn non_king_rear_lights_as_relative_pin() {
    // White rook e1 pins Black knight e7 (front) to the Black queen on
    // e8 (rear) — king parked on a8, so this is a *relative* pin, not the
    // absolute one. The knight may legally move; it just drops the queen.
    let pos = Position::from_fen("k3q3/4n3/8/8/8/8/8/4R2K b - - 0 1").unwrap();
    let threats = find_latent_threats(&pos, Color::Black);
    // It must be classified RelativePin, never the absolute Pin.
    assert!(
        classify(&threats, TacticPattern::Pin).is_empty(),
        "a non-king rear must not be the absolute Pin; got {threats:#?}"
    );
    let pins = classify(&threats, TacticPattern::RelativePin);
    let hit = pins
        .iter()
        .find(|t| {
            t.discoverer == Square::E1 && t.vehicle == Some(Square::E7) && t.target == Square::E8
        })
        .unwrap_or_else(|| panic!("expected Re1 + Ne7 → Qe8 relative pin; got {threats:#?}"));
    assert_eq!(hit.trigger_shape, TriggerShape::VehicleConstrained);
    // gain proxy = queen - knight = 6 (≥ the minor-piece bar).
    assert_eq!(hit.min_gain, 6);
}

#[test]
fn skewer_lights_when_higher_value_front_lower_value_behind() {
    // White rook e1, Black queen on e7 (high value), Black bishop on
    // e8 (low value behind). Slider attack forces Qe7 to move,
    // exposing the bishop. Black king parked on h8 — NOT on f8 — so
    // it doesn't defend the bishop; otherwise the tightened
    // SEE-ish gain calc (correctly) suppresses the skewer because
    // R-takes-B gets recaptured by the king.
    let pos = Position::from_fen("4b2k/4q3/8/8/8/8/8/4R2K b - - 0 1").unwrap();
    let threats = find_latent_threats(&pos, Color::Black);
    let skewers = classify(&threats, TacticPattern::Skewer);
    assert!(
        skewers.iter().any(|t| {
            t.discoverer == Square::E1
                && t.vehicle == Some(Square::E7)
                && t.target == Square::E8
        }),
        "expected Re1 skewers qe7 → be8; got {threats:#?}"
    );
}

#[test]
fn ordering_is_stable_pattern_then_squares() {
    // Reuse the case-study desperado FEN — it produces multiple
    // standing threats; verify the sort key is consistent.
    let pos =
        Position::from_fen("r1b1kb1r/1p3ppp/p2pqn2/4pNB1/4P3/2N5/PPP2PPP/R2QK2R w KQkq - 1 9")
            .unwrap();
    let threats = find_latent_threats(&pos, Color::White);
    for w in threats.windows(2) {
        let a = (super::pattern_key(w[0].pattern), w[0].discoverer, w[0].target);
        let b = (super::pattern_key(w[1].pattern), w[1].discoverer, w[1].target);
        assert!(a <= b, "out of order: {a:?} then {b:?} in {threats:#?}");
    }
}

#[test]
fn discovered_attack_suppressed_when_slider_would_just_blunder() {
    // Black Qe8 / Be5 / Re1 along the e-file — superficially the
    // same shape as the qxe6 case study, BUT the white rook is
    // defended (Kf1 attacks e1) and the queen-takes-rook trade is
    // -4 for Black. A "permissive" predicate that gates on raw
    // target.value would falsely flag this as a loaded DA. The
    // tightened SEE-ish gate must suppress it.
    //
    // We do NOT want to see Qe8/Be5 → Re1 in the report.
    let pos = Position::from_fen("4q3/8/8/4b3/8/8/8/4R K1 b - - 0 1").unwrap_or_else(|_| {
        // Fallback with the rook defended by the king on f1 (so f1
        // adjacent to e1 → defender). Compose a clean board.
        Position::from_fen("4q3/8/8/4b3/8/8/8/4RK1k b - - 0 1").unwrap()
    });
    let threats = find_latent_threats(&pos, Color::White);
    let discoveries = classify(&threats, TacticPattern::DiscoveredAttack);
    assert!(
        !discoveries.iter().any(|t| {
            t.discoverer == Square::E8
                && t.vehicle == Some(Square::E5)
                && t.target == Square::E1
        }),
        "queen-blunder shape must not light as a DA when target is defended \
         and slider outranks it; got {threats:#?}",
    );
}

#[test]
fn discovered_attack_lights_when_target_undefended_even_if_lower_value() {
    // Same e-file alignment but the rook is undefended now — full
    // gain = 5 (queen takes rook freely). Must light.
    let pos = Position::from_fen("4q3/8/8/4b3/8/8/8/4R2K b - - 5 1").unwrap_or_else(|_| {
        Position::from_fen("4q2k/8/8/4b3/8/8/8/4R2K b - - 5 1").unwrap()
    });
    let threats = find_latent_threats(&pos, Color::White);
    let discoveries = classify(&threats, TacticPattern::DiscoveredAttack);
    assert!(
        discoveries.iter().any(|t| {
            t.discoverer == Square::E8
                && t.vehicle == Some(Square::E5)
                && t.target == Square::E1
        }),
        "undefended rook must light as a DA target; got {threats:#?}",
    );
}

#[test]
fn skips_pawn_x_pawn_alignments_below_threshold() {
    // White bishop b2, Black pawn d4, Black pawn f6. Bxd4 captures
    // pawn, but d4 sits as a "vehicle" with f6 behind — gain = 1
    // (pawn). Should NOT be reported (below the minor-piece gate).
    let pos = Position::from_fen("8/8/5p2/8/3p4/8/1B6/4K2k w - - 0 1").unwrap();
    let threats = find_latent_threats(&pos, Color::Black);
    let discoveries = classify(&threats, TacticPattern::DiscoveredAttack);
    assert!(
        discoveries.is_empty(),
        "low-value pawn rear shouldn't fire latent DA; got {threats:#?}"
    );
}
