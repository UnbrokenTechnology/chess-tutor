//! KBNK — mate with king + bishop + knight against a lone king.
//!
//! Mate is only forceable into a corner that shares the bishop's
//! colour, so the evaluator drives the weak king there with the
//! `PushToCorners` table (flipped vertically for a light bishop).

use super::{is_lone_king, PUSH_CLOSE, PUSH_TO_CORNERS};
use crate::attacks::square_distance;
use crate::bitboard::{square_bb, DARK_SQUARES};
use crate::movegen::legal_moves_vec;
use crate::position::Position;
use crate::types::{Color, PieceType, Value};

/// Returns `Some(strong_side)` if the material is exactly K+B+N vs K.
pub(super) fn strong_side(pos: &Position) -> Option<Color> {
    for &strong in Color::both().iter() {
        let weak = !strong;
        if !is_lone_king(pos, weak) {
            continue;
        }
        if pos.count(strong, PieceType::Pawn) != 0
            || pos.count(strong, PieceType::Queen) != 0
            || pos.count(strong, PieceType::Rook) != 0
        {
            continue;
        }
        if pos.count(strong, PieceType::Bishop) == 1 && pos.count(strong, PieceType::Knight) == 1 {
            return Some(strong);
        }
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

    let winner_k = pos.king_square(strong);
    let loser_k = pos.king_square(weak);
    let bishop_sq = pos.pieces_of(strong, PieceType::Bishop).lsb();
    let distance = square_distance(winner_k, loser_k) as usize;

    let bishop_on_dark = (square_bb(bishop_sq) & DARK_SQUARES).any();
    let indexed_sq = if bishop_on_dark {
        loser_k.index()
    } else {
        loser_k.flip_vertical().index()
    };

    let score = Value::KNOWN_WIN.0 + PUSH_CLOSE[distance] + PUSH_TO_CORNERS[indexed_sq];

    Value(if strong == Color::White { score } else { -score })
}

#[cfg(test)]
mod tests {
    use super::super::{probe, ProbeResult};
    use super::*;

    #[test]
    fn fires_with_bishop_plus_knight_vs_lone_king() {
        let p = Position::from_fen("7k/8/8/8/8/8/8/2B1K1N1 w - - 0 1").unwrap();
        assert!(strong_side(&p).is_some());
        assert!(matches!(probe(&p), ProbeResult::Override(_)));
    }

    #[test]
    fn drives_weak_king_toward_dark_corner_with_dark_bishop() {
        let p_target = Position::from_fen("7k/8/8/8/8/8/8/2B1K1N1 w - - 0 1").unwrap();
        let p_worse = Position::from_fen("8/7k/8/8/8/8/8/2B1K1N1 w - - 0 1").unwrap();
        let v_t = match probe(&p_target) {
            ProbeResult::Override(v) => v,
            _ => panic!(),
        };
        let v_w = match probe(&p_worse) {
            ProbeResult::Override(v) => v,
            _ => panic!(),
        };
        assert!(v_t > v_w);
    }

    #[test]
    fn drives_weak_king_toward_light_corner_with_light_bishop() {
        let p_target = Position::from_fen("k7/8/8/8/8/8/8/4K1NB w - - 0 1").unwrap();
        let p_worse = Position::from_fen("7k/8/8/8/8/8/8/4K1NB w - - 0 1").unwrap();
        let v_t = match probe(&p_target) {
            ProbeResult::Override(v) => v,
            _ => panic!(),
        };
        let v_w = match probe(&p_worse) {
            ProbeResult::Override(v) => v,
            _ => panic!(),
        };
        assert!(v_t > v_w);
    }

    #[test]
    fn scores_above_known_win() {
        let p = Position::from_fen("7k/8/8/8/8/8/8/2B1K1N1 w - - 0 1").unwrap();
        if let ProbeResult::Override(v) = probe(&p) {
            assert!(v.0 >= Value::KNOWN_WIN.0);
        } else {
            panic!()
        }
    }
}
