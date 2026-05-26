use super::*;

// ---- FEN roundtrip -----------------------------------------------

const STARTPOS_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

#[test]
fn fen_roundtrips_startpos() {
    let p = Position::from_fen(STARTPOS_FEN).unwrap();
    assert_eq!(p.to_fen(), STARTPOS_FEN);
}

#[test]
fn fen_roundtrips_italian_after_four_halfmoves() {
    // 1. e4 e5 2. Nf3 Nc6
    let fen = "r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3";
    let p = Position::from_fen(fen).unwrap();
    assert_eq!(p.to_fen(), fen);
    assert_eq!(p.piece_on(Square::E4), Some(Piece::WhitePawn));
    assert_eq!(p.piece_on(Square::E5), Some(Piece::BlackPawn));
    assert_eq!(p.piece_on(Square::F3), Some(Piece::WhiteKnight));
    assert_eq!(p.piece_on(Square::C6), Some(Piece::BlackKnight));
    assert_eq!(p.side_to_move(), Color::White);
}

#[test]
fn fen_roundtrips_en_passant_when_capturable() {
    // En passant is only recorded when a side-to-move pawn can
    // actually capture onto it (SF11 position.cpp:262-273). White's
    // e5 pawn can play exd6, so d6 is a real ep target and survives
    // the round-trip.
    let fen = "4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1";
    let p = Position::from_fen(fen).unwrap();
    assert_eq!(p.en_passant(), Some(Square::D6));
    assert_eq!(p.to_fen(), fen);
}

#[test]
fn fen_drops_phantom_en_passant_with_no_capturer() {
    // After 1. e4 from the start no black pawn can capture on e3, so
    // the ep square is dropped — matching SF11 and keeping key() in
    // sync with the do_move path (the same position reached by playing
    // 1. e4 also has no ep square).
    let fen = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1";
    let p = Position::from_fen(fen).unwrap();
    assert_eq!(p.en_passant(), None);
    assert_eq!(
        p.to_fen(),
        "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1"
    );
}

#[test]
fn fen_roundtrips_no_castling_rights() {
    let fen = "4k3/8/8/8/8/8/8/4K3 w - - 0 1";
    let p = Position::from_fen(fen).unwrap();
    assert_eq!(p.castling_rights(), CastlingRights::NONE);
    assert_eq!(p.to_fen(), fen);
}

#[test]
fn fen_roundtrips_partial_castling_rights() {
    // White can only castle kingside, black can only castle queenside.
    let fen = "r3k2r/8/8/8/8/8/8/R3K2R w Kq - 0 1";
    let p = Position::from_fen(fen).unwrap();
    assert!(p.castling_rights().contains(CastlingRights::WHITE_KING));
    assert!(!p.castling_rights().contains(CastlingRights::WHITE_QUEEN));
    assert!(!p.castling_rights().contains(CastlingRights::BLACK_KING));
    assert!(p.castling_rights().contains(CastlingRights::BLACK_QUEEN));
    assert_eq!(p.to_fen(), fen);
}

// ---- FEN validation ---------------------------------------------

#[test]
fn fen_rejects_missing_fields() {
    assert!(matches!(
        Position::from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR"),
        Err(FenError::MissingField(_))
    ));
}

#[test]
fn fen_rejects_wrong_rank_count() {
    let bad = "rnbqkbnr/pppppppp/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
    assert!(matches!(
        Position::from_fen(bad),
        Err(FenError::BadPiecePlacement(_))
    ));
}

#[test]
fn fen_rejects_rank_with_too_many_files() {
    let bad = "rnbqkbnr1/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
    assert!(matches!(
        Position::from_fen(bad),
        Err(FenError::BadPiecePlacement(_))
    ));
}

#[test]
fn fen_rejects_rank_with_too_few_files() {
    let bad = "rnbqkbnr/ppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
    assert!(matches!(
        Position::from_fen(bad),
        Err(FenError::BadPiecePlacement(_))
    ));
}

#[test]
fn fen_rejects_unknown_piece_char() {
    let bad = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPXPP/RNBQKBNR w KQkq - 0 1";
    assert!(matches!(
        Position::from_fen(bad),
        Err(FenError::BadPiecePlacement(_))
    ));
}

#[test]
fn fen_rejects_bad_side_to_move() {
    let bad = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR q KQkq - 0 1";
    assert_eq!(Position::from_fen(bad), Err(FenError::BadSideToMove));
}

#[test]
fn fen_rejects_bad_castling_rights() {
    let bad = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkx - 0 1";
    assert_eq!(Position::from_fen(bad), Err(FenError::BadCastlingRights));
}

#[test]
fn fen_rejects_bad_en_passant_square() {
    let bad = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq z9 0 1";
    assert_eq!(Position::from_fen(bad), Err(FenError::BadEnPassant));
}

#[test]
fn fen_rejects_non_numeric_clock() {
    let bad = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - x 1";
    assert!(matches!(
        Position::from_fen(bad),
        Err(FenError::BadClock(_))
    ));
}

#[test]
fn fen_rejects_position_with_no_white_king() {
    // Two kings on d8/e8 but no white king.
    let bad = "4kkk1/8/8/8/8/8/8/8 w - - 0 1";
    assert_eq!(
        Position::from_fen(bad),
        Err(FenError::MissingKing(Color::White))
    );
}

#[test]
fn fen_rejects_position_with_two_white_kings() {
    let bad = "4k3/8/8/8/8/8/8/KK6 w - - 0 1";
    // Two white kings => popcount != 1 on white king bb.
    assert_eq!(
        Position::from_fen(bad),
        Err(FenError::MissingKing(Color::White))
    );
}

// ---- Zobrist initial key -----------------------------------------

#[test]
fn startpos_key_is_non_zero_and_stable() {
    let p = Position::startpos();
    assert_ne!(p.key(), 0);
    // The key must match a from-scratch recomputation.
    assert_eq!(p.key(), p.compute_key_from_scratch());
}

#[test]
fn different_positions_have_different_keys() {
    let a = Position::startpos();
    let b = Position::from_fen("rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1")
        .unwrap();
    assert_ne!(a.key(), b.key());
}

// ---- Pawn-only Zobrist key ---------------------------------------

#[test]
fn startpos_pawn_key_matches_scratch_and_is_non_zero() {
    let p = Position::startpos();
    assert_ne!(p.pawn_key(), 0);
    assert_eq!(p.pawn_key(), p.compute_pawn_key_from_scratch());
}

#[test]
fn pawn_key_is_equal_for_identical_pawn_structures_only() {
    // Two different full positions that happen to share the same pawn
    // placement must produce the same pawn_key, even though their main
    // keys differ.
    let a =
        Position::from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap();
    let b = Position::from_fen("4k3/pppppppp/8/8/8/8/PPPPPPPP/4K3 w - - 0 1").unwrap();
    assert_eq!(a.pawn_key(), b.pawn_key());
    assert_ne!(a.key(), b.key());
}

#[test]
fn pawn_key_differs_when_pawns_differ() {
    let a = Position::startpos();
    // Same startpos but with the e2 pawn pushed to e4 — different pawn
    // structure, so pawn_key must change.
    let b = Position::from_fen("rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1")
        .unwrap();
    assert_ne!(a.pawn_key(), b.pawn_key());
}

#[test]
fn empty_pawn_position_has_no_pawns_base_key() {
    // Kings alone — no pawns to XOR in, so pawn_key equals the
    // `noPawns` base constant exactly.
    let p = Position::from_fen("4k3/8/8/8/8/8/8/4K3 w - - 0 1").unwrap();
    assert_eq!(p.pawn_key(), crate::zobrist::no_pawns_key());
}
