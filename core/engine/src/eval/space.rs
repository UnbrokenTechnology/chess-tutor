//! Space evaluation: a middle-game-only bonus for controlling squares
//! in the centre portion of our own camp, weighted quadratically by how
//! many pieces we have on the board.
//!
//! Mirrors `Evaluation::space<Us>()`. The short-circuit at low total
//! non-pawn material ensures this term vanishes in the endgame, matching
//! the reference's `SpaceThreshold` gate.

use super::Evaluator;
use crate::bitboard::{CENTER_FILES, RANK_2, RANK_3, RANK_4, RANK_5, RANK_6, RANK_7};
use crate::types::{Color, Direction, PieceType, Score, Value};

/// Below this amount of total non-pawn material the space term is not
/// worth evaluating — the board is too open for space considerations to
/// matter.
const SPACE_THRESHOLD: Value = Value(12_222);

pub(crate) fn evaluate(e: &Evaluator<'_>, us: Color) -> Score {
    let pos = e.pos;
    if pos.non_pawn_material_total().0 < SPACE_THRESHOLD.0 {
        return Score::ZERO;
    }

    let them = !us;
    let them_idx = them.index();

    let down = Direction(-Direction::pawn_push(us).0);
    let space_mask = match us {
        Color::White => CENTER_FILES & (RANK_2 | RANK_3 | RANK_4),
        Color::Black => CENTER_FILES & (RANK_7 | RANK_6 | RANK_5),
    };

    // Safe squares inside the space mask: not our pawns, not attacked
    // by enemy pawns.
    let safe = space_mask
        & !pos.pieces_of(us, PieceType::Pawn)
        & !e.attacked_by[them_idx][PieceType::Pawn.index()];

    // "Behind" = squares within three pushes behind our pawns (pawns
    // shifted back one, two, and three ranks).
    let mut behind = pos.pieces_of(us, PieceType::Pawn);
    behind |= behind.shift(down);
    behind |= behind.shift(Direction(down.0 + down.0));

    let bonus =
        safe.popcount() as i32 + (behind & safe & !e.attacked_by_all[them_idx]).popcount() as i32;
    let weight = pos.pieces_by_color(us).popcount() as i32 - 1;

    Score::new(bonus * weight * weight / 16, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::position::Position;

    fn space_score(fen: &str, us: Color) -> Score {
        let pos = Position::from_fen(fen).unwrap();
        let mut e = Evaluator::new(&pos);
        e.initialize(Color::White);
        e.initialize(Color::Black);
        super::super::pieces::evaluate(&mut e, Color::White);
        super::super::pieces::evaluate(&mut e, Color::Black);
        evaluate(&e, us)
    }

    #[test]
    fn startpos_space_is_symmetric() {
        let fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        let w = space_score(fen, Color::White);
        let b = space_score(fen, Color::Black);
        assert_eq!(w, b);
        // Both sides control exactly their own 3-rank central band;
        // and material is high enough to fire the term.
        assert!(w.mg().0 > 0);
    }

    #[test]
    fn endgame_space_is_zero() {
        // King-and-pawn endgame has total non-pawn material well below
        // SPACE_THRESHOLD — term should be exactly zero.
        let p = Position::from_fen("4k3/8/8/8/3P4/8/8/4K3 w - - 0 1").unwrap();
        let mut e = Evaluator::new(&p);
        e.initialize(Color::White);
        e.initialize(Color::Black);
        super::super::pieces::evaluate(&mut e, Color::White);
        super::super::pieces::evaluate(&mut e, Color::Black);
        assert_eq!(evaluate(&e, Color::White), Score::ZERO);
    }
}
