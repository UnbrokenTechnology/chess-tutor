use super::*;
use crate::bitboard::{square_bb, FILE_A, FILE_H};

// ---- Knight ------------------------------------------------------

#[test]
fn knight_from_center_reaches_eight_squares() {
    let attacks = knight_attacks(Square::E4);
    assert_eq!(attacks.popcount(), 8);
    // Known targets from e4: d2, f2, c3, g3, c5, g5, d6, f6.
    let expected_algebraic = ["d2", "f2", "c3", "g3", "c5", "g5", "d6", "f6"];
    for sq in &expected_algebraic {
        let s = Square::from_algebraic(sq).unwrap();
        assert!(attacks.contains(s), "knight from e4 should reach {}", sq);
    }
}

#[test]
fn knight_from_corner_reaches_two_squares() {
    assert_eq!(knight_attacks(Square::A1).popcount(), 2);
    assert_eq!(knight_attacks(Square::H1).popcount(), 2);
    assert_eq!(knight_attacks(Square::A8).popcount(), 2);
    assert_eq!(knight_attacks(Square::H8).popcount(), 2);
}

#[test]
fn knight_attacks_dont_wrap_files() {
    let from_a4 = knight_attacks(Square::from_algebraic("a4").unwrap());
    // No knight jump from a4 should land on the h-file.
    assert!((from_a4 & FILE_H).is_empty());
}

// ---- King --------------------------------------------------------

#[test]
fn king_from_center_reaches_eight_squares() {
    assert_eq!(king_attacks(Square::E4).popcount(), 8);
}

#[test]
fn king_from_corner_reaches_three_squares() {
    assert_eq!(king_attacks(Square::A1).popcount(), 3);
    assert_eq!(king_attacks(Square::H8).popcount(), 3);
}

#[test]
fn king_attacks_dont_wrap() {
    let from_a1 = king_attacks(Square::A1);
    assert!((from_a1 & FILE_H).is_empty());
}

// ---- Pawn --------------------------------------------------------

#[test]
fn white_pawn_from_e4_attacks_d5_and_f5() {
    let a = pawn_attacks_from(Color::White, Square::E4);
    assert_eq!(a.popcount(), 2);
    assert!(a.contains(Square::D5));
    assert!(a.contains(Square::from_algebraic("f5").unwrap()));
}

#[test]
fn black_pawn_from_e5_attacks_d4_and_f4() {
    let a = pawn_attacks_from(Color::Black, Square::E5);
    assert_eq!(a.popcount(), 2);
    assert!(a.contains(Square::D4));
    assert!(a.contains(Square::from_algebraic("f4").unwrap()));
}

#[test]
fn pawn_attacks_from_a_file_dont_wrap() {
    let a4 = Square::from_algebraic("a4").unwrap();
    let w = pawn_attacks_from(Color::White, a4);
    assert_eq!(w.popcount(), 1);
    assert!((w & FILE_H).is_empty());
}

// ---- Slider pseudo attacks --------------------------------------

#[test]
fn bishop_pseudo_from_a1_covers_long_diagonal() {
    let a = bishop_pseudo_attacks(Square::A1);
    assert_eq!(a.popcount(), 7);
    for sq in &["b2", "c3", "d4", "e5", "f6", "g7", "h8"] {
        assert!(a.contains(Square::from_algebraic(sq).unwrap()));
    }
}

#[test]
fn rook_pseudo_from_d4_covers_rank_and_file() {
    let a = rook_pseudo_attacks(Square::D4);
    assert_eq!(a.popcount(), 14);
    // Every square on the d-file (except d4) and the 4th rank (except d4).
    for f in 0..8u8 {
        let on_rank = Square::from_index(3 * 8 + f);
        if on_rank != Square::D4 {
            assert!(a.contains(on_rank));
        }
    }
    for r in 0..8u8 {
        let on_file = Square::from_index(r * 8 + 3);
        if on_file != Square::D4 {
            assert!(a.contains(on_file));
        }
    }
}

#[test]
fn queen_pseudo_is_bishop_union_rook() {
    for i in 0u8..64 {
        let s = Square::from_index(i);
        assert_eq!(
            queen_pseudo_attacks(s),
            bishop_pseudo_attacks(s) | rook_pseudo_attacks(s),
        );
    }
}

// ---- Symmetry ----------------------------------------------------

#[test]
fn knight_attacks_are_symmetric() {
    // If a knight on `a` attacks `b`, then a knight on `b` attacks `a`.
    for i in 0u8..64 {
        let s = Square::from_index(i);
        let mut bb = knight_attacks(s);
        while !bb.is_empty() {
            let t = bb.pop_lsb();
            assert!(
                knight_attacks(t).contains(s),
                "knight attacks should be symmetric: {} <-> {}",
                s.to_algebraic(),
                t.to_algebraic()
            );
        }
    }
}

#[test]
fn king_attacks_are_symmetric() {
    for i in 0u8..64 {
        let s = Square::from_index(i);
        let mut bb = king_attacks(s);
        while !bb.is_empty() {
            let t = bb.pop_lsb();
            assert!(king_attacks(t).contains(s));
        }
    }
}

// ---- Line / Between ---------------------------------------------

#[test]
fn line_between_two_rank_squares_is_the_whole_rank() {
    let a1 = Square::A1;
    let h1 = Square::H1;
    let line = line_bb(a1, h1);
    // The 1st rank contains 8 squares and they should all be present.
    assert_eq!(line.popcount(), 8);
    for f in 0..8u8 {
        assert!(line.contains(Square::from_index(f)));
    }
}

#[test]
fn line_between_two_file_squares_is_the_whole_file() {
    let a1 = Square::A1;
    let a8 = Square::A8;
    let line = line_bb(a1, a8);
    assert_eq!(line, FILE_A);
}

#[test]
fn line_between_diagonal_squares_is_the_diagonal() {
    let line = line_bb(Square::A1, Square::H8);
    assert_eq!(line.popcount(), 8);
    for sq in &["a1", "b2", "c3", "d4", "e5", "f6", "g7", "h8"] {
        assert!(line.contains(Square::from_algebraic(sq).unwrap()));
    }
}

#[test]
fn line_between_unrelated_squares_is_empty() {
    // a1 and b3 share neither a rank, a file, nor a diagonal.
    assert_eq!(
        line_bb(Square::A1, Square::from_algebraic("b3").unwrap()),
        Bitboard::EMPTY
    );
}

#[test]
fn line_of_a_square_with_itself_is_empty() {
    for i in 0u8..64 {
        let s = Square::from_index(i);
        assert_eq!(line_bb(s, s), Bitboard::EMPTY);
    }
}

#[test]
fn line_is_symmetric() {
    for a in 0u8..64 {
        for b in 0u8..64 {
            let sa = Square::from_index(a);
            let sb = Square::from_index(b);
            assert_eq!(line_bb(sa, sb), line_bb(sb, sa));
        }
    }
}

#[test]
fn between_two_rank_squares_is_strictly_between() {
    // a1 to h1: between is b1..g1 (6 squares).
    let bb = between_bb(Square::A1, Square::H1);
    assert_eq!(bb.popcount(), 6);
    for f in 1..7u8 {
        assert!(bb.contains(Square::from_index(f)));
    }
    assert!(!bb.contains(Square::A1));
    assert!(!bb.contains(Square::H1));
}

#[test]
fn between_two_file_squares_is_strictly_between() {
    // a1 to a8: between is a2..a7.
    let bb = between_bb(Square::A1, Square::A8);
    assert_eq!(bb.popcount(), 6);
    for r in 1..7u8 {
        assert!(bb.contains(Square::from_index(r * 8)));
    }
}

#[test]
fn between_two_diagonal_squares_is_strictly_between() {
    // a1 to h8 along the long diagonal: between is b2..g7 (6 squares).
    let bb = between_bb(Square::A1, Square::H8);
    assert_eq!(bb.popcount(), 6);
    for sq in &["b2", "c3", "d4", "e5", "f6", "g7"] {
        assert!(bb.contains(Square::from_algebraic(sq).unwrap()));
    }
}

#[test]
fn between_adjacent_squares_is_empty() {
    assert_eq!(between_bb(Square::E4, Square::E5), Bitboard::EMPTY);
    assert_eq!(between_bb(Square::E4, Square::D5), Bitboard::EMPTY);
}

#[test]
fn between_unrelated_squares_is_empty() {
    assert_eq!(
        between_bb(Square::A1, Square::from_algebraic("b3").unwrap()),
        Bitboard::EMPTY,
    );
}

#[test]
fn between_is_symmetric() {
    for a in 0u8..64 {
        for b in 0u8..64 {
            let sa = Square::from_index(a);
            let sb = Square::from_index(b);
            assert_eq!(between_bb(sa, sb), between_bb(sb, sa));
        }
    }
}

// ---- Aligned -----------------------------------------------------

#[test]
fn aligned_detects_collinear_triples() {
    // a1, d4, h8 all on the a1-h8 diagonal.
    assert!(aligned(Square::A1, Square::D4, Square::H8));
    // a1, h1, d1 all on rank 1.
    assert!(aligned(Square::A1, Square::H1, Square::D1));
}

#[test]
fn aligned_rejects_non_collinear_triples() {
    // a1, h8 on the long diagonal; e4 isn't.
    assert!(!aligned(Square::A1, Square::H8, Square::E4));
}

// ---- Square distance --------------------------------------------

#[test]
fn square_distance_matches_runtime_king_distance() {
    // The static table must agree with the on-the-fly computation.
    for a in 0u8..64 {
        for b in 0u8..64 {
            let sa = Square::from_index(a);
            let sb = Square::from_index(b);
            assert_eq!(square_distance(sa, sb), king_distance(sa, sb));
        }
    }
}

// ---- Pawn attacks table vs bitboard helper ----------------------

#[test]
fn pawn_attacks_table_matches_bitboard_shift() {
    // The per-square PAWN_ATTACKS table should agree with the bitboard
    // shift-based pawn_attacks on single-square bitboards.
    for i in 0u8..64 {
        let s = Square::from_index(i);
        assert_eq!(
            pawn_attacks_from(Color::White, s),
            square_bb(s).pawn_attacks(Color::White),
        );
        assert_eq!(
            pawn_attacks_from(Color::Black, s),
            square_bb(s).pawn_attacks(Color::Black),
        );
    }
}

// ---- Pseudo-attack dispatch --------------------------------------

#[test]
fn pseudo_attacks_dispatch_matches_specialised_tables() {
    let s = Square::E4;
    assert_eq!(pseudo_attacks(PieceType::Knight, s), knight_attacks(s));
    assert_eq!(pseudo_attacks(PieceType::King, s), king_attacks(s));
    assert_eq!(
        pseudo_attacks(PieceType::Bishop, s),
        bishop_pseudo_attacks(s)
    );
    assert_eq!(pseudo_attacks(PieceType::Rook, s), rook_pseudo_attacks(s));
    assert_eq!(pseudo_attacks(PieceType::Queen, s), queen_pseudo_attacks(s));
    // Pawns aren't covered by pseudo-attacks (color-dependent).
    assert_eq!(pseudo_attacks(PieceType::Pawn, s), Bitboard::EMPTY);
}
