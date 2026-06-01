//! Silent-sequencing depth-honesty note (PLAN §4.3).
//!
//! The prose (summary + detail, with the "you" / "they" reframe) is
//! produced by the shared teaching translator ([`chess_tutor_teaching`])
//! from a [`Claim::DepthHonesty`]; the shared salience (the
//! silent-sequencing gate via the engine
//! [`chess_tutor_engine::analysis::is_silent_sequencing`]) lives in
//! [`depth_honesty_claim`]. This builder owns only the *structured* card
//! surface the translator deliberately doesn't carry — the category, the
//! fixed heading, and the (neutral) sentiment.
//!
//! Split out of `retrospective_view`; assembled by
//! [`super::build_retrospective_view`].
//!
//! When a move the engine hates qualifies as **silent sequencing** — the
//! gap is invisible at human depth, large at full depth, and no detector
//! fires — the retrospective must be **honest about its own limits**: no
//! "blunder" stamp, no fabricated mechanism. Lying about a mechanism is
//! worse than admitting we can't explain it.

use chess_tutor_engine::analysis::{MoveAnalysis, PriorMove};
use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::Color;

use chess_tutor_teaching::claim::depth_honesty_claim;
use chess_tutor_teaching::phrasing::{phrase, Locale, Perspective, PhrasingContext, Verbosity};

use crate::view::{RetrospectiveCategory, RetrospectiveItem, Sentiment};

/// Build the depth-honesty note, or `None` when the move isn't silent
/// sequencing (the overwhelmingly common case).
///
/// `best` / `user` are the engine's preferred move and the user's move
/// from the same root; `pre_move_pos` is the position they were played
/// from; `prior_move` feeds the detector chain's recapture guard.
/// `root_stm` is the side that moved.
///
/// `perspective` selects "you" vs "they" in the translator's prose.
pub(super) fn build_depth_honesty_item(
    pre_move_pos: &Position,
    best: &MoveAnalysis,
    user: &MoveAnalysis,
    root_stm: Color,
    prior_move: Option<PriorMove>,
    perspective: Perspective,
) -> Option<RetrospectiveItem> {
    let claim = depth_honesty_claim(pre_move_pos, best, user, root_stm, prior_move)?;
    let ctx = PhrasingContext {
        perspective,
        locale: Locale::En,
        verbosity: Verbosity::Normal,
        reveal_moves: false,
    };
    let phrasing = phrase(&claim, &ctx);
    Some(RetrospectiveItem {
        category: RetrospectiveCategory::Secondary,
        // Fixed heading (the renderer / tests key on it); the translator's
        // summary carries the full perspective-correct sentence.
        heading: "No shorter lesson here".to_string(),
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
    use chess_tutor_engine::san;
    use chess_tutor_engine::types::Move;

    const QC8_FEN: &str = "1r1q2nr/p3k3/2Bbbpp1/7p/2Q5/8/PPPP1PPP/R1B1R1K1 b - - 0 1";

    fn analyses_for(fen: &str, user_san: &str) -> (Position, Vec<MoveAnalysis>, Move) {
        let mut pos = Position::from_fen(fen).unwrap();
        let user_mv = san::parse(&mut pos, user_san).unwrap();
        let mut pos = Position::from_fen(fen).unwrap();
        let legal = chess_tutor_engine::movegen::legal_moves_vec(&mut pos.clone());
        let mut engine = Engine::new(16);
        let params = SearchParams {
            max_depth: 14,
            multi_pv: legal.len(),
            force_include: vec![user_mv],
            threads: 1,
            ..SearchParams::default()
        };
        let analyses = analyze_position(&mut engine, &mut pos, params);
        (Position::from_fen(fen).unwrap(), analyses, user_mv)
    }

    #[test]
    fn fires_on_qc8_with_no_blunder_stamp_or_fake_mechanism() {
        let (pre, analyses, user_mv) = analyses_for(QC8_FEN, "Qc8");
        let best = &analyses[0];
        let user = analyses.iter().find(|a| a.mv == user_mv).unwrap();
        let item = build_depth_honesty_item(&pre, best, user, Color::Black, None, Perspective::Player)
            .expect("…Qc8 must produce a depth-honesty note");
        let blob = format!("{} {} {}", item.heading, item.summary, item.detail).to_lowercase();
        assert!(!blob.contains("blunder"), "must not stamp blunder: {blob}");
        assert!(!blob.contains("you walked into"), "no fake walked-into: {blob}");
        assert!(blob.contains("calculation depth"));
    }

    #[test]
    fn silent_when_user_played_best() {
        let (pre, analyses, _) = analyses_for(QC8_FEN, "Be5");
        let best = &analyses[0];
        assert!(
            build_depth_honesty_item(&pre, best, best, Color::Black, None, Perspective::Player)
                .is_none()
        );
    }
}

#[cfg(test)]
mod integration_check {
    use super::super::build_retrospective_view;
    use chess_tutor_engine::analysis::analyze_position;
    use chess_tutor_engine::engine::{Engine, SearchParams};
    use chess_tutor_engine::position::Position;
    use chess_tutor_engine::san;

    #[test]
    fn qc8_full_view_has_depth_honesty_and_no_blunder_or_override() {
        let fen = "1r1q2nr/p3k3/2Bbbpp1/7p/2Q5/8/PPPP1PPP/R1B1R1K1 b - - 0 1";
        let mut pos = Position::from_fen(fen).unwrap();
        let qc8 = san::parse(&mut pos, "Qc8").unwrap();
        let mut pos = Position::from_fen(fen).unwrap();
        let legal = chess_tutor_engine::movegen::legal_moves_vec(&mut pos.clone());
        let mut engine = Engine::new(16);
        let analyses = analyze_position(&mut engine, &mut pos, SearchParams {
            max_depth: 14, multi_pv: legal.len(), force_include: vec![qc8], threads: 1,
            ..SearchParams::default()
        });
        let pre = Position::from_fen(fen).unwrap();
        let vm = build_retrospective_view(
            &pre,
            &analyses,
            qc8,
            false,
            false,
            None,
            chess_tutor_teaching::phrasing::Perspective::Player,
        );
        let has_depth_honesty = vm.items.iter().any(|i| i.heading == "No shorter lesson here");
        assert!(has_depth_honesty, "qc8 view must carry the depth-honesty note");
        for it in &vm.items {
            let blob = format!("{} {}", it.heading, it.detail).to_lowercase();
            assert!(!blob.contains("you walked into"), "no walked-into card on qc8: {}", it.heading);
        }
    }
}
