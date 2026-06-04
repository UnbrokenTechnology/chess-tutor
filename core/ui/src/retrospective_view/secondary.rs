//! Secondary-term card builder — one card per residual eval term.
//!
//! Replaces the old single aggregated "Other shifts" card. That card was
//! uninformative both folded ("2 helped, 1 hurt" says nothing) and
//! expanded ("development +0.48, flank attacks +0.31, king safety −0.04"
//! demands you already know the terms). Each residual term now gets its
//! own labelled card with its own student-POV chip + sentiment, so the
//! feedback zone stays scannable.
//!
//! Salience: a term is promoted to a default card only when its student-
//! POV impact clears [`SECONDARY_PROMOTE_CP`]; smaller residual terms
//! surface only under the panel's "Show all signals" toggle (`show_all`).
//! The shared consumed-term skip + mover-POV sign flip still come from the
//! teaching crate's [`secondary_claim`] (called untrimmed); this builder
//! owns the per-term split, the student-POV re-sign, and the structured
//! card surface.
//!
//! Split out of `retrospective_view`; the orchestrator
//! ([`super::build_retrospective_view`]) assembles the cards.

use chess_tutor_engine::analysis::{MoveAnalysis, TermId};
use chess_tutor_engine::types::Color;

use chess_tutor_teaching::claim::{secondary_claim, Claim};
use chess_tutor_teaching::phrasing::Perspective;

use crate::view::{RetrospectiveCategory, RetrospectiveItem, Sentiment};

/// Minimum student-POV magnitude (canonical pawn=100 cp) for a residual
/// term to earn a default card. Below this the term is real but minor, so
/// it hides behind "Show all signals" — keeps a quiet move's card list
/// short while never losing a signal. 20 cp ≈ 0.20 pawns.
const SECONDARY_PROMOTE_CP: i32 = 20;

/// Build one card per residual eval-term shift for an analysed move. The
/// term deltas come from [`secondary_claim`] mover-POV (positive helped
/// the mover); under the `Opponent` perspective a mover-helping shift
/// hurts the student, so the chip + sentiment flip.
///
/// `show_all` drops the [`SECONDARY_PROMOTE_CP`] floor so every non-zero
/// residual term gets a card; otherwise only the meaningful movers do.
pub(super) fn build_secondary_items(
    user: &MoveAnalysis,
    root_stm: Color,
    skip: &[TermId],
    show_all: bool,
    perspective: Perspective,
) -> Vec<RetrospectiveItem> {
    // Untrimmed (100% coverage): we apply our own per-term magnitude gate
    // below rather than the claim's cumulative-coverage trim.
    let Some(Claim::Secondary { terms }) = secondary_claim(user, root_stm, skip, 100.0) else {
        return Vec::new();
    };

    // Re-sign to student POV, gate by magnitude, biggest-impact first.
    let mut rows: Vec<(TermId, i32)> = terms
        .into_iter()
        .map(|(term, mover_cp)| {
            let student_cp = match perspective {
                Perspective::Player => mover_cp,
                Perspective::Opponent => -mover_cp,
            };
            (term, student_cp)
        })
        .filter(|(_, cp)| show_all || cp.abs() >= SECONDARY_PROMOTE_CP)
        .collect();
    rows.sort_by_key(|(_, cp)| std::cmp::Reverse(cp.abs()));

    rows.into_iter().map(|(term, cp)| term_card(term, cp)).collect()
}

/// One residual-term card: the term's plain-English label, its student-POV
/// chip, and a sentiment colour. `student_cp` is guaranteed non-zero
/// (`secondary_claim` drops zero deltas), so the sign is unambiguous.
fn term_card(term: TermId, student_cp: i32) -> RetrospectiveItem {
    let pawns = student_cp as f32 / 100.0;
    let sentiment = if student_cp > 0 {
        Sentiment::Positive
    } else {
        Sentiment::Negative
    };
    RetrospectiveItem {
        category: RetrospectiveCategory::Secondary,
        heading: capitalize_first(term.pretty_label()),
        summary: format!("{pawns:+.2} pawns"),
        detail: String::new(),
        score_delta_pawns: Some(pawns),
        sentiment,
        annotations: Vec::new(),
    }
}

/// Capitalize the first character of a `pretty_label` (which is lowercase
/// — "development", "king safety") for a card heading.
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
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
    fn per_term_cards_replace_the_aggregated_other_shifts_card() {
        let (_pos, analyses, e4) = analyses_for_e4();
        let user = analyses.iter().find(|a| a.mv == e4).unwrap();
        // With no terms skipped, every residual term fires under show_all.
        let all = build_secondary_items(user, Color::White, &[], true, Perspective::Player);
        assert!(!all.is_empty(), "1.e4 should shift several residual terms");
        // No aggregated card survives; each card names a single term with a
        // capitalized heading and carries a chip.
        for card in &all {
            assert_ne!(card.heading, "Other shifts");
            assert!(
                card.heading.chars().next().is_some_and(|c| c.is_uppercase()),
                "heading should be capitalized: {:?}",
                card.heading
            );
            assert!(card.score_delta_pawns.is_some());
        }
        // The promote gate trims the default (non-show_all) list to the
        // meaningful movers — never more cards than show_all surfaces.
        let promoted = build_secondary_items(user, Color::White, &[], false, Perspective::Player);
        assert!(promoted.len() <= all.len());
    }
}
