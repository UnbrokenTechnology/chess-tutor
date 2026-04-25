//! Zobrist hashing: a 64-bit fingerprint of a chess position.
//!
//! Two positions with identical piece placement, side to move, castling
//! rights, and en-passant target hash to the same key. Each piece-on-square
//! combination contributes a fixed random 64-bit value (by XOR), as does
//! each castling-rights value, each possible en-passant file, and the
//! "black to move" flag.
//!
//! The trick that makes this useful: since XOR is its own inverse, keys can
//! be maintained incrementally. When a piece moves from `f` to `t`, we XOR
//! out `piece_square_key(piece, f)` and XOR in `piece_square_key(piece, t)`
//! — two ops, regardless of how many pieces are on the board. The
//! transposition table uses these keys as its lookup index.
//!
//! The random table is generated at compile time from a seeded xorshift
//! PRNG. Fixed seed, deterministic keys across runs, no startup cost.
//! Collisions are statistically possible (~2^32 positions via the birthday
//! bound) but not a correctness concern: the TT always verifies its entry.

use crate::types::{CastlingRights, Piece, Square};

// =========================================================================
// Compile-time PRNG for filling the tables
// =========================================================================

const SEED: u64 = 0x2545_F491_4F6C_DD1D;

const fn xorshift64(mut x: u64) -> u64 {
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

// =========================================================================
// Tables
// =========================================================================

struct Tables {
    /// Per-piece, per-square random. Indexed by `Piece::index()` which runs
    /// 1..=6 (white) and 9..=14 (black). The unused slots (0, 7, 8, 15) are
    /// left zero and will never be read because callers always hold a valid
    /// `Piece` before indexing.
    piece_square: [[u64; 64]; 16],
    /// Per-castling-rights-mask random. The mask is 4 bits (one per
    /// {white,black}×{king,queen}), so there are 16 values.
    castling: [u64; 16],
    /// Per-file random for the en-passant target. File-indexed, since the
    /// rank is always 3 (white's ep target) or 6 (black's) and fully
    /// determined by side-to-move.
    ep_file: [u64; 8],
    /// XOR-ed into the key when it's Black's turn.
    side_to_move: u64,
    /// Base value for the pawn-only key, XOR-ed in unconditionally so an
    /// empty pawn structure hashes to a stable non-zero value. Future pawn
    /// hash tables use zero as an unoccupied-slot sentinel; this constant
    /// keeps real positions from colliding with that sentinel.
    no_pawns: u64,
}

const fn build_tables() -> Tables {
    let mut state = SEED;
    let mut piece_square = [[0u64; 64]; 16];
    let mut castling = [0u64; 16];
    let mut ep_file = [0u64; 8];

    let mut p = 0;
    while p < 16 {
        let mut s = 0;
        while s < 64 {
            state = xorshift64(state);
            piece_square[p][s] = state;
            s += 1;
        }
        p += 1;
    }

    let mut c = 0;
    while c < 16 {
        state = xorshift64(state);
        castling[c] = state;
        c += 1;
    }

    let mut f = 0;
    while f < 8 {
        state = xorshift64(state);
        ep_file[f] = state;
        f += 1;
    }

    state = xorshift64(state);
    let side_to_move = state;

    state = xorshift64(state);
    let no_pawns = state;

    Tables {
        piece_square,
        castling,
        ep_file,
        side_to_move,
        no_pawns,
    }
}

static TABLES: Tables = build_tables();

// =========================================================================
// Public keys
// =========================================================================

pub fn piece_square_key(piece: Piece, square: Square) -> u64 {
    TABLES.piece_square[piece.index()][square.index()]
}

pub fn castling_key(rights: CastlingRights) -> u64 {
    TABLES.castling[rights.0 as usize]
}

pub fn ep_key(square: Square) -> u64 {
    TABLES.ep_file[square.file().index()]
}

pub fn side_to_move_key() -> u64 {
    TABLES.side_to_move
}

/// Base value for the pawn-only Zobrist key. XOR this into the key at
/// construction so empty-pawn-structure positions still hash to a stable
/// non-zero value — keeps future pawn hash tables from confusing "no entry"
/// with "entry for an empty-pawn position."
pub fn no_pawns_key() -> u64 {
    TABLES.no_pawns
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Color, PieceType};

    #[test]
    fn piece_square_keys_are_non_zero_for_real_pieces() {
        for &color in &Color::both() {
            for &pt in &[
                PieceType::Pawn,
                PieceType::Knight,
                PieceType::Bishop,
                PieceType::Rook,
                PieceType::Queen,
                PieceType::King,
            ] {
                let piece = Piece::new(color, pt);
                for i in 0u8..64 {
                    let k = piece_square_key(piece, Square::from_index(i));
                    assert!(
                        k != 0,
                        "piece {:?} on {} produced a zero key",
                        piece,
                        Square::from_index(i).to_algebraic()
                    );
                }
            }
        }
    }

    #[test]
    fn piece_square_keys_are_distinct() {
        // The full set of (piece, square) random keys should produce no
        // collisions. A 64-bit random hashed 12*64 = 768 times: collision
        // probability is vanishingly small, and a duplicate here means the
        // table itself is malformed.
        let mut seen = std::collections::HashSet::with_capacity(768);
        for &color in &Color::both() {
            for &pt in &[
                PieceType::Pawn,
                PieceType::Knight,
                PieceType::Bishop,
                PieceType::Rook,
                PieceType::Queen,
                PieceType::King,
            ] {
                let piece = Piece::new(color, pt);
                for i in 0u8..64 {
                    let k = piece_square_key(piece, Square::from_index(i));
                    assert!(seen.insert(k), "duplicate piece-square key");
                }
            }
        }
    }

    #[test]
    fn castling_key_for_none_differs_from_all() {
        assert_ne!(
            castling_key(CastlingRights::NONE),
            castling_key(CastlingRights::ALL),
        );
    }

    #[test]
    fn ep_key_depends_only_on_file() {
        // Any two squares on the same file must produce the same ep key
        // (ranks are determined by side-to-move, so the rank isn't part of
        // the zobrist contribution).
        assert_eq!(ep_key(Square::E3), ep_key(Square::E6));
        assert_ne!(ep_key(Square::D3), ep_key(Square::E3));
    }

    #[test]
    fn side_to_move_key_is_non_zero() {
        assert_ne!(side_to_move_key(), 0);
    }

    #[test]
    fn no_pawns_key_is_non_zero_and_distinct() {
        // The pawn-only base value must be non-zero (so empty pawn
        // structures don't hash to the empty-slot sentinel) and distinct
        // from the other singletons.
        assert_ne!(no_pawns_key(), 0);
        assert_ne!(no_pawns_key(), side_to_move_key());
    }

    #[test]
    fn xor_is_self_inverse() {
        // Fundamental property the incremental update relies on.
        let key = piece_square_key(Piece::WhitePawn, Square::E2);
        let other = piece_square_key(Piece::WhitePawn, Square::E4);
        let combined = key ^ other;
        assert_eq!(combined ^ other, key);
        assert_eq!(combined ^ key, other);
    }
}
