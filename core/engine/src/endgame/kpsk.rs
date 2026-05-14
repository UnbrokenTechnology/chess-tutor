//! KPsK — king + ≥2 pawns vs lone king. Single rule: if all the
//! strong-side pawns are ahead of the weak king, all on a single
//! rook file (a or h), and the weak king is within one file of the
//! pawns, it's a draw.

use crate::bitboard::{forward_ranks_bb, FILE_A, FILE_H};
use crate::position::Position;
use crate::types::{Color, PieceType, ScaleFactor, Value};

pub(super) fn strong_side(pos: &Position) -> Option<Color> {
    if pos.non_pawn_material(Color::White) != Value::ZERO
        || pos.non_pawn_material(Color::Black) != Value::ZERO
    {
        return None;
    }
    let pw = pos.count(Color::White, PieceType::Pawn);
    let pb = pos.count(Color::Black, PieceType::Pawn);
    if pw >= 2 && pb == 0 {
        Some(Color::White)
    } else if pb >= 2 && pw == 0 {
        Some(Color::Black)
    } else {
        None
    }
}

pub(super) fn evaluate(pos: &Position, strong: Color) -> ScaleFactor {
    let weak = !strong;
    let weak_ksq = pos.king_square(weak);
    let pawns = pos.pieces_of(strong, PieceType::Pawn);

    let all_ahead_of_king = (pawns & !forward_ranks_bb(weak, weak_ksq)).is_empty();
    let on_rook_file = (pawns & !FILE_A).is_empty() || (pawns & !FILE_H).is_empty();
    let weak_king_close = file_distance(weak_ksq, pawns.lsb()) <= 1;

    if all_ahead_of_king && on_rook_file && weak_king_close {
        return ScaleFactor::DRAW;
    }

    ScaleFactor::NONE
}

fn file_distance(a: crate::types::Square, b: crate::types::Square) -> u8 {
    let af = a.file() as i8;
    let bf = b.file() as i8;
    (af - bf).unsigned_abs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocked_rook_pawns_are_a_draw() {
        // Two white pawns on a3 and a4; black king on a8 — pawns are
        // ahead of the king on the a-file and the king is on the same
        // file.
        let p = Position::from_fen("k7/8/8/8/P7/P7/8/4K3 w - - 0 1").unwrap();
        let strong = strong_side(&p).expect("signature");
        assert_eq!(evaluate(&p, strong), ScaleFactor::DRAW);
    }

    #[test]
    fn centre_pawns_are_not_kpsk_draw() {
        let p = Position::from_fen("4k3/8/8/8/8/3P4/3P4/4K3 w - - 0 1").unwrap();
        let strong = strong_side(&p).expect("signature");
        assert_eq!(evaluate(&p, strong), ScaleFactor::NONE);
    }
}
