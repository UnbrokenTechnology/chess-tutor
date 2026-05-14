//! KBPPKB — king + bishop + 2 pawns vs king + bishop. Detects a few
//! basic draws with opposite-coloured bishops where the defender
//! firmly controls the lead pawn's path.

use crate::bitboard::opposite_colors;
use crate::magics::bishop_attacks;
use crate::position::Position;
use crate::types::{Color, PieceType, ScaleFactor, Square, Value};

pub(super) fn strong_side(pos: &Position) -> Option<Color> {
    for &strong in Color::both().iter() {
        let weak = !strong;
        if pos.non_pawn_material(strong) == Value::BISHOP_MG
            && pos.count(strong, PieceType::Bishop) == 1
            && pos.count(strong, PieceType::Pawn) == 2
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
    let strong_bsq = pos.pieces_of(strong, PieceType::Bishop).lsb();
    let weak_bsq = pos.pieces_of(weak, PieceType::Bishop).lsb();

    if !opposite_colors(strong_bsq, weak_bsq) {
        return ScaleFactor::NONE;
    }

    let weak_ksq = pos.king_square(weak);
    let strong_pawns = pos.pieces_of(strong, PieceType::Pawn);
    let mut iter = strong_pawns.into_iter();
    let psq1 = iter.next().expect("KBPPKB signature guarantees 2 pawns");
    let psq2 = iter.next().expect("KBPPKB signature guarantees 2 pawns");

    let r1 = psq1.rank().from_perspective(strong);
    let r2 = psq2.rank().from_perspective(strong);
    let push = crate::types::Direction::pawn_push(strong);

    // Identify the frontmost pawn ("blockSq1" = the square IN FRONT of
    // the frontmost pawn) and a "blockSq2" companion sitting on the
    // other pawn's file at the frontmost pawn's rank.
    let (block_sq1, block_sq2) = if r1 > r2 {
        let block1 = psq1 + push;
        let block2 = Square::new(psq2.file(), psq1.rank());
        (block1, block2)
    } else {
        let block1 = psq2 + push;
        let block2 = Square::new(psq1.file(), psq2.rank());
        (block1, block2)
    };

    let fd_pawns = file_distance(psq1, psq2);
    match fd_pawns {
        0 => {
            // Both pawns on same file.
            if weak_ksq.file() == block_sq1.file()
                && weak_ksq.rank().from_perspective(strong)
                    >= block_sq1.rank().from_perspective(strong)
                && opposite_colors(weak_ksq, strong_bsq)
            {
                return ScaleFactor::DRAW;
            }
            ScaleFactor::NONE
        }
        1 => {
            // Adjacent files.
            let weak_bishop_attacks = bishop_attacks(weak_bsq, pos.occupied());
            let bishop_covers_block2 = weak_bsq == block_sq2
                || weak_bishop_attacks.contains(block_sq2);
            let bishop_covers_block1 = weak_bsq == block_sq1
                || weak_bishop_attacks.contains(block_sq1);

            if weak_ksq == block_sq1
                && opposite_colors(weak_ksq, strong_bsq)
                && (bishop_covers_block2 || rank_distance(psq1, psq2) >= 2)
            {
                return ScaleFactor::DRAW;
            }
            if weak_ksq == block_sq2
                && opposite_colors(weak_ksq, strong_bsq)
                && bishop_covers_block1
            {
                return ScaleFactor::DRAW;
            }
            ScaleFactor::NONE
        }
        _ => ScaleFactor::NONE,
    }
}

fn file_distance(a: Square, b: Square) -> u8 {
    let af = a.file() as i8;
    let bf = b.file() as i8;
    (af - bf).unsigned_abs()
}

fn rank_distance(a: Square, b: Square) -> u8 {
    let ar = a.rank() as i8;
    let br = b.rank() as i8;
    (ar - br).unsigned_abs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_colour_bishops_does_not_draw() {
        let p = Position::from_fen("4k3/8/8/8/3P4/3P4/8/2B1K2b w - - 0 1").unwrap();
        let strong = strong_side(&p).expect("KBPPKB signature");
        // Both bishops on same colour → no scaling.
        if !opposite_colors(
            pos_bishop(&p, Color::White),
            pos_bishop(&p, Color::Black),
        ) {
            assert_eq!(evaluate(&p, strong), ScaleFactor::NONE);
        }
    }

    fn pos_bishop(pos: &Position, c: Color) -> Square {
        pos.pieces_of(c, PieceType::Bishop).lsb()
    }
}
