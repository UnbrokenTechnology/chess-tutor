use super::super::test_support::ma_with_pv;
use super::*;
use crate::position::Position;
use crate::types::{Color, Move, PieceType, Square};

/// Count pieces of both colours that are attacked and undefended.
fn count_hanging(pos: &Position, root_stm: Color) -> (usize, usize) {
    (
        list_hanging(pos, root_stm).len(),
        list_hanging(pos, !root_stm).len(),
    )
}

#[test]
fn threats_outcome_empty_when_no_hangs_pre_or_post() {
    let pos = Position::startpos();
    let e4 = Move::normal(Square::E2, Square::E4);
    let ma = ma_with_pv(vec![e4], Some(0));
    let outcome = compute_threats_outcome(&ma, &pos, Color::White);
    assert!(outcome.ours_hanging.is_empty());
    assert!(outcome.theirs_hanging.is_empty());
    assert_eq!(outcome.ours_hanging_delta, 0);
    assert_eq!(outcome.theirs_hanging_delta, 0);
}

#[test]
fn threats_outcome_detects_move_that_creates_our_hang() {
    let fen = "4k3/8/8/8/8/4p3/8/1N4K1 w - - 0 1";
    let pos = Position::from_fen(fen).unwrap();
    let (pre_ours, pre_theirs) = count_hanging(&pos, Color::White);
    assert_eq!(pre_ours, 0);
    assert_eq!(pre_theirs, 0);

    let nd2 = Move::normal(Square::B1, Square::D2);
    let ma = ma_with_pv(vec![nd2], Some(0));
    let outcome = compute_threats_outcome(&ma, &pos, Color::White);
    let hanging = outcome
        .ours_hanging
        .iter()
        .find(|p| p.location.square == Square::D2 && p.location.piece == PieceType::Knight)
        .unwrap_or_else(|| {
            panic!(
                "expected our knight on d2 to be hanging, got {:?}",
                outcome.ours_hanging
            )
        });
    assert_eq!(outcome.ours_hanging_delta, 1);
    assert_eq!(hanging.attackers.len(), 1);
    assert_eq!(hanging.attackers[0].square, Square::E3);
    assert_eq!(hanging.attackers[0].piece, PieceType::Pawn);
}

#[test]
fn threats_outcome_no_hangs_when_defender_present() {
    let fen = "4k3/8/8/8/8/4p3/4K3/1N6 w - - 0 1";
    let pos = Position::from_fen(fen).unwrap();
    let nd2 = Move::normal(Square::B1, Square::D2);
    let ma = ma_with_pv(vec![nd2], Some(0));
    let outcome = compute_threats_outcome(&ma, &pos, Color::White);
    assert_eq!(outcome.ours_hanging_delta, 0);
    assert_eq!(outcome.theirs_hanging_delta, 0);
}

#[test]
fn threats_outcome_sign_flipped_for_white_pov() {
    let fen = "1n4k1/8/4P3/8/8/8/8/4K3 b - - 0 1";
    let pos = Position::from_fen(fen).unwrap();
    let nd7 = Move::normal(Square::B8, Square::D7);
    let ma = ma_with_pv(vec![nd7], Some(0));
    let outcome = compute_threats_outcome(&ma, &pos, Color::White);
    let hanging = outcome
        .theirs_hanging
        .iter()
        .find(|p| p.location.square == Square::D7 && p.location.piece == PieceType::Knight)
        .unwrap_or_else(|| {
            panic!(
                "expected opponent's knight on d7 to be hanging from white POV, got {:?}",
                outcome.theirs_hanging
            )
        });
    assert_eq!(outcome.theirs_hanging_delta, 1);
    assert_eq!(hanging.attackers.len(), 1);
    assert_eq!(hanging.attackers[0].square, Square::E6);
    assert_eq!(hanging.attackers[0].piece, PieceType::Pawn);
}

#[test]
fn threats_outcome_empty_pv_uses_pre_move_position() {
    let pos = Position::startpos();
    let ma = ma_with_pv(Vec::new(), None);
    let outcome = compute_threats_outcome(&ma, &pos, Color::White);
    assert!(outcome.ours_hanging.is_empty());
    assert!(outcome.theirs_hanging.is_empty());
}

#[test]
fn threats_outcome_records_multiple_attackers() {
    // Black knight on d5 attacked by d1 rook + e4 pawn; no
    // black defenders.
    let fen = "4k3/8/8/3n4/4P3/8/8/3R2K1 b - - 0 1";
    let pos = Position::from_fen(fen).unwrap();
    let hanging = list_hanging(&pos, Color::Black);
    let knight = hanging
        .iter()
        .find(|p| p.location.square == Square::D5)
        .expect("knight on d5 should be hanging");
    assert_eq!(knight.attackers.len(), 2);
    // Attackers ordered by ascending square index — d1 (3)
    // before e4 (28).
    assert_eq!(knight.attackers[0].square, Square::D1);
    assert_eq!(knight.attackers[0].piece, PieceType::Rook);
    assert_eq!(knight.attackers[1].square, Square::E4);
    assert_eq!(knight.attackers[1].piece, PieceType::Pawn);
}

// ---- list_see_losing ---------------------------------------------

#[test]
fn see_losing_flags_defended_piece_overloaded_by_cheap_attackers() {
    let fen = "4k3/8/3p4/4N3/6n1/8/8/4R1K1 b - - 0 1";
    let pos = Position::from_fen(fen).unwrap();
    let see_losing = list_see_losing(&pos, Color::White);
    let entry = see_losing
        .iter()
        .find(|p| p.location.square == Square::E5)
        .expect("e5 knight should be SEE-losing");
    assert_eq!(entry.location.piece, PieceType::Knight);
    assert_eq!(entry.attackers.len(), 2);
}

#[test]
fn see_losing_does_not_flag_equal_defended_trade() {
    let fen = "k3r3/8/8/4R3/8/8/8/K3R3 b - - 0 1";
    let pos = Position::from_fen(fen).unwrap();
    let see_losing = list_see_losing(&pos, Color::White);
    assert!(
        see_losing.iter().all(|p| p.location.square != Square::E5),
        "even-trade rook should not be flagged, got {:?}",
        see_losing
    );
}

#[test]
fn see_losing_skips_strictly_hanging_piece() {
    let fen = "4k3/8/8/8/8/4p3/8/1N4K1 w - - 0 1";
    let pos = Position::from_fen(fen).unwrap();
    let mut scratch = pos.clone();
    scratch.do_move(Move::normal(Square::B1, Square::D2));
    let see_losing = list_see_losing(&scratch, Color::White);
    assert!(
        see_losing.is_empty(),
        "hanging-only pieces belong on the hanging list, got {:?}",
        see_losing
    );
}

#[test]
fn compute_threats_outcome_populates_see_losing_delta() {
    let pre_fen = "4k3/3p4/8/4N3/6n1/8/8/4R1K1 b - - 0 1";
    let pre = Position::from_fen(pre_fen).unwrap();
    let push = Move::normal(Square::D7, Square::D6);
    let ma = ma_with_pv(vec![push], Some(0));
    let outcome = compute_threats_outcome(&ma, &pre, Color::White);
    assert_eq!(
        outcome.ours_see_losing_delta, 1,
        "d7-d6 should create one SEE-losing piece on our side"
    );
    assert_eq!(outcome.theirs_see_losing_delta, 0);
}

#[test]
fn see_losing_skips_king_only_attacker_against_defended_target() {
    // The Qxf5# scenario: black queen on f5 is "attacked" by the
    // white king on f4 (only attacker), defended by the black
    // knight on d4. The king can't legally take a defended piece,
    // so the queen is NOT SEE-losing. Without the king-exclusion
    // filter, Value::mg_of_piece(King) == 0 makes the king look
    // like a free-of-cost first captor and SEE returns a phantom
    // losing verdict.
    let fen = "7k/8/8/5q2/3n1K2/8/8/8 w - - 0 1";
    let pos = Position::from_fen(fen).unwrap();
    let see_losing = list_see_losing(&pos, Color::Black);
    assert!(
        see_losing.iter().all(|p| p.location.square != Square::F5),
        "queen on f5 must not be flagged as SEE-losing — only attacker is the king, which can't capture a defended piece; got {see_losing:?}"
    );
}

#[test]
fn see_losing_still_fires_when_king_is_one_of_several_attackers() {
    // Two attackers on the target: cheapest non-king should still
    // drive the SEE call. Black queen on e5, attacked by white
    // king on e4 AND white knight on g4 (knight reaches e5),
    // defended by black knight on c4.
    // Nxe5 — Nxe5 sequence: white wins queen for knight (+ ~6).
    let fen = "7k/8/8/4q3/2n1K1N1/8/8/8 b - - 0 1";
    let pos = Position::from_fen(fen).unwrap();
    let see_losing = list_see_losing(&pos, Color::Black);
    let entry = see_losing
        .iter()
        .find(|p| p.location.square == Square::E5)
        .unwrap_or_else(|| {
            panic!("queen on e5 should still be SEE-losing via the knight attacker, got {see_losing:?}")
        });
    // The displayed attackers list intentionally still includes
    // the king — it's a geometric attacker even though it can't
    // legally initiate.
    assert_eq!(entry.attackers.len(), 2);
}

// ---- list_pressured ---------------------------------------------

#[test]
fn list_pressured_safe_pawn_threat_fires_against_minor() {
    let fen = "4k3/8/5n2/4P3/8/8/8/4K3 w - - 0 1";
    let pos = Position::from_fen(fen).unwrap();
    let pressured = list_pressured(&pos, Color::Black);
    let entry = pressured
        .iter()
        .find(|p| p.location.square == Square::F6)
        .unwrap_or_else(|| panic!("expected f6 knight in pressured list, got {pressured:?}"));
    assert_eq!(entry.kind, PressureKind::SafePawnThreat);
    assert_eq!(entry.location.piece, PieceType::Knight);
    assert_eq!(entry.attackers.len(), 1);
    assert_eq!(entry.attackers[0].square, Square::E5);
    assert_eq!(entry.attackers[0].piece, PieceType::Pawn);
}

#[test]
fn list_pressured_unsafe_pawn_threat_does_not_fire() {
    let fen = "4k3/8/3p1n2/4P3/8/8/8/4K3 w - - 0 1";
    let pos = Position::from_fen(fen).unwrap();
    let pressured = list_pressured(&pos, Color::Black);
    assert!(
        pressured.iter().all(|p| p.location.square != Square::F6
            || p.kind != PressureKind::SafePawnThreat),
        "f6 knight should not appear under SafePawnThreat when attacker pawn is itself attacked, got {pressured:?}",
    );
}

#[test]
fn list_pressured_minor_on_major_fires() {
    let fen = "4k3/r7/2N5/8/8/8/8/4K3 w - - 0 1";
    let pos = Position::from_fen(fen).unwrap();
    let pressured = list_pressured(&pos, Color::Black);
    let entry = pressured
        .iter()
        .find(|p| p.location.square == Square::A7 && p.kind == PressureKind::MinorOnMajor)
        .unwrap_or_else(|| panic!("expected a7 rook MinorOnMajor entry, got {pressured:?}"));
    assert_eq!(entry.location.piece, PieceType::Rook);
    assert_eq!(entry.attackers.len(), 1);
    assert_eq!(entry.attackers[0].square, Square::C6);
    assert_eq!(entry.attackers[0].piece, PieceType::Knight);
}

#[test]
fn list_pressured_rook_on_queen_fires() {
    let fen = "3q1k2/8/8/8/8/8/8/3R2K1 w - - 0 1";
    let pos = Position::from_fen(fen).unwrap();
    let pressured = list_pressured(&pos, Color::Black);
    let entry = pressured
        .iter()
        .find(|p| p.location.square == Square::D8 && p.kind == PressureKind::RookOnQueen)
        .unwrap_or_else(|| panic!("expected d8 queen RookOnQueen entry, got {pressured:?}"));
    assert_eq!(entry.location.piece, PieceType::Queen);
    assert_eq!(entry.attackers.len(), 1);
    assert_eq!(entry.attackers[0].square, Square::D1);
    assert_eq!(entry.attackers[0].piece, PieceType::Rook);
}

#[test]
fn list_pressured_no_dedup_with_hanging() {
    let fen = "4k3/r7/2N5/8/8/8/8/4K3 w - - 0 1";
    let pos = Position::from_fen(fen).unwrap();
    let hanging = list_hanging(&pos, Color::Black);
    let pressured = list_pressured(&pos, Color::Black);
    assert!(
        hanging.iter().any(|h| h.location.square == Square::A7),
        "a7 rook should be hanging in this position",
    );
    assert!(
        pressured.iter().any(|p| p.location.square == Square::A7),
        "list_pressured should NOT filter out the hanging rook — found {pressured:?}",
    );
}

#[test]
fn compute_threats_outcome_populates_pressured_delta() {
    let pre_fen = "1N2k3/r7/8/8/8/8/8/6K1 w - - 0 1";
    let pre = Position::from_fen(pre_fen).unwrap();
    let pre_pressured = list_pressured(&pre, Color::Black);
    assert!(
        pre_pressured
            .iter()
            .all(|p| p.location.square != Square::A7),
        "pre-move should have no pressure on a7, got {pre_pressured:?}",
    );

    let nc6 = Move::normal(Square::B8, Square::C6);
    let ma = ma_with_pv(vec![nc6], Some(0));
    let outcome = compute_threats_outcome(&ma, &pre, Color::White);
    assert_eq!(
        outcome.theirs_pressured_delta, 1,
        "Nc6 should create one new pressure on the opponent's a7 rook"
    );
    let entry = outcome
        .theirs_pressured
        .iter()
        .find(|p| p.location.square == Square::A7)
        .unwrap_or_else(|| {
            panic!(
                "expected a7 in theirs_pressured, got {:?}",
                outcome.theirs_pressured
            )
        });
    assert_eq!(entry.kind, PressureKind::MinorOnMajor);
}

#[test]
fn list_pressured_minor_on_minor_does_not_fire() {
    let fen = "4k3/5n2/8/8/2B5/8/8/4K3 w - - 0 1";
    let pos = Position::from_fen(fen).unwrap();
    let pressured = list_pressured(&pos, Color::Black);
    assert!(
        pressured
            .iter()
            .all(|p| p.kind != PressureKind::MinorOnMajor),
        "MinorOnMajor must require the target to be rook or queen, got {pressured:?}",
    );
}

#[test]
fn threats_outcome_ignores_kings() {
    let fen = "4k3/8/8/8/8/8/4Q3/4K3 b - - 0 1";
    let pos = Position::from_fen(fen).unwrap();
    let (ours, theirs) = count_hanging(&pos, Color::Black);
    assert_eq!(ours, 0, "king in check must not count as hanging");
    assert_eq!(theirs, 0, "white king should not count either");
}

// ---- filter_guaranteed_targets ----------------------------------

#[test]
fn guarantee_filter_drops_e5_after_nf3_because_nc6_defends() {
    // After 1.e4 e5 2.Nf3 — Black to move. e5 looks hanging
    // statically (attacked by Nf3, no defender), but 2...Nc6
    // defends. The guarantee filter must drop e5 so the
    // retrospective doesn't tell the student they can win the
    // pawn — Nxe5? would lose a knight after ...Nxe5.
    let fen = "rnbqkbnr/pppp1ppp/8/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R b KQkq - 1 2";
    let pos = Position::from_fen(fen).unwrap();
    let hanging = list_hanging(&pos, Color::Black);
    assert!(
        hanging.iter().any(|h| h.location.square == Square::E5),
        "static check should still flag e5 as hanging (precondition for test)"
    );
    let guaranteed = filter_guaranteed_targets(&pos, &hanging, Color::White);
    assert!(
        guaranteed.iter().all(|h| h.location.square != Square::E5),
        "Nc6 defends e5 — guarantee filter must drop the entry, got {guaranteed:?}"
    );
}

#[test]
fn guarantee_filter_keeps_target_when_opponent_is_in_stalemate() {
    // Edge-case branch: when the opponent has no legal moves at
    // all, every static target is trivially "guaranteed"
    // (there's no response that could refute). Game-over
    // territory; teaching value moot. But the branch should
    // exercise so we know it doesn't panic.
    // Stalemate position: black king on h8, white queen on g6,
    // white king on f7. Black to move, no legal moves.
    let fen = "7k/5K2/6Q1/8/8/8/8/8 b - - 0 1";
    let pos = Position::from_fen(fen).unwrap();
    // Construct a synthetic HangingPiece (the test doesn't care
    // whether it's actually hanging — the filter's stalemate
    // short-circuit returns true unconditionally).
    let synthetic = HangingPiece {
        location: PieceLocation {
            square: Square::H8,
            piece: PieceType::King,
        },
        attackers: Vec::new(),
    };
    let kept = filter_guaranteed_targets(&pos, &[synthetic.clone()], Color::White);
    assert_eq!(kept.len(), 1, "stalemate branch must keep all targets");
}
