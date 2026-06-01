//! Material card builder.
//!
//! Split out of `retrospective_view`; the orchestrator
//! ([`super::build_retrospective_view`]) assembles the cards.
//!
//! The prose (heading + summary + per-capture detail) is produced by the
//! shared teaching translator ([`chess_tutor_teaching`]) from a
//! [`Claim::Material`]; this builder owns only the *structured* card
//! surface the translator deliberately doesn't carry — sentiment, the
//! white-POV score chip, and the per-square board annotations.

use chess_tutor_engine::analysis::MaterialOutcome;
use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::Color;

use chess_tutor_teaching::claim::{material_claim_realized, Claim};
use chess_tutor_teaching::phrasing::{
    phrase, Locale, Perspective, PhrasingContext, Verbosity,
};

use crate::view::{
    AnnotationKind, BoardAnnotation, RetrospectiveCategory, RetrospectiveItem, Sentiment,
};

use super::helpers::*;

// ---------------------------------------------------------------------
// Material
// ---------------------------------------------------------------------

pub(super) fn build_material_item(
    _pre_move_pos: &Position,
    outcome: &MaterialOutcome,
    root_stm: Color,
    perspective: Perspective,
) -> Option<RetrospectiveItem> {
    // Salience + the realized-window selection (past-tense "you won
    // material" only describes the mover's own move + a forced opponent
    // recapture; a bare hang is the threats card's job) live in the
    // shared claim builder. `None` ⇒ nothing settled to narrate here.
    let claim = material_claim_realized(outcome, root_stm)?;
    let Claim::Material {
        events,
        net_points,
        net_mg_cp,
        ..
    } = &claim
    else {
        unreachable!("material_claim_realized always returns Claim::Material");
    };
    let net_points = *net_points;
    let net = *net_mg_cp;

    // Heading prose from the translator: perspective-correct, with the
    // directional reframe ("You won a bishop" / "They lost … — you win
    // material"). The numeric summary line, sentiment, chip, and per-event
    // detail below stay structured (no "you") — the translator
    // deliberately carries only the short headline.
    let ctx = PhrasingContext {
        perspective,
        locale: Locale::En,
        verbosity: Verbosity::Normal,
        reveal_moves: false,
    };
    let phrasing = phrase(&claim, &ctx);

    let sentiment = if net_points > 0 {
        Sentiment::Positive
    } else if net_points < 0 {
        Sentiment::Negative
    } else {
        Sentiment::Neutral
    };

    // Numeric summary line (structured, no "you"): the exact engine cp
    // valuation so the student can see the slight B-vs-N / phase asymmetry
    // the classical point count hides.
    let summary = if net_points == 0 {
        if net == 0 {
            format!("{} captures, balanced", events.len())
        } else {
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

    // Detail: the per-capture step list plus the phase-dependent trade
    // note (the structured per-event breakdown the translator's short
    // summary doesn't carry).
    let mut detail_lines: Vec<String> = Vec::new();
    for ev in events {
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
    // Phase-dependent teaching note. When point parity is even but the
    // engine's cp valuation leans meaningfully one way, that's a learning
    // opportunity: classical 3=3 hides that bishops favor open positions /
    // endgames, two minors usually beat a rook in middlegame, etc. Surface
    // the fact; let the student internalise the why.
    if net_points == 0 {
        let ev_refs: Vec<&_> = events.iter().collect();
        if let Some(note) = phase_dependent_trade_note(&ev_refs, root_stm) {
            detail_lines.push(note);
        }
    }
    let detail = detail_lines.join("\n");

    // Annotations: highlight every square where a capture resolved. We
    // don't have the PV here directly (the outcome doesn't expose it), so
    // from/to arrows would require a recomputation pass. Square highlights
    // are precise enough to point the student at each capture without that
    // work.
    let mut annotations = Vec::new();
    for ev in events {
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

    // Chip on the card: signed cp delta from White's POV so the existing
    // per-card delta chip math stays consistent. Only show a chip when the
    // point-value parity also says non-even — a small cp lean on a fair-
    // points trade isn't headline-worthy.
    let score_delta_pawns = if net_points != 0 {
        let sign = if root_stm == Color::White { 1 } else { -1 };
        Some((net * sign) as f32 / 100.0)
    } else {
        None
    };

    Some(RetrospectiveItem {
        category: RetrospectiveCategory::Material,
        heading: phrasing.summary,
        summary,
        detail,
        score_delta_pawns,
        sentiment,
        annotations,
    })
}
