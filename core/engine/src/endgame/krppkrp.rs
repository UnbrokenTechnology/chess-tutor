//! KRPPKRP — rook + 2 pawns vs rook + pawn. A single rule: if the
//! strong side has no passed pawn and the weak king is actively
//! placed in front of both pawns, the position is drawish.

use crate::position::Position;
use crate::types::{Color, PieceType, ScaleFactor, Value};

/// SF11's `KRPPKRPScaleFactors` — rank-indexed scale factor for the
/// blockaded-no-passed-pawn pattern.
const RANK_SCALE: [i32; 8] = [0, 9, 10, 14, 21, 44, 0, 0];

pub(super) fn strong_side(pos: &Position) -> Option<Color> {
    for &strong in Color::both().iter() {
        let weak = !strong;
        if pos.non_pawn_material(strong) == Value::ROOK_MG
            && pos.count(strong, PieceType::Rook) == 1
            && pos.count(strong, PieceType::Pawn) == 2
            && pos.non_pawn_material(weak) == Value::ROOK_MG
            && pos.count(weak, PieceType::Rook) == 1
            && pos.count(weak, PieceType::Pawn) == 1
        {
            return Some(strong);
        }
    }
    None
}

pub(super) fn evaluate(pos: &Position, strong: Color) -> ScaleFactor {
    let weak = !strong;
    let strong_pawns = pos.pieces_of(strong, PieceType::Pawn);
    // Iterate two pawns out of the bitboard.
    let mut iter = strong_pawns.into_iter();
    let psq1 = iter.next().expect("KRPPKRP signature guarantees 2 pawns");
    let psq2 = iter.next().expect("KRPPKRP signature guarantees 2 pawns");
    let weak_ksq = pos.king_square(weak);

    if pos.pawn_passed(strong, psq1) || pos.pawn_passed(strong, psq2) {
        return ScaleFactor::NONE;
    }

    let r1 = psq1.rank().from_perspective(strong);
    let r2 = psq2.rank().from_perspective(strong);
    let max_rank = if r1 >= r2 { r1 } else { r2 };

    if file_distance(weak_ksq, psq1) <= 1
        && file_distance(weak_ksq, psq2) <= 1
        && weak_ksq.rank().from_perspective(strong) > max_rank
    {
        return ScaleFactor(RANK_SCALE[max_rank.index()]);
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
    fn signature_detects_2v1_rook_pawn_endgame() {
        let p = Position::from_fen("4k3/p7/8/8/8/8/PP6/4K3 w - - 0 1").unwrap();
        // No rooks → signature should NOT match.
        assert!(strong_side(&p).is_none());
    }

    #[test]
    fn signature_matches_with_rooks() {
        let p = Position::from_fen("r3k3/p7/8/8/8/8/PP6/R3K3 w - - 0 1").unwrap();
        assert_eq!(strong_side(&p), Some(Color::White));
    }

    #[test]
    fn passed_pawn_returns_none() {
        // White pawns on a2 and h2 (no shared files with the black
        // pawn on c7), making both passed.
        let p = Position::from_fen("r3k3/2p5/8/8/8/8/P6P/R3K3 w - - 0 1").unwrap();
        let r = evaluate(&p, Color::White);
        assert_eq!(r, ScaleFactor::NONE);
    }
}
