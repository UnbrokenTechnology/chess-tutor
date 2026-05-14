//! KNNKP — two knights vs king + pawn (theoretical win with technique).
//!
//! The reference's bare `2N - P + PushToEdges` is too flat for our
//! search depth to feel the pawn advancing, so we add three gradients
//! on top: ranks-from-promotion (deters abandoning the blockade), king
//! proximity, and a free-knight-to-weak-king proxy so the non-blockading
//! knight is pulled forward.

use super::{PUSH_CLOSE, PUSH_TO_EDGES};
use crate::attacks::square_distance;
use crate::movegen::legal_moves_vec;
use crate::position::Position;
use crate::types::{Color, PieceType, Value};

pub(super) fn strong_side(pos: &Position) -> Option<Color> {
    for &strong in Color::both().iter() {
        let weak = !strong;
        if pos.count(strong, PieceType::Pawn) != 0
            || pos.count(strong, PieceType::Queen) != 0
            || pos.count(strong, PieceType::Rook) != 0
            || pos.count(strong, PieceType::Bishop) != 0
            || pos.count(strong, PieceType::Knight) != 2
        {
            continue;
        }
        if pos.non_pawn_material(weak) != Value::ZERO {
            continue;
        }
        if pos.count(weak, PieceType::Pawn) != 1 {
            continue;
        }
        return Some(strong);
    }
    None
}

pub(super) fn evaluate(pos: &Position, strong: Color) -> Value {
    let weak = !strong;

    if pos.side_to_move() == weak {
        let mut scratch = pos.clone();
        if legal_moves_vec(&mut scratch).is_empty() {
            return Value::DRAW;
        }
    }

    let pawn_sq = pos.pieces_of(weak, PieceType::Pawn).lsb();
    let strong_ksq = pos.king_square(strong);
    let weak_ksq = pos.king_square(weak);

    let ranks_from_promotion = match strong {
        Color::White => pawn_sq.rank() as i32,
        Color::Black => 7 - pawn_sq.rank() as i32,
    };

    let king_distance = square_distance(strong_ksq, weak_ksq) as usize;

    let mut min_knight_dist: usize = 8;
    for n_sq in pos.pieces_of(strong, PieceType::Knight) {
        let d = square_distance(n_sq, weak_ksq) as usize;
        if d < min_knight_dist {
            min_knight_dist = d;
        }
    }

    let score = 2 * Value::KNIGHT_EG.0 - Value::PAWN_EG.0
        + ranks_from_promotion * 150
        + PUSH_CLOSE[king_distance]
        + PUSH_CLOSE[min_knight_dist.min(7)]
        + PUSH_TO_EDGES[weak_ksq.index()];

    Value(if strong == Color::White { score } else { -score })
}

#[cfg(test)]
mod tests {
    use super::super::{probe, ProbeResult};
    use super::*;

    #[test]
    fn detects_signature() {
        let p = Position::from_fen("4k3/4p3/8/8/8/8/8/1N2K1N1 w - - 0 1").unwrap();
        assert_eq!(strong_side(&p), Some(Color::White));
        assert!(matches!(probe(&p), ProbeResult::Override(_)));
    }

    #[test]
    fn scores_a_winning_advantage_for_strong_side() {
        let p = Position::from_fen("4k3/4p3/8/8/8/8/8/1N2K1N1 w - - 0 1").unwrap();
        if let ProbeResult::Override(v) = probe(&p) {
            assert!(v.0 > Value::KNIGHT_EG.0);
        } else {
            panic!()
        }
    }

    #[test]
    fn drives_weak_king_toward_edge() {
        let p_corner = Position::from_fen("7k/7p/8/8/8/8/8/1N2K1N1 w - - 0 1").unwrap();
        let p_centre = Position::from_fen("8/4p3/4k3/8/8/8/8/1N2K1N1 w - - 0 1").unwrap();
        let v_c = match probe(&p_corner) {
            ProbeResult::Override(v) => v,
            _ => panic!(),
        };
        let v_m = match probe(&p_centre) {
            ProbeResult::Override(v) => v,
            _ => panic!(),
        };
        assert!(v_c > v_m);
    }

    #[test]
    fn returns_negative_when_strong_side_is_black() {
        let p = Position::from_fen("1n2k1n1/8/8/8/8/8/4P3/4K3 w - - 0 1").unwrap();
        if let ProbeResult::Override(v) = probe(&p) {
            assert!(v.0 < -Value::KNIGHT_EG.0);
        } else {
            panic!()
        }
    }

    #[test]
    fn prefers_pawn_far_from_promotion() {
        let p_far = Position::from_fen("4k3/4p3/8/8/8/8/8/1N2K1N1 w - - 0 1").unwrap();
        let p_near = Position::from_fen("4k3/8/8/8/8/8/4p3/1N2K1N1 w - - 0 1").unwrap();
        let v_far = match probe(&p_far) {
            ProbeResult::Override(v) => v,
            _ => panic!(),
        };
        let v_near = match probe(&p_near) {
            ProbeResult::Override(v) => v,
            _ => panic!(),
        };
        assert!(v_far.0 > v_near.0);
    }
}
