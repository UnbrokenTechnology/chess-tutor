//! The "guaranteed winnable" filter: narrow a static threat list down
//! to targets that survive *every* legal opponent response.

use super::types::HangingPiece;
use crate::bitboard::Bitboard;
use crate::position::Position;
use crate::types::{Color, Move, PieceType, Square, Value};

/// Return the subset of `targets` that are *guaranteed* winnable on
/// `our_color`'s next move — i.e. for every legal opponent response,
/// the target piece is still on its square AND our cheapest attacker
/// still has a SEE-positive capture there.
///
/// `pos_after_user_move` is the position immediately after the user
/// moved, with the opponent to move. Each `HangingPiece` in `targets`
/// must already describe an opponent piece in that position
/// (typically [`ThreatsOutcome::theirs_hanging`] or
/// [`ThreatsOutcome::theirs_see_losing`]).
///
/// Why this matters: the raw lists are a static after-our-move
/// snapshot. A piece can look "hanging" right after we attack it but
/// the opponent's response (defend, move the piece, capture an
/// attacker, or force us to deal with a bigger threat) refutes the
/// win. Surfacing "you can win material" before this filter produces
/// false positives that mis-teach the student.
///
/// Edge cases:
/// - Opponent has no legal moves (stalemate/mate): every target is
///   trivially "guaranteed". Acceptable; the game is over and the
///   material claim is moot.
/// - Opponent captures one of our attackers: re-running
///   `attackers_to`/SEE on the post-response position handles this
///   automatically.
/// - Opponent moves the target to a still-attacked square: counts as
///   refuted. We don't chase the target to its new square; teaching
///   value is "the original threat persists no matter what."
///
/// Known false-positive (not yet handled): when the "hanging" target
/// is actually a sacrifice setup — opponent left it there to bait
/// our capture into a prepared tactic (fork, pin, mating net). Every
/// passive opponent response leaves the piece capturable, so we
/// pass the static check; but the right move for us is to *not*
/// take, because the next move after our capture lands us in the
/// tactic. Detecting this needs a one-ply search of *our* response
/// (take vs. refuse) and an SEE/eval check on the position after
/// the opponent's follow-up. See memory note on the sacrifice-tactic
/// misfire for future work.
///
/// [`ThreatsOutcome::theirs_hanging`]: super::ThreatsOutcome::theirs_hanging
/// [`ThreatsOutcome::theirs_see_losing`]: super::ThreatsOutcome::theirs_see_losing
pub fn filter_guaranteed_targets(
    pos_after_user_move: &Position,
    targets: &[HangingPiece],
    our_color: Color,
) -> Vec<HangingPiece> {
    targets
        .iter()
        .filter(|h| is_target_guaranteed(pos_after_user_move, h, our_color))
        .cloned()
        .collect()
}

fn is_target_guaranteed(
    pos_after_user_move: &Position,
    hanging: &HangingPiece,
    our_color: Color,
) -> bool {
    let target_sq = hanging.location.square;
    let mut scratch_for_movegen = pos_after_user_move.clone();
    let legal = crate::movegen::legal_moves_vec(&mut scratch_for_movegen);
    if legal.is_empty() {
        // Stalemate / mate — no responses to refute the claim. Edge
        // case; game is over so the "win material" framing is moot.
        return true;
    }
    for mv in legal {
        let mut scratch = pos_after_user_move.clone();
        scratch.do_move(mv);
        if !target_still_winnable(&scratch, target_sq, our_color) {
            return false;
        }
    }
    true
}

fn target_still_winnable(pos: &Position, target_sq: Square, our_color: Color) -> bool {
    let Some(piece_on_target) = pos.piece_on(target_sq) else {
        // Target moved off its square.
        return false;
    };
    if piece_on_target.color() == our_color {
        // Shouldn't happen — opponent just moved — but guard anyway.
        return false;
    }
    let occupied = pos.occupied();
    let attackers_to_sq = pos.attackers_to(target_sq, occupied);
    let our_attackers = attackers_to_sq & pos.pieces_by_color(our_color);
    if our_attackers == Bitboard::EMPTY {
        return false;
    }
    // SEE the cheapest-attacker capture. Threshold = 1 cp (any
    // strictly-positive material gain). Matches list_see_losing's
    // convention.
    //
    // Kings can only legally initiate the capture when the target
    // has no defenders; against a defended piece the king would be
    // moving into check. Without this filter, Value::mg_of_piece(King)
    // == 0 makes the king look like a costless first captor and
    // SEE returns a spurious "winnable" verdict for what's actually
    // an illegal move.
    let target_defended =
        (attackers_to_sq & pos.pieces_by_color(!our_color)) != Bitboard::EMPTY;
    let candidates = if target_defended {
        our_attackers & !pos.pieces(PieceType::King)
    } else {
        our_attackers
    };
    let mut cheapest_from: Option<Square> = None;
    let mut cheapest_value = i32::MAX;
    for from in candidates {
        if let Some(p) = pos.piece_on(from) {
            let v = Value::mg_of_piece(p.kind()).0;
            if v < cheapest_value {
                cheapest_value = v;
                cheapest_from = Some(from);
            }
        }
    }
    let Some(from) = cheapest_from else {
        return false;
    };
    pos.see_ge(Move::normal(from, target_sq), Value(1))
}
