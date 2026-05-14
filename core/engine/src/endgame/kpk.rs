//! KPK — king + pawn vs bare king (bitbase-precise).

use super::is_lone_king;
use crate::bitbases;
use crate::movegen::legal_moves_vec;
use crate::position::Position;
use crate::types::{Color, PieceType, Value};

pub(super) fn strong_side(pos: &Position) -> Option<Color> {
    for &strong in Color::both().iter() {
        let weak = !strong;
        if !is_lone_king(pos, weak) {
            continue;
        }
        if pos.non_pawn_material(strong) != Value::ZERO {
            continue;
        }
        if pos.count(strong, PieceType::Pawn) != 1 {
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

    let pawn_sq = pos.pieces_of(strong, PieceType::Pawn).lsb();
    let strong_ksq = pos.king_square(strong);
    let weak_ksq = pos.king_square(weak);

    let n_pawn = bitbases::normalize(strong, pawn_sq, pawn_sq);
    let n_strong_ksq = bitbases::normalize(strong, pawn_sq, strong_ksq);
    let n_weak_ksq = bitbases::normalize(strong, pawn_sq, weak_ksq);

    let bb_stm = if pos.side_to_move() == strong {
        Color::White
    } else {
        Color::Black
    };

    if !bitbases::kpk_probe(n_strong_ksq, n_pawn, n_weak_ksq, bb_stm) {
        return Value::DRAW;
    }

    let rank_bonus = n_pawn.rank() as i32;
    let score = Value::KNOWN_WIN.0 + Value::PAWN_EG.0 + rank_bonus;
    Value(if strong == Color::White { score } else { -score })
}

#[cfg(test)]
mod tests {
    use super::super::{probe, ProbeResult};
    use super::*;

    #[test]
    fn wrong_rook_pawn_scores_as_draw() {
        let p = Position::from_fen("k7/8/K7/P7/8/8/8/8 b - - 0 1").unwrap();
        assert!(matches!(probe(&p), ProbeResult::Override(v) if v == Value::DRAW));
    }

    #[test]
    fn king_pawn_with_opposition_is_a_win() {
        let p = Position::from_fen("4k3/8/4K3/4P3/8/8/8/8 w - - 0 1").unwrap();
        if let ProbeResult::Override(v) = probe(&p) {
            assert!(v.0 > Value::KNOWN_WIN.0);
        } else {
            panic!()
        }
    }

    #[test]
    fn rook_pawn_with_weak_king_in_front_draws() {
        let p = Position::from_fen("8/8/7k/8/7P/7K/8/8 b - - 0 1").unwrap();
        assert!(matches!(probe(&p), ProbeResult::Override(v) if v == Value::DRAW));
    }

    #[test]
    fn returns_black_signed_value_with_strong_black() {
        let p = Position::from_fen("8/8/8/8/4p3/4k3/8/4K3 b - - 0 1").unwrap();
        if let ProbeResult::Override(v) = probe(&p) {
            assert!(v.0 < -Value::KNOWN_WIN.0);
        } else {
            panic!()
        }
    }

    #[test]
    fn only_fires_with_exactly_one_pawn() {
        // Two pawns — KPK's signature must reject this. The position
        // still resolves via the KXK fallback (which fires on
        // K+pawns vs K when no fortress matches).
        let p = Position::from_fen("7k/8/8/8/4P3/4P3/4K3/8 w - - 0 1").unwrap();
        assert!(strong_side(&p).is_none());
        assert!(matches!(probe(&p), ProbeResult::Override(_)));
    }
}
