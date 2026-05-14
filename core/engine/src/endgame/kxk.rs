//! KXK — mate a lone king with any winning configuration.

use super::{is_lone_king, PUSH_CLOSE, PUSH_TO_EDGES};
use crate::attacks::square_distance;
use crate::bitboard::{DARK_SQUARES, LIGHT_SQUARES};
use crate::movegen::legal_moves_vec;
use crate::position::Position;
use crate::types::{Color, PieceType, Value};

pub(super) fn evaluate(pos: &Position, strong: Color) -> Value {
    let weak = !strong;

    // Stalemate detection: if the weak side is to move with no legal
    // moves, it's a draw regardless of how much material we have.
    if pos.side_to_move() == weak {
        let mut scratch = pos.clone();
        if legal_moves_vec(&mut scratch).is_empty() {
            return Value::DRAW;
        }
    }

    let winner_k = pos.king_square(strong);
    let loser_k = pos.king_square(weak);
    let distance = square_distance(winner_k, loser_k) as usize;

    let mut score = pos.non_pawn_material(strong).0
        + pos.count(strong, PieceType::Pawn) as i32 * Value::PAWN_EG.0
        + PUSH_TO_EDGES[loser_k.index()]
        + PUSH_CLOSE[distance];

    let q = pos.count(strong, PieceType::Queen);
    let r = pos.count(strong, PieceType::Rook);
    let b = pos.count(strong, PieceType::Bishop);
    let n = pos.count(strong, PieceType::Knight);
    let bishops = pos.pieces_of(strong, PieceType::Bishop);
    let opp_colour_bishops =
        b >= 2 && (bishops & DARK_SQUARES).any() && (bishops & LIGHT_SQUARES).any();

    let clearly_winning = q > 0 || r > 0 || (b > 0 && n > 0) || opp_colour_bishops;
    if clearly_winning {
        let pedestal = Value::KNOWN_WIN.0;
        let cap = Value::MATE.0 - Value::MAX_PLY - 1;
        score = (score + pedestal).min(cap);
    }

    // Stalemate guard handled above; here `is_lone_king` is used only
    // as a defensive check that the caller routed correctly.
    debug_assert!(is_lone_king(pos, weak));

    Value(if strong == Color::White { score } else { -score })
}

#[cfg(test)]
mod tests {
    use super::super::{probe, ProbeResult};
    use crate::position::Position;
    use crate::types::Value;

    #[test]
    fn prefers_driving_loser_king_to_edge() {
        let p_corner = Position::from_fen("7k/8/5K2/6Q1/8/8/8/8 w - - 0 1").unwrap();
        let p_centre = Position::from_fen("8/8/8/3k1K2/6Q1/8/8/8 w - - 0 1").unwrap();
        let v_corner = match probe(&p_corner) {
            ProbeResult::Override(v) => v,
            _ => panic!("KXK should fire"),
        };
        let v_centre = match probe(&p_centre) {
            ProbeResult::Override(v) => v,
            _ => panic!("KXK should fire"),
        };
        assert!(
            v_corner > v_centre,
            "corner king must score higher for winner ({:?} vs {:?})",
            v_corner,
            v_centre
        );
    }

    #[test]
    fn rewards_winner_king_proximity() {
        let p_close = Position::from_fen("7k/8/5KQ1/8/8/8/8/8 w - - 0 1").unwrap();
        let p_far = Position::from_fen("7k/8/6Q1/8/8/8/8/4K3 w - - 0 1").unwrap();
        let v_close = match probe(&p_close) {
            ProbeResult::Override(v) => v,
            _ => panic!(),
        };
        let v_far = match probe(&p_far) {
            ProbeResult::Override(v) => v,
            _ => panic!(),
        };
        assert!(v_close > v_far);
    }

    #[test]
    fn returns_draw_on_stalemate() {
        let p = Position::from_fen("k1K5/8/1Q6/8/8/8/8/8 b - - 0 1").unwrap();
        assert!(matches!(probe(&p), ProbeResult::Override(v) if v == Value::DRAW));
    }

    #[test]
    fn does_not_fire_for_insufficient_material() {
        for fen in [
            "7k/8/8/8/8/8/8/N3K3 w - - 0 1",
            "7k/8/8/8/8/8/8/B3K3 w - - 0 1",
            "7k/8/8/8/8/8/8/4K3 w - - 0 1",
        ] {
            let p = Position::from_fen(fen).unwrap();
            assert_eq!(probe(&p), ProbeResult::None, "expected None for {fen}");
        }
    }

    #[test]
    fn does_not_fire_with_two_same_colour_bishops() {
        let p = Position::from_fen("7k/8/8/8/8/8/8/B1B4K w - - 0 1").unwrap();
        assert_eq!(probe(&p), ProbeResult::None);
    }

    #[test]
    fn fires_with_two_opposite_colour_bishops() {
        let p = Position::from_fen("7k/8/8/8/8/8/8/2B2B1K w - - 0 1").unwrap();
        assert!(matches!(probe(&p), ProbeResult::Override(_)));
    }

    #[test]
    fn returns_white_signed_value_with_strong_white() {
        let p = Position::from_fen("7k/8/5KQ1/8/8/8/8/8 w - - 0 1").unwrap();
        if let ProbeResult::Override(v) = probe(&p) {
            assert!(v.0 > Value::QUEEN_MG.0);
        } else {
            panic!()
        }
    }

    #[test]
    fn returns_black_signed_value_with_strong_black() {
        let p = Position::from_fen("K7/8/2kq4/8/8/8/8/8 w - - 0 1").unwrap();
        if let ProbeResult::Override(v) = probe(&p) {
            assert!(v.0 < -Value::QUEEN_MG.0);
        } else {
            panic!()
        }
    }
}
