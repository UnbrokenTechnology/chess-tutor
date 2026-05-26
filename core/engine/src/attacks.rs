//! Compile-time attack tables that don't depend on the occupancy.
//!
//! Every table in this file is populated at compile time — the engine has no
//! `init()` to call for these. Smaller tables are `const` (inlined by the
//! compiler); the two 64 KB square-pair tables (`LINE_BB`, `BETWEEN_BB`) are
//! `static` to avoid inline-copying 64 KB at every use site. Slider attacks
//! that *do* depend on the occupancy (rook/bishop/queen through occupied
//! squares) will live in a separate module with its magic-bitboard tables.
//!
//! What's here:
//!
//! - `KNIGHT_ATTACKS[sq]`, `KING_ATTACKS[sq]`, `PAWN_ATTACKS[color][sq]` —
//!   the attack set for a leaper, indexed by square.
//! - `BISHOP_PSEUDO[sq]`, `ROOK_PSEUDO[sq]`, `QUEEN_PSEUDO[sq]` — the attack
//!   set for a slider *as if the board were empty*. Useful as a bounding
//!   set: a slider on `s` can only ever attack `c` if `PSEUDO[s]` contains
//!   `c`, regardless of the occupancy. Used to short-circuit pin detection.
//! - `LINE_BB[a][b]` — the full rank/file/diagonal through two squares that
//!   are aligned, or empty if they aren't.
//! - `BETWEEN_BB[a][b]` — squares strictly between `a` and `b` on that line,
//!   exclusive of the endpoints.
//! - `SQUARE_DISTANCE[a][b]` — the king-step (Chebyshev) distance between
//!   two squares.

use crate::bitboard::{king_distance, Bitboard};
use crate::types::{Color, PieceType, Square};

// =========================================================================
// Per-square, per-color king-step distance
// =========================================================================

pub const SQUARE_DISTANCE: [[u8; 64]; 64] = {
    let mut table = [[0u8; 64]; 64];
    let mut a = 0usize;
    while a < 64 {
        let mut b = 0usize;
        while b < 64 {
            table[a][b] = king_distance(Square::from_index(a as u8), Square::from_index(b as u8));
            b += 1;
        }
        a += 1;
    }
    table
};

// =========================================================================
// Leaper attack tables (knight, king, pawn)
// =========================================================================

/// The eight knight jumps, expressed as `(file_delta, rank_delta)` pairs.
const KNIGHT_STEPS: [(i8, i8); 8] = [
    (-2, -1),
    (-2, 1),
    (-1, -2),
    (-1, 2),
    (1, -2),
    (1, 2),
    (2, -1),
    (2, 1),
];

/// The eight king steps.
const KING_STEPS: [(i8, i8); 8] = [
    (-1, -1),
    (-1, 0),
    (-1, 1),
    (0, -1),
    (0, 1),
    (1, -1),
    (1, 0),
    (1, 1),
];

const fn build_leaper_attacks(steps: &[(i8, i8); 8]) -> [Bitboard; 64] {
    let mut table = [Bitboard::EMPTY; 64];
    let mut i = 0usize;
    while i < 64 {
        let file = (i & 7) as i8;
        let rank = (i >> 3) as i8;
        let mut bb: u64 = 0;
        let mut k = 0;
        while k < 8 {
            let (df, dr) = steps[k];
            let nf = file + df;
            let nr = rank + dr;
            if nf >= 0 && nf < 8 && nr >= 0 && nr < 8 {
                bb |= 1u64 << ((nr * 8 + nf) as u32);
            }
            k += 1;
        }
        table[i] = Bitboard(bb);
        i += 1;
    }
    table
}

pub const KNIGHT_ATTACKS: [Bitboard; 64] = build_leaper_attacks(&KNIGHT_STEPS);
pub const KING_ATTACKS: [Bitboard; 64] = build_leaper_attacks(&KING_STEPS);

const fn build_pawn_attacks() -> [[Bitboard; 64]; 2] {
    // White pawns attack +7 (NW) and +9 (NE) provided they don't wrap
    // around the a- or h- file. Black pawns attack -7 (SE) and -9 (SW).
    let white_steps: [(i8, i8); 2] = [(-1, 1), (1, 1)];
    let black_steps: [(i8, i8); 2] = [(-1, -1), (1, -1)];
    let mut table = [[Bitboard::EMPTY; 64]; 2];
    let mut i = 0usize;
    while i < 64 {
        let file = (i & 7) as i8;
        let rank = (i >> 3) as i8;
        let mut w: u64 = 0;
        let mut b: u64 = 0;
        let mut k = 0;
        while k < 2 {
            let (df, dr) = white_steps[k];
            let nf = file + df;
            let nr = rank + dr;
            if nf >= 0 && nf < 8 && nr >= 0 && nr < 8 {
                w |= 1u64 << ((nr * 8 + nf) as u32);
            }
            let (df, dr) = black_steps[k];
            let nf = file + df;
            let nr = rank + dr;
            if nf >= 0 && nf < 8 && nr >= 0 && nr < 8 {
                b |= 1u64 << ((nr * 8 + nf) as u32);
            }
            k += 1;
        }
        table[0][i] = Bitboard(w);
        table[1][i] = Bitboard(b);
        i += 1;
    }
    table
}

pub const PAWN_ATTACKS: [[Bitboard; 64]; 2] = build_pawn_attacks();

// =========================================================================
// Slider pseudo-attacks (empty-board rays)
// =========================================================================

const BISHOP_DIRS: [(i8, i8); 4] = [(-1, -1), (-1, 1), (1, -1), (1, 1)];
const ROOK_DIRS: [(i8, i8); 4] = [(-1, 0), (1, 0), (0, -1), (0, 1)];

/// Cast rays from a square in the given four directions until each one walks
/// off the board. Returns the union of all squares touched.
const fn cast_rays(square_index: usize, dirs: &[(i8, i8); 4]) -> Bitboard {
    let file = (square_index & 7) as i8;
    let rank = (square_index >> 3) as i8;
    let mut bb: u64 = 0;
    let mut d = 0;
    while d < 4 {
        let (df, dr) = dirs[d];
        let mut nf = file + df;
        let mut nr = rank + dr;
        while nf >= 0 && nf < 8 && nr >= 0 && nr < 8 {
            bb |= 1u64 << ((nr * 8 + nf) as u32);
            nf += df;
            nr += dr;
        }
        d += 1;
    }
    Bitboard(bb)
}

const fn build_slider_pseudo(dirs: &[(i8, i8); 4]) -> [Bitboard; 64] {
    let mut table = [Bitboard::EMPTY; 64];
    let mut i = 0usize;
    while i < 64 {
        table[i] = cast_rays(i, dirs);
        i += 1;
    }
    table
}

pub const BISHOP_PSEUDO: [Bitboard; 64] = build_slider_pseudo(&BISHOP_DIRS);
pub const ROOK_PSEUDO: [Bitboard; 64] = build_slider_pseudo(&ROOK_DIRS);

const fn build_queen_pseudo() -> [Bitboard; 64] {
    let mut table = [Bitboard::EMPTY; 64];
    let mut i = 0usize;
    while i < 64 {
        table[i] = Bitboard(BISHOP_PSEUDO[i].raw() | ROOK_PSEUDO[i].raw());
        i += 1;
    }
    table
}

pub const QUEEN_PSEUDO: [Bitboard; 64] = build_queen_pseudo();

// =========================================================================
// Line and between bitboards
// =========================================================================

/// `LINE_BB[a][b]` is the full rank, file, or diagonal through `a` and `b`
/// when they are aligned, and the empty bitboard when they aren't. A square
/// is never aligned with itself.
///
/// Declared `static` rather than `const` because the table is 64 KB — `const`
/// would inline-copy it at every use site.
pub static LINE_BB: [[Bitboard; 64]; 64] = build_line_bb();

/// `BETWEEN_BB[a][b]` is the set of squares strictly between `a` and `b` on
/// their common rank/file/diagonal. Empty if the squares aren't aligned.
pub static BETWEEN_BB: [[Bitboard; 64]; 64] = build_between_bb();

const fn build_line_bb() -> [[Bitboard; 64]; 64] {
    let mut result = [[Bitboard::EMPTY; 64]; 64];
    let mut a = 0usize;
    while a < 64 {
        let fa = (a & 7) as i32;
        let ra = (a >> 3) as i32;
        let mut b = 0usize;
        while b < 64 {
            if a != b {
                let fb = (b & 7) as i32;
                let rb = (b >> 3) as i32;
                let df = fa - fb;
                let dr = ra - rb;
                let on_rank = dr == 0;
                let on_file = df == 0;
                let on_diag_a1h8 = df == dr;
                let on_diag_a8h1 = df == -dr;
                if on_rank || on_file || on_diag_a1h8 || on_diag_a8h1 {
                    // Build the full line by testing each square for membership.
                    let mut line: u64 = 0;
                    let mut i = 0usize;
                    while i < 64 {
                        let fi = (i & 7) as i32;
                        let ri = (i >> 3) as i32;
                        let on_line = if on_rank {
                            ri == ra
                        } else if on_file {
                            fi == fa
                        } else if on_diag_a1h8 {
                            ri - fi == ra - fa
                        } else {
                            ri + fi == ra + fa
                        };
                        if on_line {
                            line |= 1u64 << i;
                        }
                        i += 1;
                    }
                    result[a][b] = Bitboard(line);
                }
            }
            b += 1;
        }
        a += 1;
    }
    result
}

const fn build_between_bb() -> [[Bitboard; 64]; 64] {
    let mut result = [[Bitboard::EMPTY; 64]; 64];
    let mut a = 0usize;
    while a < 64 {
        let mut b = 0usize;
        while b < 64 {
            let line = LINE_BB[a][b].raw();
            if line != 0 {
                // Intersect the line with the range of bit indices strictly
                // between `a` and `b`. This works because for every kind of
                // line we handle here — rank, file, or diagonal — the bits
                // are monotonic along the line: stepping from `min` to `max`
                // by the line's stride only touches line bits whose indices
                // fall in `(min, max)`.
                let (lo, hi) = if a < b { (a, b) } else { (b, a) };
                let mask = if hi > lo + 1 {
                    let lo_bit = 1u64 << (lo + 1);
                    let hi_bit = 1u64 << hi;
                    hi_bit.wrapping_sub(lo_bit)
                } else {
                    0
                };
                result[a][b] = Bitboard(line & mask);
            }
            b += 1;
        }
        a += 1;
    }
    result
}

// =========================================================================
// Accessors
// =========================================================================

pub const fn knight_attacks(square: Square) -> Bitboard {
    KNIGHT_ATTACKS[square.index()]
}

pub const fn king_attacks(square: Square) -> Bitboard {
    KING_ATTACKS[square.index()]
}

pub const fn pawn_attacks_from(color: Color, square: Square) -> Bitboard {
    PAWN_ATTACKS[color.index()][square.index()]
}

pub const fn bishop_pseudo_attacks(square: Square) -> Bitboard {
    BISHOP_PSEUDO[square.index()]
}

pub const fn rook_pseudo_attacks(square: Square) -> Bitboard {
    ROOK_PSEUDO[square.index()]
}

pub const fn queen_pseudo_attacks(square: Square) -> Bitboard {
    QUEEN_PSEUDO[square.index()]
}

pub fn line_bb(a: Square, b: Square) -> Bitboard {
    LINE_BB[a.index()][b.index()]
}

pub fn between_bb(a: Square, b: Square) -> Bitboard {
    BETWEEN_BB[a.index()][b.index()]
}

pub const fn square_distance(a: Square, b: Square) -> u8 {
    SQUARE_DISTANCE[a.index()][b.index()]
}

/// True when `c` lies on the same rank, file, or diagonal as `a` and `b`.
/// Pin and skewer detection is built on top of this.
pub fn aligned(a: Square, b: Square, c: Square) -> bool {
    !Bitboard(LINE_BB[a.index()][b.index()].raw() & (1u64 << c.raw())).is_empty()
}

/// Non-occupancy pseudo-attack dispatch for the four piece types that use
/// pseudo-attack bounding. Pawns aren't in this table because their attacks
/// are color-dependent — use `pawn_attacks_from` instead.
pub const fn pseudo_attacks(piece_type: PieceType, square: Square) -> Bitboard {
    match piece_type {
        PieceType::Knight => KNIGHT_ATTACKS[square.index()],
        PieceType::Bishop => BISHOP_PSEUDO[square.index()],
        PieceType::Rook => ROOK_PSEUDO[square.index()],
        PieceType::Queen => QUEEN_PSEUDO[square.index()],
        PieceType::King => KING_ATTACKS[square.index()],
        PieceType::Pawn => Bitboard::EMPTY,
    }
}

/// Occupancy-aware attack dispatch. For leapers (knight, king) this ignores
/// the occupancy and returns the same result as `pseudo_attacks`. For sliders
/// (bishop, rook, queen) it routes through the magic-bitboard lookup. Pawns
/// aren't handled here; the caller must use `pawn_attacks_from` because
/// pawn attacks depend on color, not occupancy.
pub fn attacks_bb(piece_type: PieceType, square: Square, occupancy: Bitboard) -> Bitboard {
    match piece_type {
        PieceType::Knight => KNIGHT_ATTACKS[square.index()],
        PieceType::King => KING_ATTACKS[square.index()],
        PieceType::Bishop => crate::magics::bishop_attacks(square, occupancy),
        PieceType::Rook => crate::magics::rook_attacks(square, occupancy),
        PieceType::Queen => crate::magics::queen_attacks(square, occupancy),
        PieceType::Pawn => Bitboard::EMPTY,
    }
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
#[path = "attacks_tests.rs"]
mod tests;
