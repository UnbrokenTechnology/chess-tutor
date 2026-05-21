//! Per-term tapered-cp deltas between two traces, plus the
//! cumulative-threshold prefix selector the retrospective uses to
//! answer "which terms carried the swing?"

use super::TermId;
use crate::eval::EvalTrace;
use crate::types::{Piece, Score};

/// The change in a single evaluation term between a before-move trace
/// and an after-move trace, both as raw `(mg, eg)` packed [`Score`]
/// deltas and as the single tapered cp number the final eval would
/// see from this swing in isolation.
///
/// Deltas are **white-POV net** (post − pre, where each operand is
/// white − black). A positive delta means "this term got better for
/// white after the move"; a negative one means "better for black".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TermDelta {
    pub term: TermId,
    /// Middlegame delta, in raw `Score` units.
    pub delta_mg: i32,
    /// Endgame delta, in raw `Score` units.
    pub delta_eg: i32,
    /// Tapered delta, in engine-internal cp at the post-move phase
    /// and scale factor. This is the number that feeds ordering.
    pub delta_tapered: i32,
    /// Piece attribution, when it can be cheaply identified. Left
    /// `None` for this phase — aggregate terms (threats, king safety,
    /// mobility) would need scratch state from
    /// [`crate::eval::Evaluator`] to attribute correctly.
    pub piece_involved: Option<Piece>,
}

/// Compute the full tapered-cp breakdown between the pre-move trace
/// and the ply-1 (immediately-after-user-move) trace.
///
/// Every term diffs against the same `post` snapshot — the board
/// state right after the user's single move, with no opponent reply
/// bundled in. This is the honest "what changed on the board" signal.
/// Tactical questions ("does this move lead to a winning combination
/// 10 plies deep?") are answered by the search's *score* and PV, not
/// by per-term deltas at a leaf-trace — see [`super::MoveAnalysis`]'s
/// `score`, the [`super::MaterialOutcome`] PV walker, and the
/// surprise classifier ([`super::SurpriseKind`]).
///
/// Each term's tapered cp uses the phase + scale factor of `post`,
/// matching how the main evaluator would taper a swing in isolation.
///
/// Returns a `Vec<TermDelta>` of length [`TermId::ALL`], sorted by
/// `|delta_tapered|` descending. Ties preserve [`TermId::ALL`] order.
pub fn compute_term_deltas(pre: &EvalTrace, post: &EvalTrace) -> Vec<TermDelta> {
    let mut deltas: Vec<TermDelta> = TermId::ALL
        .iter()
        .map(|&term| {
            let pre_score = term.net_score(pre);
            let post_score = term.net_score(post);
            let diff = post_score - pre_score;
            TermDelta {
                term,
                delta_mg: diff.mg().0,
                delta_eg: diff.eg().0,
                delta_tapered: tapered_cp(diff, post.phase, post.scale_factor),
                piece_involved: None,
            }
        })
        .collect();

    // Stable descending sort by |delta_tapered| — ties keep the
    // TermId::ALL enumeration order.
    deltas.sort_by_key(|d| std::cmp::Reverse(d.delta_tapered.abs()));
    deltas
}

/// Apply the same taper + scale-factor formula
/// `eval::evaluate_inner` applies to the total score (see
/// `core/engine/src/eval/mod.rs`):
///
/// ```text
///     mg_part = delta.mg * phase
///     eg_part = delta.eg * (128 - phase) * scale_factor / 64
///     result  = (mg_part + eg_part) / 128
/// ```
///
/// All arithmetic is plain `i32`; no overflow risk at realistic phase
/// (0..=128) and scale-factor (0..=192) ranges because per-term
/// deltas are bounded well inside `i16`.
fn tapered_cp(delta: Score, phase: i32, scale_factor: i32) -> i32 {
    const PHASE_MAX: i32 = 128;
    const SCALE_NORMAL: i32 = 64;
    let mg_part = delta.mg().0 * phase;
    let eg_part = delta.eg().0 * (PHASE_MAX - phase) * scale_factor / SCALE_NORMAL;
    (mg_part + eg_part) / PHASE_MAX
}

/// Return the smallest prefix of `deltas` whose cumulative
/// `|delta_tapered|` meets or exceeds `percent` of the sum of all
/// absolute deltas. `percent` is in `[0.0, 100.0]`.
///
/// Reasoning: "top N terms" over-narrates one-term blunders (five
/// rows where one says everything) and under-narrates subtle
/// positional trades. A cumulative threshold naturally produces a
/// one-row result for a material swing and a four-or-five-row result
/// for a complex positional combination.
///
/// Edge cases:
/// - Empty `deltas` → empty slice.
/// - Every delta zero (total = 0) → empty slice.
/// - `percent <= 0` → empty slice (you asked for 0% coverage).
/// - `percent >= 100` → the whole slice.
///
/// The input must already be sorted by `|delta_tapered|`
/// descending — that's what [`compute_term_deltas`] produces.
pub fn cumulative_prefix(deltas: &[TermDelta], percent: f32) -> &[TermDelta] {
    if deltas.is_empty() || percent <= 0.0 {
        return &[];
    }
    let total: i64 = deltas
        .iter()
        .map(|d| d.delta_tapered.unsigned_abs() as i64)
        .sum();
    if total == 0 {
        return &[];
    }
    let target = (total as f64 * (percent.min(100.0) as f64) / 100.0).ceil() as i64;
    let mut running: i64 = 0;
    for (i, d) in deltas.iter().enumerate() {
        running += d.delta_tapered.unsigned_abs() as i64;
        if running >= target {
            return &deltas[..=i];
        }
    }
    // Running below target despite visiting everything (can only
    // happen with floating-point noise on percent≈100); return the
    // whole slice.
    deltas
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Color;

    // ---- tapered_cp matches the evaluator's formula -----------------

    #[test]
    fn tapered_cp_at_pure_middlegame_is_mg_component() {
        assert_eq!(tapered_cp(Score::new(100, -40), 128, 64), 100);
    }

    #[test]
    fn tapered_cp_at_pure_endgame_is_eg_component() {
        assert_eq!(tapered_cp(Score::new(100, -40), 0, 64), -40);
    }

    #[test]
    fn tapered_cp_scale_factor_halves_endgame() {
        assert_eq!(tapered_cp(Score::new(0, 80), 0, 32), 40);
    }

    #[test]
    fn tapered_cp_midgame_blend() {
        assert_eq!(tapered_cp(Score::new(40, 0), 64, 64), 20);
        assert_eq!(tapered_cp(Score::new(0, 40), 64, 64), 20);
        assert_eq!(tapered_cp(Score::new(20, 20), 64, 64), 20);
    }

    // ---- compute_term_deltas -----------------------------------------

    /// Helper: build an EvalTrace at a specific phase + scale_factor
    /// for taper testing.
    fn trace_at(phase: i32, scale: i32) -> EvalTrace {
        let mut t = EvalTrace::zero();
        t.phase = phase;
        t.scale_factor = scale;
        t
    }

    #[test]
    fn compute_term_deltas_returns_all_terms_and_is_sorted() {
        let pre = trace_at(128, 64);
        let mut post = trace_at(128, 64);

        post.mobility[Color::White.index()].knight = Score::new(200, 0);
        post.threats[Color::White.index()].hanging = Score::new(20, 0);

        let deltas = compute_term_deltas(&pre, &post);
        assert_eq!(deltas.len(), TermId::ALL.len());
        assert_eq!(deltas[0].term, TermId::MobilityKnight);
        assert_eq!(deltas[0].delta_mg, 200);
        assert_eq!(deltas[0].delta_tapered, 200);
        assert_eq!(deltas[1].term, TermId::ThreatsHanging);
        assert_eq!(deltas[1].delta_mg, 20);
    }

    #[test]
    fn compute_term_deltas_material_diffs_against_post_not_some_deeper_trace() {
        // Every term — Material included — reads from the same `post`
        // snapshot. The teaching surface uses the immediate ply-1
        // state; tactical projections deeper in the PV are surfaced
        // through `MoveAnalysis.score` / `MaterialOutcome` / the
        // surprise classifier, not through term deltas.
        let pre = trace_at(128, 64);
        let mut post = trace_at(128, 64);
        post.material.piece_value = Score::new(80, 80);

        let deltas = compute_term_deltas(&pre, &post);
        let mat = deltas
            .iter()
            .find(|d| d.term == TermId::MaterialPieceValue)
            .unwrap();
        assert_eq!(mat.delta_mg, 80);
    }

    #[test]
    fn compute_term_deltas_zero_trace_yields_all_zeros() {
        let pre = trace_at(128, 64);
        let post = trace_at(128, 64);
        let deltas = compute_term_deltas(&pre, &post);
        assert!(deltas.iter().all(|d| d.delta_tapered == 0));
        assert_eq!(deltas.len(), TermId::ALL.len());
    }

    #[test]
    fn compute_term_deltas_signs_flip_for_black_gain() {
        let pre = trace_at(128, 64);
        let mut post = trace_at(128, 64);
        post.pawns[Color::Black.index()].isolated = Score::new(-5, -15);
        let mut pre_with_pawn = pre;
        pre_with_pawn.pawns[Color::Black.index()].isolated = Score::new(-20, -60);
        let deltas = compute_term_deltas(&pre_with_pawn, &post);
        let d = deltas
            .iter()
            .find(|d| d.term == TermId::PawnsIsolated)
            .expect("PawnsIsolated missing");
        assert_eq!(d.delta_mg, -15);
    }

    // ---- cumulative_prefix -------------------------------------------

    fn make_delta(term: TermId, tapered: i32) -> TermDelta {
        TermDelta {
            term,
            delta_mg: tapered,
            delta_eg: tapered,
            delta_tapered: tapered,
            piece_involved: None,
        }
    }

    #[test]
    fn cumulative_prefix_one_dominant_term_returns_one() {
        let deltas = vec![
            make_delta(TermId::MaterialPieceValue, 90),
            make_delta(TermId::KingPawnShield, 5),
            make_delta(TermId::MobilityKnight, 5),
        ];
        let prefix = cumulative_prefix(&deltas, 75.0);
        assert_eq!(prefix.len(), 1);
        assert_eq!(prefix[0].term, TermId::MaterialPieceValue);
    }

    #[test]
    fn cumulative_prefix_distributed_terms_return_several() {
        let deltas = vec![
            make_delta(TermId::MaterialPieceValue, 30),
            make_delta(TermId::KingPawnShield, 25),
            make_delta(TermId::MobilityKnight, 20),
            make_delta(TermId::ThreatsHanging, 15),
            make_delta(TermId::Space, 10),
        ];
        let prefix = cumulative_prefix(&deltas, 75.0);
        assert_eq!(prefix.len(), 3);
    }

    #[test]
    fn cumulative_prefix_percent_at_extremes() {
        let deltas = vec![
            make_delta(TermId::MaterialPieceValue, 50),
            make_delta(TermId::MobilityKnight, 30),
            make_delta(TermId::KingPawnShield, 20),
        ];
        assert!(cumulative_prefix(&deltas, 0.0).is_empty());
        assert_eq!(cumulative_prefix(&deltas, 100.0).len(), 3);
    }

    #[test]
    fn cumulative_prefix_all_zero_deltas_returns_empty() {
        let deltas = vec![
            make_delta(TermId::MaterialPieceValue, 0),
            make_delta(TermId::MobilityKnight, 0),
        ];
        assert!(cumulative_prefix(&deltas, 75.0).is_empty());
    }

    #[test]
    fn cumulative_prefix_empty_input_returns_empty() {
        let deltas: Vec<TermDelta> = Vec::new();
        assert!(cumulative_prefix(&deltas, 75.0).is_empty());
    }

    #[test]
    fn cumulative_prefix_uses_absolute_value() {
        let deltas = vec![
            make_delta(TermId::MaterialPieceValue, -80),
            make_delta(TermId::MobilityKnight, 15),
            make_delta(TermId::KingPawnShield, 5),
        ];
        let prefix = cumulative_prefix(&deltas, 75.0);
        assert_eq!(prefix.len(), 1);
        assert_eq!(prefix[0].term, TermId::MaterialPieceValue);
    }
}
