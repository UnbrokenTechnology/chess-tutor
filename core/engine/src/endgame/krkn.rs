//! KRKN — rook vs knight, drawish (edge-push + king/knight separation).

use super::{PUSH_AWAY, PUSH_TO_EDGES};
use crate::attacks::square_distance;
use crate::position::Position;
use crate::types::{Color, PieceType, Value};

pub(super) fn evaluate(pos: &Position, strong: Color) -> Value {
    let weak = !strong;
    let loser_k = pos.king_square(weak);
    let knight_sq = pos.pieces_of(weak, PieceType::Knight).lsb();

    let dist = square_distance(loser_k, knight_sq) as usize;
    let score = PUSH_TO_EDGES[loser_k.index()] + PUSH_AWAY[dist];

    Value(if strong == Color::White { score } else { -score })
}

#[cfg(test)]
mod tests {
    use super::super::{probe, ProbeResult};
    use super::*;

    #[test]
    fn fires_and_returns_drawish_score() {
        let p = Position::from_fen("4k3/4n3/8/8/8/8/8/3RK3 w - - 0 1").unwrap();
        if let ProbeResult::Override(v) = probe(&p) {
            assert!(v.0.abs() < Value::ROOK_EG.0 / 4);
        } else {
            panic!()
        }
    }

    #[test]
    fn prefers_separating_king_and_knight() {
        let p_adjacent = Position::from_fen("4k3/4n3/8/8/8/8/8/3RK3 w - - 0 1").unwrap();
        let p_separated = Position::from_fen("4k3/8/8/8/n7/8/8/3RK3 w - - 0 1").unwrap();
        let v_a = match probe(&p_adjacent) {
            ProbeResult::Override(v) => v,
            _ => panic!(),
        };
        let v_s = match probe(&p_separated) {
            ProbeResult::Override(v) => v,
            _ => panic!(),
        };
        assert!(v_s > v_a);
    }
}
