//! KQKR — queen vs rook, technical mate.

use super::{PUSH_CLOSE, PUSH_TO_EDGES};
use crate::attacks::square_distance;
use crate::movegen::legal_moves_vec;
use crate::position::Position;
use crate::types::{Color, Value};

pub(super) fn evaluate(pos: &Position, strong: Color) -> Value {
    let weak = !strong;

    if pos.side_to_move() == weak {
        let mut scratch = pos.clone();
        if legal_moves_vec(&mut scratch).is_empty() {
            return Value::DRAW;
        }
    }

    let winner_k = pos.king_square(strong);
    let loser_k = pos.king_square(weak);
    let distance = square_distance(winner_k, loser_k) as usize;

    let base = Value::QUEEN_EG.0 - Value::ROOK_EG.0
        + PUSH_TO_EDGES[loser_k.index()]
        + PUSH_CLOSE[distance];
    let pedestal = Value::KNOWN_WIN.0;
    let cap = Value::MATE.0 - Value::MAX_PLY - 1;
    let score = (base + pedestal).min(cap);

    Value(if strong == Color::White { score } else { -score })
}

#[cfg(test)]
mod tests {
    use super::super::{probe, ProbeResult};
    use super::*;

    #[test]
    fn fires_and_scores_above_known_win() {
        let p = Position::from_fen("7k/8/5K2/8/3r4/8/3Q4/8 w - - 0 1").unwrap();
        if let ProbeResult::Override(v) = probe(&p) {
            assert!(v.0 >= Value::KNOWN_WIN.0);
        } else {
            panic!()
        }
    }

    #[test]
    fn drives_loser_king_to_edge() {
        let p_edge = Position::from_fen("7k/8/5K2/8/3r4/8/3Q4/8 w - - 0 1").unwrap();
        let p_centre = Position::from_fen("8/8/5K2/3k4/3r4/8/3Q4/8 w - - 0 1").unwrap();
        let v_e = match probe(&p_edge) {
            ProbeResult::Override(v) => v,
            _ => panic!(),
        };
        let v_c = match probe(&p_centre) {
            ProbeResult::Override(v) => v,
            _ => panic!(),
        };
        assert!(v_e > v_c);
    }
}
