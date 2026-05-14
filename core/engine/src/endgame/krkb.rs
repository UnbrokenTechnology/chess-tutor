//! KRKB — rook vs bishop, drawish (edge-push only).
//!
//! The whole purpose of this specialist is to *dampen* classical eval's
//! claim that being up the exchange is ~+400. Theoretical result is a
//! draw with best defence, and SF's score is intentionally small
//! (≤ 100) so the engine doesn't chase phantom wins.

use super::PUSH_TO_EDGES;
use crate::position::Position;
use crate::types::{Color, Value};

pub(super) fn evaluate(pos: &Position, strong: Color) -> Value {
    let weak = !strong;
    let loser_k = pos.king_square(weak);
    let score = PUSH_TO_EDGES[loser_k.index()];
    Value(if strong == Color::White { score } else { -score })
}

#[cfg(test)]
mod tests {
    use super::super::{probe, ProbeResult};
    use super::*;

    #[test]
    fn fires_and_returns_drawish_score() {
        let p = Position::from_fen("4k3/4b3/8/8/8/8/8/3RK3 w - - 0 1").unwrap();
        if let ProbeResult::Override(v) = probe(&p) {
            assert!(v.0.abs() < Value::ROOK_EG.0 / 4);
        } else {
            panic!()
        }
    }

    #[test]
    fn drives_loser_king_to_edge() {
        let p_edge = Position::from_fen("k7/8/8/4b3/8/8/8/3RK3 w - - 0 1").unwrap();
        let p_centre = Position::from_fen("8/8/4k3/4b3/8/8/8/3RK3 w - - 0 1").unwrap();
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
