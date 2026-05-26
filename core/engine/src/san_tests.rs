use super::*;

fn parse_fen(s: &str) -> Position {
    Position::from_fen(s).expect("valid FEN")
}

// ---- parsing ---------------------------------------------------------

#[test]
fn parses_pawn_push() {
    let mut pos = Position::startpos();
    let mv = parse(&mut pos, "e4").unwrap();
    assert_eq!(mv.from(), Square::E2);
    assert_eq!(mv.to(), Square::E4);
}

#[test]
fn parses_knight_move() {
    let mut pos = Position::startpos();
    let mv = parse(&mut pos, "Nf3").unwrap();
    assert_eq!(mv.from(), Square::G1);
    assert_eq!(mv.to(), Square::F3);
}

#[test]
fn parses_capture_with_x() {
    let mut pos = parse_fen("rnbqkbnr/ppp1pppp/8/3p4/4P3/8/PPPP1PPP/RNBQKBNR w KQkq d6 0 2");
    let mv = parse(&mut pos, "exd5").unwrap();
    assert_eq!(mv.from(), Square::E4);
    assert_eq!(mv.to(), Square::D5);
}

#[test]
fn parses_capture_without_x() {
    // Same position as above, but user types a lenient `ed5`.
    let mut pos = parse_fen("rnbqkbnr/ppp1pppp/8/3p4/4P3/8/PPPP1PPP/RNBQKBNR w KQkq d6 0 2");
    let mv = parse(&mut pos, "ed5").unwrap();
    assert_eq!(mv.from(), Square::E4);
    assert_eq!(mv.to(), Square::D5);
}

#[test]
fn parses_lenient_missing_x_and_check() {
    // Black queen to c6 captures a bishop and checks the white king
    // on c1 along the c-file. Real SAN is `Qxc6+`; user types `Qc6`.
    let mut pos = parse_fen("4k3/3q4/2B5/8/8/8/8/2K5 b - - 0 1");
    let mv = parse(&mut pos, "Qc6").unwrap();
    assert_eq!(mv.from(), Square::D7);
    assert_eq!(mv.to(), Square::C6);
}

#[test]
fn parses_castling_both_notations() {
    let fen = "r3k2r/pppbqppp/2n2n2/3pp3/3PP3/2N2N2/PPPBQPPP/R3K2R w KQkq - 0 1";
    for s in ["O-O", "0-0"] {
        let mut pos = parse_fen(fen);
        let mv = parse(&mut pos, s).unwrap();
        assert_eq!(mv.kind(), MoveKind::Castling);
        assert_eq!(mv.to(), Square::G1);
    }
    for s in ["O-O-O", "0-0-0"] {
        let mut pos = parse_fen(fen);
        let mv = parse(&mut pos, s).unwrap();
        assert_eq!(mv.kind(), MoveKind::Castling);
        assert_eq!(mv.to(), Square::C1);
    }
}

#[test]
fn parses_promotion() {
    let mut pos = parse_fen("8/4P3/8/8/8/8/8/4K2k w - - 0 1");
    let mv = parse(&mut pos, "e8=Q").unwrap();
    assert_eq!(mv.kind(), MoveKind::Promotion);
    assert_eq!(mv.promoted_to(), PieceType::Queen);
}

#[test]
fn parses_promotion_without_equals() {
    let mut pos = parse_fen("8/4P3/8/8/8/8/8/4K2k w - - 0 1");
    let mv = parse(&mut pos, "e8Q").unwrap();
    assert_eq!(mv.promoted_to(), PieceType::Queen);
}

#[test]
fn parses_promotion_with_check() {
    let mut pos = parse_fen("8/4P3/8/8/8/8/8/4K2k w - - 0 1");
    let mv = parse(&mut pos, "e8=Q+").unwrap();
    assert_eq!(mv.promoted_to(), PieceType::Queen);
}

#[test]
fn rejects_unknown_piece_letter() {
    let mut pos = Position::startpos();
    assert!(parse(&mut pos, "Ze4").is_err());
}

#[test]
fn reports_ambiguity() {
    // Two knights on b1 and f1; both reach d2.
    let mut pos = parse_fen("4k3/8/8/8/8/8/8/1N1K1N2 w - - 0 1");
    assert!(parse(&mut pos, "Nd2").is_err(), "Nd2 should be ambiguous");
}

#[test]
fn disambig_required_by_rank() {
    // Two rooks on the c-file with c2 empty between them. Both
    // can move to c2; file disambig is useless (both on c), so
    // the rank digit is what makes SAN unique.
    let mut pos = parse_fen("4k3/8/8/8/8/2R5/8/2R1K3 w - - 0 1");
    assert!(parse(&mut pos, "Rc2").is_err(), "Rc2 should be ambiguous");
    let mv = parse(&mut pos, "R1c2").unwrap();
    assert_eq!(mv.from(), Square::C1);
    let mv = parse(&mut pos, "R3c2").unwrap();
    assert_eq!(mv.from(), Square::C3);
}

// ---- formatting ------------------------------------------------------

#[test]
fn formats_pawn_push() {
    let mut pos = Position::startpos();
    let mv = parse(&mut pos, "e4").unwrap();
    assert_eq!(format(&pos, mv), "e4");
}

#[test]
fn formats_capture() {
    let mut pos = parse_fen("rnbqkbnr/ppp1pppp/8/3p4/4P3/8/PPPP1PPP/RNBQKBNR w KQkq d6 0 2");
    let mv = parse(&mut pos, "exd5").unwrap();
    assert_eq!(format(&pos, mv), "exd5");
}

#[test]
fn formats_with_check() {
    // Rf8 delivers check. Use a position where a rook move checks.
    let mut pos = parse_fen("4k3/8/8/8/8/8/8/4K2R w - - 0 1");
    let mv = parse(&mut pos, "Rh8").unwrap();
    assert_eq!(format(&pos, mv), "Rh8+");
}

#[test]
fn formats_mate() {
    // Back-rank mate: rook to e8 with black king trapped by its own
    // pawns on f7/g7/h7.
    let mut pos = parse_fen("6k1/5ppp/8/8/8/8/5PPP/4R1K1 w - - 0 1");
    let mv = parse(&mut pos, "Re8").unwrap();
    assert_eq!(format(&pos, mv), "Re8#");
}

#[test]
fn formats_castling() {
    let mut pos = parse_fen("r3k2r/pppbqppp/2n2n2/3pp3/3PP3/2N2N2/PPPBQPPP/R3K2R w KQkq - 0 1");
    let mv = parse(&mut pos, "O-O").unwrap();
    assert_eq!(format(&pos, mv), "O-O");
    let mut pos2 =
        parse_fen("r3k2r/pppbqppp/2n2n2/3pp3/3PP3/2N2N2/PPPBQPPP/R3K2R w KQkq - 0 1");
    let mv = parse(&mut pos2, "O-O-O").unwrap();
    assert_eq!(format(&pos2, mv), "O-O-O");
}

#[test]
fn formats_promotion() {
    // Black king on h8: promoting to a queen on e8 checks along
    // rank 8 but the king can escape to h7.
    let mut pos = parse_fen("7k/4P3/8/8/8/8/8/4K3 w - - 0 1");
    let mv = parse(&mut pos, "e8=Q").unwrap();
    assert_eq!(format(&pos, mv), "e8=Q+");
}

#[test]
fn formats_file_disambig() {
    // Two knights on b1 and f1; both reach d2.
    let mut pos = parse_fen("4k3/8/8/8/8/8/8/1N1K1N2 w - - 0 1");
    let mv_b = parse(&mut pos, "Nbd2").unwrap();
    assert_eq!(format(&pos, mv_b), "Nbd2");
    let mv_f = parse(&mut pos, "Nfd2").unwrap();
    assert_eq!(format(&pos, mv_f), "Nfd2");
}

#[test]
fn formats_rank_disambig() {
    let mut pos = parse_fen("4k3/8/8/8/R7/8/R7/4K3 w - - 0 1");
    let mv = parse(&mut pos, "R2a3").unwrap();
    assert_eq!(format(&pos, mv), "R2a3");
}
