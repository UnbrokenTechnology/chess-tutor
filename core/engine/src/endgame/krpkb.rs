//! KRPKB — rook + pawn vs bishop. Two fortress patterns when the pawn
//! is on a rook file (a or h) and the bishop / king geometry forms a
//! fortress: rank-5 same-colour-as-bishop pawn (moderate or strong
//! reduction depending on king proximity), and a rank-6 pawn the
//! bishop attacks from a reasonable distance.

use crate::attacks::{bishop_pseudo_attacks, square_distance};
use crate::bitboard::{FILE_A, FILE_H};
use crate::position::Position;
use crate::types::{Color, Direction, PieceType, Rank, ScaleFactor, Value};

pub(super) fn strong_side(pos: &Position) -> Option<Color> {
    for &strong in Color::both().iter() {
        let weak = !strong;
        if pos.non_pawn_material(strong) == Value::ROOK_MG
            && pos.count(strong, PieceType::Rook) == 1
            && pos.count(strong, PieceType::Pawn) == 1
            && pos.non_pawn_material(weak) == Value::BISHOP_MG
            && pos.count(weak, PieceType::Bishop) == 1
            && pos.count(weak, PieceType::Pawn) == 0
        {
            return Some(strong);
        }
    }
    None
}

pub(super) fn evaluate(pos: &Position, strong: Color) -> ScaleFactor {
    let weak = !strong;

    // Only fortresses on the rook files (a/h).
    if (pos.pieces(PieceType::Pawn) & (FILE_A | FILE_H)).is_empty() {
        return ScaleFactor::NONE;
    }

    let weak_ksq = pos.king_square(weak);
    let weak_bsq = pos.pieces_of(weak, PieceType::Bishop).lsb();
    let strong_psq = pos.pieces_of(strong, PieceType::Pawn).lsb();
    let strong_ksq = pos.king_square(strong);
    let rel_rank = strong_psq.rank().from_perspective(strong);
    let push = Direction::pawn_push(strong);

    // Rank 5 with bishop same colour as the pawn square — chance of
    // fortress.
    if rel_rank == Rank::R5
        && !crate::bitboard::opposite_colors(weak_bsq, strong_psq)
    {
        let three_ahead = strong_psq + push + push + push;
        let d = square_distance(three_ahead, weak_ksq);
        if d <= 2 && !(d == 0 && weak_ksq == strong_ksq + push + push) {
            return ScaleFactor(24);
        }
        return ScaleFactor(48);
    }

    // Rank 6 with the bishop covering the step-square from a
    // reasonable distance and the weak king near the corner.
    if rel_rank == Rank::R6 {
        let two_ahead = strong_psq + push + push;
        let one_ahead = strong_psq + push;
        if square_distance(two_ahead, weak_ksq) <= 1
            && bishop_pseudo_attacks(weak_bsq).contains(one_ahead)
            && file_distance(weak_bsq, strong_psq) >= 2
        {
            return ScaleFactor(8);
        }
    }

    ScaleFactor::NONE
}

fn file_distance(a: crate::types::Square, b: crate::types::Square) -> u8 {
    let af = a.file() as i8;
    let bf = b.file() as i8;
    (af - bf).unsigned_abs()
}

#[cfg(test)]
mod tests {
    use super::super::{probe, ProbeResult};
    use super::*;

    #[test]
    fn dispatcher_recognises_signature() {
        // KR+P vs KB on the a-file. Validate at least the dispatcher
        // routes through — fortress details are position-specific.
        let p = Position::from_fen("4k3/b7/8/P7/8/8/8/R3K3 w - - 0 1").unwrap();
        assert!(strong_side(&p).is_some());
        assert!(matches!(probe(&p), ProbeResult::Scale { .. } | ProbeResult::None));
    }

    #[test]
    fn no_rook_pawn_returns_none() {
        // Pawn on d-file — not a fortress pattern.
        let p = Position::from_fen("4k3/b7/8/3P4/8/8/8/R3K3 w - - 0 1").unwrap();
        let r = evaluate(&p, Color::White);
        assert_eq!(r, ScaleFactor::NONE);
    }
}
