//! KQKRPs — KQ vs KR + pawns. Tests for the third-rank-rook fortress
//! where the weak side's rook on its own 3rd rank is defended by a
//! pawn the rook can attack, with the weak king on rank 1 or 2 and
//! the strong king cut off on rank ≥ 4.

use crate::attacks::{king_attacks, pawn_attacks_from};
use crate::position::Position;
use crate::types::{Color, PieceType, Rank, ScaleFactor, Value};

pub(super) fn strong_side(pos: &Position) -> Option<Color> {
    for &strong in Color::both().iter() {
        let weak = !strong;
        if pos.count(strong, PieceType::Pawn) == 0
            && pos.non_pawn_material(strong) == Value::QUEEN_MG
            && pos.count(strong, PieceType::Queen) == 1
            && pos.count(weak, PieceType::Rook) == 1
            && pos.count(weak, PieceType::Pawn) >= 1
        {
            return Some(strong);
        }
    }
    None
}

pub(super) fn evaluate(pos: &Position, strong: Color) -> ScaleFactor {
    let weak = !strong;
    let king_sq = pos.king_square(weak);
    let strong_king_sq = pos.king_square(strong);
    let rook_sq = pos.pieces_of(weak, PieceType::Rook).lsb();

    let king_rel = king_sq.rank().from_perspective(weak);
    let strong_king_rel = strong_king_sq.rank().from_perspective(weak);
    let rook_rel = rook_sq.rank().from_perspective(weak);

    // Weak king on rank ≤ 2 (its own back two ranks), strong king
    // pushed away (rank ≥ 4 from weak's POV), weak rook on its 3rd
    // rank, and the king/pawn/rook geometry is the classic fortress:
    // a weak pawn that is BOTH adjacent to the king AND covered by
    // the rook's pawn-attack pattern (as if the rook were a strong-
    // side pawn).
    if king_rel <= Rank::R2
        && strong_king_rel >= Rank::R4
        && rook_rel == Rank::R3
        && (pos.pieces_of(weak, PieceType::Pawn)
            & king_attacks(king_sq)
            & pawn_attacks_from(strong, rook_sq))
        .any()
    {
        return ScaleFactor::DRAW;
    }

    ScaleFactor::NONE
}

#[cfg(test)]
mod tests {
    use super::super::{probe, ProbeResult};
    use super::*;

    #[test]
    fn fortress_pattern_is_a_draw() {
        // White Q vs black K+R+P. Classic third-rank rook fortress:
        // black king on g8, rook on h6 (third rank from black's POV),
        // pawn on g7 covered by the rook from h6 (pawn-attack pattern
        // of a white pawn on h6 attacks g7).
        let p = Position::from_fen("6k1/6p1/7r/8/8/2K5/1Q6/8 w - - 0 1").unwrap();
        let r = evaluate(&p, Color::White);
        assert_eq!(r, ScaleFactor::DRAW);
    }

    #[test]
    fn weak_king_far_advanced_does_not_draw() {
        // Weak king on rank 4 (not on its back two ranks) — fortress
        // doesn't apply.
        let p = Position::from_fen("8/8/8/4k3/8/r7/8/3QK3 w - - 0 1").unwrap();
        let r = evaluate(&p, Color::White);
        assert_eq!(r, ScaleFactor::NONE);
    }

    #[test]
    #[ignore = "scaling dispatch gated off — see mod.rs SCALING_ENABLED"]
    fn dispatcher_routes_to_kqkrps() {
        let p = Position::from_fen("6k1/6p1/7r/8/8/2K5/1Q6/8 w - - 0 1").unwrap();
        assert!(matches!(probe(&p), ProbeResult::Scale { factor, .. } if factor == ScaleFactor::DRAW));
    }
}
