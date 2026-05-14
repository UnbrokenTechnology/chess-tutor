//! KNPK — king + knight + pawn vs king. One rule: rook-pawn on the
//! 7th rank with the defending king in the corner is a draw.

use crate::attacks::square_distance;
use crate::bitbases;
use crate::position::Position;
use crate::types::{Color, PieceType, ScaleFactor, Square, Value};

pub(super) fn strong_side(pos: &Position) -> Option<Color> {
    for &strong in Color::both().iter() {
        let weak = !strong;
        if pos.non_pawn_material(strong) == Value::KNIGHT_MG
            && pos.count(strong, PieceType::Knight) == 1
            && pos.count(strong, PieceType::Pawn) == 1
            && pos.non_pawn_material(weak) == Value::ZERO
            && pos.count(weak, PieceType::Pawn) == 0
        {
            return Some(strong);
        }
    }
    None
}

pub(super) fn evaluate(pos: &Position, strong: Color) -> ScaleFactor {
    let weak = !strong;
    let pawn_sq = pos.pieces_of(strong, PieceType::Pawn).lsb();
    let weak_ksq = pos.king_square(weak);

    // Normalise so the strong side is white and the pawn is on files A-D.
    let n_pawn = bitbases::normalize(strong, pawn_sq, pawn_sq);
    let n_weak_ksq = bitbases::normalize(strong, pawn_sq, weak_ksq);

    if n_pawn == Square::A7 && square_distance(Square::A8, n_weak_ksq) <= 1 {
        return ScaleFactor::DRAW;
    }
    ScaleFactor::NONE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rook_pawn_with_king_in_corner_draws() {
        // White N+P (P=a7), black K on a8.
        let p = Position::from_fen("k7/P7/8/8/8/8/2N5/4K3 b - - 0 1").unwrap();
        let strong = strong_side(&p).expect("KNPK signature");
        assert_eq!(evaluate(&p, strong), ScaleFactor::DRAW);
    }

    #[test]
    fn centre_pawn_does_not_draw() {
        let p = Position::from_fen("4k3/8/8/8/4P3/8/2N5/4K3 w - - 0 1").unwrap();
        let strong = strong_side(&p).expect("KNPK signature");
        assert_eq!(evaluate(&p, strong), ScaleFactor::NONE);
    }
}
