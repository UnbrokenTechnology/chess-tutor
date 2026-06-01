//! Static-vs-search override note (PLAN §4.2 — "the hard one").
//!
//! The prose (heading + summary + detail, with the "you" / "they"
//! reframe) is produced by the shared teaching translator
//! ([`chess_tutor_teaching`]) from a [`Claim::OverrideNote`]; the shared
//! salience (the static-vs-search direction comparison, the per-axis
//! margin gate) lives in [`override_note_claim`]. This builder owns only
//! the *structured* card surface the translator deliberately doesn't
//! carry — the category and sentiment.
//!
//! Split out of `retrospective_view`; assembled by
//! [`super::build_retrospective_view`].
//!
//! Some positions are exactly where the per-term ledger *lies*: the
//! recommended move is a **static downgrade** yet a **search upgrade**,
//! and only search rescues the ranking. When this happens the narration
//! must **say so out loud** and never invent a positional justification.

use chess_tutor_engine::analysis::MoveAnalysis;
use chess_tutor_engine::types::Color;

use chess_tutor_teaching::claim::{override_note_claim, Claim};
use chess_tutor_teaching::phrasing::{phrase, Locale, Perspective, PhrasingContext, Verbosity};

use crate::view::{RetrospectiveCategory, RetrospectiveItem, Sentiment};

/// Build the static-vs-search override note, or `None` when the term
/// ledger and the search agree (the common case — no note).
///
/// `perspective` selects "you" vs "they" in the translator's prose. The
/// card stays Neutral (informational) regardless of mover.
pub(super) fn build_override_note_item(
    best: &MoveAnalysis,
    user: &MoveAnalysis,
    root_stm: Color,
    perspective: Perspective,
) -> Option<RetrospectiveItem> {
    let claim = override_note_claim(best, user, root_stm)?;
    let ctx = PhrasingContext {
        perspective,
        locale: Locale::En,
        verbosity: Verbosity::Normal,
        reveal_moves: false,
    };
    let phrasing = phrase(&claim, &ctx);
    let Claim::OverrideNote { .. } = &claim else {
        unreachable!("override_note_claim always returns Claim::OverrideNote");
    };
    Some(RetrospectiveItem {
        category: RetrospectiveCategory::Secondary,
        heading: "The term breakdown is misleading here".to_string(),
        summary: phrasing.summary,
        detail: phrasing.detail.unwrap_or_default(),
        score_delta_pawns: None,
        sentiment: Sentiment::Neutral,
        annotations: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chess_tutor_engine::analysis::analyze_position;
    use chess_tutor_engine::engine::{Engine, SearchParams};
    use chess_tutor_engine::position::Position;
    use chess_tutor_engine::san;
    use chess_tutor_engine::types::Move;

    /// The positional-punish-after-qe6 FEN. `Ne3` (search +1.7) is a
    /// ~1.9-pawn static downgrade vs `O-O` (static +1.96). The note must
    /// fire and must never call Ne3 "positionally strong".
    const POSITIONAL_PUNISH_FEN: &str =
        "r1b1kb1r/1p3ppp/p2pqn2/4pNB1/4P3/2N5/PPP2PPP/R2QK2R w KQkq - 0 1";

    fn analyses_for(fen: &str, user_san: &str) -> (Vec<MoveAnalysis>, Move) {
        let mut pos = Position::from_fen(fen).unwrap();
        let user_mv = san::parse(&mut pos, user_san).unwrap();
        let mut pos = Position::from_fen(fen).unwrap();
        let legal = chess_tutor_engine::movegen::legal_moves_vec(&mut pos.clone());
        let mut engine = Engine::new(16);
        let params = SearchParams {
            max_depth: 12,
            multi_pv: legal.len(),
            force_include: vec![user_mv],
            threads: 1,
            ..SearchParams::default()
        };
        (analyze_position(&mut engine, &mut pos, params), user_mv)
    }

    #[test]
    fn fires_on_positional_punish_and_never_calls_recommended_move_strong() {
        let (analyses, user_mv) = analyses_for(POSITIONAL_PUNISH_FEN, "O-O");
        let best = &analyses[0];
        let user = analyses.iter().find(|a| a.mv == user_mv).unwrap();
        let item = build_override_note_item(best, user, Color::White, Perspective::Player)
            .expect("static ledger lies here — the override note must fire");
        let blob = format!("{} {} {}", item.heading, item.summary, item.detail);
        assert!(
            !blob.to_lowercase().contains("positionally strong"),
            "must never call the recommended move positionally strong: {blob}"
        );
        assert!(blob.contains("search overrules") || blob.contains("trust the search"));
    }

    #[test]
    fn silent_when_static_and_search_agree() {
        let mut pos = Position::startpos();
        let mut engine = Engine::new(16);
        let analyses = analyze_position(
            &mut engine,
            &mut pos,
            SearchParams {
                max_depth: 6,
                multi_pv: 4,
                threads: 1,
                ..SearchParams::default()
            },
        );
        let best = &analyses[0];
        if let Some(other) = analyses.iter().find(|a| a.mv != best.mv) {
            assert!(
                build_override_note_item(best, other, Color::White, Perspective::Player).is_none(),
                "no static-vs-search lie in the opening — note must stay silent"
            );
        }
    }
}
