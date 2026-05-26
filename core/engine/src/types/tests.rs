use super::*;

// ---- Color --------------------------------------------------------

#[test]
fn color_toggle_is_involutive() {
    assert_eq!(!Color::White, Color::Black);
    assert_eq!(!Color::Black, Color::White);
    assert_eq!(!!Color::White, Color::White);
}

// ---- Piece --------------------------------------------------------

#[test]
fn piece_new_round_trips_through_color_and_kind() {
    for &color in &Color::both() {
        for &kind in &[
            PieceType::Pawn,
            PieceType::Knight,
            PieceType::Bishop,
            PieceType::Rook,
            PieceType::Queen,
            PieceType::King,
        ] {
            let p = Piece::new(color, kind);
            assert_eq!(p.color(), color);
            assert_eq!(p.kind(), kind);
        }
    }
}

#[test]
fn piece_flip_color_swaps_sides() {
    assert_eq!(Piece::WhitePawn.flip_color(), Piece::BlackPawn);
    assert_eq!(Piece::BlackQueen.flip_color(), Piece::WhiteQueen);
}

#[test]
fn option_piece_fits_in_one_byte() {
    // Niche optimisation: Piece's discriminants skip 0, 7, 8, 15 so
    // Option<Piece> should reuse the zero slot for None.
    assert_eq!(std::mem::size_of::<Option<Piece>>(), 1);
}

// ---- Square -------------------------------------------------------

#[test]
fn square_file_and_rank_decompose_index() {
    for i in 0u8..64 {
        let sq = Square::from_index(i);
        let f = sq.file().index() as u8;
        let r = sq.rank().index() as u8;
        assert_eq!(f, i & 7);
        assert_eq!(r, i >> 3);
        assert_eq!(Square::new(sq.file(), sq.rank()), sq);
    }
}

#[test]
fn square_flip_vertical_swaps_ranks() {
    assert_eq!(Square::A1.flip_vertical(), Square::A8);
    assert_eq!(Square::H8.flip_vertical(), Square(7)); // H1
}

#[test]
fn square_from_perspective_mirrors_for_black() {
    assert_eq!(Square::E4.from_perspective(Color::White), Square::E4);
    // E4 (rank 4) from Black's view is rank 5, same file => E5.
    assert_eq!(Square::E4.from_perspective(Color::Black), Square::E5);
}

#[test]
fn square_algebraic_roundtrip() {
    for i in 0u8..64 {
        let sq = Square::from_index(i);
        let s = sq.to_algebraic();
        assert_eq!(Square::from_algebraic(&s), Some(sq));
    }
}

#[test]
fn square_algebraic_rejects_garbage() {
    assert_eq!(Square::from_algebraic(""), None);
    assert_eq!(Square::from_algebraic("z3"), None);
    assert_eq!(Square::from_algebraic("a9"), None);
    assert_eq!(Square::from_algebraic("e44"), None);
}

#[test]
fn square_plus_direction_steps_by_offset() {
    assert_eq!(Square::E4 + Direction::NORTH, Square::E5);
    assert_eq!(Square::E4 + Direction::SOUTH, Square(20)); // E3
    assert_eq!(Square::E4 + Direction::EAST, Square(29)); // F4
}

// ---- File / Rank --------------------------------------------------

#[test]
fn file_folds_to_queenside() {
    assert_eq!(File::A.fold_to_queenside(), File::A);
    assert_eq!(File::D.fold_to_queenside(), File::D);
    assert_eq!(File::E.fold_to_queenside(), File::D);
    assert_eq!(File::H.fold_to_queenside(), File::A);
}

#[test]
fn rank_from_perspective_flips_for_black() {
    assert_eq!(Rank::R1.from_perspective(Color::White), Rank::R1);
    assert_eq!(Rank::R1.from_perspective(Color::Black), Rank::R8);
    assert_eq!(Rank::R4.from_perspective(Color::Black), Rank::R5);
}

// ---- Score packing ------------------------------------------------

#[test]
fn score_new_round_trips_positive_pairs() {
    let s = Score::new(100, 200);
    assert_eq!(s.mg(), Value(100));
    assert_eq!(s.eg(), Value(200));
}

#[test]
fn score_new_round_trips_negative_mg() {
    // The rounding trick in eg() exists precisely so a negative mg value
    // doesn't drag the eg value down by one.
    let s = Score::new(-1, 200);
    assert_eq!(s.mg(), Value(-1));
    assert_eq!(s.eg(), Value(200));
}

#[test]
fn score_new_round_trips_negative_both() {
    let s = Score::new(-100, -200);
    assert_eq!(s.mg(), Value(-100));
    assert_eq!(s.eg(), Value(-200));
}

#[test]
fn score_new_round_trips_extremes() {
    let s = Score::new(i16::MIN as i32, i16::MAX as i32);
    assert_eq!(s.mg(), Value(i16::MIN as i32));
    assert_eq!(s.eg(), Value(i16::MAX as i32));
}

#[test]
fn score_addition_is_componentwise() {
    let a = Score::new(10, 20);
    let b = Score::new(3, 7);
    let sum = a + b;
    assert_eq!(sum.mg(), Value(13));
    assert_eq!(sum.eg(), Value(27));
}

#[test]
fn score_multiplication_is_componentwise() {
    let s = Score::new(10, -20);
    let tripled = s * 3;
    assert_eq!(tripled.mg(), Value(30));
    assert_eq!(tripled.eg(), Value(-60));
}

#[test]
fn score_division_is_componentwise() {
    let s = Score::new(10, -20);
    let halved = s / 2;
    assert_eq!(halved.mg(), Value(5));
    assert_eq!(halved.eg(), Value(-10));
}

#[test]
fn score_negation_is_componentwise() {
    let s = Score::new(10, -20);
    let n = -s;
    assert_eq!(n.mg(), Value(-10));
    assert_eq!(n.eg(), Value(20));
}

// ---- Value --------------------------------------------------------

#[test]
fn mate_in_and_mated_in_are_symmetric() {
    for ply in 0..50 {
        assert_eq!(Value::mate_in(ply), -Value::mated_in(ply));
    }
}

#[test]
fn piece_value_table_is_phase_indexed() {
    assert_eq!(PIECE_VALUE[0][Piece::WhiteQueen.index()], Value::QUEEN_MG);
    assert_eq!(PIECE_VALUE[1][Piece::BlackPawn.index()], Value::PAWN_EG);
}

// ---- CastlingRights ----------------------------------------------

#[test]
fn castling_rights_combine() {
    let all = CastlingRights::WHITE | CastlingRights::BLACK;
    assert_eq!(all, CastlingRights::ALL);
    assert!(all.contains(CastlingRights::WHITE_KING));
    assert!(all.intersects(CastlingRights::QUEEN_SIDE));
}

#[test]
fn castling_rights_not_only_affects_used_bits() {
    let empty = !CastlingRights::ALL;
    assert_eq!(empty, CastlingRights::NONE);
}

// ---- Move encoding ------------------------------------------------

#[test]
fn move_normal_round_trips_from_and_to() {
    let m = Move::normal(Square::E2, /* E4 */ Square::E4);
    assert_eq!(m.from(), Square::E2);
    assert_eq!(m.to(), Square::E4);
    assert_eq!(m.kind(), MoveKind::Normal);
}

#[test]
fn move_promotion_round_trips_promoted_piece() {
    let from = Square::from_index(48); // A7
    let to = Square::from_index(56); // A8
    for &pt in &[
        PieceType::Knight,
        PieceType::Bishop,
        PieceType::Rook,
        PieceType::Queen,
    ] {
        let m = Move::promotion(from, to, pt);
        assert_eq!(m.kind(), MoveKind::Promotion);
        assert_eq!(m.from(), from);
        assert_eq!(m.to(), to);
        assert_eq!(m.promoted_to(), pt);
    }
}

#[test]
fn move_en_passant_and_castling_tags() {
    let ep = Move::en_passant(Square::from_index(36), Square::from_index(43));
    assert_eq!(ep.kind(), MoveKind::EnPassant);
    let cs = Move::castling(Square::E1, Square::G1);
    assert_eq!(cs.kind(), MoveKind::Castling);
}

#[test]
fn move_none_and_null_are_invalid() {
    assert!(!Move::NONE.is_valid());
    assert!(!Move::NULL.is_valid());
    assert!(Move::normal(Square::E2, Square::E4).is_valid());
}

#[test]
fn move_from_raw_round_trips_packed_bits() {
    // Packing a move and reconstructing from the raw u16 must
    // reproduce an identical `Move`. This is the property the TT
    // relies on when storing and reloading moves.
    let cases = [
        Move::normal(Square::E2, Square::E4),
        Move::en_passant(Square::E5, Square::D6),
        Move::castling(Square::E1, Square::G1),
        Move::promotion(Square::A7, Square::A8, PieceType::Queen),
        Move::NONE,
    ];
    for m in cases {
        assert_eq!(Move::from_raw(m.raw()), m);
    }
}

// ---- Depth -------------------------------------------------------

#[test]
fn depth_constants_match_reference() {
    // Stockfish 11: DEPTH_NONE = -6, DEPTH_QS_CHECKS = 0,
    // DEPTH_QS_NO_CHECKS = -1, DEPTH_QS_RECAPTURES = -5.
    assert_eq!(Depth::NONE.0, -6);
    assert_eq!(Depth::QS_CHECKS.0, 0);
    assert_eq!(Depth::QS_NO_CHECKS.0, -1);
    assert_eq!(Depth::QS_RECAPTURES.0, -5);
    assert_eq!(Depth::OFFSET, -6);
}

#[test]
fn depth_arithmetic_adjusts_by_integer() {
    assert_eq!((Depth(4) + 2).0, 6);
    assert_eq!((Depth(4) - 2).0, 2);
}

// ---- Bound -------------------------------------------------------

#[test]
fn bound_from_u8_round_trips_all_variants() {
    // The TT packs the bound into the lower 2 bits of a status byte.
    // Decoding must recover the same variant regardless of the
    // upper bits (which hold unrelated flags).
    for b in [Bound::None, Bound::Upper, Bound::Lower, Bound::Exact] {
        for upper in 0..=255u8 {
            let packed = (upper & !0x3) | b.as_u8();
            assert_eq!(Bound::from_u8(packed), b);
        }
    }
}
