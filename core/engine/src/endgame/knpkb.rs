//! KNPKB — king + knight + pawn vs king + bishop. If the bishop's
//! attack line crosses the pawn's path forward, the position scales
//! by the weak-king-to-pawn distance (smaller distance → closer to
//! draw). Otherwise no scaling.

use crate::attacks::square_distance;
use crate::bitboard::forward_file_bb;
use crate::magics::bishop_attacks;
use crate::position::Position;
use crate::types::{Color, PieceType, ScaleFactor, Value};

pub(super) fn strong_side(pos: &Position) -> Option<Color> {
    for &strong in Color::both().iter() {
        let weak = !strong;
        if pos.non_pawn_material(strong) == Value::KNIGHT_MG
            && pos.count(strong, PieceType::Knight) == 1
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
    let pawn_sq = pos.pieces_of(strong, PieceType::Pawn).lsb();
    let bishop_sq = pos.pieces_of(weak, PieceType::Bishop).lsb();
    let weak_ksq = pos.king_square(weak);

    if (forward_file_bb(strong, pawn_sq) & bishop_attacks(bishop_sq, pos.occupied())).any() {
        return ScaleFactor(square_distance(weak_ksq, pawn_sq) as i32);
    }
    ScaleFactor::NONE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_matches_knp_vs_kb() {
        let p = Position::from_fen("4k3/4b3/8/8/4P3/8/2N5/4K3 w - - 0 1").unwrap();
        assert_eq!(strong_side(&p), Some(Color::White));
    }

    #[test]
    fn signature_rejects_no_bishop() {
        let p = Position::from_fen("4k3/8/8/8/4P3/8/2N5/4K3 w - - 0 1").unwrap();
        assert!(strong_side(&p).is_none());
    }
}
