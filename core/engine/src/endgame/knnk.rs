//! KNN vs bare K — unconditional draw (two knights can't force mate).

use super::is_lone_king;
use crate::position::Position;
use crate::types::{Color, PieceType};

/// True iff the material is exactly K + 2 knights vs lone K (no pawns).
pub(super) fn matches(pos: &Position) -> bool {
    for &strong in Color::both().iter() {
        let weak = !strong;
        if !is_lone_king(pos, weak) {
            continue;
        }
        if pos.count(strong, PieceType::Pawn) != 0
            || pos.count(strong, PieceType::Queen) != 0
            || pos.count(strong, PieceType::Rook) != 0
            || pos.count(strong, PieceType::Bishop) != 0
        {
            continue;
        }
        if pos.count(strong, PieceType::Knight) == 2 {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::super::{probe, ProbeResult};
    use super::*;
    use crate::types::Value;

    #[test]
    fn knn_vs_bare_king_is_drawn() {
        let p = Position::from_fen("4k3/8/8/8/8/8/8/1N2K1N1 w - - 0 1").unwrap();
        assert!(matches!(probe(&p), ProbeResult::Override(v) if v == Value::DRAW));
    }

    #[test]
    fn knn_vs_bare_king_draws_when_black_has_the_knights() {
        let p = Position::from_fen("1n2k1n1/8/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        assert!(matches!(probe(&p), ProbeResult::Override(v) if v == Value::DRAW));
    }

    #[test]
    fn does_not_fire_with_pawns() {
        let p = Position::from_fen("4k3/4p3/8/8/8/8/8/1N2K1N1 w - - 0 1").unwrap();
        assert!(!matches(&p));
    }
}
