use super::*;

// ---- Masks -------------------------------------------------------

#[test]
fn file_masks_are_disjoint_and_cover_the_board() {
    let mut all = Bitboard::EMPTY;
    for (f, m) in FILE_MASKS.iter().copied().enumerate() {
        assert_eq!(m.popcount(), 8, "file {} should have 8 squares", f);
        assert!((all & m).is_empty(), "files must be disjoint");
        all |= m;
    }
    assert_eq!(all, Bitboard::ALL);
}

#[test]
fn rank_masks_are_disjoint_and_cover_the_board() {
    let mut all = Bitboard::EMPTY;
    for (r, m) in RANK_MASKS.iter().copied().enumerate() {
        assert_eq!(m.popcount(), 8, "rank {} should have 8 squares", r);
        assert!((all & m).is_empty(), "ranks must be disjoint");
        all |= m;
    }
    assert_eq!(all, Bitboard::ALL);
}

#[test]
fn dark_and_light_squares_partition_the_board() {
    assert_eq!(DARK_SQUARES | LIGHT_SQUARES, Bitboard::ALL);
    assert!((DARK_SQUARES & LIGHT_SQUARES).is_empty());
    assert_eq!(DARK_SQUARES.popcount(), 32);
}

#[test]
fn a1_is_a_dark_square() {
    assert!(DARK_SQUARES.contains(Square::A1));
    assert!(!DARK_SQUARES.contains(Square::H1));
}

#[test]
fn center_mask_is_the_four_central_squares() {
    assert_eq!(CENTER.popcount(), 4);
    assert!(CENTER.contains(Square::D4));
    assert!(CENTER.contains(Square::E4));
    assert!(CENTER.contains(Square::D5));
    assert!(CENTER.contains(Square::E5));
}

// ---- square_bb ---------------------------------------------------

#[test]
fn square_bb_round_trips_for_every_square() {
    for i in 0u8..64 {
        let sq = Square::from_index(i);
        let bb = square_bb(sq);
        assert_eq!(bb.popcount(), 1);
        assert_eq!(bb.lsb(), sq);
        assert_eq!(bb.msb(), sq);
        assert!(bb.contains(sq));
    }
}

// ---- Shifts ------------------------------------------------------

#[test]
fn shift_north_moves_every_square_up_one_rank() {
    let shifted = RANK_2.shift_north();
    assert_eq!(shifted, RANK_3);
}

#[test]
fn shift_east_clears_the_h_file() {
    // Start with every square set on the a..h files and shift east. The
    // h-file squares must not wrap to the a-file.
    let shifted = Bitboard::ALL.shift_east();
    assert!(
        (shifted & FILE_A).is_empty(),
        "a-file must be empty after east shift"
    );
    assert_eq!(shifted.popcount(), 64 - 8);
}

#[test]
fn shift_west_clears_the_a_file() {
    let shifted = Bitboard::ALL.shift_west();
    assert!(
        (shifted & FILE_H).is_empty(),
        "h-file must be empty after west shift"
    );
}

#[test]
fn shift_north_east_doesnt_wrap_around_h_file() {
    let h1 = square_bb(Square::H1);
    assert!(h1.shift_north_east().is_empty());
}

#[test]
fn shift_north_west_doesnt_wrap_around_a_file() {
    let a1 = square_bb(Square::A1);
    assert!(a1.shift_north_west().is_empty());
}

#[test]
fn shift_by_direction_matches_concrete_shifts() {
    let starting_square = square_bb(Square::E4);
    let cases: &[(Direction, Bitboard)] = &[
        (Direction::NORTH, starting_square.shift_north()),
        (Direction::SOUTH, starting_square.shift_south()),
        (Direction::EAST, starting_square.shift_east()),
        (Direction::WEST, starting_square.shift_west()),
        (Direction::NORTH_EAST, starting_square.shift_north_east()),
        (Direction::NORTH_WEST, starting_square.shift_north_west()),
        (Direction::SOUTH_EAST, starting_square.shift_south_east()),
        (Direction::SOUTH_WEST, starting_square.shift_south_west()),
    ];
    for (d, expected) in cases {
        assert_eq!(
            starting_square.shift(*d),
            *expected,
            "shift by direction {:?} disagreed with concrete shift",
            d
        );
    }
}

// ---- Pawn attacks -----------------------------------------------

#[test]
fn white_pawn_attacks_from_e4() {
    let e4 = square_bb(Square::E4);
    let attacks = e4.pawn_attacks(Color::White);
    // e4 attacks d5 and f5.
    assert_eq!(attacks.popcount(), 2);
    assert!(attacks.contains(Square::D5));
    assert!(attacks.contains(Square::from_index(37))); // F5
}

#[test]
fn black_pawn_attacks_from_e5() {
    let e5 = square_bb(Square::E5);
    let attacks = e5.pawn_attacks(Color::Black);
    // e5 attacks d4 and f4.
    assert_eq!(attacks.popcount(), 2);
    assert!(attacks.contains(Square::D4));
    assert!(attacks.contains(Square::from_index(29))); // F4
}

#[test]
fn edge_pawn_attacks_dont_wrap() {
    // a4 is on file A; its only attack is b5 for white.
    let a4 = square_bb(Square::from_index(24)); // A4
    let attacks = a4.pawn_attacks(Color::White);
    assert_eq!(attacks.popcount(), 1);
    assert!(attacks.contains(Square::from_index(33))); // B5
}

#[test]
fn pawn_double_attacks_requires_two_defenders() {
    // e4 and g4 together double-attack f5.
    let pawns = square_bb(Square::E4) | square_bb(Square::from_index(30)); // G4
    let double = pawns.pawn_double_attacks(Color::White);
    assert_eq!(double.popcount(), 1);
    assert!(double.contains(Square::from_index(37))); // F5
}

// ---- popcount / lsb / msb / pop_lsb -----------------------------

#[test]
fn popcount_matches_u64_builtin() {
    let bb = Bitboard(0xAB_CD_EF_01_23_45_67_89);
    assert_eq!(bb.popcount(), bb.0.count_ones());
}

#[test]
fn more_than_one_is_true_for_two_or_more_bits() {
    assert!(!Bitboard::EMPTY.more_than_one());
    assert!(!square_bb(Square::A1).more_than_one());
    assert!((square_bb(Square::A1) | square_bb(Square::H8)).more_than_one());
}

#[test]
fn lsb_msb_find_correct_squares() {
    let bb = square_bb(Square::B1) | square_bb(Square::from_index(40)); // B1 | A6
    assert_eq!(bb.lsb(), Square::B1);
    assert_eq!(bb.msb(), Square::from_index(40));
}

#[test]
fn pop_lsb_empties_in_ascending_order() {
    let mut bb = square_bb(Square::E4) | square_bb(Square::A1) | square_bb(Square::H8);
    assert_eq!(bb.pop_lsb(), Square::A1);
    assert_eq!(bb.pop_lsb(), Square::E4);
    assert_eq!(bb.pop_lsb(), Square::H8);
    assert!(bb.is_empty());
}

// ---- adjacent / forward / pawn span -----------------------------

#[test]
fn adjacent_files_of_e_are_d_and_f() {
    let a = adjacent_files_bb(Square::E4);
    assert_eq!(a, FILE_D | FILE_F);
}

#[test]
fn adjacent_files_of_a_only_yield_b() {
    let a = adjacent_files_bb(Square::A1);
    assert_eq!(a, FILE_B);
}

#[test]
fn forward_ranks_from_white_e4_covers_ranks_5_through_8() {
    let fwd = forward_ranks_bb(Color::White, Square::E4);
    let expected = RANK_5 | RANK_6 | RANK_7 | RANK_8;
    assert_eq!(fwd, expected);
}

#[test]
fn forward_ranks_from_black_d3_covers_ranks_1_and_2() {
    let fwd = forward_ranks_bb(Color::Black, Square::from_index(19)); // D3
    let expected = RANK_1 | RANK_2;
    assert_eq!(fwd, expected);
}

#[test]
fn passed_pawn_span_covers_file_and_adjacents_ahead() {
    let span = passed_pawn_span(Color::White, Square::E4);
    // For a white pawn on e4: d,e,f files on ranks 5..=8 => 12 squares.
    assert_eq!(span.popcount(), 12);
    assert!(span.contains(Square::D5));
    assert!(span.contains(Square::E5));
    assert!(span.contains(Square::from_index(37))); // F5
}

// ---- Opposite colors & distances --------------------------------

#[test]
fn opposite_colors_detects_adjacent_squares() {
    // Adjacent squares along any axis always flip color.
    assert!(opposite_colors(Square::A1, Square::B1));
    assert!(opposite_colors(Square::A1, Square::A2));
    // Squares two steps apart along a rank or file are the same color.
    assert!(!opposite_colors(Square::A1, Square::A3));
    // Opposite corners of the board are both dark.
    assert!(!opposite_colors(Square::A1, Square::H8));
}

#[test]
fn distances_are_correct_between_known_pairs() {
    assert_eq!(file_distance(Square::A1, Square::H1), 7);
    assert_eq!(rank_distance(Square::A1, Square::A8), 7);
    assert_eq!(king_distance(Square::A1, Square::H8), 7);
    assert_eq!(king_distance(Square::E4, Square::D5), 1);
    assert_eq!(king_distance(Square::E4, Square::E4), 0);
}

// ---- Iteration --------------------------------------------------

#[test]
fn iteration_yields_squares_in_ascending_order() {
    let bb = square_bb(Square::E4) | square_bb(Square::A1) | square_bb(Square::H8);
    let collected: Vec<Square> = bb.into_iter().collect();
    assert_eq!(collected, vec![Square::A1, Square::E4, Square::H8]);
}

#[test]
fn iteration_over_empty_yields_nothing() {
    let collected: Vec<Square> = Bitboard::EMPTY.into_iter().collect();
    assert!(collected.is_empty());
}

// ---- Region masks -----------------------------------------------

#[test]
fn king_flank_spans_three_files_at_edges_four_elsewhere() {
    // The reference shrinks the flank by one file at the a- and h-files
    // so an edge-castled king doesn't get credit for squares it can't
    // reasonably shelter behind. Interior files get the full four-file
    // flank.
    let expected = [24, 32, 32, 32, 32, 32, 32, 24];
    for f in 0..8 {
        assert_eq!(
            KING_FLANK[f].popcount(),
            expected[f],
            "flank for file {} had unexpected size",
            f
        );
    }
}
