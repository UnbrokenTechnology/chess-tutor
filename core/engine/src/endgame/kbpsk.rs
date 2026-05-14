//! KBPsK — king + bishop + one or more pawns vs king (+ possibly pawns).
//!
//! Two fortress patterns:
//!
//! 1. **Wrong-rook-pawn**: all strong-side pawns on file A or H with a
//!    bishop of the wrong colour for the queening square, and the
//!    weak king close to the corner. A textbook fortress.
//!
//! 2. **Blocked B/G-file pawn race**: all pawns on a single B or G
//!    file, with the strong side's lead pawn blocked on the 7th rank
//!    by a weak pawn the bishop can't attack (or one pawn left).

use crate::attacks::square_distance;
use crate::bitboard::{file_bb, opposite_colors};
use crate::position::Position;
use crate::types::{Color, Direction, File, PieceType, Rank, ScaleFactor, Square, Value};

/// Returns `Some(strong)` when strong side has K+B+(≥1 pawn) and nothing
/// else. Weak side is unconstrained.
pub(super) fn strong_side(pos: &Position) -> Option<Color> {
    Color::both().into_iter().find(|&strong| {
        pos.non_pawn_material(strong) == Value::BISHOP_MG
            && pos.count(strong, PieceType::Bishop) == 1
            && pos.count(strong, PieceType::Pawn) >= 1
    })
}

pub(super) fn evaluate(pos: &Position, strong: Color) -> ScaleFactor {
    let weak = !strong;
    let strong_pawns = pos.pieces_of(strong, PieceType::Pawn);
    let pawns_file = strong_pawns.lsb().file();
    let pawns_on_single_file = (strong_pawns & !file_bb(pawns_file)).is_empty();

    // --- Pattern 1: wrong-rook-pawn -----------------------------------
    if pawns_on_single_file && matches!(pawns_file, File::A | File::H) {
        let bishop_sq = pos.pieces_of(strong, PieceType::Bishop).lsb();
        let queening_sq = Square::new(pawns_file, Rank::R8).from_perspective(strong);
        let weak_ksq = pos.king_square(weak);

        if opposite_colors(queening_sq, bishop_sq) && square_distance(queening_sq, weak_ksq) <= 1 {
            return ScaleFactor::DRAW;
        }
    }

    // --- Pattern 2: blocked B/G-file pawn race ------------------------
    if matches!(pawns_file, File::B | File::G)
        && (pos.pieces(PieceType::Pawn) & !file_bb(pawns_file)).is_empty()
        && pos.non_pawn_material(weak) == Value::ZERO
        && pos.count(weak, PieceType::Pawn) >= 1
    {
        // Frontmost weak pawn from `strong`'s POV (i.e. the weak pawn
        // closest to the strong side's back rank — the one most likely
        // to be the blockading pawn).
        let weak_pawns = pos.pieces_of(weak, PieceType::Pawn);
        let weak_pawn_sq = weak_pawns.frontmost(strong);

        let strong_ksq = pos.king_square(strong);
        let weak_ksq = pos.king_square(weak);
        let bishop_sq = pos.pieces_of(strong, PieceType::Bishop).lsb();

        let push = Direction::pawn_push(weak);
        let weak_pawn_blocker_sq = weak_pawn_sq + push;
        let pawn_strong_relative_rank = weak_pawn_sq.rank().from_perspective(strong);

        if pawn_strong_relative_rank == Rank::R7
            && pos
                .pieces_of(strong, PieceType::Pawn)
                .contains(weak_pawn_blocker_sq)
            && (opposite_colors(bishop_sq, weak_pawn_sq)
                || pos.count(strong, PieceType::Pawn) == 1)
        {
            let strong_king_dist = square_distance(weak_pawn_sq, strong_ksq);
            let weak_king_dist = square_distance(weak_pawn_sq, weak_ksq);

            // Weak king is on its back two ranks (relative-rank ≥ 7
            // from strong's POV), within 2 of the blocking pawn, and
            // the strong king is no closer.
            if weak_ksq.rank().from_perspective(strong) >= Rank::R7
                && weak_king_dist <= 2
                && weak_king_dist <= strong_king_dist
            {
                return ScaleFactor::DRAW;
            }
        }
    }

    ScaleFactor::NONE
}

#[cfg(test)]
mod tests {
    use super::super::{probe, ProbeResult};
    use super::*;

    #[test]
    fn wrong_rook_pawn_with_weak_king_in_corner_is_draw() {
        // White K+B(b1, light)+P(a6), black K on a8 — the textbook
        // wrong-rook-pawn fortress.
        // Bishop b1 is on the LIGHT square; promotion square a8 is also
        // a LIGHT square — same colour, so the bishop CAN guard. Not a
        // fortress.
        // Try bishop on c1 (dark) so promotion a8 (light) is opposite.
        let p = Position::from_fen("k7/8/P7/8/8/8/8/2B1K3 w - - 0 1").unwrap();
        let r = evaluate(&p, Color::White);
        assert_eq!(r, ScaleFactor::DRAW);
    }

    #[test]
    fn correct_colour_bishop_does_not_draw() {
        // Same shape but bishop colour matches the queening square.
        // White Bishop on a3 (dark), promotion a8 (light) — same? Let me check:
        // a3: file a (=0), rank 3 (=2). (0 + 2) = 2 → even → light? Standard
        // chess: a1 is dark (file 0, rank 0; 0+0=0 even, but a1 is dark by
        // convention). Need to check our opposite_colors definition.
        // Simpler: pick a bishop square that's the SAME colour as a8.
        // a8 is a LIGHT square; b1 is also a LIGHT square in standard
        // colouring. So bishop on b1 matches a8 = no fortress.
        let p = Position::from_fen("k7/8/P7/8/8/8/8/1B2K3 w - - 0 1").unwrap();
        let r = evaluate(&p, Color::White);
        assert_eq!(r, ScaleFactor::NONE);
    }

    #[test]
    fn pawns_on_centre_file_do_not_draw() {
        // Pawns on D-file — no fortress applies.
        let p = Position::from_fen("4k3/8/3P4/8/8/8/8/2B1K3 w - - 0 1").unwrap();
        let r = evaluate(&p, Color::White);
        assert_eq!(r, ScaleFactor::NONE);
    }

    #[test]
    fn no_pawns_does_not_fire_signature() {
        let p = Position::from_fen("4k3/8/8/8/8/8/8/2B1K3 w - - 0 1").unwrap();
        assert!(strong_side(&p).is_none());
    }

    #[test]
    #[ignore = "scaling dispatch gated off — see mod.rs SCALING_ENABLED"]
    fn dispatcher_routes_to_kbpsk_fortress() {
        let p = Position::from_fen("k7/8/P7/8/8/8/8/2B1K3 w - - 0 1").unwrap();
        match probe(&p) {
            ProbeResult::Scale {
                strong_side,
                factor,
            } => {
                assert_eq!(strong_side, Color::White);
                assert_eq!(factor, ScaleFactor::DRAW);
            }
            other => panic!("expected Scale, got {other:?}"),
        }
    }
}
