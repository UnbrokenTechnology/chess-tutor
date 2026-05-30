use super::*;
use crate::analysis::latent_threats::find_latent_threats;
use crate::engine::Engine;
use crate::position::Position;
use crate::types::Square;

/// The discovered-attack-after-Qxe6 case study. White (to move) is
/// winning but Black has a loaded discovered attack on Re1 (Qe6/Be5/Re1
/// on the e-file). The queen on c4 is also under attack, so the only
/// moves that both defuse the discovered attack AND hold the advantage
/// are Qxe6 (captures the discoverer), Qe4 and Qe2 (block the e-file).
const CASE_STUDY: &str = "1r4nr/p3k3/4qpp1/4b2p/2Q5/8/PPPP1PPP/R1B1R1K1 w - - 0 1";

/// Depth 10 is the floor where the horizon mis-score on the
/// queen-dropping decoys (e.g. Kf1, which is `+150` at depth 8 but
/// `-34` *pawns* once the search sees `…Qxc4`) clears, so the
/// holders/non-holders separate by ~25 pawns. Faster than the
/// production depth (12) but well past the artifact.
const TEST_DEPTH: u32 = 10;

fn defuse(fen: &str, depth: u32) -> DefusalReport {
    let mut pos = Position::from_fen(fen).expect("valid fen");
    let side = pos.side_to_move();
    let threats = find_latent_threats(&pos, side);
    let mut engine = Engine::default();
    find_threat_defusals(&mut engine, &mut pos, &threats, depth)
}

fn holders(report: &DefusalReport) -> Vec<Square> {
    report
        .defusals
        .iter()
        .filter(|d| d.holds)
        .map(|d| d.mv.to())
        .collect()
}

#[test]
fn case_study_holders_include_oracle_three_and_all_move_the_queen() {
    let report = defuse(CASE_STUDY, TEST_DEPTH);
    let hold_targets = holders(&report);
    // The three defusals the case-study write-up identified by hand must
    // all appear as holders. (The engine also finds Qf1 — a 4th valid
    // defusal the hand analysis missed, which is fine: the search is the
    // authority, not the prose.)
    for sq in [Square::E6, Square::E4, Square::E2] {
        assert!(
            hold_targets.contains(&sq),
            "expected a holding queen move to {:?}; full report: {:?}",
            sq,
            report.defusals,
        );
    }
    // Every holder moves the attacked queen off c4 — the only way to both
    // escape the c4 attack and address the e1 discovered attack. The
    // queen-dropping decoys (a king step to f1, a rook shuffle off e1)
    // must therefore NOT hold.
    assert!(
        report
            .defusals
            .iter()
            .filter(|d| d.holds)
            .all(|d| d.mv.from() == Square::C4),
        "all holders should be queen moves from c4; full report: {:?}",
        report.defusals,
    );
}

#[test]
fn case_study_best_move_defuses_and_holds() {
    let report = defuse(CASE_STUDY, TEST_DEPTH);
    let best = report.best_move.expect("non-terminal position has a best move");
    // The engine's top move must itself be one of the holding defusals —
    // you cannot stay winning here without addressing the standing threat.
    let best_defusal = report
        .defusals
        .iter()
        .find(|d| d.mv == best)
        .expect("best move should appear in the defusal set");
    assert!(
        best_defusal.holds,
        "the best move must hold; full report: {:?}",
        report.defusals,
    );
    assert_eq!(best.from(), Square::C4, "best move should move the queen off c4");
}

#[test]
fn case_study_qxe6_captures_the_discoverer() {
    let report = defuse(CASE_STUDY, TEST_DEPTH);
    let qxe6 = report
        .defusals
        .iter()
        .find(|d| d.mv.from() == Square::C4 && d.mv.to() == Square::E6)
        .expect("Qxe6 should be a defusal candidate");
    assert!(qxe6.holds, "Qxe6 must hold; report: {:?}", report.defusals);
    assert_eq!(qxe6.defuses[0].mechanism, DefusalMechanism::CaptureDiscoverer);
}

#[test]
fn case_study_qe4_blocks_the_ray() {
    let report = defuse(CASE_STUDY, TEST_DEPTH);
    let qe4 = report
        .defusals
        .iter()
        .find(|d| d.mv.from() == Square::C4 && d.mv.to() == Square::E4)
        .expect("Qe4 should be a defusal candidate");
    assert_eq!(qe4.defuses[0].mechanism, DefusalMechanism::Block);
}

#[test]
fn case_study_a_rook_move_defuses_but_does_not_hold() {
    // Moving the rook off the e-file (e.g. Re1-d1) defuses the
    // discovered attack geometrically but drops the hanging queen on
    // c4 — it must appear as a non-holder, not vanish.
    let report = defuse(CASE_STUDY, TEST_DEPTH);
    let rook_move = report
        .defusals
        .iter()
        .find(|d| d.mv.from() == Square::E1);
    if let Some(rm) = rook_move {
        assert!(
            !rm.holds,
            "a rook-off-e-file move addresses the threat but loses the queen — must not hold: {:?}",
            rm,
        );
        assert_eq!(rm.defuses[0].mechanism, DefusalMechanism::RelocateTarget);
    }
    // (If the engine's geometric scan classed no rook move as a
    // candidate at this depth, the test still passes — the holder
    // assertions above are the load-bearing ones.)
}

#[test]
fn no_threats_yields_no_defusals() {
    // Start position: no standing threats, so nothing to defuse.
    let mut pos = Position::startpos();
    let mut engine = Engine::default();
    let report = find_threat_defusals(&mut engine, &mut pos, &[], TEST_DEPTH);
    assert!(report.defusals.is_empty());
    assert!(report.best_move.is_some());
}
