//! Secondary terms ("Other shifts" / Helped / Hurt fallback) card builder.
//!
//! The prose (the helped/hurt lists) is produced by the shared teaching
//! translator ([`chess_tutor_teaching`]) from a [`Claim::Secondary`]; the
//! shared salience (the cumulative-coverage trim, the consumed-term skip,
//! the mover-POV sign flip) lives in [`secondary_claim`]. This builder
//! owns only the *structured* card surface the translator deliberately
//! doesn't carry — the sentiment and the net score chip.
//!
//! Split out of `retrospective_view`; the orchestrator
//! ([`super::build_retrospective_view`]) assembles the cards.

use chess_tutor_engine::analysis::{MoveAnalysis, TermId};
use chess_tutor_engine::types::Color;

use chess_tutor_teaching::claim::{secondary_claim, Claim, SECONDARY_DEFAULT_TOP_PERCENT};
use chess_tutor_teaching::phrasing::{phrase, Locale, Perspective, PhrasingContext, Verbosity};

use crate::view::{RetrospectiveCategory, RetrospectiveItem, Sentiment};

/// Build the "Other shifts" card for one analysed move. `perspective`
/// selects the student-POV sentiment colour (the term deltas are
/// mover-POV; under `Opponent` a mover-helping shift hurts the student,
/// so the sentiment + the signed chip flip — the translator's content
/// stays mover-POV by design, see `phrase_secondary`).
///
/// `show_all` bypasses the 50%-coverage trim so every residual term with
/// a non-zero delta appears as a row. The GUI's collapsible card keeps
/// the noise out of the way until the user expands.
pub(super) fn build_secondary_item(
    user: &MoveAnalysis,
    root_stm: Color,
    skip: &[TermId],
    show_all: bool,
    perspective: Perspective,
) -> Option<RetrospectiveItem> {
    let top_percent = if show_all { 100.0 } else { SECONDARY_DEFAULT_TOP_PERCENT };
    let claim = secondary_claim(user, root_stm, skip, top_percent)?;
    let ctx = PhrasingContext {
        perspective,
        locale: Locale::En,
        verbosity: Verbosity::Normal,
        reveal_moves: false,
    };
    Some(secondary_item(&claim, &ctx))
}

/// Turn one [`Claim::Secondary`] into a card — prose from the
/// translator, structured surface (sentiment, net score chip, terse
/// summary) computed here from the claim's mover-POV term deltas.
fn secondary_item(claim: &Claim, ctx: &PhrasingContext) -> RetrospectiveItem {
    let phrasing = phrase(claim, ctx);
    let Claim::Secondary { terms } = claim else {
        unreachable!("secondary_claim always returns Claim::Secondary");
    };

    // Deltas are mover-POV (positive = helped the mover). The helped/hurt
    // *counts* are mover-relative and stay so (matching the translator's
    // mover-POV content); but the sentiment colour and the signed chip read
    // from the *student's* side, so under the opponent perspective a
    // mover-helping net is a negative for the student — flip the sign.
    let helped = terms.iter().filter(|(_, cp)| *cp > 0).count();
    let hurt = terms.iter().filter(|(_, cp)| *cp < 0).count();
    let mover_net: i32 = terms.iter().map(|(_, cp)| *cp).sum();
    let student_net = match ctx.perspective {
        Perspective::Player => mover_net,
        Perspective::Opponent => -mover_net,
    };

    let sentiment = if student_net > 0 {
        Sentiment::Positive
    } else if student_net < 0 {
        Sentiment::Negative
    } else {
        Sentiment::Mixed
    };
    let summary = match (helped, hurt) {
        (h, 0) => format!("{h} helped"),
        (0, t) => format!("{t} hurt"),
        (h, t) => format!("{h} helped, {t} hurt"),
    };

    RetrospectiveItem {
        category: RetrospectiveCategory::Secondary,
        heading: "Other shifts".to_string(),
        summary,
        detail: phrasing.detail.unwrap_or_default(),
        score_delta_pawns: Some(student_net as f32 / 100.0),
        sentiment,
        annotations: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chess_tutor_engine::analysis::{analyze_position, MoveAnalysis};
    use chess_tutor_engine::engine::{Engine, SearchParams};
    use chess_tutor_engine::movegen::legal_moves_vec;
    use chess_tutor_engine::position::Position;
    use chess_tutor_engine::types::Square;

    fn analyses_for_e4() -> (Position, Vec<MoveAnalysis>, chess_tutor_engine::types::Move) {
        let mut pos = Position::startpos();
        let e4 = legal_moves_vec(&mut pos)
            .into_iter()
            .find(|m| m.from() == Square::E2 && m.to() == Square::E4)
            .unwrap();
        let mut engine = Engine::new(16);
        let analyses = analyze_position(
            &mut engine,
            &mut pos,
            SearchParams {
                max_depth: 6,
                multi_pv: 2,
                force_include: vec![e4],
                threads: 1,
                ..SearchParams::default()
            },
        );
        (pos, analyses, e4)
    }

    #[test]
    fn smoke_other_shifts_card_renders_via_translator() {
        let (_pos, analyses, e4) = analyses_for_e4();
        let user = analyses.iter().find(|a| a.mv == e4).unwrap();
        // With no terms skipped, some shift should survive the trim on
        // a real opening move.
        if let Some(card) = build_secondary_item(user, Color::White, &[], false, Perspective::Player) {
            assert_eq!(card.heading, "Other shifts");
            // Detail comes from the translator's helped/hurt lists.
            assert!(
                card.detail.contains("Also helped") || card.detail.contains("Also hurt"),
                "unexpected detail: {}",
                card.detail
            );
        }
    }
}
