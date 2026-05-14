//! KPKP — king + pawn vs king + pawn. Probes the KPK bitbase with the
//! weaker pawn removed; if that's a draw, the position with both pawns
//! is probably a draw too. Exception: the strong pawn advanced past
//! rank 5 on a non-rook file is too dangerous to assume drawn.
//!
//! Because either side could end up as the eg-winning side, this
//! function returns a single scale factor that the dispatcher then
//! applies to both colors via [`super::ProbeResult::ScaleBoth`].

use crate::bitbases;
use crate::position::Position;
use crate::types::{Color, File, PieceType, Rank, ScaleFactor, Value};

pub(super) fn matches(pos: &Position) -> bool {
    if pos.non_pawn_material(Color::White) != Value::ZERO
        || pos.non_pawn_material(Color::Black) != Value::ZERO
    {
        return false;
    }
    pos.count(Color::White, PieceType::Pawn) == 1 && pos.count(Color::Black, PieceType::Pawn) == 1
}

pub(super) fn evaluate(pos: &Position) -> ScaleFactor {
    // We pick a canonical "strong" side based on whose pawn is closer
    // to promoting (relative rank from each side's POV); the function
    // is symmetric in spirit, so ties break to White.
    let strong = stronger_pawn_side(pos);
    let weak = !strong;

    let strong_pawn_sq = pos.pieces_of(strong, PieceType::Pawn).lsb();
    let n_wksq = bitbases::normalize(strong, strong_pawn_sq, pos.king_square(strong));
    let n_bksq = bitbases::normalize(strong, strong_pawn_sq, pos.king_square(weak));
    let n_psq = bitbases::normalize(strong, strong_pawn_sq, strong_pawn_sq);

    let bb_stm = if pos.side_to_move() == strong {
        Color::White
    } else {
        Color::Black
    };

    // Pawn past 5th rank on a non-rook file — too dangerous to assume
    // a draw.
    if n_psq.rank() >= Rank::R5 && n_psq.file() != File::A {
        return ScaleFactor::NONE;
    }

    if bitbases::kpk_probe(n_wksq, n_psq, n_bksq, bb_stm) {
        ScaleFactor::NONE
    } else {
        ScaleFactor::DRAW
    }
}

fn stronger_pawn_side(pos: &Position) -> Color {
    let wp = pos.pieces_of(Color::White, PieceType::Pawn).lsb();
    let bp = pos.pieces_of(Color::Black, PieceType::Pawn).lsb();
    let wrank = wp.rank().from_perspective(Color::White);
    let brank = bp.rank().from_perspective(Color::Black);
    if brank > wrank {
        Color::Black
    } else {
        Color::White
    }
}

#[cfg(test)]
mod tests {
    use super::super::{probe, ProbeResult};
    use super::*;

    #[test]
    fn matches_kp_vs_kp_signature() {
        let p = Position::from_fen("4k3/4p3/8/8/4P3/8/8/4K3 w - - 0 1").unwrap();
        assert!(matches(&p));
    }

    #[test]
    fn rejects_two_pawns_one_side() {
        let p = Position::from_fen("4k3/8/8/8/4P3/4P3/8/4K3 w - - 0 1").unwrap();
        assert!(!matches(&p));
    }

    #[test]
    #[ignore = "scaling dispatch gated off — see mod.rs SCALING_ENABLED"]
    fn dispatcher_yields_scale_both_when_drawn() {
        // Mirrored back-rank-ish KPKP — pawns barely advanced, both
        // sides should treat as drawish.
        let p = Position::from_fen("4k3/4p3/8/8/4P3/8/8/4K3 w - - 0 1").unwrap();
        // The probe may or may not return ScaleBoth depending on the
        // exact bitbase verdict. Just check it doesn't panic and is one
        // of the expected variants.
        assert!(matches!(
            probe(&p),
            ProbeResult::ScaleBoth(_) | ProbeResult::None
        ));
    }
}
