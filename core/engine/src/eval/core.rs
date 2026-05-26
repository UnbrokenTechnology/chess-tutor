//! The evaluation orchestrator — [`evaluate_inner`] assembles every
//! per-term contribution into a single tapered, scaled, side-to-move
//! [`Value`], optionally capturing a per-term [`EvalTrace`]. The public
//! entry points in [`super`] are thin wrappers over this.

use super::scale::scale_factor;
use super::{initiative, king, passed, pieces, space, threats};
use super::{EvalTrace, Evaluator, MaterialBreakdown, PHASE_MAX, SCALE_NORMAL, TEMPO};
use crate::opponent::{EvalCategory, EvalMask};
use crate::pawns::PawnsEval;
use crate::position::Position;
use crate::types::{Color, PieceType, Score, Value};

/// `Σ count(pt) × piece_value(pt)` over `pt ∈ {Pawn, Knight, Bishop,
/// Rook, Queen}`, white minus black, expressed as a packed
/// (mg, eg) [`Score`]. Kings have no piece value. This is the part
/// of `pos.psq_score()` that depends only on piece counts —
/// subtracting it from the PSQT total leaves the pure positional
/// PSQT contribution.
fn piece_value_balance(pos: &Position) -> Score {
    let mut mg = 0i32;
    let mut eg = 0i32;
    for pt in [
        PieceType::Pawn,
        PieceType::Knight,
        PieceType::Bishop,
        PieceType::Rook,
        PieceType::Queen,
    ] {
        let net = pos.count(Color::White, pt) as i32 - pos.count(Color::Black, pt) as i32;
        if net == 0 {
            continue;
        }
        mg += Value::mg_of_piece(pt).0 * net;
        eg += Value::eg_of_piece(pt).0 * net;
    }
    Score::new(mg, eg)
}

pub(super) fn evaluate_inner(
    pos: &Position,
    pawns_eval: PawnsEval,
    mask: EvalMask,
    mut trace: Option<&mut EvalTrace>,
) -> Value {
    let mut e = Evaluator::new_with_pawns(pos, pawns_eval);

    // If material reports a specialised endgame evaluator (`ProbeResult::
    // Override`), trust it and skip the classical terms entirely.
    // Scaling-function results (`Scale` / `ScaleBoth`) flow through
    // `material.scale_factor` and are consumed at the tapering step.
    if let Some(v) = e.material.endgame_value {
        let signed = if pos.side_to_move() == Color::White {
            v
        } else {
            -v
        };
        if let Some(t) = trace.as_mut() {
            t.final_value = signed;
        }
        return signed;
    }

    // Seed the running score with the incrementally-maintained PSQ score
    // (material + positional), the material imbalance polynomial, and
    // the pawn-structure score — exactly the same three "free" terms the
    // reference picks up before any work happens. `EvalCategory::
    // PawnStructure` can mask off the pawn term; material and imbalance
    // are always live (disabling them would make the bot blind to piece
    // values, which isn't a useful teaching mode).
    let material = pos.psq_score();
    let imbalance = e.material.imbalance;
    let mut score = material + imbalance;
    if !mask.is_disabled(EvalCategory::PawnStructure) {
        score += e.pawns.score();
    }

    // --- Lazy eval (SF11 evaluate.cpp:790-793) ---
    //
    // When material + imbalance + pawn-structure already gives a
    // very lopsided position, the rest of the eval (pieces,
    // mobility, king safety, threats, passed pawns, space,
    // initiative) is unlikely to flip the sign or even change it
    // materially — and the parent's pruning decisions only care
    // about "are we above/below beta" anyway. Bail with the
    // current `score` averaged across game phases.
    //
    // Gated on `trace.is_none()`: the teaching layer always
    // requests the full breakdown, so we never lazy-bail when a
    // trace was requested. The threshold widens with non-pawn
    // material (richer positions can swing further). No `+TEMPO`
    // on the lazy bail — matches SF11 search.cpp line 793.
    //
    // The prior 2026-05-12 attempt regressed best-move stability
    // because the search's pruning stack hadn't been tuned to
    // tolerate the approximation noise. statScore-LMR + cutNode +
    // CMP + ProbCut have since landed; the hypothesis (per
    // memory + SF design) is that those features absorb the
    // noise.
    if trace.is_none() {
        let lazy_v = (score.mg().0 + score.eg().0) / 2;
        let lazy_thresh = 1400 + pos.non_pawn_material_total().0 / 64;
        if lazy_v.abs() > lazy_thresh {
            let signed = if pos.side_to_move() == Color::White {
                lazy_v
            } else {
                -lazy_v
            };
            return Value(signed);
        }
    }

    e.initialize(Color::White);
    e.initialize(Color::Black);

    // Per-piece-type positional terms, interleaved with mobility
    // accumulation. The pieces walk also populates attack tables that
    // king / threats / passed read later — we always *run* it, even
    // when masked, and only gate the contribution to `score`. This
    // keeps the dependent terms producing the right values when their
    // own categories are still enabled.
    let white_pieces = pieces::evaluate(&mut e, Color::White);
    let black_pieces = pieces::evaluate(&mut e, Color::Black);
    if !mask.is_disabled(EvalCategory::Pieces) {
        score += white_pieces.total() - black_pieces.total();
    }
    if !mask.is_disabled(EvalCategory::Mobility) {
        score +=
            e.mobility[Color::White.index()].total() - e.mobility[Color::Black.index()].total();
    }

    let white_king = king::evaluate(&e, Color::White);
    let black_king = king::evaluate(&e, Color::Black);
    if !mask.is_disabled(EvalCategory::KingSafety) {
        score += white_king.total() - black_king.total();
    }

    let white_threats = threats::evaluate(&e, Color::White);
    let black_threats = threats::evaluate(&e, Color::Black);
    if !mask.is_disabled(EvalCategory::Threats) {
        score += white_threats.total() - black_threats.total();
    }

    let white_passed = passed::evaluate(&e, Color::White);
    let black_passed = passed::evaluate(&e, Color::Black);
    if !mask.is_disabled(EvalCategory::PassedPawns) {
        score += white_passed.total() - black_passed.total();
    }

    let white_space = space::evaluate(&e, Color::White);
    let black_space = space::evaluate(&e, Color::Black);
    if !mask.is_disabled(EvalCategory::Space) {
        score += white_space - black_space;
    }

    // Initiative is a multiplier on the (mg, eg) tail of `score` —
    // when masked off we just skip it. The argument `score` is the
    // running sum *after* the previous (possibly-masked) categories,
    // which is what we want: initiative scales the bot's current
    // picture of the position, not a hypothetical unmasked one.
    let initiative_score = initiative::evaluate(&e, score);
    if !mask.is_disabled(EvalCategory::Initiative) {
        score += initiative_score;
    }

    // Tapered interpolation between mg and eg scores. The eg half is
    // additionally scaled by the side-specific ScaleFactor.
    let phase = e.material.game_phase.0;
    let eg_val = score.eg().0;
    let winning_side = if eg_val > 0 {
        Color::White
    } else {
        Color::Black
    };
    let sf = scale_factor(&e, eg_val, winning_side).0;

    let mg_part = score.mg().0 * phase;
    let eg_part = score.eg().0 * (PHASE_MAX - phase) * sf / SCALE_NORMAL;
    let v = (mg_part + eg_part) / PHASE_MAX;

    let stm_signed = if pos.side_to_move() == Color::White {
        v
    } else {
        -v
    };
    let final_value = Value(stm_signed) + TEMPO;

    if let Some(t) = trace.as_mut() {
        // Split `material` (= pos.psq_score()) into raw piece values
        // and the PSQT positional bonus. Cheap: 5 popcount lookups
        // per colour. piece_value changes only on captures, so a
        // teaching narrator can attribute "you lost material" only
        // to actual captures rather than to PSQ-shift artifacts.
        let piece_value = piece_value_balance(pos);
        t.material = MaterialBreakdown {
            piece_value,
            psq_positional: material - piece_value,
        };
        t.imbalance = imbalance;
        t.pawns = e.pawns.breakdowns;
        t.pieces = [white_pieces, black_pieces];
        t.mobility = e.mobility;
        t.king = [white_king, black_king];
        t.threats = [white_threats, black_threats];
        t.passed = [white_passed, black_passed];
        t.space = [white_space, black_space];
        t.initiative = initiative_score;
        t.total = score;
        t.phase = phase;
        t.scale_factor = sf;
        t.tempo = TEMPO;
        t.final_value = final_value;
    }

    final_value
}
