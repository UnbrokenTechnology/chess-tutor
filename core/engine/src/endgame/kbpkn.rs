//! KBPKN — king + bishop + pawn vs king + knight. One rule: if the
//! defending king is somewhere on the pawn's file ahead of it and
//! either is on the wrong colour for the bishop OR is on rank ≤ 6,
//! it's a draw.

use crate::bitboard::opposite_colors;
use crate::position::Position;
use crate::types::{Color, PieceType, Rank, ScaleFactor, Value};

pub(super) fn strong_side(pos: &Position) -> Option<Color> {
    for &strong in Color::both().iter() {
        let weak = !strong;
        if pos.non_pawn_material(strong) == Value::BISHOP_MG
            && pos.count(strong, PieceType::Bishop) == 1
            && pos.count(strong, PieceType::Pawn) == 1
            && pos.non_pawn_material(weak) == Value::KNIGHT_MG
            && pos.count(weak, PieceType::Knight) == 1
            && pos.count(weak, PieceType::Pawn) == 0
        {
            return Some(strong);
        }
    }
    None
}

pub(super) fn evaluate(pos: &Position, strong: Color) -> ScaleFactor {
    let weak = !strong;
    let pawn_sq = pos.pieces_of(strong, PieceType::Pawn).lsb();
    let strong_bsq = pos.pieces_of(strong, PieceType::Bishop).lsb();
    let weak_ksq = pos.king_square(weak);

    if weak_ksq.file() == pawn_sq.file()
        && pawn_sq.rank().from_perspective(strong) < weak_ksq.rank().from_perspective(strong)
        && (opposite_colors(weak_ksq, strong_bsq)
            || weak_ksq.rank().from_perspective(strong) <= Rank::R6)
    {
        return ScaleFactor::DRAW;
    }

    ScaleFactor::NONE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defending_king_blocks_pawn_with_wrong_colour_draws() {
        // White pawn on e4 with white bishop on a1 (dark squares).
        // Black king on e6 — e6 is a light square (file 4, rank 5: 4+5=9 odd).
        // opposite_colors(e6, a1) is TRUE (one light, one dark).
        // So the rule fires.
        let p = Position::from_fen("8/8/4k3/8/4P3/8/2n5/B3K3 w - - 0 1").unwrap();
        let strong = strong_side(&p).expect("KBPKN signature");
        assert_eq!(evaluate(&p, strong), ScaleFactor::DRAW);
    }

    #[test]
    fn pawn_not_blocked_does_not_draw() {
        // King not on pawn's file.
        let p = Position::from_fen("8/8/3k4/8/4P3/8/2n5/B3K3 w - - 0 1").unwrap();
        let strong = strong_side(&p).expect("KBPKN signature");
        assert_eq!(evaluate(&p, strong), ScaleFactor::NONE);
    }
}
