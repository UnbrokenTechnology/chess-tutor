//! KRPKR — rook + pawn vs rook. The most important practical
//! rook-endgame; this port covers a handful of the well-known drawn
//! configurations (third-rank defence, sixth-rank checking distance,
//! Philidor draws, blockade-and-cut-off, advanced-pawn pre-promotion
//! wins).
//!
//! The reference comment notes this code is "not very pretty" and
//! "copied from Glaurung 1.x." Our port preserves the case structure
//! verbatim because each branch encodes a specific endgame-theory
//! pattern; rearranging risks breaking the precondition chains.

use crate::attacks::square_distance;
use crate::bitbases;
use crate::position::Position;
use crate::types::{Color, File, PieceType, Rank, ScaleFactor, Square, Value};

pub(super) fn strong_side(pos: &Position) -> Option<Color> {
    for &strong in Color::both().iter() {
        let weak = !strong;
        if pos.non_pawn_material(strong) == Value::ROOK_MG
            && pos.count(strong, PieceType::Rook) == 1
            && pos.count(strong, PieceType::Pawn) == 1
            && pos.non_pawn_material(weak) == Value::ROOK_MG
            && pos.count(weak, PieceType::Rook) == 1
            && pos.count(weak, PieceType::Pawn) == 0
        {
            return Some(strong);
        }
    }
    None
}

pub(super) fn evaluate(pos: &Position, strong: Color) -> ScaleFactor {
    let weak = !strong;

    // Normalise so the strong side is white and the pawn is on files
    // A-D (mirror across the centre if needed). Same `normalize` helper
    // used by the KPK bitbase probe.
    let strong_pawn_sq = pos.pieces_of(strong, PieceType::Pawn).lsb();
    let wksq = bitbases::normalize(strong, strong_pawn_sq, pos.king_square(strong));
    let bksq = bitbases::normalize(strong, strong_pawn_sq, pos.king_square(weak));
    let wrsq = bitbases::normalize(
        strong,
        strong_pawn_sq,
        pos.pieces_of(strong, PieceType::Rook).lsb(),
    );
    let wpsq = bitbases::normalize(strong, strong_pawn_sq, strong_pawn_sq);
    let brsq = bitbases::normalize(
        strong,
        strong_pawn_sq,
        pos.pieces_of(weak, PieceType::Rook).lsb(),
    );

    let f = wpsq.file();
    let r = wpsq.rank();
    let queening_sq = Square::new(f, Rank::R8);
    let tempo = (pos.side_to_move() == strong) as i32;

    // (1) Third-rank defence: pawn not past rank 5, weak king covers
    // queening square, strong king on the queenside half, and either
    // weak rook on rank 6 OR pawn≤R3 and strong rook not on rank 6.
    if r <= Rank::R5
        && square_distance(bksq, queening_sq) <= 1
        && wksq.index() <= Square::H5.index()
        && (brsq.rank() == Rank::R6 || (r <= Rank::R3 && wrsq.rank() != Rank::R6))
    {
        return ScaleFactor::DRAW;
    }

    // (2) Checking-from-behind on 6th rank with king covering queening.
    if r == Rank::R6
        && square_distance(bksq, queening_sq) <= 1
        && (wksq.rank() as i32) + tempo <= Rank::R6 as i32
        && (brsq.rank() == Rank::R1
            || (tempo == 0 && file_distance(brsq, wpsq) >= 3))
    {
        return ScaleFactor::DRAW;
    }

    // (3) Pawn on 6th/7th, weak king sits on queening square, weak
    // rook on rank 1.
    if r >= Rank::R6
        && bksq == queening_sq
        && brsq.rank() == Rank::R1
        && (tempo == 0 || square_distance(wksq, wpsq) >= 2)
    {
        return ScaleFactor::DRAW;
    }

    // (4) a7/a8 vs g7/h7 with rook on a-file.
    if wpsq == Square::A7
        && wrsq == Square::A8
        && (bksq == Square::H7 || bksq == Square::G7)
        && brsq.file() == File::A
        && (brsq.rank() <= Rank::R3 || wksq.file() >= File::D || wksq.rank() <= Rank::R5)
    {
        return ScaleFactor::DRAW;
    }

    // (5) Weak king blockades the pawn and strong king too far.
    if r <= Rank::R5
        && bksq.raw() == wpsq.raw() + 8
        && square_distance(wksq, wpsq) as i32 - tempo >= 2
        && square_distance(wksq, brsq) as i32 - tempo >= 2
    {
        return ScaleFactor::DRAW;
    }

    // (6) Pawn on 7th supported by rook from behind on the same file,
    // attacking king closer to queening, defender can't gain tempo
    // checking the strong rook. Winning, scale near MAX.
    if r == Rank::R7
        && f != File::A
        && wrsq.file() == f
        && wrsq != queening_sq
        && (square_distance(wksq, queening_sq) as i32)
            < square_distance(bksq, queening_sq) as i32 - 2 + tempo
        && (square_distance(wksq, queening_sq) as i32)
            < square_distance(bksq, wrsq) as i32 + tempo
    {
        return ScaleFactor(
            ScaleFactor::MAX.0 - 2 * square_distance(wksq, queening_sq) as i32,
        );
    }

    // (7) Same as (6) but pawn further back. Strong rook on pawn's
    // file behind the pawn; attacking king closer to queening and to
    // the step-square in front of the pawn.
    let wpsq_north = Square::from_index(wpsq.raw() + 8);
    if f != File::A
        && wrsq.file() == f
        && wrsq.index() < wpsq.index()
        && (square_distance(wksq, queening_sq) as i32)
            < square_distance(bksq, queening_sq) as i32 - 2 + tempo
        && (square_distance(wksq, wpsq_north) as i32)
            < square_distance(bksq, wpsq_north) as i32 - 2 + tempo
        && (square_distance(bksq, wrsq) as i32 + tempo >= 3
            || ((square_distance(wksq, queening_sq) as i32) < square_distance(bksq, wrsq) as i32 + tempo
                && (square_distance(wksq, wpsq_north) as i32)
                    < square_distance(bksq, wrsq) as i32 + tempo))
    {
        return ScaleFactor(
            ScaleFactor::MAX.0
                - 8 * square_distance(wpsq, queening_sq) as i32
                - 2 * square_distance(wksq, queening_sq) as i32,
        );
    }

    // (8) Pawn on rank ≤ 4 with weak king somewhere in the pawn's
    // path (greater square index = ahead of pawn in normalised frame).
    if r <= Rank::R4 && bksq.index() > wpsq.index() {
        if bksq.file() == wpsq.file() {
            return ScaleFactor(10);
        }
        if file_distance(bksq, wpsq) == 1 && square_distance(wksq, bksq) > 2 {
            return ScaleFactor(24 - 2 * square_distance(wksq, bksq) as i32);
        }
    }

    ScaleFactor::NONE
}

fn file_distance(a: Square, b: Square) -> u8 {
    let af = a.file() as i8;
    let bf = b.file() as i8;
    (af - bf).unsigned_abs()
}

#[cfg(test)]
mod tests {
    use super::super::{probe, ProbeResult};
    use super::*;

    #[test]
    fn philidor_third_rank_defence_is_draw() {
        // Textbook Philidor draw: white K+R+P, black K+R, white pawn
        // on e4, black rook on a6 (third rank from black's POV).
        // Black to move.
        let p = Position::from_fen("4k3/8/r7/8/4PK2/8/8/R7 b - - 0 1").unwrap();
        let r = evaluate(&p, Color::White);
        assert_eq!(r, ScaleFactor::DRAW);
    }

    #[test]
    #[ignore = "scaling dispatch gated off — see mod.rs SCALING_ENABLED"]
    fn dispatcher_routes_to_krpkr() {
        let p = Position::from_fen("4k3/8/r7/8/4PK2/8/8/R7 b - - 0 1").unwrap();
        assert!(matches!(probe(&p), ProbeResult::Scale { .. }));
    }

    #[test]
    fn unrelated_position_does_not_draw() {
        // Strong side has a 6th-rank passed pawn with the right
        // geometry to win — should NOT return DRAW.
        let p = Position::from_fen("8/8/3P4/3K4/8/r7/8/2k3R1 w - - 0 1").unwrap();
        let r = evaluate(&p, Color::White);
        assert_ne!(r, ScaleFactor::DRAW);
    }
}
