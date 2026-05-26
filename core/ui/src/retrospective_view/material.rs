//! Material card builder.
//!
//! Split out of `retrospective_view`; the orchestrator
//! ([`super::build_retrospective_view`]) assembles the cards.

use chess_tutor_engine::analysis::MaterialOutcome;
use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::Color;

use crate::view::{
    AnnotationKind, BoardAnnotation, RetrospectiveCategory,
    RetrospectiveItem, Sentiment,
};

use super::helpers::*;

// ---------------------------------------------------------------------
// Material
// ---------------------------------------------------------------------

pub(super) fn build_material_item(
    _pre_move_pos: &Position,
    outcome: &MaterialOutcome,
    root_stm: Color,
) -> Option<RetrospectiveItem> {
    // Past tense ("You won material") only describes what actually
    // resolved in the position the student is looking at — the
    // user's move plus any forced opponent recapture. The
    // realized_events accessor enforces this; deeper PV captures
    // are reserved for hypothetical framings (CLI's "Best line:").
    let events: Vec<&_> = outcome.realized_events().collect();
    if events.is_empty() {
        return None;
    }
    // Suppress on hangs: when the user's ply-0 move was *not* a
    // capture and the opponent's ply-1 best response is to take one
    // of our pieces, that's a hanging piece — not a completed loss.
    // The threats card surfaces this case with proper present-tense
    // framing ("Your piece is hanging") plus attacker arrows and a
    // target-square highlight, and the opponent might still miss the
    // capture (a 1400 bot blunders these regularly). Calling it
    // "You lost material" frames the hang as a settled fact, which
    // confuses students about whether the loss has actually happened.
    let first_event_is_opponent_capture =
        events.first().is_some_and(|ev| ev.captor != root_stm);
    if first_event_is_opponent_capture {
        return None;
    }
    // Pedagogical "is this an even trade?" uses classical point values
    // (P:1, N:3, B:3, R:5, Q:9), not engine-internal cp. A B-for-N
    // swap (net -44 cp midgame) reads as Even to a student, and
    // surfacing it as "You lost material" mis-teaches. The cp net
    // still drives the numeric summary so the student can see the
    // exact engine valuation — we just don't let the cp gap pick the
    // headline.
    let net_points = realized_point_net(&events, root_stm);
    let net = outcome.realized_net_mg_cp(root_stm);
    let (heading, sentiment) = if net_points > 0 {
        ("You won material", Sentiment::Positive)
    } else if net_points < 0 {
        ("You lost material", Sentiment::Negative)
    } else {
        ("Even trade", Sentiment::Neutral)
    };

    let summary = if net_points == 0 {
        if net == 0 {
            format!("{} captures, balanced", events.len())
        } else {
            // Fair-point trade with a small cp lean — show the cp
            // delta in parens so the student can see how the engine
            // values the slight asymmetry (B vs N etc.) without it
            // re-headlining as a loss.
            format!(
                "{} captures, even by point value ({:+.2} engine cp)",
                events.len(),
                net as f32 / 100.0
            )
        }
    } else {
        format!(
            "net {:+} point{}, {:+.2} pawns engine cp",
            net_points,
            if net_points.abs() == 1 { "" } else { "s" },
            net as f32 / 100.0
        )
    };

    // Detail: list each capture step.
    let mut detail_lines: Vec<String> = Vec::new();
    for ev in &events {
        let captor_label = piece_name(ev.captor_piece);
        let captured_label = piece_name(ev.captured_piece);
        let sign = if ev.captor == root_stm {
            "you take"
        } else {
            "opponent takes"
        };
        detail_lines.push(format!(
            "Ply {}: {} a {} with {} on {}.",
            ev.ply + 1,
            sign,
            captured_label,
            article(captor_label),
            ev.square.to_algebraic()
        ));
    }
    // Phase-dependent teaching note. When point parity is even but
    // the engine's cp valuation leans meaningfully one way, that's a
    // learning opportunity: classical 3=3 hides that bishops favor
    // open positions / endgames, two minors usually beat a rook in
    // middlegame, etc. Surface the fact; let the student internalise
    // the why.
    if net_points == 0 {
        if let Some(note) = phase_dependent_trade_note(&events, root_stm) {
            detail_lines.push(note);
        }
    }
    let detail = detail_lines.join("\n");

    // Annotations: highlight every square where a capture resolved.
    // We don't have the PV here directly (the outcome doesn't expose
    // it), so from/to arrows would require a recomputation pass.
    // Square highlights are precise enough to point the student at
    // each capture without that work.
    let mut annotations = Vec::new();
    for ev in &events {
        let kind = if ev.captor == root_stm {
            AnnotationKind::Capture
        } else {
            AnnotationKind::Threat
        };
        annotations.push(BoardAnnotation::SquareHighlight {
            square: ev.square,
            kind,
        });
    }

    // Chip on the card: signed cp delta from White's POV so the
    // existing per-card delta chip math stays consistent. Only show
    // a chip when the point-value parity also says non-even — a
    // small cp lean on a fair-points trade isn't headline-worthy.
    let score_delta_pawns = if net_points != 0 {
        let sign = if root_stm == Color::White { 1 } else { -1 };
        Some((net * sign) as f32 / 100.0)
    } else {
        None
    };

    Some(RetrospectiveItem {
        category: RetrospectiveCategory::Material,
        heading: heading.to_string(),
        summary,
        detail,
        score_delta_pawns,
        sentiment,
        annotations,
    })
}

