//! KRKP — rook vs pawn, four geometric cases.

use crate::attacks::square_distance;
use crate::bitboard::forward_file_bb;
use crate::movegen::legal_moves_vec;
use crate::position::Position;
use crate::types::{Color, PieceType, Rank, Square, Value};

pub(super) fn evaluate(pos: &Position, strong: Color) -> Value {
    let weak = !strong;

    if pos.side_to_move() == weak {
        let mut scratch = pos.clone();
        if legal_moves_vec(&mut scratch).is_empty() {
            return Value::DRAW;
        }
    }

    // Reframe with `strong` as white: the rest of this function reads
    // cleanly in "white attacks, black defends a pawn marching toward
    // rank 1" terms.
    let wksq = pos.king_square(strong).from_perspective(strong);
    let bksq = pos.king_square(weak).from_perspective(strong);
    let rsq = pos
        .pieces_of(strong, PieceType::Rook)
        .lsb()
        .from_perspective(strong);
    let psq = pos
        .pieces_of(weak, PieceType::Pawn)
        .lsb()
        .from_perspective(strong);

    let queening_sq = Square::new(psq.file(), Rank::R1);
    let strong_to_move = pos.side_to_move() == strong;
    let weak_to_move = !strong_to_move;

    // The two winning branches happen to compute the same expression
    // (rook value minus king-to-pawn distance), but they encode
    // distinct geometric preconditions worth keeping separate so the
    // structure mirrors SF11's endgame.cpp:212-219.
    #[allow(clippy::if_same_then_else)]
    let raw_score: i32 = if forward_file_bb(Color::White, wksq).contains(psq) {
        Value::ROOK_EG.0 - square_distance(wksq, psq) as i32
    } else if square_distance(bksq, psq) as i32 >= 3 + i32::from(weak_to_move)
        && square_distance(bksq, rsq) >= 3
    {
        Value::ROOK_EG.0 - square_distance(wksq, psq) as i32
    } else if bksq.rank() <= Rank::R3
        && square_distance(bksq, psq) == 1
        && wksq.rank() >= Rank::R4
        && square_distance(wksq, psq) as i32 > 2 + i32::from(strong_to_move)
    {
        80 - 8 * square_distance(wksq, psq) as i32
    } else {
        let psq_south = Square::from_index(psq.raw() - 8);
        200 - 8
            * (square_distance(wksq, psq_south) as i32
                - square_distance(bksq, psq_south) as i32
                - square_distance(psq, queening_sq) as i32)
    };

    Value(if strong == Color::White { raw_score } else { -raw_score })
}

#[cfg(test)]
mod tests {
    use super::super::{probe, ProbeResult};
    use crate::position::Position;
    use crate::types::Value;

    #[test]
    fn wins_when_king_in_front_of_pawn() {
        let p = Position::from_fen("8/8/4K3/8/4p3/8/8/4k2R w - - 0 1").unwrap();
        if let ProbeResult::Override(v) = probe(&p) {
            assert!(v.0 > Value::ROOK_EG.0 / 2);
        } else {
            panic!()
        }
    }

    #[test]
    fn drawish_when_pawn_is_far_advanced_and_supported() {
        let p = Position::from_fen("8/8/8/7R/7K/8/2kp4/8 w - - 0 1").unwrap();
        if let ProbeResult::Override(v) = probe(&p) {
            assert!(v.0.abs() < Value::ROOK_EG.0 / 2);
        } else {
            panic!()
        }
    }

    #[test]
    fn fires_when_strong_side_is_black() {
        let p = Position::from_fen("4K2r/8/8/8/4P3/4k3/8/8 w - - 0 1").unwrap();
        if let ProbeResult::Override(v) = probe(&p) {
            assert!(v.0 < 0);
        } else {
            panic!()
        }
    }
}
