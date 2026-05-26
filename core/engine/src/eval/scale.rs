//! Endgame scale factor — how much to trust the endgame half of the
//! tapered score, based on drawish-material heuristics and the 50-move
//! clock. Consumed by [`super::core::evaluate_inner`] at the tapering step.

use super::Evaluator;
use crate::types::{Color, PieceType, ScaleFactor, Value};

pub(super) fn scale_factor(e: &Evaluator<'_>, eg: i32, strong_side: Color) -> ScaleFactor {
    let base = e.material.scale_factor[strong_side.index()];
    if base != ScaleFactor::NORMAL {
        return base;
    }

    // Apply general "how likely is this to be drawn" heuristics only when
    // the material-level scale is NORMAL. Opposite-coloured bishops with
    // no other non-pawn material is the classic drawish endgame.
    let npm = e.pos.non_pawn_material_total().0;
    let bishop_mg_double = Value::BISHOP_MG.0 * 2;

    let sf_opp_bishops_only = e.pos.opposite_bishops() && npm == bishop_mg_double;
    let mut sf = if sf_opp_bishops_only {
        22
    } else {
        let pawn_count = e.pos.count(strong_side, PieceType::Pawn) as i32;
        let multiplier = if e.pos.opposite_bishops() { 2 } else { 7 };
        base.0.min(36 + multiplier * pawn_count)
    };

    // Draw down further based on how long it's been since a capture or
    // pawn move — the closer to the 50-move rule, the drawishre.
    let rule50 = e.pos.halfmove_clock() as i32;
    sf = sf.max(0).saturating_sub((rule50 - 12).max(0) / 4);

    // Silence unused var lint — eg is reserved here for the future
    // lazy-eval shortcut the reference uses before reaching this
    // function. Keeping the parameter documents the intended signature.
    let _ = eg;

    ScaleFactor(sf)
}
