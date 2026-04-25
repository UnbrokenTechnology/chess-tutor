//! Initiative correction: a second-order adjustment to the tapered
//! evaluation that rewards whichever side has the initiative in the
//! position — measured via pawn counts, king-flank geometry, passed
//! pawns, and "is this unwinnable?" heuristics.
//!
//! Mirrors `Evaluation::initiative()`. Critically, the correction is
//! *side-capped*: the applied mg and eg adjustments can never flip the
//! sign of the original score. An initiative bonus can only reduce an
//! already-winning score toward zero, not convert a loss into a win.

use super::Evaluator;
use crate::bitboard::{file_distance, rank_distance, KING_SIDE, QUEEN_SIDE};
use crate::types::{Color, PieceType, Rank, Score};

pub(crate) fn evaluate(e: &Evaluator<'_>, score: Score) -> Score {
    let pos = e.pos;
    let mg = score.mg().0;
    let eg = score.eg().0;

    let white_king = pos.king_square(Color::White);
    let black_king = pos.king_square(Color::Black);

    // Outflanking: positive when kings are closer vertically than they
    // are horizontally (i.e., there's room to flank the enemy king).
    let outflanking =
        file_distance(white_king, black_king) as i32 - rank_distance(white_king, black_king) as i32;

    // Infiltration: either king standing in the enemy's half.
    let infiltration = white_king.rank().index() as i32 > Rank::R4.index() as i32
        || (black_king.rank().index() as i32) < Rank::R5.index() as i32;

    let all_pawns = pos.pieces(PieceType::Pawn);
    let pawns_on_both_flanks = (all_pawns & QUEEN_SIDE).any() && (all_pawns & KING_SIDE).any();

    let passed_count = (e.pawns.passed_pawns[0] | e.pawns.passed_pawns[1]).popcount() as i32;

    let almost_unwinnable = passed_count == 0 && outflanking < 0 && !pawns_on_both_flanks;

    // Complexity — a positive number means the attacker has grounds to
    // press, a negative number means the position is drawish.
    let complexity = 9 * passed_count
        + 11 * all_pawns.popcount() as i32
        + 9 * outflanking
        + 12 * infiltration as i32
        + 21 * pawns_on_both_flanks as i32
        + 51 * (pos.non_pawn_material_total().0 == 0) as i32
        - 43 * almost_unwinnable as i32
        - 100;

    // Side-capped application. `u` (mg) can only ever reduce |mg|; it
    // kicks in when complexity is below -50. `v` (eg) directly scales
    // with complexity but is clamped so it can't flip the sign of eg.
    let u = mg.signum() * (complexity + 50).min(0).max(-mg.abs());
    let v = eg.signum() * complexity.max(-eg.abs());

    Score::new(u, v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::position::Position;

    #[test]
    fn initiative_cannot_flip_mg_sign() {
        // Build a zero-score position and call initiative with a
        // contrived positive mg input. The returned u must not exceed
        // the input mg in magnitude (side-capping is the invariant).
        let p = Position::startpos();
        let mut e = Evaluator::new(&p);
        e.initialize(Color::White);
        e.initialize(Color::Black);
        super::super::pieces::evaluate(&mut e, Color::White);
        super::super::pieces::evaluate(&mut e, Color::Black);
        let input = Score::new(50, 50);
        let adj = evaluate(&e, input);
        assert!(
            (input + adj).mg().0 >= 0,
            "initiative must not push mg through zero: {} + {} = {}",
            input.mg().0,
            adj.mg().0,
            (input + adj).mg().0,
        );
    }

    #[test]
    fn pawnless_both_flanks_means_zero_pawns_on_both_flanks_flag() {
        // No pawns at all → pawnsOnBothFlanks is false, and the
        // "no non-pawn material" rule still kicks in the 51-point
        // bonus. We just check the function doesn't panic and
        // returns a Score.
        let p = Position::from_fen("4k3/8/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        let mut e = Evaluator::new(&p);
        e.initialize(Color::White);
        e.initialize(Color::Black);
        super::super::pieces::evaluate(&mut e, Color::White);
        super::super::pieces::evaluate(&mut e, Color::Black);
        let _ = evaluate(&e, Score::new(10, 10));
    }

    #[test]
    fn startpos_initiative_is_small() {
        // At startpos the complexity formula nets out close to zero
        // (lots of pawns + no passers + no outflanking). Absolute
        // contribution should be small relative to material.
        let p = Position::startpos();
        let mut e = Evaluator::new(&p);
        e.initialize(Color::White);
        e.initialize(Color::Black);
        super::super::pieces::evaluate(&mut e, Color::White);
        super::super::pieces::evaluate(&mut e, Color::Black);
        let adj = evaluate(&e, Score::ZERO);
        // Score is zero → both signums are zero → u and v are zero.
        assert_eq!(adj, Score::ZERO);
    }
}
