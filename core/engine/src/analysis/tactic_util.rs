//! Shared tactic-detection primitives, hand-transliterated from
//! lichess-puzzler's `tagger/util.py` (`reference/lichess-puzzler/`,
//! AGPL-3.0 — never shipped, never modified). See
//! [`super::tactic_outcome`]'s `//!` for the full provenance / licensing
//! note (idea-expression dichotomy; this is independently authored Rust,
//! not copied source).
//!
//! These were originally private to [`super::tactic_outcome`]; they moved
//! here once a second consumer appeared (the trapped-piece detector, and
//! — later — the trapped-piece board overlay). Keeping them in one place
//! means "hanging" / "bad spot" / "defended" mean exactly the same thing
//! everywhere in the teaching layer.

use crate::attacks::{attacks_bb, pawn_attacks_from};
use crate::bitboard::{square_bb, Bitboard};
use crate::movegen::legal_moves_vec;
use crate::position::Position;
use crate::types::{Color, Piece, PieceType, Square};

/// Ranking value where the king is the most valuable piece — used for
/// the fork's "is the target worth more than the forker" test, where a
/// forked king (a check) always counts. Mirrors `util.king_values`
/// (P1 N3 B3 R5 Q9 K99).
pub(crate) fn king_value(pt: PieceType) -> i32 {
    match pt {
        PieceType::Pawn => 1,
        PieceType::Knight | PieceType::Bishop => 3,
        PieceType::Rook => 5,
        PieceType::Queen => 9,
        PieceType::King => 99,
    }
}

/// The squares the piece on `from` attacks, given current occupancy.
/// Pawns use their colour-specific capture pattern; every other piece
/// uses the occupancy-aware slider/leaper tables. Empty when `from` is
/// vacant.
pub(crate) fn attacks_from_square(pos: &Position, from: Square) -> Bitboard {
    match pos.piece_on(from) {
        Some(p) => match p.kind() {
            PieceType::Pawn => pawn_attacks_from(p.color(), from),
            other => attacks_bb(other, from, pos.occupied()),
        },
        None => Bitboard::EMPTY,
    }
}

/// Enemy pieces (relative to `pov`) standing on squares the piece on
/// `from` attacks. Mirrors `util.attacked_opponent_squares`. Ordered by
/// ascending square index.
pub(crate) fn attacked_opponent_squares(
    pos: &Position,
    from: Square,
    pov: Color,
) -> Vec<(Piece, Square)> {
    let enemy = !pov;
    let mut out = Vec::new();
    for sq in attacks_from_square(pos, from) & pos.pieces_by_color(enemy) {
        if let Some(p) = pos.piece_on(sq) {
            out.push((p, sq));
        }
    }
    out
}

/// Whether the piece of colour `target_color` on `target_sq` has a
/// defender. Mirrors `util.is_defended`: a direct friendly attacker, or
/// "ray defense" — a friendly slider hidden behind an enemy slider that
/// attacks the square, revealed once that enemy attacker is removed.
fn is_defended(pos: &Position, target_sq: Square, target_color: Color) -> bool {
    let occ = pos.occupied();
    let friends = pos.pieces_by_color(target_color);
    let attackers = pos.attackers_to(target_sq, occ);
    if (attackers & friends).any() {
        return true;
    }
    // Ray defense: an enemy slider attacks the square; removing it from
    // the occupancy may reveal a friendly slider defending from behind.
    let enemy = pos.pieces_by_color(!target_color);
    let sliders = pos.pieces(PieceType::Rook)
        | pos.pieces(PieceType::Bishop)
        | pos.pieces(PieceType::Queen);
    for asq in attackers & enemy & sliders {
        let reduced = occ ^ square_bb(asq);
        if (pos.attackers_to(target_sq, reduced) & friends).any() {
            return true;
        }
    }
    false
}

/// `!is_defended` — the piece is attacked-or-not but has no defender.
/// Mirrors `util.is_hanging` (callers pair it with an "is attacked"
/// check where the distinction matters).
pub(crate) fn is_hanging(pos: &Position, target_sq: Square, target_color: Color) -> bool {
    !is_defended(pos, target_sq, target_color)
}

/// Whether the piece on `target_sq` can be captured by a strictly
/// lower-valued enemy piece (kings excluded). Mirrors
/// `util.can_be_taken_by_lower_piece`.
fn can_be_taken_by_lower_piece(pos: &Position, target_sq: Square) -> bool {
    let Some(target) = pos.piece_on(target_sq) else {
        return false;
    };
    let target_value = king_value(target.kind());
    let enemy = pos.pieces_by_color(!target.color());
    for asq in pos.attackers_to(target_sq, pos.occupied()) & enemy {
        if let Some(attacker) = pos.piece_on(asq) {
            if attacker.kind() != PieceType::King && king_value(attacker.kind()) < target_value {
                return true;
            }
        }
    }
    false
}

/// Whether the piece on `square` is in a "bad spot" — attacked by an
/// enemy AND either hanging or takeable by a lower piece. Mirrors
/// `util.is_in_bad_spot`. `false` for an empty square.
pub(crate) fn is_in_bad_spot(pos: &Position, square: Square) -> bool {
    let Some(piece) = pos.piece_on(square) else {
        return false;
    };
    let enemy = pos.pieces_by_color(!piece.color());
    let attacked = (pos.attackers_to(square, pos.occupied()) & enemy).any();
    attacked
        && (is_hanging(pos, square, piece.color()) || can_be_taken_by_lower_piece(pos, square))
}

/// Whether the piece on `square` is **trapped** — attacked, with no move
/// to a safe square and no favourable trade out. Port of
/// `util.is_trapped`.
///
/// **Precondition (mirrors lichess):** the piece on `square` must belong
/// to `pos.side_to_move()`. The predicate enumerates the *owner's* legal
/// moves to test whether the piece can be saved, so it only makes sense
/// when the trapped piece's owner is the side to move. Returns `false`
/// for a piece of the other colour, for an empty square, or for a
/// pawn/king.
///
/// A piece is trapped iff, with its owner to move:
/// 1. the owner is not in check and the piece is not pinned (those are
///    different problems — lichess excludes both),
/// 2. the piece is already in a bad spot (attacked + hanging-or-takeable),
/// 3. it has no legal move that either captures something worth at least
///    as much (a trade out) or lands on a square that is *not* a bad spot
///    (an escape).
pub(crate) fn is_trapped(pos: &Position, square: Square) -> bool {
    // (1) In check or pinned → not "trapped" (a distinct lesson).
    if pos.checkers().any() {
        return false;
    }
    let owner = pos.side_to_move();
    let Some(piece) = pos.piece_on(square) else {
        return false;
    };
    if piece.color() != owner {
        return false;
    }
    if (pos.blockers_for_king(owner) & square_bb(square)).any() {
        return false; // pinned to its own king
    }
    if matches!(piece.kind(), PieceType::Pawn | PieceType::King) {
        return false;
    }
    // (2) Must already be in trouble — otherwise it isn't trapped, it's
    // just sitting somewhere with limited squares.
    if !is_in_bad_spot(pos, square) {
        return false;
    }

    // (3) Look for an escape. A trapped (non-pawn, non-king) piece's
    // moves are all `Normal`, so the captured target is simply whatever
    // stands on the destination.
    let piece_value = king_value(piece.kind());
    let mut scratch = pos.clone();
    let moves = legal_moves_vec(&mut scratch);
    for mv in moves {
        if mv.from() != square {
            continue;
        }
        // Can it trade itself for an equal-or-greater piece? Then it can
        // bail out at no loss — not trapped.
        if let Some(captured) = pos.piece_on(mv.to()) {
            if king_value(captured.kind()) >= piece_value {
                return false;
            }
        }
        // Does the destination escape the bad spot?
        let st = scratch.do_move(mv);
        let safe = !is_in_bad_spot(&scratch, mv.to());
        scratch.undo_move(mv, st);
        if safe {
            return false;
        }
    }
    true
}

/// If the piece on `square` is [`is_trapped`], return the bitboard of its
/// legal destination squares — the "cage." When a piece is trapped, every
/// square it can legally move to is unsafe (that's the definition), so the
/// full destination set is exactly the ring of dead squares the overlay
/// paints around it. `None` when the piece is not trapped. Same
/// precondition as [`is_trapped`] (owner must be the side to move).
pub(crate) fn trapped_cage(pos: &Position, square: Square) -> Option<Bitboard> {
    if !is_trapped(pos, square) {
        return None;
    }
    let mut scratch = pos.clone();
    let mut dead = Bitboard::EMPTY;
    for mv in legal_moves_vec(&mut scratch) {
        if mv.from() == square {
            dead = dead.with(mv.to());
        }
    }
    Some(dead)
}

#[cfg(test)]
#[path = "tactic_util_tests.rs"]
mod tests;
