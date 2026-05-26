use super::*;
use crate::bitboard::square_bb;

// ---- Relevant mask shape ----------------------------------------

#[test]
fn rook_mask_excludes_self_and_edges() {
    // A rook on d4 sees 10 relevant squares: d5, d6, d7 (N), d3, d2 (S),
    // e4, f4, g4 (E), c4, b4 (W). Edge squares d1, d8, a4, h4 and d4
    // itself are all excluded.
    let mask = relevant_mask(&ROOK_DIRS, Square::D4);
    assert_eq!(mask.popcount(), 10);
    assert!(!mask.contains(Square::D4));
    assert!(!mask.contains(Square::D1));
    assert!(!mask.contains(Square::D8));
    for sq in &["d5", "d6", "d7", "d3", "d2", "e4", "f4", "g4", "c4", "b4"] {
        assert!(
            mask.contains(Square::from_algebraic(sq).unwrap()),
            "rook mask on d4 should include {}",
            sq
        );
    }
}

#[test]
fn rook_mask_on_corner_has_twelve_bits() {
    // A rook on a1 sees b1..g1 (6) and a2..a7 (6).
    assert_eq!(relevant_mask(&ROOK_DIRS, Square::A1).popcount(), 12);
    assert_eq!(relevant_mask(&ROOK_DIRS, Square::H1).popcount(), 12);
    assert_eq!(relevant_mask(&ROOK_DIRS, Square::A8).popcount(), 12);
    assert_eq!(relevant_mask(&ROOK_DIRS, Square::H8).popcount(), 12);
}

#[test]
fn bishop_mask_sizes() {
    // Corner bishop: 6 relevant inner-diagonal squares (b2..g7 from a1).
    assert_eq!(relevant_mask(&BISHOP_DIRS, Square::A1).popcount(), 6);
    // Centre bishop: 9 relevant squares.
    assert_eq!(relevant_mask(&BISHOP_DIRS, Square::D4).popcount(), 9);
}

// ---- Ground-truth ray casting ------------------------------------

#[test]
fn rook_from_a1_empty_board_covers_rank_1_and_a_file() {
    let attacks = ray_attacks(&ROOK_DIRS, Square::A1, Bitboard::EMPTY);
    assert_eq!(attacks.popcount(), 14);
    // Entire rank 1 except a1.
    for f in 1..8u8 {
        assert!(attacks.contains(Square::from_index(f)));
    }
    // Entire a-file except a1.
    for r in 1..8u8 {
        assert!(attacks.contains(Square::from_index(r * 8)));
    }
}

#[test]
fn rook_stops_at_first_occupied_square_on_ray() {
    // Rook on a1, blocker on d1. Should attack b1, c1, d1 (capture) — not
    // e1..h1, not anything behind d1 on the rank.
    let occ = square_bb(Square::D1);
    let attacks = ray_attacks(&ROOK_DIRS, Square::A1, occ);
    assert!(attacks.contains(Square::B1));
    assert!(attacks.contains(Square::C1));
    assert!(attacks.contains(Square::D1));
    assert!(!attacks.contains(Square::E1));
    assert!(!attacks.contains(Square::F1));
}

#[test]
fn bishop_from_a1_empty_board_covers_long_diagonal() {
    let attacks = ray_attacks(&BISHOP_DIRS, Square::A1, Bitboard::EMPTY);
    assert_eq!(attacks.popcount(), 7);
    for sq in &["b2", "c3", "d4", "e5", "f6", "g7", "h8"] {
        assert!(attacks.contains(Square::from_algebraic(sq).unwrap()));
    }
}

// ---- Subset enumeration -----------------------------------------

#[test]
fn subset_enumeration_visits_each_subset_exactly_once() {
    // Pick a small but non-trivial mask and verify the Carry-Rippler
    // trick visits each of the 2^popcount subsets exactly once.
    let mask = Bitboard(0b1010_0101);
    let mut seen: Vec<u64> = Vec::new();
    for_each_subset(mask, |s| seen.push(s.raw()));
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 1usize << mask.popcount());
    // Every element must be a subset of the mask.
    for s in seen {
        assert_eq!(s & !mask.raw(), 0);
    }
}

// ---- Magic lookups vs ray casting --------------------------------

#[test]
fn rook_magic_matches_ray_casting_on_empty_board() {
    for i in 0u8..64 {
        let sq = Square::from_index(i);
        assert_eq!(
            rook_attacks(sq, Bitboard::EMPTY),
            ray_attacks(&ROOK_DIRS, sq, Bitboard::EMPTY),
        );
    }
}

#[test]
fn bishop_magic_matches_ray_casting_on_empty_board() {
    for i in 0u8..64 {
        let sq = Square::from_index(i);
        assert_eq!(
            bishop_attacks(sq, Bitboard::EMPTY),
            ray_attacks(&BISHOP_DIRS, sq, Bitboard::EMPTY),
        );
    }
}

#[test]
fn magic_matches_ray_casting_across_every_subset_on_a_few_squares() {
    // Full verification on four representative squares: corner, edge,
    // centre, near-edge. For each, enumerate every subset of the mask
    // and confirm the magic table agrees with the naive ray cast.
    let squares = [Square::A1, Square::E4, Square::D1, Square::H4];
    for &sq in &squares {
        let rm = relevant_mask(&ROOK_DIRS, sq);
        for_each_subset(rm, |occ| {
            assert_eq!(
                rook_attacks(sq, occ),
                ray_attacks(&ROOK_DIRS, sq, occ),
                "rook disagreement on {} with occupancy 0x{:016x}",
                sq.to_algebraic(),
                occ.raw()
            );
        });

        let bm = relevant_mask(&BISHOP_DIRS, sq);
        for_each_subset(bm, |occ| {
            assert_eq!(
                bishop_attacks(sq, occ),
                ray_attacks(&BISHOP_DIRS, sq, occ),
                "bishop disagreement on {} with occupancy 0x{:016x}",
                sq.to_algebraic(),
                occ.raw()
            );
        });
    }
}

#[test]
fn magic_ignores_bits_outside_the_mask() {
    // Occupancy bits that fall outside the relevant mask should not
    // change the computed attacks. Pick a square, compute attacks with
    // a clean occupancy, then OR in some outside bits and confirm the
    // lookup is unchanged.
    let sq = Square::D4;
    let base_occ = square_bb(Square::D6) | square_bb(Square::F4);
    let noisy_occ = base_occ
        | square_bb(Square::D8) // on the d-file edge, outside mask
        | square_bb(Square::A4) // on the 4th rank edge, outside mask
        | square_bb(Square::D4); // the rook's own square, outside mask
    assert_eq!(rook_attacks(sq, base_occ), rook_attacks(sq, noisy_occ));
}

#[test]
fn queen_attacks_are_union_of_rook_and_bishop() {
    let sq = Square::E4;
    let occ = square_bb(Square::E6) | square_bb(Square::B4) | square_bb(Square::H7);
    assert_eq!(
        queen_attacks(sq, occ),
        rook_attacks(sq, occ) | bishop_attacks(sq, occ),
    );
}

#[test]
fn rook_attacks_include_capturing_square() {
    // Rook on a1 with a blocker on a4: the rook attacks a2, a3, and a4
    // (the capture square), but not a5..a8.
    let occ = square_bb(Square::from_algebraic("a4").unwrap());
    let attacks = rook_attacks(Square::A1, occ);
    for sq in &["a2", "a3", "a4"] {
        assert!(attacks.contains(Square::from_algebraic(sq).unwrap()));
    }
    for sq in &["a5", "a6", "a7", "a8"] {
        assert!(!attacks.contains(Square::from_algebraic(sq).unwrap()));
    }
}

#[test]
fn warm_up_is_idempotent() {
    // Calling warm_up before any attack query should not change behaviour.
    warm_up();
    warm_up();
    let r = rook_attacks(Square::E4, Bitboard::EMPTY);
    assert!(r.any());
}
