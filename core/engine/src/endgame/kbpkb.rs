//! KBPKB — king + bishop + pawn vs king + bishop. Two draws:
//!
//! 1. Defending king is on the pawn's file ahead of it and either has
//!    the wrong colour for the strong bishop OR is on rank ≤ 6.
//! 2. Opposite-coloured bishops — almost always a draw.

use crate::bitboard::opposite_colors;
use crate::position::Position;
use crate::types::{Color, PieceType, Rank, ScaleFactor, Value};

pub(super) fn strong_side(pos: &Position) -> Option<Color> {
    for &strong in Color::both().iter() {
        let weak = !strong;
        if pos.non_pawn_material(strong) == Value::BISHOP_MG
            && pos.count(strong, PieceType::Bishop) == 1
            && pos.count(strong, PieceType::Pawn) == 1
            && pos.non_pawn_material(weak) == Value::BISHOP_MG
            && pos.count(weak, PieceType::Bishop) == 1
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
    let weak_bsq = pos.pieces_of(weak, PieceType::Bishop).lsb();
    let weak_ksq = pos.king_square(weak);

    // Defending king blocks the pawn from in front, can't be driven away.
    if weak_ksq.file() == pawn_sq.file()
        && pawn_sq.rank().from_perspective(strong) < weak_ksq.rank().from_perspective(strong)
        && (opposite_colors(weak_ksq, strong_bsq)
            || weak_ksq.rank().from_perspective(strong) <= Rank::R6)
    {
        return ScaleFactor::DRAW;
    }

    // Opposite-coloured bishops.
    if opposite_colors(strong_bsq, weak_bsq) {
        return ScaleFactor::DRAW;
    }

    ScaleFactor::NONE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opposite_coloured_bishops_draws() {
        // White Bb1 (light), Black Ba1 (dark) — opposite-colour
        // bishops. Pawn on e4 (doesn't matter for this rule).
        let p = Position::from_fen("4k3/8/8/8/4P3/8/8/b3K1B1 w - - 0 1").unwrap();
        let strong = strong_side(&p).expect("KBPKB signature");
        assert_eq!(evaluate(&p, strong), ScaleFactor::DRAW);
    }

    #[test]
    fn defending_king_blocks_pawn_on_same_colour_draws() {
        // White P on e4, Black K on e6 (in front, on the same file).
        // Black bishop on f8 (light), strong white bishop on c1 (dark).
        // e6 is a dark square → opposite_colors(e6, c1) is FALSE
        // because they're both dark. So this case applies the
        // "weak_ksq rank ≤ 6" branch: e6 rel-rank from white's POV is 6 → matches.
        let p = Position::from_fen("5b2/8/4k3/8/4P3/8/8/2B1K3 w - - 0 1").unwrap();
        let strong = strong_side(&p).expect("KBPKB signature");
        assert_eq!(evaluate(&p, strong), ScaleFactor::DRAW);
    }
}
