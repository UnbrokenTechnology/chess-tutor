use super::*;
use crate::bitboard::square_bb;
// ---- InvariantKind ----------------------------------------------
#[test]
fn piece_on_matches_an_actual_piece() {
    let pos = Position::startpos();
    assert!(check_invariant(
        &pos,
        &InvariantKind::PieceOn {
            square: Square::E1,
            piece: Piece::WhiteKing
        }
    ));
    assert!(!check_invariant(
        &pos,
        &InvariantKind::PieceOn {
            square: Square::E1,
            piece: Piece::BlackKing
        }
    ));
}
#[test]
fn square_empty_and_all_empty_agree() {
    let pos = Position::startpos();
    assert!(check_invariant(
        &pos,
        &InvariantKind::SquareEmpty { square: Square::E4 }
    ));
    assert!(!check_invariant(
        &pos,
        &InvariantKind::SquareEmpty { square: Square::E2 }
    ));
    let mid_board = square_bb(Square::E4) | square_bb(Square::D4) | square_bb(Square::F4);
    assert!(check_invariant(
        &pos,
        &InvariantKind::AllEmpty { mask: mid_board }
    ));
}
#[test]
fn any_piece_of_color_lights_up_friendly_squares() {
    let pos = Position::startpos();
    assert!(check_invariant(
        &pos,
        &InvariantKind::AnyPieceOfColor {
            color: Color::Black,
            square: Square::F8
        }
    ));
    assert!(!check_invariant(
        &pos,
        &InvariantKind::AnyPieceOfColor {
            color: Color::White,
            square: Square::F8
        }
    ));
}
#[test]
fn piece_count_and_no_piece_in_mask() {
    let pos = Position::startpos();
    assert!(check_invariant(
        &pos,
        &InvariantKind::PieceCount {
            color: Color::White,
            piece_type: PieceType::Pawn,
            count: 8,
        }
    ));
    // No white knights on rank 4.
    let rank4 = crate::bitboard::rank_bb(crate::types::Rank::R4);
    assert!(check_invariant(
        &pos,
        &InvariantKind::NoPieceInMask {
            color: Color::White,
            piece_type: PieceType::Knight,
            mask: rank4,
        }
    ));
}
#[test]
fn attacker_count_and_not_attacked_by() {
    // After 1.e4 e5 2.Nf3 f6: black's f6 pawn is the only
    // defender of e5, and h5 is currently not attacked by black.
    let mut pos = Position::startpos();
    for san_text in ["e4", "e5", "Nf3", "f6"] {
        let mv = san::parse(&mut pos, san_text).unwrap();
        let _ = pos.do_move(mv);
    }
    assert!(check_invariant(
        &pos,
        &InvariantKind::AttackerCountByColor {
            color: Color::Black,
            square: Square::E5,
            count: 1,
        }
    ));
    assert!(check_invariant(
        &pos,
        &InvariantKind::NotAttackedBy {
            color: Color::Black,
            square: Square::H5
        }
    ));
    assert!(check_invariant(
        &pos,
        &InvariantKind::AttackersEqual {
            color: Color::Black,
            square: Square::E5,
            mask: square_bb(Square::F6),
        }
    ));
}
#[test]
fn ray_clear_sees_through_empty_squares() {
    // Startpos: a queen on d1 does NOT see h5 (path d1→e2 is
    // blocked by the white e-pawn).
    let pos = Position::startpos();
    assert!(!check_invariant(
        &pos,
        &InvariantKind::RayClear {
            from: Square::D1,
            to: Square::H5
        }
    ));
    // After 1.e4 e5 2.Nf3 f6, a queen on h5 WOULD see e5 along
    // rank 5 (f5 and g5 are empty), and would also see e8 along
    // the h5-e8 diagonal (g6 and f7 are empty since 2...f6
    // vacated f7).
    let mut pos = Position::startpos();
    for san_text in ["e4", "e5", "Nf3", "f6"] {
        let mv = san::parse(&mut pos, san_text).unwrap();
        let _ = pos.do_move(mv);
    }
    assert!(check_invariant(
        &pos,
        &InvariantKind::RayClear {
            from: Square::H5,
            to: Square::E5
        }
    ));
    assert!(check_invariant(
        &pos,
        &InvariantKind::RayClear {
            from: Square::H5,
            to: Square::E8
        }
    ));
}
#[test]
fn ray_clear_rejects_non_aligned_squares() {
    // d1 and e3 are not on a shared rank / file / diagonal.
    let pos = Position::startpos();
    assert!(!check_invariant(
        &pos,
        &InvariantKind::RayClear {
            from: Square::D1,
            to: Square::E3
        }
    ));
}
// ---- TriggerPattern ---------------------------------------------
#[test]
fn trigger_pattern_matches_with_and_without_from() {
    let mut pos = Position::startpos();
    for san_text in ["e4", "e5", "Nf3"] {
        let mv = san::parse(&mut pos, san_text).unwrap();
        let _ = pos.do_move(mv);
    }
    // Build the Damiano trigger: black pawn to f6.
    let f6_move = san::parse(&mut pos.clone(), "f6").unwrap();
    let trigger_wildcard = TriggerPattern {
        mover: Color::Black,
        piece_type: PieceType::Pawn,
        to: Square::F6,
        from: None,
    };
    let trigger_strict = TriggerPattern {
        mover: Color::Black,
        piece_type: PieceType::Pawn,
        to: Square::F6,
        from: Some(Square::F7),
    };
    assert!(trigger_wildcard.matches(Color::Black, &pos, f6_move));
    assert!(trigger_strict.matches(Color::Black, &pos, f6_move));
    // Wrong side to move — should reject even if the move is
    // otherwise the right shape.
    assert!(!trigger_wildcard.matches(Color::White, &pos, f6_move));
}
// ---- Scan from the start position --------------------------------
#[test]
fn startpos_has_no_pre_move_threats() {
    // No trap in the library fires from the standard start position —
    // none of their triggers (e.g. Damiano's ...f6) are matched by
    // the side-to-move's legal set here.
    let pos = Position::startpos();
    assert!(scan_threats(&pos).is_empty());
}
#[test]
fn scan_after_move_is_empty_when_no_trigger_matches() {
    // 1.Nc3 isn't any library trap's trigger, so nothing fires.
    let mut pos = Position::startpos();
    let mv = san::parse(&mut pos, "Nc3").unwrap();
    let after = {
        let mut p = pos.clone();
        let _ = p.do_move(mv);
        p
    };
    assert!(scan_after_move(
        &after,
        Color::White,
        PieceType::Knight,
        Square::B1,
        Square::C3
    )
    .is_empty());
}
