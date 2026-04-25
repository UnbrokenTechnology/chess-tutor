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
mod tests {
    use super::*;
    use crate::types::{MoveKind, Piece, Rank};

    // ---- Pseudo-legal counts from startpos --------------------------

    #[test]
    fn startpos_has_20_pseudo_legal_moves() {
        let p = Position::startpos();
        let moves = pseudo_legal_moves_vec(&p);
        // 16 pawn moves (8 pawns × {single push, double push}) + 4 knight
        // moves (b1→{a3, c3}, g1→{f3, h3}). No other piece has a legal move
        // at start: bishops/queens/rooks/king are blocked by pawns, and
        // castling requires clear squares.
        assert_eq!(moves.len(), 20, "startpos pseudo-legal move count");
    }

    #[test]
    fn startpos_has_8_pawn_single_pushes_and_8_double_pushes() {
        let p = Position::startpos();
        let moves = pseudo_legal_moves_vec(&p);
        let pawn_moves: Vec<_> = moves
            .iter()
            .filter(|m| p.piece_on(m.from()) == Some(Piece::WhitePawn))
            .collect();
        assert_eq!(pawn_moves.len(), 16);
        let single = pawn_moves
            .iter()
            .filter(|m| m.to().rank() == Rank::R3)
            .count();
        let double = pawn_moves
            .iter()
            .filter(|m| m.to().rank() == Rank::R4)
            .count();
        assert_eq!(single, 8);
        assert_eq!(double, 8);
    }

    #[test]
    fn startpos_has_no_promotions() {
        let p = Position::startpos();
        assert!(!pseudo_legal_moves_vec(&p)
            .iter()
            .any(|m| m.kind() == MoveKind::Promotion));
    }

    // ---- Promotion generation ---------------------------------------

    #[test]
    fn pawn_on_seventh_rank_has_four_promotion_pushes() {
        // White pawn on a7 can push to a8 with four promotions.
        let p = Position::from_fen("4k3/P7/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        let moves = pseudo_legal_moves_vec(&p);
        let promos: Vec<_> = moves
            .iter()
            .filter(|m| m.kind() == MoveKind::Promotion && m.from() == Square::A7)
            .collect();
        assert_eq!(promos.len(), 4);
        let pieces: Vec<_> = promos.iter().map(|m| m.promoted_to()).collect();
        assert!(pieces.contains(&PieceType::Queen));
        assert!(pieces.contains(&PieceType::Rook));
        assert!(pieces.contains(&PieceType::Bishop));
        assert!(pieces.contains(&PieceType::Knight));
    }

    #[test]
    fn pawn_capturing_to_promote_emits_four_per_capture() {
        // White pawn a7 captures a rook on b8: a7xb8=Q/R/B/N. Also the
        // straight push a7→a8 (promo). Total: 4 capture-promos + 4 push-promos.
        let p = Position::from_fen("1r2k3/P7/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        let moves = pseudo_legal_moves_vec(&p);
        let promos: Vec<_> = moves
            .iter()
            .filter(|m| m.kind() == MoveKind::Promotion && m.from() == Square::A7)
            .collect();
        assert_eq!(promos.len(), 8);
    }

    // ---- En passant --------------------------------------------------

    #[test]
    fn en_passant_generation_emits_one_ep_move_per_attacker() {
        // White pawn on e5, black pawn on d5 from a previous double push.
        // EP target d6. White should have an ep capture: e5xd6.
        let p = Position::from_fen("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 3").unwrap();
        let moves = pseudo_legal_moves_vec(&p);
        let ep: Vec<_> = moves
            .iter()
            .filter(|m| m.kind() == MoveKind::EnPassant)
            .collect();
        assert_eq!(ep.len(), 1);
        assert_eq!(ep[0].from(), Square::E5);
        assert_eq!(ep[0].to(), Square::D6);
    }

    // ---- Knight and slider generation -------------------------------

    #[test]
    fn lone_knight_in_center_has_eight_moves() {
        let p = Position::from_fen("4k3/8/8/4N3/8/8/8/4K3 w - - 0 1").unwrap();
        let moves = pseudo_legal_moves_vec(&p);
        let n_moves: Vec<_> = moves.iter().filter(|m| m.from() == Square::E5).collect();
        assert_eq!(n_moves.len(), 8);
    }

    #[test]
    fn bishop_is_blocked_by_friendly_piece() {
        // White bishop on a1, white pawn on d4 blocks the diagonal.
        // Bishop attacks b2, c3 (blocked by d4 which is friendly).
        let p = Position::from_fen("4k3/8/8/8/3P4/8/8/B3K3 w - - 0 1").unwrap();
        let moves = pseudo_legal_moves_vec(&p);
        let b_moves: Vec<_> = moves.iter().filter(|m| m.from() == Square::A1).collect();
        let targets: Vec<_> = b_moves.iter().map(|m| m.to()).collect();
        assert!(targets.contains(&Square::B2));
        assert!(targets.contains(&Square::C3));
        // d4 is friendly: not a target.
        assert!(!targets.contains(&Square::D4));
        // Everything past d4 is unreachable.
        assert!(!targets.contains(&Square::E5));
    }

    #[test]
    fn rook_captures_first_enemy_on_ray() {
        // White rook on a1, black knight on a5. Rook reaches a2..a5
        // (capturing the knight), and nothing beyond.
        let p = Position::from_fen("4k3/8/8/n7/8/8/8/R3K3 w - - 0 1").unwrap();
        let moves = pseudo_legal_moves_vec(&p);
        let r_moves: Vec<_> = moves.iter().filter(|m| m.from() == Square::A1).collect();
        let targets: Vec<_> = r_moves.iter().map(|m| m.to()).collect();
        assert!(targets.contains(&Square::A5));
        assert!(!targets.contains(&Square::A6));
        assert!(!targets.contains(&Square::A7));
    }

    // ---- Castling generation -----------------------------------------

    #[test]
    fn castling_is_generated_when_all_conditions_hold() {
        // Standard back-rank with nothing between king and rook(s).
        let p = Position::from_fen("4k3/8/8/8/8/8/8/R3K2R w KQ - 0 1").unwrap();
        let moves = pseudo_legal_moves_vec(&p);
        let castles: Vec<_> = moves
            .iter()
            .filter(|m| m.kind() == MoveKind::Castling)
            .collect();
        assert_eq!(castles.len(), 2);
        let tos: Vec<_> = castles.iter().map(|m| m.to()).collect();
        assert!(tos.contains(&Square::G1));
        assert!(tos.contains(&Square::C1));
    }

    #[test]
    fn castling_blocked_by_piece_in_the_way() {
        // Friendly bishop on f1 blocks kingside castling. Queenside still fine.
        let p = Position::from_fen("4k3/8/8/8/8/8/8/R3KB1R w KQ - 0 1").unwrap();
        let moves = pseudo_legal_moves_vec(&p);
        let castles: Vec<_> = moves
            .iter()
            .filter(|m| m.kind() == MoveKind::Castling)
            .collect();
        assert_eq!(castles.len(), 1);
        assert_eq!(castles[0].to(), Square::C1);
    }

    #[test]
    fn cannot_castle_while_in_check() {
        // Black rook on e3 checks white king on e1 up the e-file.
        // Castling should not be generated, even though the physical squares
        // are clear.
        let p = Position::from_fen("4k3/8/8/8/8/4r3/8/R3K2R w KQ - 0 1").unwrap();
        let moves = pseudo_legal_moves_vec(&p);
        let castles: Vec<_> = moves
            .iter()
            .filter(|m| m.kind() == MoveKind::Castling)
            .collect();
        assert!(castles.is_empty());
    }

    #[test]
    fn cannot_castle_through_attacked_square() {
        // Black rook on f8 attacks f1 along the f-file. The white king
        // would pass through f1 to reach g1, so kingside castling is
        // illegal. Queenside is unaffected.
        let p = Position::from_fen("5r2/4k3/8/8/8/8/8/R3K2R w KQ - 0 1").unwrap();
        let moves = pseudo_legal_moves_vec(&p);
        let castles: Vec<_> = moves
            .iter()
            .filter(|m| m.kind() == MoveKind::Castling)
            .collect();
        assert_eq!(castles.len(), 1);
        assert_eq!(
            castles[0].to(),
            Square::C1,
            "kingside blocked, queenside ok"
        );
    }

    #[test]
    fn can_castle_queenside_even_if_b_file_square_attacked() {
        // Black bishop on a3 attacks b2 (but b1 is only crossed by the
        // rook, not the king — so queenside castling is legal).
        // Attackers of b1 from black bishop on a3? Bishop on a3 attacks
        // b2, b4, c1, c5... actually bishop on a3 moves diagonally:
        // a3→b2, a3→b4, a3→c1 (hits c1 — attacking c1!).
        // So let me pick a different square. Bishop on h3 attacks g2,
        // g4, f1, f5, e6, d7, c8... attacks f1 (kingside problem).
        // Use: black bishop on a6. Attacks: b5, b7, c4, c8, d3, e2, f1.
        // So f1 is attacked → kingside problem, not queenside.
        // For a b-file-only attack on b1: black knight on d2 attacks
        // b1, b3, c4, e4, f3, f1. Wait f1 too, no good.
        // Simpler: rook on b7. Attacks b-file including b1. Nothing else.
        let p = Position::from_fen("4k3/1r6/8/8/8/8/8/R3K2R w KQ - 0 1").unwrap();
        let moves = pseudo_legal_moves_vec(&p);
        let castles: Vec<_> = moves
            .iter()
            .filter(|m| m.kind() == MoveKind::Castling)
            .collect();
        let tos: Vec<_> = castles.iter().map(|m| m.to()).collect();
        // Kingside is unrelated: should still be available. Queenside: b1
        // is attacked but the king doesn't pass through b1 — only d1 and
        // c1 matter for the king's safety.
        assert!(tos.contains(&Square::G1));
        assert!(tos.contains(&Square::C1));
    }

    // ---- Legal filter eliminates king-in-check moves ----------------

    #[test]
    fn legal_filter_removes_king_in_check_moves() {
        // White rook on e2 is pinned to its king on e1 by the black rook on
        // e6. If the white rook steps off the e-file, the king is exposed
        // — the legal filter must reject those moves.
        let p_start = Position::from_fen("4k3/8/4r3/8/8/8/4R3/4K3 w - - 0 1").unwrap();
        let pseudo = pseudo_legal_moves_vec(&p_start);
        let mut p = p_start.clone();
        let legal = legal_moves_vec(&mut p);
        assert!(
            legal.len() < pseudo.len(),
            "some pseudo-legal moves must be rejected by legality"
        );
        // Specifically: the white rook on e2 cannot move off the e-file
        // because it would expose the king.
        let rook_off_efile: Vec<_> = legal
            .iter()
            .filter(|m| m.from() == Square::E2 && m.to().file() != crate::types::File::E)
            .collect();
        assert!(
            rook_off_efile.is_empty(),
            "pinned rook cannot leave the e-file"
        );
    }

    // ---- Perft on known positions ------------------------------------

    /// Perft values for the standard starting position, from the chess
    /// programming wiki / Stockfish test suite:
    /// d1=20, d2=400, d3=8902, d4=197281.
    #[test]
    fn perft_startpos_depth_1() {
        let mut p = Position::startpos();
        assert_eq!(perft(&mut p, 1), 20);
    }

    #[test]
    fn perft_startpos_depth_2() {
        let mut p = Position::startpos();
        assert_eq!(perft(&mut p, 2), 400);
    }

    #[test]
    fn perft_startpos_depth_3() {
        let mut p = Position::startpos();
        assert_eq!(perft(&mut p, 3), 8902);
    }

    /// "Position 2" (Kiwipete) from chessprogramming.org — a tactical
    /// position that exercises captures, castling, en-passant, and
    /// promotions. Known values: d1=48, d2=2039, d3=97862.
    const KIWIPETE: &str = "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1";

    #[test]
    fn perft_kiwipete_depth_1() {
        let mut p = Position::from_fen(KIWIPETE).unwrap();
        assert_eq!(perft(&mut p, 1), 48);
    }

    #[test]
    fn perft_kiwipete_depth_2() {
        let mut p = Position::from_fen(KIWIPETE).unwrap();
        assert_eq!(perft(&mut p, 2), 2039);
    }

    /// Position 3 from the same wiki page — an endgame with lots of
    /// checks. Known values: d1=14, d2=191, d3=2812, d4=43238.
    const POSITION_3: &str = "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1";

    #[test]
    fn perft_position_3_depth_3() {
        let mut p = Position::from_fen(POSITION_3).unwrap();
        assert_eq!(perft(&mut p, 3), 2812);
    }

    /// Position 4 (the "Talkchess" / "KiwiPete variant B" position) —
    /// includes a stalemate trap. Known values: d1=6, d2=264, d3=9467.
    const POSITION_4: &str = "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1";

    #[test]
    fn perft_position_4_depth_2() {
        let mut p = Position::from_fen(POSITION_4).unwrap();
        assert_eq!(perft(&mut p, 2), 264);
    }
}
