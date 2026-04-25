//! 64-bit bitboards and the const-expr operations over them.
//!
//! A `Bitboard` is a set of squares represented as a `u64` with bit `i`
//! meaning "square `i` is in the set," using the row-major layout defined in
//! `types.rs` (bit 0 = a1, bit 7 = h1, bit 56 = a8, bit 63 = h8).
//!
//! Only operations that don't need runtime-populated tables live here:
//! masks of files/ranks/regions, directional shifts, pawn attack projections,
//! bit manipulation primitives, and king-move distance. The runtime tables
//! (slider magics, pseudo-attacks by piece type, line-between, per-pair
//! chebyshev distance) will land in a separate module once we need them for
//! move generation and evaluation.

use std::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, BitXor, BitXorAssign, Not, Shl, Shr};

use crate::types::{Color, Direction, File, Rank, Square};

// =========================================================================
// Bitboard type
// =========================================================================

/// A set of squares encoded in 64 bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[repr(transparent)]
pub struct Bitboard(pub u64);

impl Bitboard {
    pub const EMPTY: Bitboard = Bitboard(0);
    pub const ALL: Bitboard = Bitboard(!0u64);

    /// `true` iff no square is set.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// `true` iff at least one square is set.
    pub const fn any(self) -> bool {
        self.0 != 0
    }

    /// `true` iff `square` is in the set.
    pub const fn contains(self, square: Square) -> bool {
        (self.0 & (1u64 << square.raw())) != 0
    }

    /// Add `square` to the set.
    pub const fn with(self, square: Square) -> Bitboard {
        Bitboard(self.0 | (1u64 << square.raw()))
    }

    /// Remove `square` from the set.
    pub const fn without(self, square: Square) -> Bitboard {
        Bitboard(self.0 & !(1u64 << square.raw()))
    }

    /// Number of squares in the set.
    pub const fn popcount(self) -> u32 {
        self.0.count_ones()
    }

    /// `true` iff more than one square is set. Cheaper than `popcount() > 1`.
    pub const fn more_than_one(self) -> bool {
        (self.0 & self.0.wrapping_sub(1)) != 0
    }

    /// The least-significant square in the set. Caller must ensure `self` is
    /// non-empty; `trailing_zeros` on zero is 64 which is `Square::NONE`, but
    /// production code should guard it.
    pub const fn lsb(self) -> Square {
        Square::from_index(self.0.trailing_zeros() as u8)
    }

    /// The most-significant square in the set. Caller must ensure `self` is
    /// non-empty.
    pub const fn msb(self) -> Square {
        Square::from_index(63 - self.0.leading_zeros() as u8)
    }

    /// Remove and return the least-significant square. `self` must be non-empty.
    pub fn pop_lsb(&mut self) -> Square {
        let s = self.lsb();
        self.0 &= self.0 - 1;
        s
    }

    /// The front-most square from `color`'s point of view: MSB for white
    /// (largest rank number), LSB for black (smallest rank number).
    pub const fn frontmost(self, color: Color) -> Square {
        match color {
            Color::White => self.msb(),
            Color::Black => self.lsb(),
        }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }

    // -- Directional shifts --------------------------------------------

    pub const fn shift_north(self) -> Bitboard {
        Bitboard(self.0 << 8)
    }

    pub const fn shift_south(self) -> Bitboard {
        Bitboard(self.0 >> 8)
    }

    pub const fn shift_east(self) -> Bitboard {
        Bitboard((self.0 & !FILE_H.0) << 1)
    }

    pub const fn shift_west(self) -> Bitboard {
        Bitboard((self.0 & !FILE_A.0) >> 1)
    }

    pub const fn shift_north_east(self) -> Bitboard {
        Bitboard((self.0 & !FILE_H.0) << 9)
    }

    pub const fn shift_north_west(self) -> Bitboard {
        Bitboard((self.0 & !FILE_A.0) << 7)
    }

    pub const fn shift_south_east(self) -> Bitboard {
        Bitboard((self.0 & !FILE_H.0) >> 7)
    }

    pub const fn shift_south_west(self) -> Bitboard {
        Bitboard((self.0 & !FILE_A.0) >> 9)
    }

    pub const fn shift_north_north(self) -> Bitboard {
        Bitboard(self.0 << 16)
    }

    pub const fn shift_south_south(self) -> Bitboard {
        Bitboard(self.0 >> 16)
    }

    /// Dispatch a shift by one of the eight king-move directions (or N+N, S+S).
    /// Unknown directions return the empty set.
    pub fn shift(self, direction: Direction) -> Bitboard {
        match direction.0 {
            8 => self.shift_north(),
            -8 => self.shift_south(),
            1 => self.shift_east(),
            -1 => self.shift_west(),
            9 => self.shift_north_east(),
            7 => self.shift_north_west(),
            -7 => self.shift_south_east(),
            -9 => self.shift_south_west(),
            16 => self.shift_north_north(),
            -16 => self.shift_south_south(),
            _ => Bitboard::EMPTY,
        }
    }

    /// Bitboard of squares attacked by any of the pawns in `self`, of the given
    /// color. Cheaper than iterating pawns one at a time because both pawns on
    /// the same rank share the same shift.
    pub const fn pawn_attacks(self, color: Color) -> Bitboard {
        match color {
            Color::White => Bitboard(self.shift_north_west().0 | self.shift_north_east().0),
            Color::Black => Bitboard(self.shift_south_west().0 | self.shift_south_east().0),
        }
    }

    /// Bitboard of squares attacked *by two or more* pawns in `self`. Useful
    /// for king-safety and outpost evaluation.
    pub const fn pawn_double_attacks(self, color: Color) -> Bitboard {
        match color {
            Color::White => Bitboard(self.shift_north_west().0 & self.shift_north_east().0),
            Color::Black => Bitboard(self.shift_south_west().0 & self.shift_south_east().0),
        }
    }
}

/// Single-square bitboard.
pub const fn square_bb(square: Square) -> Bitboard {
    Bitboard(1u64 << square.raw())
}

// =========================================================================
// Bitwise operator impls
// =========================================================================

impl BitAnd for Bitboard {
    type Output = Bitboard;
    fn bitand(self, rhs: Bitboard) -> Bitboard {
        Bitboard(self.0 & rhs.0)
    }
}

impl BitOr for Bitboard {
    type Output = Bitboard;
    fn bitor(self, rhs: Bitboard) -> Bitboard {
        Bitboard(self.0 | rhs.0)
    }
}

impl BitXor for Bitboard {
    type Output = Bitboard;
    fn bitxor(self, rhs: Bitboard) -> Bitboard {
        Bitboard(self.0 ^ rhs.0)
    }
}

impl Not for Bitboard {
    type Output = Bitboard;
    fn not(self) -> Bitboard {
        Bitboard(!self.0)
    }
}

impl BitAndAssign for Bitboard {
    fn bitand_assign(&mut self, rhs: Bitboard) {
        self.0 &= rhs.0;
    }
}

impl BitOrAssign for Bitboard {
    fn bitor_assign(&mut self, rhs: Bitboard) {
        self.0 |= rhs.0;
    }
}

impl BitXorAssign for Bitboard {
    fn bitxor_assign(&mut self, rhs: Bitboard) {
        self.0 ^= rhs.0;
    }
}

impl BitAnd<Square> for Bitboard {
    type Output = Bitboard;
    fn bitand(self, rhs: Square) -> Bitboard {
        self & square_bb(rhs)
    }
}

impl BitOr<Square> for Bitboard {
    type Output = Bitboard;
    fn bitor(self, rhs: Square) -> Bitboard {
        self | square_bb(rhs)
    }
}

impl BitXor<Square> for Bitboard {
    type Output = Bitboard;
    fn bitxor(self, rhs: Square) -> Bitboard {
        self ^ square_bb(rhs)
    }
}

impl Shl<u32> for Bitboard {
    type Output = Bitboard;
    fn shl(self, rhs: u32) -> Bitboard {
        Bitboard(self.0 << rhs)
    }
}

impl Shr<u32> for Bitboard {
    type Output = Bitboard;
    fn shr(self, rhs: u32) -> Bitboard {
        Bitboard(self.0 >> rhs)
    }
}

// =========================================================================
// Iteration
// =========================================================================

/// Iterator that yields the squares of a bitboard in ascending order and
/// consumes the bitboard as it goes.
pub struct Squares(pub Bitboard);

impl Iterator for Squares {
    type Item = Square;
    fn next(&mut self) -> Option<Square> {
        if self.0.is_empty() {
            None
        } else {
            Some(self.0.pop_lsb())
        }
    }
}

impl IntoIterator for Bitboard {
    type Item = Square;
    type IntoIter = Squares;
    fn into_iter(self) -> Squares {
        Squares(self)
    }
}

// =========================================================================
// File / rank / region masks
// =========================================================================

pub const FILE_A: Bitboard = Bitboard(0x0101_0101_0101_0101);
pub const FILE_B: Bitboard = Bitboard(0x0101_0101_0101_0101 << 1);
pub const FILE_C: Bitboard = Bitboard(0x0101_0101_0101_0101 << 2);
pub const FILE_D: Bitboard = Bitboard(0x0101_0101_0101_0101 << 3);
pub const FILE_E: Bitboard = Bitboard(0x0101_0101_0101_0101 << 4);
pub const FILE_F: Bitboard = Bitboard(0x0101_0101_0101_0101 << 5);
pub const FILE_G: Bitboard = Bitboard(0x0101_0101_0101_0101 << 6);
pub const FILE_H: Bitboard = Bitboard(0x0101_0101_0101_0101 << 7);

pub const RANK_1: Bitboard = Bitboard(0xFF);
pub const RANK_2: Bitboard = Bitboard(0xFF << 8);
pub const RANK_3: Bitboard = Bitboard(0xFF << 16);
pub const RANK_4: Bitboard = Bitboard(0xFF << 24);
pub const RANK_5: Bitboard = Bitboard(0xFF << 32);
pub const RANK_6: Bitboard = Bitboard(0xFF << 40);
pub const RANK_7: Bitboard = Bitboard(0xFF << 48);
pub const RANK_8: Bitboard = Bitboard(0xFF << 56);

/// The a1..h8 diagonal's dark-square pattern. Doubles as a "half the board"
/// checker useful for bishop-color tests and mop-up endgames.
pub const DARK_SQUARES: Bitboard = Bitboard(0xAA55_AA55_AA55_AA55);
pub const LIGHT_SQUARES: Bitboard = Bitboard(0x55AA_55AA_55AA_55AA);

pub const QUEEN_SIDE: Bitboard = Bitboard(FILE_A.0 | FILE_B.0 | FILE_C.0 | FILE_D.0);
pub const CENTER_FILES: Bitboard = Bitboard(FILE_C.0 | FILE_D.0 | FILE_E.0 | FILE_F.0);
pub const KING_SIDE: Bitboard = Bitboard(FILE_E.0 | FILE_F.0 | FILE_G.0 | FILE_H.0);
pub const CENTER: Bitboard = Bitboard((FILE_D.0 | FILE_E.0) & (RANK_4.0 | RANK_5.0));

/// The "king flank" for each file: roughly the three files centred on the
/// king's file, clamped at the edges. Indexed by `File::index()`.
pub const KING_FLANK: [Bitboard; 8] = [
    Bitboard(QUEEN_SIDE.0 ^ FILE_D.0),
    Bitboard(QUEEN_SIDE.0),
    Bitboard(QUEEN_SIDE.0),
    Bitboard(CENTER_FILES.0),
    Bitboard(CENTER_FILES.0),
    Bitboard(KING_SIDE.0),
    Bitboard(KING_SIDE.0),
    Bitboard(KING_SIDE.0 ^ FILE_E.0),
];

const FILE_MASKS: [Bitboard; 8] = [
    FILE_A, FILE_B, FILE_C, FILE_D, FILE_E, FILE_F, FILE_G, FILE_H,
];
const RANK_MASKS: [Bitboard; 8] = [
    RANK_1, RANK_2, RANK_3, RANK_4, RANK_5, RANK_6, RANK_7, RANK_8,
];

pub const fn file_bb(file: File) -> Bitboard {
    FILE_MASKS[file.index()]
}

pub const fn rank_bb(rank: Rank) -> Bitboard {
    RANK_MASKS[rank.index()]
}

/// Files immediately adjacent to `square`'s file. For a-file returns file b;
/// for h-file returns file g; otherwise returns both neighbours.
pub const fn adjacent_files_bb(square: Square) -> Bitboard {
    let f = file_bb(square.file());
    Bitboard(f.shift_east().0 | f.shift_west().0)
}

/// All squares on ranks strictly in front of `square` from `color`'s point of
/// view. For white on d3 that's ranks 4..=8; for black on d3 that's ranks 1..=2.
pub const fn forward_ranks_bb(color: Color, square: Square) -> Bitboard {
    let rank = square.rank().index();
    match color {
        // All ranks above the current rank. Rank 8 has nothing in front.
        Color::White => {
            let keep_mask: u64 = if rank == 7 {
                0
            } else {
                u64::MAX << (8 * (rank + 1))
            };
            Bitboard(keep_mask)
        }
        // All ranks below the current rank. Rank 1 has nothing in front.
        Color::Black => {
            let keep_mask: u64 = if rank == 0 {
                0
            } else {
                (1u64 << (8 * rank)) - 1
            };
            Bitboard(keep_mask)
        }
    }
}

/// The file in front of `square` from `color`'s point of view, not including
/// `square` itself.
pub const fn forward_file_bb(color: Color, square: Square) -> Bitboard {
    Bitboard(forward_ranks_bb(color, square).0 & file_bb(square.file()).0)
}

/// All squares on adjacent files that a pawn on `square` (of `color`) could
/// ever attack if it advances down its file. Used for backward-pawn detection.
pub const fn pawn_attack_span(color: Color, square: Square) -> Bitboard {
    Bitboard(forward_ranks_bb(color, square).0 & adjacent_files_bb(square).0)
}

/// All squares an enemy pawn or pawn-defender must occupy to stop a pawn on
/// `square` (of `color`) from promoting: the file in front plus both adjacent
/// files in front. Used for passed-pawn detection.
pub const fn passed_pawn_span(color: Color, square: Square) -> Bitboard {
    Bitboard(
        forward_ranks_bb(color, square).0
            & (adjacent_files_bb(square).0 | file_bb(square.file()).0),
    )
}

/// True when two squares are on differently-coloured tiles.
pub const fn opposite_colors(a: Square, b: Square) -> bool {
    let ra = (a.raw() & 7) ^ (a.raw() >> 3);
    let rb = (b.raw() & 7) ^ (b.raw() >> 3);
    (ra & 1) != (rb & 1)
}

// =========================================================================
// King-step distance (file / rank / chebyshev)
// =========================================================================

/// The file-distance between two squares.
pub const fn file_distance(a: Square, b: Square) -> u8 {
    (a.raw() & 7).abs_diff(b.raw() & 7)
}

/// The rank-distance between two squares.
pub const fn rank_distance(a: Square, b: Square) -> u8 {
    (a.raw() >> 3).abs_diff(b.raw() >> 3)
}

/// Chebyshev (king-step) distance between two squares.
pub const fn king_distance(a: Square, b: Square) -> u8 {
    let fd = file_distance(a, b);
    let rd = rank_distance(a, b);
    if fd > rd {
        fd
    } else {
        rd
    }
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
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
}
