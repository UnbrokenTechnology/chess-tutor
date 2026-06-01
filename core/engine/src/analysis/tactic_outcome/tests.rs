use super::super::test_support::{ma_with_pv, ma_with_pv_score};
use super::*;
use crate::types::{Color, Move, Square};

// Royal fork: white knight on b5 plays Nc7+, forking the black king on
// e8 and the rook on a8. Used by several tests.
const ROYAL_FORK_FEN: &str = "r3k3/8/8/1N6/8/8/8/6K1 w - - 0 1";

fn pos(fen: &str) -> Position {
    Position::from_fen(fen).unwrap()
}

// ---- detect_fork: positive cases ------------------------------------

#[test]
fn knight_royal_fork_fires() {
    let pre = pos(ROYAL_FORK_FEN);
    let nc7 = Move::normal(Square::B5, Square::C7);
    let hit = detect_line_tactic(&pre, &[nc7], Color::White, 0, None).expect("fork should fire");

    assert_eq!(hit.pattern, TacticPattern::Fork);
    assert_eq!(hit.pv_ply, 0);
    assert_eq!(hit.primary_piece, Square::C7);
    assert_eq!(hit.targets, vec![Square::A8, Square::E8]);
    // Single move, no capture in window → not yet realized.
    assert_eq!(hit.confidence, Confidence::Medium);
}

#[test]
fn fork_with_capture_continuation_is_high_confidence() {
    let pre = pos(ROYAL_FORK_FEN);
    // Nc7+ Kd8 Nxa8 — the rook falls within the four-ply window.
    let pv = [
        Move::normal(Square::B5, Square::C7),
        Move::normal(Square::E8, Square::D8),
        Move::normal(Square::C7, Square::A8),
    ];
    let hit = detect_line_tactic(&pre, &pv, Color::White, 0, None).expect("fork should fire");
    assert_eq!(hit.confidence, Confidence::High);
    assert_eq!(hit.material_gain, Some(Value::ROOK_MG.0));
}

#[test]
fn pawn_fork_fires_via_pawn_attack_pattern() {
    // White pawn d4 pushes to d5, forking two black rooks on c6 and e6.
    // Neither rook attacks d5, so the pawn is safe and both rooks outvalue it.
    let pre = pos("k7/8/2r1r3/8/3P4/8/8/7K w - - 0 1");
    let d5 = Move::normal(Square::D4, Square::D5);
    let hit = detect_line_tactic(&pre, &[d5], Color::White, 0, None).expect("pawn fork should fire");
    assert_eq!(hit.pattern, TacticPattern::Fork);
    assert_eq!(hit.primary_piece, Square::D5);
    assert_eq!(hit.targets, vec![Square::C6, Square::E6]);
}

#[test]
fn queen_forks_two_hanging_knights_via_hanging_branch() {
    // White queen a1 → d4 attacks two undefended black knights on b4 and
    // f4. Both are worth less than the queen, so they qualify only through
    // the "hanging and can't recapture" arm, not the value arm. The king
    // is on a8 — off every line from d4 — so it isn't a third target.
    let pre = pos("k7/8/8/8/1n3n2/8/8/Q6K w - - 0 1");
    let qd4 = Move::normal(Square::A1, Square::D4);
    let hit = detect_line_tactic(&pre, &[qd4], Color::White, 0, None).expect("queen fork should fire");
    assert_eq!(hit.pattern, TacticPattern::Fork);
    assert_eq!(hit.targets, vec![Square::B4, Square::F4]);
}

// ---- detect_fork: negative cases ------------------------------------

#[test]
fn king_mover_is_not_a_fork() {
    // A king landing next to a black rook (c5) and knight (e5) is never
    // treated as a fork — a king can't be the forking piece.
    let pre = pos("7k/8/8/2r1n3/3K4/8/8/8 w - - 0 1");
    let kd5 = Move::normal(Square::D4, Square::D5);
    assert!(detect_line_tactic(&pre, &[kd5], Color::White, 0, None).is_none());
}

#[test]
fn single_target_is_not_a_fork() {
    // Knight check with only one valuable target (the king) — one target is
    // not a fork.
    let pre = pos("4k3/8/8/1N6/8/8/8/6K1 w - - 0 1");
    let nc7 = Move::normal(Square::B5, Square::C7);
    assert!(detect_line_tactic(&pre, &[nc7], Color::White, 0, None).is_none());
}

#[test]
fn forker_in_bad_spot_is_not_a_fork() {
    // Same royal-fork geometry, but a black bishop on a5 attacks c7. The
    // knight would be hanging there, so the fork is illusory.
    let pre = pos("r3k3/8/8/bN6/8/8/8/6K1 w - - 0 1");
    let nc7 = Move::normal(Square::B5, Square::C7);
    assert!(detect_line_tactic(&pre, &[nc7], Color::White, 0, None).is_none());
}

// ---- compute_tactic_outcome: the three slots ------------------------

#[test]
fn outcome_reports_user_played_fork() {
    let pre = pos(ROYAL_FORK_FEN);
    let pv = vec![
        Move::normal(Square::B5, Square::C7),
        Move::normal(Square::E8, Square::D8),
        Move::normal(Square::C7, Square::A8),
    ];
    let ma = ma_with_pv(pv, Some(2));
    let outcome = compute_tactic_outcome(&ma, &ma, &pre, Color::White, None);

    let hit = outcome.user_played_tactic.expect("user played a fork");
    assert_eq!(hit.pattern, TacticPattern::Fork);
    assert_eq!(hit.confidence, Confidence::High);
    // Same move as best → nothing "missed".
    assert!(outcome.user_missed_tactic.is_none());
    assert!(outcome.user_walked_into.is_none());
}

#[test]
fn outcome_reports_missed_fork_when_user_chose_another_move() {
    let pre = pos(ROYAL_FORK_FEN);
    // Best forks and wins material (a real eval edge); the user shuffled
    // the king, keeping the position equal. Both the win-probability gap
    // and the cp gap clear the "don't nag" gate.
    let best = ma_with_pv_score(vec![Move::normal(Square::B5, Square::C7)], Some(0), Value(600));
    let user = ma_with_pv_score(vec![Move::normal(Square::G1, Square::F1)], Some(0), Value::ZERO);

    let outcome = compute_tactic_outcome(&best, &user, &pre, Color::White, None);
    let missed = outcome.user_missed_tactic.expect("best line had a fork");
    assert_eq!(missed.pattern, TacticPattern::Fork);
    assert!(outcome.user_played_tactic.is_none());
}

#[test]
fn missed_tactic_suppressed_when_user_move_nearly_as_good() {
    // Same missed fork, but the user's move keeps a near-equal eval to best
    // (tiny win% gap) — "you missed THE move" would be a lie, so suppress.
    let pre = pos(ROYAL_FORK_FEN);
    let best = ma_with_pv_score(vec![Move::normal(Square::B5, Square::C7)], Some(0), Value(60));
    let user = ma_with_pv_score(vec![Move::normal(Square::G1, Square::F1)], Some(0), Value(40));
    let outcome = compute_tactic_outcome(&best, &user, &pre, Color::White, None);
    assert!(outcome.user_missed_tactic.is_none());
}

#[test]
fn missed_tactic_surfaces_even_when_already_winning() {
    // Regression: in winning positions win% saturates near the asymptote,
    // so the original "win% gap" gate suppressed missed cards even when
    // the absolute cp gap was huge. The student deserves to learn about
    // the named tactic regardless of how crushing the position already
    // is — so an absolute-cp gate (200 cp ≈ 1 pawn) backs up the win%
    // gate. User-reported FEN: white up a queen, every move keeps win%
    // above 0.9; the +1000 cp improvement (winning a rook via Nxc7+)
    // would silently disappear without the cp gate.
    let pre = pos(ROYAL_FORK_FEN);
    let best = ma_with_pv_score(vec![Move::normal(Square::B5, Square::C7)], Some(0), Value(3000));
    let user = ma_with_pv_score(vec![Move::normal(Square::G1, Square::F1)], Some(0), Value(2000));
    let outcome = compute_tactic_outcome(&best, &user, &pre, Color::White, None);
    let missed = outcome.user_missed_tactic.expect(
        "named fork on the best line must surface despite the user already winning",
    );
    assert_eq!(missed.pattern, TacticPattern::Fork);
}

#[test]
fn outcome_has_no_missed_tactic_when_user_played_best() {
    let pre = pos(ROYAL_FORK_FEN);
    let ma = ma_with_pv(vec![Move::normal(Square::B5, Square::C7)], Some(0));
    let outcome = compute_tactic_outcome(&ma, &ma, &pre, Color::White, None);
    assert!(outcome.user_missed_tactic.is_none());
}

#[test]
fn outcome_reports_walked_into_fork() {
    // White (the user) pushes a quiet pawn; black replies with Nc2+,
    // forking the white king on e1 and the rook on a1.
    let pre = pos("4k3/8/8/8/1n6/8/7P/R3K3 w - - 0 1");
    let pv = vec![
        Move::normal(Square::H2, Square::H3),
        Move::normal(Square::B4, Square::C2),
    ];
    let ma = ma_with_pv(pv, Some(1));
    let outcome = compute_tactic_outcome(&ma, &ma, &pre, Color::White, None);

    let walked = outcome.user_walked_into.expect("walked into a fork");
    assert_eq!(walked.pattern, TacticPattern::Fork);
    assert_eq!(walked.pv_ply, 1);
    assert_eq!(walked.primary_piece, Square::C2);
    assert_eq!(walked.targets, vec![Square::A1, Square::E1]);
    // The quiet pawn push itself is no tactic.
    assert!(outcome.user_played_tactic.is_none());
}

// ---- latent walked-into (PLAN §4.1 pre-emptive wiring) --------------

#[test]
fn walked_into_latent_fires_when_move_leaves_standing_alignment() {
    // discovered-attack-after-qxe6 case study. White (the user) plays the
    // natural-looking Qc5+ (c4c5) instead of the defusing Qxe6+. Qc5+ does
    // NOT remove the black queen on e6, so the discovered-attack alignment
    // (qe6 -> be5 -> Re1) stays loaded against White. The search realizes
    // the rook loss several plies out — past the 4-ply detector window — so
    // the PV-based walked-into detector finds nothing, and only the latent
    // fallback catches it.
    let pre = pos("1r4nr/p3k3/4qpp1/4b2p/2Q5/8/PPPP1PPP/R1B1R1K1 w - - 0 1");
    // Qc5+ then a forced quiet king move; the bishop never moves in this
    // short line, so the discovery is not *played* — it's merely standing.
    let pv = vec![
        Move::normal(Square::C4, Square::C5),
        Move::normal(Square::E7, Square::F7),
    ];
    let ma = ma_with_pv(pv, Some(1));
    let outcome = compute_tactic_outcome(&ma, &ma, &pre, Color::White, None);

    let walked = outcome
        .user_walked_into
        .expect("the standing discovered attack on Re1 must surface pre-emptively");
    assert_eq!(walked.pattern, TacticPattern::DiscoveredAttack);
    assert_eq!(walked.pv_ply, 1);
    // The discoverer is the black queen on e6; the target is the rook e1.
    assert_eq!(walked.primary_piece, Square::E6);
    assert_eq!(walked.targets, vec![Square::E1]);
}

// ---- detect_hanging_capture -----------------------------------------

// White rook on d1 captures an undefended black bishop on d5.
const HANGING_BISHOP_FEN: &str = "4k3/8/8/3b4/8/8/8/3RK3 w - - 0 1";

#[test]
fn capturing_an_undefended_piece_fires_hanging_capture() {
    let pre = pos(HANGING_BISHOP_FEN);
    let rxd5 = Move::normal(Square::D1, Square::D5);
    let hit = detect_line_tactic(&pre, &[rxd5], Color::White, 0, None).expect("hanging capture");
    assert_eq!(hit.pattern, TacticPattern::HangingCapture);
    assert_eq!(hit.pv_ply, 0);
    assert_eq!(hit.primary_piece, Square::D5);
    assert_eq!(hit.targets, vec![Square::D5]);
    assert_eq!(hit.confidence, Confidence::High);
    assert_eq!(hit.material_gain, Some(Value::BISHOP_MG.0));
}

#[test]
fn capturing_a_defended_piece_is_not_a_hanging_capture() {
    // A black pawn on e6 defends (and would recapture on) d5.
    let pre = pos("4k3/8/4p3/3b4/8/8/8/3RK3 w - - 0 1");
    let rxd5 = Move::normal(Square::D1, Square::D5);
    assert!(detect_line_tactic(&pre, &[rxd5], Color::White, 0, None).is_none());
}

#[test]
fn capturing_a_pawn_is_not_a_hanging_capture() {
    // Pawns are excluded — "you won a free pawn" isn't the lesson.
    let pre = pos("4k3/8/8/3p4/8/8/8/3RK3 w - - 0 1");
    let rxd5 = Move::normal(Square::D1, Square::D5);
    assert!(detect_line_tactic(&pre, &[rxd5], Color::White, 0, None).is_none());
}

#[test]
fn quiet_move_next_to_a_hanging_piece_is_not_a_capture() {
    // Rd1-d4 attacks the hanging bishop but doesn't capture it.
    let pre = pos(HANGING_BISHOP_FEN);
    let rd4 = Move::normal(Square::D1, Square::D4);
    assert!(detect_line_tactic(&pre, &[rd4], Color::White, 0, None).is_none());
}

#[test]
fn outcome_reports_user_played_hanging_capture() {
    let pre = pos(HANGING_BISHOP_FEN);
    let ma = ma_with_pv(vec![Move::normal(Square::D1, Square::D5)], Some(0));
    let outcome = compute_tactic_outcome(&ma, &ma, &pre, Color::White, None);
    let hit = outcome.user_played_tactic.expect("free-piece capture");
    assert_eq!(hit.pattern, TacticPattern::HangingCapture);
}

// ---- recapture guard (lichess op_capture) ---------------------------

// Black's undefended bishop sits on e5, attacked by the white queen on
// h5 — Qxe5 looks like a free piece in isolation.
const RECAPTURE_ON_E5_FEN: &str = "4k3/8/8/4b2Q/8/8/8/6K1 w - - 0 1";

#[test]
fn even_recapture_is_not_flagged_as_free_piece() {
    // The bishop only sits on e5 because black just played Bxe5, taking a
    // white knight there. Qxe5 is the far side of a B-for-N trade, not a
    // won piece — so with the prior move known, it must NOT fire.
    let pre = pos(RECAPTURE_ON_E5_FEN);
    let qxe5 = Move::normal(Square::H5, Square::E5);
    let ma = ma_with_pv(vec![qxe5], Some(0));
    let prior = PriorMove {
        mv: Move::normal(Square::D6, Square::E5),
        captured: Some(PieceType::Knight),
    };
    let outcome = compute_tactic_outcome(&ma, &ma, &pre, Color::White, Some(prior));
    assert!(outcome.user_played_tactic.is_none());
}

#[test]
fn recapture_guard_is_skipped_without_prior_move() {
    // Same position, no move history: we can't tell a recapture from a
    // hang, so the (now-acknowledged) false positive still fires. This
    // pins down that the guard — not some other check — is what suppresses
    // the even-recapture case.
    let pre = pos(RECAPTURE_ON_E5_FEN);
    let qxe5 = Move::normal(Square::H5, Square::E5);
    let ma = ma_with_pv(vec![qxe5], Some(0));
    let outcome = compute_tactic_outcome(&ma, &ma, &pre, Color::White, None);
    assert_eq!(
        outcome.user_played_tactic.map(|h| h.pattern),
        Some(TacticPattern::HangingCapture)
    );
}

#[test]
fn hanging_a_queen_survives_the_recapture_guard() {
    // The opponent grabbed a pawn with their queen (Qxe5 taking a pawn),
    // leaving the queen hanging to our rook. Winning a queen for a pawn is
    // a genuine free piece — a lower-valued prior capture must not suppress.
    let pre = pos("k7/8/8/4q3/8/8/8/4R1K1 w - - 0 1");
    let rxe5 = Move::normal(Square::E1, Square::E5);
    let ma = ma_with_pv(vec![rxe5], Some(0));
    let prior = PriorMove {
        mv: Move::normal(Square::F6, Square::E5),
        captured: Some(PieceType::Pawn),
    };
    let outcome = compute_tactic_outcome(&ma, &ma, &pre, Color::White, Some(prior));
    assert_eq!(
        outcome.user_played_tactic.map(|h| h.pattern),
        Some(TacticPattern::HangingCapture)
    );
}

#[test]
fn recapture_guard_only_applies_on_the_same_square() {
    // The prior capture was on a different square (b2), so even though it
    // was a heavy piece, it doesn't excuse the hang on e5.
    let pre = pos(RECAPTURE_ON_E5_FEN);
    let qxe5 = Move::normal(Square::H5, Square::E5);
    let ma = ma_with_pv(vec![qxe5], Some(0));
    let prior = PriorMove {
        mv: Move::normal(Square::A1, Square::B2),
        captured: Some(PieceType::Rook),
    };
    let outcome = compute_tactic_outcome(&ma, &ma, &pre, Color::White, Some(prior));
    assert_eq!(
        outcome.user_played_tactic.map(|h| h.pattern),
        Some(TacticPattern::HangingCapture)
    );
}

#[test]
fn prior_move_new_resolves_the_captured_piece() {
    // PriorMove::new reads the captured piece off the board it was played
    // in: a white knight stood on e5 before ...Bxe5.
    let before = pos("4k3/8/3b4/4N3/7Q/8/8/6K1 b - - 0 1");
    let bxe5 = Move::normal(Square::D6, Square::E5);
    let prior = PriorMove::new(&before, bxe5);
    assert_eq!(prior.captured, Some(PieceType::Knight));
    assert_eq!(prior.mv.to(), Square::E5);
}

// ---- detect_removing_defender ---------------------------------------

// White pawn e5 captures the black knight f6, which was the only
// defender of the black bishop on d5 that the white rook on d1 attacks.
// The knight is itself defended by the g7 pawn, so this is purely a
// remove-the-defender (not a free capture of the knight).
const REMOVE_DEFENDER_FEN: &str = "4k3/6p1/5n2/3bP3/8/8/8/3RK3 w - - 0 1";

#[test]
fn capturing_the_sole_defender_fires_removing_defender() {
    let pre = pos(REMOVE_DEFENDER_FEN);
    let exf6 = Move::normal(Square::E5, Square::F6);
    let hit = detect_line_tactic(&pre, &[exf6], Color::White, 0, None).expect("removing the defender");
    assert_eq!(hit.pattern, TacticPattern::RemovingDefender);
    assert_eq!(hit.pv_ply, 0);
    assert_eq!(hit.primary_piece, Square::F6);
    assert_eq!(hit.targets, vec![Square::D5]);
}

#[test]
fn not_attacking_f2f7_when_the_capturer_is_just_lost() {
    // From the real game (White queen on g7). Qxf7+ grabs the f7 pawn next
    // to the uncastled king, but f7 is defended by the king AND the c7
    // queen — the capturer is simply recaptured. Geometry says "attack on
    // f7"; SEE says it's a queen blunder. Must not fire.
    let pre = pos("rnb1k2r/ppq2pQ1/3bp1np/8/2B5/5N2/PPP2PPP/RNB2RK1 w - - 1 4");
    let qxf7 = Move::normal(Square::G7, Square::F7);
    assert!(
        detect_line_tactic(&pre, &[qxf7], Color::White, 0, None).is_none(),
        "Qxf7+ loses the queen to the recapture — not an attack-on-f7 tactic"
    );
}

#[test]
fn not_removing_defender_when_capturing_the_defender_loses_material() {
    // The defender (rook h8) is itself guarded by a knight (g6), so taking
    // it with the queen is SEE −4 — a queen blunder, not a tactic. From the
    // real game (before the Qxh8 desperado). Geometry says "remove the
    // rook to win the h6 pawn"; SEE says no. Must not fire.
    let pre = pos("r1b1k2r/ppqn1pQ1/4p1np/1B2b3/7N/8/PPP2PPP/RNB2RK1 w - - 5 6");
    let qxh8 = Move::normal(Square::G7, Square::H8);
    assert!(
        detect_line_tactic(&pre, &[qxh8], Color::White, 0, None).is_none(),
        "Qxh8 loses the queen (rook is knight-defended) — not a removing-the-defender tactic"
    );
}

#[test]
fn not_removing_defender_when_target_has_a_second_defender() {
    // A black pawn on c6 also defends d5, so removing the f6 knight
    // doesn't leave the bishop hanging.
    let pre = pos("4k3/6p1/2p2n2/3bP3/8/8/8/3RK3 w - - 0 1");
    let exf6 = Move::normal(Square::E5, Square::F6);
    assert!(detect_line_tactic(&pre, &[exf6], Color::White, 0, None).is_none());
}

#[test]
fn outcome_reports_user_played_removing_defender() {
    let pre = pos(REMOVE_DEFENDER_FEN);
    let ma = ma_with_pv(vec![Move::normal(Square::E5, Square::F6)], Some(0));
    let outcome = compute_tactic_outcome(&ma, &ma, &pre, Color::White, None);
    let hit = outcome.user_played_tactic.expect("removing the defender");
    assert_eq!(hit.pattern, TacticPattern::RemovingDefender);
}

// ---- detect_trapped_piece -------------------------------------------

// White bishop on e4; the black knight on a8 is fenced by white pawns
// c5 (covers b6) and d6 (covers c7). White plays Bd5 keeping the knight
// attacked while it has no safe square — black is poised to lose it.
const SPRING_TRAP_FEN: &str = "n6k/8/3P4/2P5/4B3/8/8/6K1 w - - 0 1";

#[test]
fn move_trapping_a_corner_knight_fires() {
    let pre = pos(SPRING_TRAP_FEN);
    let bd5 = Move::normal(Square::E4, Square::D5);
    let hit = detect_line_tactic(&pre, &[bd5], Color::White, 0, None).expect("trapped piece");
    assert_eq!(hit.pattern, TacticPattern::TrappedPiece);
    assert_eq!(hit.pv_ply, 0);
    assert_eq!(hit.primary_piece, Square::A8);
    assert_eq!(hit.targets, vec![Square::A8]);
    // No capture in the line yet — the trap is a threat, not realized.
    assert_eq!(hit.confidence, Confidence::Medium);
}

#[test]
fn move_leaving_the_knight_unattacked_does_not_fire() {
    // Bf5 steps the bishop off the a8 diagonal: the knight is no longer
    // attacked, so it isn't trapped.
    let pre = pos(SPRING_TRAP_FEN);
    let bf5 = Move::normal(Square::E4, Square::F5);
    assert!(detect_line_tactic(&pre, &[bf5], Color::White, 0, None).is_none());
}

#[test]
fn outcome_reports_user_played_trapped_piece() {
    let pre = pos(SPRING_TRAP_FEN);
    let ma = ma_with_pv(vec![Move::normal(Square::E4, Square::D5)], Some(0));
    let outcome = compute_tactic_outcome(&ma, &ma, &pre, Color::White, None);
    let hit = outcome.user_played_tactic.expect("trapped piece");
    assert_eq!(hit.pattern, TacticPattern::TrappedPiece);
    assert_eq!(hit.primary_piece, Square::A8);
}

// ---- detect_double_check --------------------------------------------

#[test]
fn knight_move_unmasking_rook_is_a_double_check() {
    // Black Ke8, white Ne5 on the e-file blocking Re1. Ne5-d6+ checks with
    // the knight and unmasks the rook: two checkers at once.
    let pre = pos("4k3/8/8/4N3/8/8/8/4R1K1 w - - 0 1");
    let nd6 = Move::normal(Square::E5, Square::D6);
    let hit = detect_line_tactic(&pre, &[nd6], Color::White, 0, None).expect("double check");
    assert_eq!(hit.pattern, TacticPattern::DoubleCheck);
    assert_eq!(hit.primary_piece, Square::D6);
    assert_eq!(hit.targets, vec![Square::E8]);
}

// ---- detect_discovered_check ----------------------------------------

#[test]
fn bishop_move_unmasking_rook_is_a_discovered_check() {
    // Black Ke8, white Be5 on the e-file blocking Re1. Be5-d4 unmasks the
    // rook's check without the bishop itself checking — a discovered check.
    let pre = pos("4k3/8/8/4B3/8/8/8/4R1K1 w - - 0 1");
    let bd4 = Move::normal(Square::E5, Square::D4);
    let hit = detect_line_tactic(&pre, &[bd4], Color::White, 0, None).expect("discovered check");
    assert_eq!(hit.pattern, TacticPattern::DiscoveredCheck);
    assert_eq!(hit.primary_piece, Square::D4);
    assert_eq!(hit.targets, vec![Square::E8]);
}

// ---- detect_skewer --------------------------------------------------

#[test]
fn rook_check_with_queen_behind_is_a_skewer() {
    // Black Ke4 with the queen on e8 behind it. Ra1-e1+ forces the king to
    // step off the e-file, winning the queen — a skewer.
    let pre = pos("4q3/8/8/8/4k3/8/8/R5K1 w - - 0 1");
    let re1 = Move::normal(Square::A1, Square::E1);
    let hit = detect_line_tactic(&pre, &[re1], Color::White, 0, None).expect("skewer");
    assert_eq!(hit.pattern, TacticPattern::Skewer);
    assert_eq!(hit.primary_piece, Square::E1);
    assert_eq!(hit.targets, vec![Square::E4, Square::E8]);
}

#[test]
fn no_skewer_when_back_piece_is_not_less_valuable() {
    // Rook attacks a black bishop (e4) with a black knight (e8) behind:
    // front and back are equal value, so it isn't a skewer.
    let pre = pos("4n2k/8/8/8/4b3/8/8/R5K1 w - - 0 1");
    let re1 = Move::normal(Square::A1, Square::E1);
    let hit = detect_line_tactic(&pre, &[re1], Color::White, 0, None);
    assert!(hit.is_none_or(|h| h.pattern != TacticPattern::Skewer));
}

// ---- detect_discovered_attack ---------------------------------------

#[test]
fn knight_move_unmasking_rook_onto_a_knight_is_a_discovered_attack() {
    // White Ne4 blocks Re1's view up the e-file to the undefended black
    // knight on e8. Ne4-c5 unmasks the attack. The target is a knight (it
    // can't fire back down the file), so the revealed rook is safe and the
    // discovery genuinely threatens material. No check, so it's a
    // discovered attack, not a discovered check.
    let pre = pos("k3n3/8/8/8/4N3/8/8/4R1K1 w - - 0 1");
    let nc5 = Move::normal(Square::E4, Square::C5);
    let hit = detect_line_tactic(&pre, &[nc5], Color::White, 0, None).expect("discovered attack");
    assert_eq!(hit.pattern, TacticPattern::DiscoveredAttack);
    assert_eq!(hit.primary_piece, Square::C5);
    assert_eq!(hit.targets, vec![Square::E8]);
}

// ---- detect_pin -----------------------------------------------------

#[test]
fn pinned_knight_attacked_by_a_pawn_is_a_pin() {
    // Black Nd6 is pinned to Kd8 by the white rook arriving on d1; a white
    // pawn on c5 attacks it. Pinned and attacked by something cheaper, the
    // knight can't escape — a pin that wins material.
    let pre = pos("3k4/8/3n4/2P5/8/8/8/R5K1 w - - 0 1");
    let rd1 = Move::normal(Square::A1, Square::D1);
    let hit = detect_line_tactic(&pre, &[rd1], Color::White, 0, None).expect("pin");
    assert_eq!(hit.pattern, TacticPattern::Pin);
    assert_eq!(hit.primary_piece, Square::D1);
    assert_eq!(hit.targets, vec![Square::D6]);
}

#[test]
fn no_pin_when_the_piece_is_not_pinned() {
    // Same shape but the black king is on a8, not behind the knight — the
    // knight isn't pinned, so no pin tactic.
    let pre = pos("k7/8/3n4/2P5/8/8/8/R5K1 w - - 0 1");
    let rd1 = Move::normal(Square::A1, Square::D1);
    let hit = detect_line_tactic(&pre, &[rd1], Color::White, 0, None);
    assert!(hit.is_none_or(|h| h.pattern != TacticPattern::Pin));
}

// ---- detect_relative_pin --------------------------------------------

#[test]
fn knight_pinned_to_queen_and_attacked_by_pawn_is_a_relative_pin() {
    // White Re1 X-rays through Black's Ne5 (front) to Qe7 (rear, more
    // valuable, NOT the king — king is parked on a8). d2-d4 attacks the
    // knight with a cheaper pawn: the knight can't stay (pawn takes it)
    // and can't flee (that drops the queen). A relative pin worth the
    // knight.
    let pre = pos("k7/4q3/8/4n3/8/8/3P4/4R1K1 w - - 0 1");
    let d4 = Move::normal(Square::D2, Square::D4);
    let hit = detect_line_tactic(&pre, &[d4], Color::White, 0, None).expect("relative pin");
    assert_eq!(hit.pattern, TacticPattern::RelativePin);
    assert_eq!(hit.primary_piece, Square::D4);
    // front (pinned knight) then rear (queen behind).
    assert_eq!(hit.targets, vec![Square::E5, Square::E7]);
    // Gain is the pinned piece we win (a knight), so a positive,
    // High-confidence hit even though d4 itself captures nothing.
    assert_eq!(hit.confidence, Confidence::High);
}

#[test]
fn no_relative_pin_when_rear_is_not_more_valuable() {
    // Same geometry but the rear piece is a pawn — moving the knight
    // costs nothing it can't afford, so there's no material-winning
    // relative pin (and a king-rear would be the absolute Pin instead).
    let pre = pos("k7/4p3/8/4n3/8/8/3P4/4R1K1 w - - 0 1");
    let d4 = Move::normal(Square::D2, Square::D4);
    let hit = detect_line_tactic(&pre, &[d4], Color::White, 0, None);
    assert!(hit.is_none_or(|h| h.pattern != TacticPattern::RelativePin));
}

// ---- sacrifice classification (cook.py:sacrifice) -------------------

// White queen on b1 takes the b7 pawn; the black rook on a7 recaptures,
// leaving white down a queen for a pawn. A textbook "material down after
// the second move" line (no geometric pattern fires on Qxb7 — it just
// takes a pawn). Reused across the sacrifice tests.
const QUEEN_SAC_FEN: &str = "6k1/rp6/8/8/8/8/8/1Q4K1 w - - 0 1";

fn queen_sac_line() -> Vec<Move> {
    vec![
        Move::normal(Square::B1, Square::B7), // Qxb7 (takes a pawn)
        Move::normal(Square::A7, Square::B7), // Rxb7 (wins the queen)
        Move::normal(Square::G1, Square::F2), // white shuffles, down Q for P
    ]
}

#[test]
fn is_sacrifice_detects_material_down_after_second_move() {
    let pre = pos(QUEEN_SAC_FEN);
    assert!(is_sacrifice(&pre, &queen_sac_line(), Color::White));
}

#[test]
fn is_sacrifice_false_for_a_winning_capture_line() {
    // The royal-fork line wins the rook — material goes up, not down.
    let pre = pos(ROYAL_FORK_FEN);
    let pv = [
        Move::normal(Square::B5, Square::C7),
        Move::normal(Square::E8, Square::D8),
        Move::normal(Square::C7, Square::A8),
    ];
    assert!(!is_sacrifice(&pre, &pv, Color::White));
}

#[test]
fn is_sacrifice_excluded_by_an_opponent_promotion() {
    // White rook takes b7, black rook recaptures (white down a rook for a
    // pawn), then black promotes a2-a1. The opponent promotion means the
    // deficit is the opponent queening, not a sacrifice — excluded.
    let pre = pos("6k1/rp6/8/8/8/8/p7/1R4K1 w - - 0 1");
    let pv = [
        Move::normal(Square::B1, Square::B7), // Rxb7
        Move::normal(Square::A7, Square::B7), // Rxb7
        Move::normal(Square::G1, Square::F1), // Kf1
        Move::promotion(Square::A2, Square::A1, PieceType::Queen), // a1=Q
    ];
    assert!(!is_sacrifice(&pre, &pv, Color::White));
}

#[test]
fn sound_sacrifice_is_played_and_suppresses_walked_into() {
    // Score 0 (equal) ⇒ the material-down line is a *sound* sacrifice. It
    // surfaces as a played Sacrifice tactic, and the opponent winning the
    // offered queen is NOT reported as "you walked into a free piece."
    let pre = pos(QUEEN_SAC_FEN);
    let ma = ma_with_pv(queen_sac_line(), Some(2));
    let outcome = compute_tactic_outcome(&ma, &ma, &pre, Color::White, None);

    let played = outcome.user_played_tactic.expect("sound sacrifice plays a tactic");
    assert_eq!(played.pattern, TacticPattern::Sacrifice);
    assert!(played.sacrifice);
    assert_eq!(played.confidence, Confidence::Medium); // material is down
    assert!(outcome.user_walked_into.is_none(), "sound sac must not read as walked-into");
}

#[test]
fn unsound_sacrifice_is_not_played_and_walked_into_fires() {
    // Same line, but the eval says white is losing (score < 0) ⇒ it's a
    // blunder, not a sacrifice. No played tactic; the opponent's Rxb7 is a
    // genuine free-piece capture the user walked into.
    let pre = pos(QUEEN_SAC_FEN);
    let losing = ma_with_pv_score(queen_sac_line(), Some(2), Value(-500));
    let outcome = compute_tactic_outcome(&losing, &losing, &pre, Color::White, None);

    assert!(outcome.user_played_tactic.is_none(), "a losing dump isn't a played tactic");
    assert_eq!(
        outcome.user_walked_into.map(|h| h.pattern),
        Some(TacticPattern::HangingCapture)
    );
}

#[test]
fn normal_tactic_hit_has_sacrifice_flag_false() {
    // A plain winning fork is not a sacrifice — the flag stays false.
    let pre = pos(ROYAL_FORK_FEN);
    let nc7 = Move::normal(Square::B5, Square::C7);
    let hit = detect_line_tactic(&pre, &[nc7], Color::White, 0, None).expect("fork");
    assert_eq!(hit.pattern, TacticPattern::Fork);
    assert!(!hit.sacrifice);
}

// ---- wave-4 multi-ply patterns --------------------------------------
//
// The per-detector fixtures (lichess-puzzler's own `tagger/test.py` cases)
// live in `detectors_tests.rs`, where each `detect_*` can be called in
// isolation. The full priority chain returns a single, highest-priority
// hit, and for several of those lines a more immediate pattern (a
// discovered check / skewer / removing-the-defender on the user's move
// itself) legitimately wins the slot — lichess assigns multiple tags, we
// surface one. The chain-level test below shows a wave-4 pattern reaching
// the verdict when nothing more immediate fires.

#[test]
fn intermezzo_fires_on_an_inserted_check() {
    // Black just played ...Bxd4 (capturing a knight); instead of recapturing
    // with Rxd4, White inserts Bxf7+ (the zwischenzug), and only after the
    // king moves does the rook take the bishop.
    let pre = pos("6k1/5p2/8/8/3b4/1B6/8/3R2K1 w - - 0 1");
    let pv = vec![
        Move::normal(Square::B3, Square::F7), // Bxf7+ (in-between check)
        Move::normal(Square::G8, Square::F7), // Kxf7
        Move::normal(Square::D1, Square::D4), // Rxd4 (delayed recapture)
    ];
    let prior = PriorMove {
        mv: Move::normal(Square::E5, Square::D4),
        captured: Some(PieceType::Knight),
    };
    let hit = detect_line_tactic(&pre, &pv, Color::White, 0, Some(prior)).expect("intermezzo");
    assert_eq!(hit.pattern, TacticPattern::Intermezzo);
    assert_eq!(hit.pv_ply, 2);
    assert_eq!(hit.primary_piece, Square::D4);
}


// ---- find_best_tactic_in_position: static scan ---------------------

#[test]
fn static_scan_finds_user_reported_nxc7_fork() {
    // User-reported case: a custom FEN with the white knight on b5 and
    // a black pawn on c7 (no defender of c7). Nxc7+ forks the king on
    // e8 and the rook on a8 — a one-look-ahead fork detectable without
    // any search. This is the coaching-panel's move-1 fallback path.
    let pos = pos(
        "rnb1kbnr/pppppppp/8/1N6/8/8/PPPPPPPP/R1BQKBNR w KQkq - 0 1",
    );
    let hit = find_best_tactic_in_position(&pos, Color::White, None)
        .expect("Nxc7+ fork should be found by static scan");
    assert_eq!(hit.pattern, TacticPattern::Fork);
    assert_eq!(hit.primary_piece, Square::C7);
    assert!(hit.targets.contains(&Square::A8));
    assert!(hit.targets.contains(&Square::E8));
    assert_eq!(hit.confidence, Confidence::High);
}

#[test]
fn static_scan_returns_none_when_no_tactic_exists() {
    // Standard starting position has no winnable tactic for white.
    // The static scan should fall silent rather than nag.
    let pos = Position::startpos();
    assert!(find_best_tactic_in_position(&pos, Color::White, None).is_none());
}

#[test]
fn static_scan_prefers_mate_over_material_fork() {
    // White to move has both a fork available AND a mate-in-1; the
    // ranker should pick the mate. Position: white queen on h5, black
    // king on f8 with the back rank cleared by both knight-on-g6 (mate-in-1
    // via Qh8#) AND a clear knight fork available.
    //
    // Simpler hand-built variant: a one-move mate alongside a less-
    // urgent fork by another piece is hard to set up cleanly; instead
    // assert the rank ordering via a synthetic side-by-side comparison.
    let mate = TacticHit {
        pattern: TacticPattern::Checkmate,
        pv_ply: 0,
        primary_piece: Square::H8,
        targets: vec![Square::F8],
        material_gain: None,
        confidence: Confidence::High,
        sacrifice: false,
        mate_pattern: Some(MatePattern::BackRank),
        key_move: None,
    };
    let fork = TacticHit {
        pattern: TacticPattern::Fork,
        pv_ply: 0,
        primary_piece: Square::C7,
        targets: vec![Square::A8, Square::E8],
        material_gain: Some(500),
        confidence: Confidence::High,
        sacrifice: false,
        mate_pattern: None,
        key_move: None,
    };
    assert!(super::hit_outranks(&mate, &fork));
    assert!(!super::hit_outranks(&fork, &mate));
}
