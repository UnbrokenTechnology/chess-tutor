//! Move generation.
//!
//! Two public entry points:
//!
//! - `generate_pseudo_legal_moves(&Position)` — every move that's legal in
//!   the piece-movement sense: the right kind of move for the piece, the
//!   destination is empty or holds an enemy, sliders respect the occupancy.
//!   Does **not** verify that the moving side's king is safe afterwards.
//!   Castling is pseudo-legal only if all the castling-safety checks pass
//!   (king not in check, squares empty, king doesn't pass through or land
//!   on an attacked square) — those are bundled in here rather than in the
//!   legal filter because they're fiddly and specific to castling.
//!
//! - `generate_legal_moves(&mut Position)` — pseudo-legal moves filtered
//!   through a do/undo pass that rejects any move leaving the moving
//!   side's king in check.
//!
//! Perft (counting leaf nodes at a given depth) is the standard
//! correctness test for move generators; helpers and a small battery of
//! known perft values live at the bottom of this file's tests.

use crate::attacks::{attacks_bb, king_attacks, pawn_attacks_from};
use crate::bitboard::{Bitboard, RANK_1, RANK_3, RANK_6, RANK_8};
use crate::position::Position;
use crate::types::{CastlingRights, Color, Direction, Move, PieceType, Square};

// =========================================================================
// MoveList
// =========================================================================

/// Theoretical upper bound on the number of legal moves in any chess
/// position is 218 (per a contrived position by James Tilburg). Round
/// up to 256 for headroom.
pub const MAX_MOVES: usize = 256;

/// Stack-allocated, fixed-capacity list of moves. Used by movegen
/// instead of `Vec<Move>` so each search node doesn't heap-allocate
/// a fresh list — profiling showed ~21% of CPU in the heap allocator
/// when the prior `Vec`-returning API was called per-node.
///
/// Size: ~520 bytes (256 × `Move` + `len`). Per-recursion-frame cost
/// is comfortably within stack budgets even at MAX_PLY.
#[derive(Clone)]
pub struct MoveList {
    moves: [Move; MAX_MOVES],
    len: usize,
}

impl MoveList {
    pub const fn new() -> Self {
        Self {
            moves: [Move::NONE; MAX_MOVES],
            len: 0,
        }
    }

    #[inline]
    pub fn push(&mut self, mv: Move) {
        debug_assert!(self.len < MAX_MOVES, "MoveList overflow");
        self.moves[self.len] = mv;
        self.len += 1;
    }

    #[inline]
    pub fn clear(&mut self) {
        self.len = 0;
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub fn as_slice(&self) -> &[Move] {
        &self.moves[..self.len]
    }

    #[inline]
    pub fn iter(&self) -> std::slice::Iter<'_, Move> {
        self.as_slice().iter()
    }

    #[inline]
    pub fn contains(&self, mv: &Move) -> bool {
        self.as_slice().contains(mv)
    }
}

impl Default for MoveList {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> IntoIterator for &'a MoveList {
    type Item = &'a Move;
    type IntoIter = std::slice::Iter<'a, Move>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

// =========================================================================
// Public entry points
// =========================================================================

/// Generate every pseudo-legal move for the side to move into `out`,
/// clearing it first. May include moves that leave the moving side's
/// king in check — use [`generate_legal_moves`] for a fully filtered
/// list.
pub fn generate_pseudo_legal_moves(pos: &Position, out: &mut MoveList) {
    out.clear();
    emit_pawn_moves(pos, out);
    emit_knight_and_slider_moves(pos, out);
    emit_king_moves(pos, out);
    emit_castling_moves(pos, out);
}

/// Generate every legal move for the side to move into `out`,
/// clearing it first. Filters the pseudo-legal list through a
/// do/undo pass that rejects any move leaving the moving side's
/// king in check. Mutates `pos` temporarily but restores it before
/// returning.
pub fn generate_legal_moves(pos: &mut Position, out: &mut MoveList) {
    let mut pseudo = MoveList::new();
    generate_pseudo_legal_moves(pos, &mut pseudo);
    let us = pos.side_to_move();
    out.clear();
    for &m in &pseudo {
        let state = pos.do_move(m);
        let king_sq = pos.king_square(us);
        let enemy_attackers = pos.attackers_to(king_sq, pos.occupied()) & pos.pieces_by_color(!us);
        pos.undo_move(m, state);
        if enemy_attackers.is_empty() {
            out.push(m);
        }
    }
}

/// Convenience wrapper that allocates a fresh `Vec<Move>`. Use only
/// from non-hot-path code (CLI helpers, tests) — search-internal
/// callers should fill a stack-allocated [`MoveList`] via
/// [`generate_legal_moves`] to avoid the per-node heap allocation.
pub fn legal_moves_vec(pos: &mut Position) -> Vec<Move> {
    let mut list = MoveList::new();
    generate_legal_moves(pos, &mut list);
    list.as_slice().to_vec()
}

/// Convenience wrapper that allocates a fresh `Vec<Move>`. See
/// [`legal_moves_vec`] for the rationale and when to prefer the
/// in-place form.
pub fn pseudo_legal_moves_vec(pos: &Position) -> Vec<Move> {
    let mut list = MoveList::new();
    generate_pseudo_legal_moves(pos, &mut list);
    list.as_slice().to_vec()
}

// =========================================================================
// Pawn moves
// =========================================================================

fn emit_pawn_moves(pos: &Position, moves: &mut MoveList) {
    let us = pos.side_to_move();
    let them = !us;
    let pawns = pos.pieces_of(us, PieceType::Pawn);
    let empty = !pos.occupied();
    let enemies = pos.pieces_by_color(them);

    // Per-color constants. Double-push rank is the rank a pawn arrives at
    // after its first step (3 for white, 6 for black); from there it can
    // push one more step if that square is empty. Promotion rank is the
    // pawn's last rank (8 for white, 1 for black).
    let (push, dpush_rank, promo_rank, cap_ne, cap_nw) = match us {
        Color::White => (
            Direction::NORTH,
            RANK_3,
            RANK_8,
            Direction::NORTH_EAST,
            Direction::NORTH_WEST,
        ),
        Color::Black => (
            Direction::SOUTH,
            RANK_6,
            RANK_1,
            Direction::SOUTH_EAST,
            Direction::SOUTH_WEST,
        ),
    };

    // ---- Single pushes (including promotions by pushing) -------------
    let single_pushed = pawns.shift(push) & empty;
    emit_pawn_quiet_pushes(single_pushed & !promo_rank, push, moves);
    emit_pawn_promotions(single_pushed & promo_rank, push, moves);

    // ---- Double pushes ----------------------------------------------
    // Only pawns whose single-push lands on the double-push-enable rank
    // (rank 3 / rank 6) can continue. The second step must also be empty.
    let double_pushed = (single_pushed & dpush_rank).shift(push) & empty;
    let mut bb = double_pushed;
    while !bb.is_empty() {
        let to = bb.pop_lsb();
        let from = to - push - push;
        moves.push(Move::normal(from, to));
    }

    // ---- Diagonal captures (including captures-with-promotion) -------
    for cap_dir in [cap_ne, cap_nw] {
        let captures = pawns.shift(cap_dir) & enemies;
        emit_pawn_quiet_pushes(captures & !promo_rank, cap_dir, moves);
        emit_pawn_promotions(captures & promo_rank, cap_dir, moves);
    }

    // ---- En-passant -------------------------------------------------
    if let Some(ep_sq) = pos.en_passant() {
        // Our pawns that could capture onto `ep_sq` are those on squares
        // from which a pawn of our color attacks `ep_sq`. By the pawn
        // attack-symmetry trick that's the pawn_attacks_from of the
        // opposite color on `ep_sq`.
        let ep_attackers = pawn_attacks_from(them, ep_sq) & pawns;
        let mut bb = ep_attackers;
        while !bb.is_empty() {
            let from = bb.pop_lsb();
            moves.push(Move::en_passant(from, ep_sq));
        }
    }
}

/// Pop each bit from `targets`, compute the source square by stepping back
/// along `step`, and emit a normal move.
fn emit_pawn_quiet_pushes(targets: Bitboard, step: Direction, moves: &mut MoveList) {
    let mut bb = targets;
    while !bb.is_empty() {
        let to = bb.pop_lsb();
        let from = to - step;
        moves.push(Move::normal(from, to));
    }
}

/// For each bit in `targets`, emit the four promotion moves.
fn emit_pawn_promotions(targets: Bitboard, step: Direction, moves: &mut MoveList) {
    let mut bb = targets;
    while !bb.is_empty() {
        let to = bb.pop_lsb();
        let from = to - step;
        for promoted in [
            PieceType::Queen,
            PieceType::Rook,
            PieceType::Bishop,
            PieceType::Knight,
        ] {
            moves.push(Move::promotion(from, to, promoted));
        }
    }
}

// =========================================================================
// Knight, bishop, rook, queen moves
// =========================================================================

fn emit_knight_and_slider_moves(pos: &Position, moves: &mut MoveList) {
    let us = pos.side_to_move();
    let our_pieces = pos.pieces_by_color(us);
    let occupancy = pos.occupied();

    for pt in [
        PieceType::Knight,
        PieceType::Bishop,
        PieceType::Rook,
        PieceType::Queen,
    ] {
        let mut pieces = pos.pieces_of(us, pt);
        while !pieces.is_empty() {
            let from = pieces.pop_lsb();
            let mut targets = attacks_bb(pt, from, occupancy) & !our_pieces;
            while !targets.is_empty() {
                let to = targets.pop_lsb();
                moves.push(Move::normal(from, to));
            }
        }
    }
}

// =========================================================================
// King moves (non-castling)
// =========================================================================

fn emit_king_moves(pos: &Position, moves: &mut MoveList) {
    let us = pos.side_to_move();
    let our_pieces = pos.pieces_by_color(us);
    let from = pos.king_square(us);
    let mut targets = king_attacks(from) & !our_pieces;
    while !targets.is_empty() {
        let to = targets.pop_lsb();
        moves.push(Move::normal(from, to));
    }
}

// =========================================================================
// Castling
// =========================================================================

fn emit_castling_moves(pos: &Position, moves: &mut MoveList) {
    let us = pos.side_to_move();
    let them = !us;
    let rights = pos.castling_rights();
    let king_sq = pos.king_square(us);

    // The precondition for any castling move: the king isn't currently in
    // check. Check once up front so we don't repeat the attacker scan for
    // each side.
    let occupancy = pos.occupied();
    let enemy_pieces = pos.pieces_by_color(them);
    let king_attackers = pos.attackers_to(king_sq, occupancy) & enemy_pieces;
    if !king_attackers.is_empty() {
        return;
    }

    // Per-color destination squares. The rook's path isn't part of the
    // safety check (only the king's path matters) but the rook's path must
    // be empty all the same.
    let (ks_right, qs_right, ks_empty, qs_empty, ks_safe, qs_safe, ks_to, qs_to) = match us {
        Color::White => (
            CastlingRights::WHITE_KING,
            CastlingRights::WHITE_QUEEN,
            // Kingside empty: f1, g1.
            [Square::F1, Square::G1].as_slice(),
            // Queenside empty: b1, c1, d1.
            [Square::B1, Square::C1, Square::D1].as_slice(),
            // Kingside safe (king passes / lands): f1, g1. (e1 covered by
            // the "king not in check" precondition.)
            [Square::F1, Square::G1].as_slice(),
            // Queenside safe: d1, c1. (b1 is only crossed by the rook.)
            [Square::D1, Square::C1].as_slice(),
            Square::G1,
            Square::C1,
        ),
        Color::Black => (
            CastlingRights::BLACK_KING,
            CastlingRights::BLACK_QUEEN,
            [Square::F8, Square::G8].as_slice(),
            [Square::B8, Square::C8, Square::D8].as_slice(),
            [Square::F8, Square::G8].as_slice(),
            [Square::D8, Square::C8].as_slice(),
            Square::G8,
            Square::C8,
        ),
    };

    if rights.contains(ks_right)
        && ks_empty.iter().all(|s| pos.piece_on(*s).is_none())
        && ks_safe
            .iter()
            .all(|s| (pos.attackers_to(*s, occupancy) & enemy_pieces).is_empty())
    {
        moves.push(Move::castling(king_sq, ks_to));
    }

    if rights.contains(qs_right)
        && qs_empty.iter().all(|s| pos.piece_on(*s).is_none())
        && qs_safe
            .iter()
            .all(|s| (pos.attackers_to(*s, occupancy) & enemy_pieces).is_empty())
    {
        moves.push(Move::castling(king_sq, qs_to));
    }
}

// =========================================================================
// Perft
// =========================================================================

/// Leaf-node count at the given depth. The workhorse correctness test for
/// move generators: for well-known positions the counts are published and
/// any deviation points to a movegen bug.
pub fn perft(pos: &mut Position, depth: u32) -> u64 {
    if depth == 0 {
        return 1;
    }
    let mut moves = MoveList::new();
    generate_legal_moves(pos, &mut moves);
    if depth == 1 {
        return moves.len() as u64;
    }
    let mut nodes = 0u64;
    for &m in &moves {
        let state = pos.do_move(m);
        nodes += perft(pos, depth - 1);
        pos.undo_move(m, state);
    }
    nodes
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
#[path = "movegen_tests.rs"]
mod tests;
