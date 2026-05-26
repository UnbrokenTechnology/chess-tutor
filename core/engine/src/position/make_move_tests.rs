use super::*;
use crate::types::Score;

// ---- do_move basics ---------------------------------------------

#[test]
fn do_move_e2_e4_produces_expected_position() {
    let mut p = Position::startpos();
    p.do_move(Move::normal(Square::E2, Square::E4));
    // No black pawn can capture on e3, so (matching SF11) the ep
    // square is NOT recorded — the canonical FEN has "-" in the ep
    // field, not "e3". Recording a non-capturable ep would diverge
    // key() from SF and break TT/repetition matching.
    assert_eq!(
        p.to_fen(),
        "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1"
    );
    assert_eq!(p.en_passant(), None);
    // The key must be the same as computing from scratch.
    assert_eq!(p.key(), p.compute_key_from_scratch());
}

#[test]
fn do_move_double_push_sets_ep_only_when_capturable() {
    // Black pawn on d4; white plays e2-e4. A black pawn on d4 attacks
    // e3, so en passant (exd... dxe3) is possible → e3 IS recorded.
    let mut p =
        Position::from_fen("4k3/8/8/8/3p4/8/4P3/4K3 w - - 0 1").unwrap();
    p.do_move(Move::normal(Square::E2, Square::E4));
    assert_eq!(p.en_passant(), Some(Square::E3));
    assert_eq!(p.key(), p.compute_key_from_scratch());
}

#[test]
fn do_move_nf3_bumps_halfmove_clock_and_flips_side() {
    let mut p = Position::startpos();
    p.do_move(Move::normal(Square::G1, Square::F3));
    assert_eq!(p.side_to_move(), Color::Black);
    assert_eq!(p.halfmove_clock(), 1);
    assert_eq!(p.piece_on(Square::F3), Some(Piece::WhiteKnight));
    assert_eq!(p.piece_on(Square::G1), None);
    assert_eq!(p.en_passant(), None);
    assert_eq!(p.key(), p.compute_key_from_scratch());
}

#[test]
fn do_move_black_move_increments_fullmove_number() {
    let mut p = Position::startpos();
    p.do_move(Move::normal(Square::E2, Square::E4));
    assert_eq!(p.fullmove_number(), 1);
    p.do_move(Move::normal(Square::E7, Square::E5));
    assert_eq!(p.fullmove_number(), 2);
}

#[test]
fn do_move_capture_zeroes_halfmove_clock_and_removes_victim() {
    // A white pawn on e4 captures a black pawn on d5. The halfmove
    // clock is set to 5 in the FEN so we can watch it reset to 0 when
    // the capture lands.
    let mut p =
        Position::from_fen("rnbqkbnr/ppp1pppp/8/3p4/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 5 3")
            .unwrap();
    assert_eq!(p.halfmove_clock(), 5);
    p.do_move(Move::normal(Square::E4, Square::D5));
    assert_eq!(p.piece_on(Square::D5), Some(Piece::WhitePawn));
    assert_eq!(p.piece_on(Square::E4), None);
    assert_eq!(p.halfmove_clock(), 0, "capture must reset halfmove clock");
    assert_eq!(p.side_to_move(), Color::Black);
    assert_eq!(p.key(), p.compute_key_from_scratch());
}

// ---- Do / undo roundtrip ----------------------------------------

fn roundtrip(fen: &str, m: Move) {
    let p0 = Position::from_fen(fen).unwrap();
    let mut p = p0.clone();
    let st = p.do_move(m);
    p.undo_move(m, st);
    assert_eq!(p, p0, "do/undo should restore position for move {:?}", m);
    assert_eq!(
        p.key(),
        p.compute_key_from_scratch(),
        "do/undo key must match scratch computation"
    );
}

#[test]
fn roundtrip_normal_pawn_move() {
    roundtrip(
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
        Move::normal(Square::E2, Square::E4),
    );
}

#[test]
fn roundtrip_knight_move() {
    roundtrip(
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
        Move::normal(Square::G1, Square::F3),
    );
}

#[test]
fn roundtrip_capture() {
    roundtrip(
        "rnbqkbnr/ppp1pppp/8/3p4/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2",
        Move::normal(Square::E4, Square::D5),
    );
}

#[test]
fn roundtrip_promotion_to_queen() {
    // White pawn on a7 promotes by pushing to a8 (no capture).
    roundtrip(
        "4k3/P7/8/8/8/8/8/4K3 w - - 0 1",
        Move::promotion(Square::A7, Square::A8, PieceType::Queen),
    );
}

#[test]
fn roundtrip_promotion_with_capture_to_knight() {
    // White pawn on a7 captures a rook on b8 and promotes to knight.
    roundtrip(
        "1r2k3/P7/8/8/8/8/8/4K3 w - - 0 1",
        Move::promotion(Square::A7, Square::B8, PieceType::Knight),
    );
}

#[test]
fn roundtrip_en_passant_capture() {
    // White pawn on e5 captures en passant on d6, removing the black
    // pawn on d5.
    roundtrip(
        "4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 3",
        Move::en_passant(Square::E5, Square::D6),
    );
}

#[test]
fn roundtrip_white_kingside_castling() {
    roundtrip(
        "4k3/8/8/8/8/8/8/4K2R w K - 0 1",
        Move::castling(Square::E1, Square::G1),
    );
}

#[test]
fn roundtrip_white_queenside_castling() {
    roundtrip(
        "4k3/8/8/8/8/8/8/R3K3 w Q - 0 1",
        Move::castling(Square::E1, Square::C1),
    );
}

#[test]
fn roundtrip_black_kingside_castling() {
    roundtrip(
        "4k2r/8/8/8/8/8/8/4K3 b k - 0 1",
        Move::castling(Square::E8, Square::G8),
    );
}

#[test]
fn roundtrip_black_queenside_castling() {
    roundtrip(
        "r3k3/8/8/8/8/8/8/4K3 b q - 0 1",
        Move::castling(Square::E8, Square::C8),
    );
}

// ---- Effects of specific special moves --------------------------

#[test]
fn en_passant_capture_removes_the_passed_pawn() {
    let mut p = Position::from_fen("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 3").unwrap();
    p.do_move(Move::en_passant(Square::E5, Square::D6));
    assert_eq!(p.piece_on(Square::D6), Some(Piece::WhitePawn));
    assert_eq!(p.piece_on(Square::E5), None);
    assert_eq!(
        p.piece_on(Square::D5),
        None,
        "en-passant victim should be gone from d5"
    );
}

#[test]
fn kingside_castling_moves_both_king_and_rook() {
    let mut p = Position::from_fen("4k3/8/8/8/8/8/8/4K2R w K - 0 1").unwrap();
    p.do_move(Move::castling(Square::E1, Square::G1));
    assert_eq!(p.piece_on(Square::G1), Some(Piece::WhiteKing));
    assert_eq!(p.piece_on(Square::F1), Some(Piece::WhiteRook));
    assert_eq!(p.piece_on(Square::E1), None);
    assert_eq!(p.piece_on(Square::H1), None);
}

#[test]
fn queenside_castling_moves_both_king_and_rook() {
    let mut p = Position::from_fen("4k3/8/8/8/8/8/8/R3K3 w Q - 0 1").unwrap();
    p.do_move(Move::castling(Square::E1, Square::C1));
    assert_eq!(p.piece_on(Square::C1), Some(Piece::WhiteKing));
    assert_eq!(p.piece_on(Square::D1), Some(Piece::WhiteRook));
    assert_eq!(p.piece_on(Square::E1), None);
    assert_eq!(p.piece_on(Square::A1), None);
}

#[test]
fn promotion_replaces_pawn_with_chosen_piece() {
    let mut p = Position::from_fen("4k3/P7/8/8/8/8/8/4K3 w - - 0 1").unwrap();
    p.do_move(Move::promotion(Square::A7, Square::A8, PieceType::Queen));
    assert_eq!(p.piece_on(Square::A8), Some(Piece::WhiteQueen));
    assert_eq!(p.piece_on(Square::A7), None);
    assert_eq!(p.count(Color::White, PieceType::Pawn), 0);
    assert_eq!(p.count(Color::White, PieceType::Queen), 1);
}

// ---- Castling-rights bookkeeping --------------------------------

#[test]
fn king_move_clears_both_castling_rights_for_that_color() {
    let mut p = Position::from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").unwrap();
    p.do_move(Move::normal(Square::E1, Square::E2));
    let rights = p.castling_rights();
    assert!(!rights.contains(CastlingRights::WHITE_KING));
    assert!(!rights.contains(CastlingRights::WHITE_QUEEN));
    // Black's rights aren't affected.
    assert!(rights.contains(CastlingRights::BLACK_KING));
    assert!(rights.contains(CastlingRights::BLACK_QUEEN));
}

#[test]
fn rook_leaving_home_clears_that_side() {
    let mut p = Position::from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").unwrap();
    p.do_move(Move::normal(Square::H1, Square::H2));
    let rights = p.castling_rights();
    assert!(!rights.contains(CastlingRights::WHITE_KING));
    assert!(rights.contains(CastlingRights::WHITE_QUEEN));
}

#[test]
fn capturing_a_rook_on_its_home_clears_that_side() {
    // White rook on a1 captures a black rook on a8 along the open
    // a-file. Both the captured rook (its home square) and the moving
    // rook (leaving its own home) lose queenside rights.
    let mut p = Position::from_fen("r3k3/8/8/8/8/8/8/R3K3 w Qq - 0 1").unwrap();
    p.do_move(Move::normal(Square::A1, Square::A8));
    let rights = p.castling_rights();
    assert!(
        !rights.contains(CastlingRights::BLACK_QUEEN),
        "black queenside right must be gone after its rook is captured"
    );
    assert!(!rights.contains(CastlingRights::WHITE_QUEEN));
}

#[test]
fn castling_itself_clears_that_color_both_rights() {
    let mut p = Position::from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").unwrap();
    p.do_move(Move::castling(Square::E1, Square::G1));
    let rights = p.castling_rights();
    assert!(!rights.contains(CastlingRights::WHITE_KING));
    assert!(!rights.contains(CastlingRights::WHITE_QUEEN));
    assert!(rights.contains(CastlingRights::BLACK_KING));
    assert!(rights.contains(CastlingRights::BLACK_QUEEN));
}

// ---- PSQ score ---------------------------------------------------

#[test]
fn startpos_psq_is_zero() {
    let p = Position::startpos();
    assert_eq!(p.psq_score(), Score::ZERO);
}

#[test]
fn psq_is_incrementally_maintained_through_a_sequence() {
    // At every step of a game the stored psq must equal a from-scratch
    // recomputation. Drift here would mean remove/put is out of sync
    // with what from_fen would compute.
    let mut p = Position::startpos();
    let moves = [
        Move::normal(Square::E2, Square::E4),
        Move::normal(Square::E7, Square::E5),
        Move::normal(Square::G1, Square::F3),
        Move::normal(Square::B8, Square::C6),
        Move::normal(Square::F1, Square::C4),
        Move::normal(Square::G8, Square::F6),
    ];
    for m in moves {
        p.do_move(m);
        assert_eq!(
            p.psq_score(),
            p.compute_psq_from_scratch(),
            "psq drift after {:?}",
            m
        );
    }
}

#[test]
fn psq_is_restored_by_undo() {
    let original = Position::startpos();
    let mut p = original.clone();
    let m = Move::normal(Square::E2, Square::E4);
    let st = p.do_move(m);
    p.undo_move(m, st);
    assert_eq!(p.psq_score(), original.psq_score());
}

#[test]
fn capture_leaves_psq_consistent() {
    // A piece is captured: the captured piece's psq contribution must
    // vanish, and the scratch recomputation must agree.
    let mut p =
        Position::from_fen("rnbqkbnr/ppp1pppp/8/3p4/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2")
            .unwrap();
    p.do_move(Move::normal(Square::E4, Square::D5));
    assert_eq!(p.psq_score(), p.compute_psq_from_scratch());
}

#[test]
fn promotion_leaves_psq_consistent() {
    let mut p = Position::from_fen("4k3/P7/8/8/8/8/8/4K3 w - - 0 1").unwrap();
    p.do_move(Move::promotion(Square::A7, Square::A8, PieceType::Queen));
    assert_eq!(p.psq_score(), p.compute_psq_from_scratch());
}

#[test]
fn castling_leaves_psq_consistent() {
    let mut p = Position::from_fen("4k3/8/8/8/8/8/8/R3K2R w KQ - 0 1").unwrap();
    p.do_move(Move::castling(Square::E1, Square::G1));
    assert_eq!(p.psq_score(), p.compute_psq_from_scratch());
}

#[test]
fn en_passant_leaves_psq_consistent() {
    let mut p = Position::from_fen("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 3").unwrap();
    p.do_move(Move::en_passant(Square::E5, Square::D6));
    assert_eq!(p.psq_score(), p.compute_psq_from_scratch());
}

// ---- Pawn-key incremental maintenance across moves --------------

#[test]
fn pawn_key_is_stable_through_long_sequence() {
    let mut p = Position::startpos();
    let moves = [
        Move::normal(Square::E2, Square::E4),
        Move::normal(Square::E7, Square::E5),
        Move::normal(Square::G1, Square::F3),
        Move::normal(Square::B8, Square::C6),
        Move::normal(Square::F1, Square::C4),
        Move::normal(Square::G8, Square::F6),
    ];
    for m in moves {
        p.do_move(m);
        assert_eq!(
            p.pawn_key(),
            p.compute_pawn_key_from_scratch(),
            "pawn_key drift after {:?}",
            m
        );
    }
}

#[test]
fn pawn_key_is_consistent_after_promotion() {
    // A pawn promoting disappears from the pawn structure; the promoted
    // piece doesn't enter the pawn_key.
    let mut p = Position::from_fen("4k3/P7/8/8/8/8/8/4K3 w - - 0 1").unwrap();
    p.do_move(Move::promotion(Square::A7, Square::A8, PieceType::Queen));
    assert_eq!(p.pawn_key(), p.compute_pawn_key_from_scratch());
    // All pawns gone => just the noPawns base.
    assert_eq!(p.pawn_key(), crate::zobrist::no_pawns_key());
}

#[test]
fn pawn_key_is_consistent_after_en_passant_capture() {
    let mut p = Position::from_fen("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 3").unwrap();
    p.do_move(Move::en_passant(Square::E5, Square::D6));
    assert_eq!(p.pawn_key(), p.compute_pawn_key_from_scratch());
}

#[test]
fn pawn_key_is_restored_by_undo() {
    let original = Position::startpos();
    let mut p = original.clone();
    let m = Move::normal(Square::E2, Square::E4);
    let st = p.do_move(m);
    p.undo_move(m, st);
    assert_eq!(p.pawn_key(), original.pawn_key());
}

// ---- Long-sequence incremental maintenance ----------------------

#[test]
fn zobrist_key_is_stable_through_long_sequence() {
    // Play a handful of moves, and at every step verify that the
    // incrementally-maintained key equals the from-scratch key.
    let mut p = Position::startpos();
    let moves = [
        Move::normal(Square::E2, Square::E4),
        Move::normal(Square::E7, Square::E5),
        Move::normal(Square::G1, Square::F3),
        Move::normal(Square::B8, Square::C6),
        Move::normal(Square::F1, Square::C4),
        Move::normal(Square::G8, Square::F6),
    ];
    for m in moves {
        p.do_move(m);
        assert_eq!(
            p.key(),
            p.compute_key_from_scratch(),
            "key drift after {:?}",
            m
        );
    }
}

#[test]
fn do_undo_across_long_sequence_restores_original() {
    let original = Position::startpos();
    let moves: [Move; 6] = [
        Move::normal(Square::E2, Square::E4),
        Move::normal(Square::E7, Square::E5),
        Move::normal(Square::G1, Square::F3),
        Move::normal(Square::B8, Square::C6),
        Move::normal(Square::F1, Square::C4),
        Move::normal(Square::G8, Square::F6),
    ];
    let mut p = original.clone();
    let mut states: Vec<StateInfo> = Vec::with_capacity(moves.len());
    for m in moves {
        states.push(p.do_move(m));
    }
    // Undo in reverse.
    for (m, st) in moves.iter().rev().zip(states.iter().rev()) {
        p.undo_move(*m, *st);
    }
    assert_eq!(p, original);
}
