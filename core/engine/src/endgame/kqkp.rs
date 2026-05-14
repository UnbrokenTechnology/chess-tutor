//! KQKP — queen vs pawn, won unless the 7th-rank fortress (a/c/f/h files).

use super::PUSH_CLOSE;
use crate::attacks::square_distance;
use crate::movegen::legal_moves_vec;
use crate::position::Position;
use crate::types::{Color, File, PieceType, Rank, Value};

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
    let pawn_sq = pos.pieces_of(weak, PieceType::Pawn).lsb();

    let distance = square_distance(winner_k, loser_k) as usize;
    let mut score = PUSH_CLOSE[distance];

    let pawn_relative_rank = pawn_sq.rank().from_perspective(weak);
    let pawn_file = pawn_sq.file();
    let in_fortress = pawn_relative_rank == Rank::R7
        && square_distance(loser_k, pawn_sq) == 1
        && matches!(pawn_file, File::A | File::C | File::F | File::H);

    if !in_fortress {
        let base = Value::QUEEN_EG.0 - Value::PAWN_EG.0;
        let pedestal = Value::KNOWN_WIN.0;
        let cap = Value::MATE.0 - Value::MAX_PLY - 1;
        score = (score + base + pedestal).min(cap);
    }

    Value(if strong == Color::White { score } else { -score })
}

#[cfg(test)]
mod tests {
    use super::super::{probe, ProbeResult};
    use super::*;

    #[test]
    fn fires_and_wins_outside_fortress() {
        let p = Position::from_fen("4k3/4p3/8/8/8/8/8/3QK3 w - - 0 1").unwrap();
        if let ProbeResult::Override(v) = probe(&p) {
            assert!(v.0 >= Value::KNOWN_WIN.0);
        } else {
            panic!()
        }
    }

    #[test]
    fn fortress_scores_drawish() {
        let p = Position::from_fen("8/8/8/8/8/2K5/pk6/3Q4 w - - 0 1").unwrap();
        if let ProbeResult::Override(v) = probe(&p) {
            assert!(v.0 < Value::KNOWN_WIN.0);
        } else {
            panic!()
        }
    }

    #[test]
    fn fortress_only_on_a_c_f_h_files() {
        let p = Position::from_fen("8/8/8/8/8/2K5/1pk5/3Q4 w - - 0 1").unwrap();
        if let ProbeResult::Override(v) = probe(&p) {
            assert!(v.0 >= Value::KNOWN_WIN.0);
        } else {
            panic!()
        }
    }
}
