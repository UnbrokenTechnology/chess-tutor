//! Sibling tests for [`super`] (`tactical_mode.rs`). These pin the gate
//! at the calibration bookends the case studies define (PLAN §5): the
//! `double-fork-after-qd8` and `discovered-attack-after-qxe6` positions
//! must fire the right named reason, and the `silent-sequencing-after-qc8`
//! position must NOT manufacture a fake mechanism (it may fire on a real
//! standing pin / loose piece — verified against the CLI, asserted only
//! for what's genuinely present).

use super::*;
use crate::analysis::tactic_outcome::TacticPattern;
use crate::position::Position;
use crate::types::Color;

#[test]
fn startpos_is_not_tactically_live() {
    let pos = Position::startpos();
    let state = classify_tactical_mode(&pos, Color::White, None);
    assert!(!state.live, "opening position should be quiet; got {state:?}");
    assert!(state.reasons.is_empty());
}

#[test]
fn double_fork_after_qd8_fires_check_followup() {
    // Position AFTER `…Qd8` (White to move). Per `double-fork-after-qd8.md`,
    // Black has a standing `…Nd3+ → …Nf2` two-step fork loaded against
    // White. The user here is White; the opponent (Black) owns the
    // check-followup, so the gate must surface an OpponentCheckFollowup.
    let pos =
        Position::from_fen("r1bqkbnr/pp3p1p/2p3p1/2n1p3/2P1P3/1P3P2/PBQPN1PP/R3KBNR w KQkq - 1 11")
            .unwrap();
    let state = classify_tactical_mode(&pos, Color::White, None);
    assert!(state.live, "position is tactically live; got {state:?}");

    let has_followup = state.reasons.iter().any(|r| {
        matches!(r, TacticalReason::OpponentCheckFollowup(cf)
            if cf.check_move.to() == crate::types::Square::D3)
    });
    assert!(
        has_followup,
        "expected an OpponentCheckFollowup with the …Nd3+ check; got {:#?}",
        state.reasons
    );
}

#[test]
fn discovered_attack_fires_opponent_latent_threat() {
    // `discovered-attack-after-qxe6.md` framing position (White to move):
    // Black has a DiscoveredAttack loaded against White's Re1 (Be5 is the
    // vehicle; …Bxh2+ fires it). The user is White; the gate must surface
    // an OpponentLatentThreat whose pattern is DiscoveredAttack.
    let pos = Position::from_fen("1r4nr/p3k3/4qpp1/4b2p/2Q5/8/PPPP1PPP/R1B1R1K1 w - - 0 1").unwrap();
    let state = classify_tactical_mode(&pos, Color::White, None);
    assert!(state.live, "position is tactically live; got {state:?}");

    let da = state.reasons.iter().any(|r| {
        matches!(r, TacticalReason::OpponentLatentThreat(t)
            if t.pattern == TacticPattern::DiscoveredAttack)
    });
    assert!(
        da,
        "expected an OpponentLatentThreat(DiscoveredAttack); got {:#?}",
        state.reasons
    );
}

#[test]
fn silent_sequencing_fires_no_fake_mechanism() {
    // Calibration FEN (Black to move). This is the over-tuning bookend:
    // the gate must not invent a mechanism the board does not contain. It
    // is NOT a quiet position — the black king on e7 is brutally exposed
    // (+4.57 for White), so what the chess-tutor CLI confirms is genuinely
    // present is rich, and the gate is ALLOWED to surface every bit of it:
    //   - a REAL absolute Pin (Re1 → be6 → ke7) (`tactics --latent`).
    //   - be6 is SEE-losing for Black, a real loose piece (`threats`).
    //   - White has `Rxe6+` / `Qxe6+` check-followups — real two-step
    //     forcing sequences off captures of the pinned bishop, each
    //     leading to a further tactic (`tactics --check-followups`).
    //   - a real self-replenishing forcing-check chain against the king.
    //
    // So the honest calibration here is the *converse* of "stay silent":
    // every named mechanism the gate fires must be one the CLI confirms.
    // Concretely:
    //   - every OpponentCheckFollowup's check must be a capture of the
    //     pinned bishop on e6 (the only checks White has — `forcing`
    //     shows exactly `Rxe6+` and `Qxe6+`), never an invented check.
    //   - every OpponentLatentThreat must be the real absolute Pin, never
    //     an invented fork / discovered attack / skewer.
    let pos =
        Position::from_fen("1r1q2nr/p3k3/2Bbbpp1/7p/2Q5/8/PPPP1PPP/R1B1R1K1 b - - 0 1").unwrap();
    let state = classify_tactical_mode(&pos, Color::Black, None);

    // White's ONLY checks on this board are Rxe6+ and Qxe6+ (both capture
    // the bishop on e6). Any check-followup the gate names must therefore
    // land its check on e6 — anything else would be a fabricated check.
    for r in &state.reasons {
        if let TacticalReason::OpponentCheckFollowup(cf) = r {
            assert_eq!(
                cf.check_move.to(),
                crate::types::Square::E6,
                "White's only checks here capture the bishop on e6; \
                 a check-followup landing elsewhere is fabricated. got {cf:?}"
            );
        }
    }

    // Every named latent-threat mechanism that fires must be honest. The
    // only real standing threat against the black king here is the
    // absolute Pin; any other pattern would be invented.
    for r in &state.reasons {
        if let TacticalReason::OpponentLatentThreat(t) = r {
            assert_eq!(
                t.pattern,
                TacticPattern::Pin,
                "the only real standing threat here is the absolute pin on the king; got {t:?}"
            );
        }
    }
}

#[test]
fn reasons_are_in_priority_order() {
    // Whatever reasons fire, they must be sorted by the documented
    // priority: InCheck < OpponentLatentThreat < OpponentCheckFollowup
    // < ForcingCheckChain < OurTactic < LoosePiece. We verify the
    // ordering invariant on a live position (the discovered-attack FEN,
    // which produces a latent threat and at least one loose piece).
    let pos = Position::from_fen("1r4nr/p3k3/4qpp1/4b2p/2Q5/8/PPPP1PPP/R1B1R1K1 w - - 0 1").unwrap();
    let state = classify_tactical_mode(&pos, Color::White, None);
    assert!(state.live);

    let rank = |r: &TacticalReason| match r {
        TacticalReason::InCheck => 0u8,
        TacticalReason::OpponentLatentThreat(_) => 1,
        TacticalReason::OpponentCheckFollowup(_) => 2,
        TacticalReason::ForcingCheckChain { .. } => 3,
        TacticalReason::OurTactic(_) => 4,
        TacticalReason::LoosePiece { .. } => 5,
    };
    let ranks: Vec<u8> = state.reasons.iter().map(rank).collect();
    let mut sorted = ranks.clone();
    sorted.sort_unstable();
    assert_eq!(ranks, sorted, "reasons must be in priority order; got {:#?}", state.reasons);
}

#[test]
fn live_iff_reasons_nonempty() {
    let pos = Position::from_fen("1r4nr/p3k3/4qpp1/4b2p/2Q5/8/PPPP1PPP/R1B1R1K1 w - - 0 1").unwrap();
    let state = classify_tactical_mode(&pos, Color::White, None);
    assert_eq!(state.live, !state.reasons.is_empty());
}
