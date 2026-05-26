//! Headline card builder.
//!
//! Split out of `retrospective_view`; the orchestrator
//! ([`super::build_retrospective_view`]) assembles the cards.

use chess_tutor_engine::analysis::{
    MoveAnalysis, MoveVerdict, SurpriseKind,
};
use chess_tutor_engine::position::Position;
use chess_tutor_engine::san;
use chess_tutor_engine::types::Color;

use crate::view::{
    AnnotationKind, BoardAnnotation, RetrospectiveHeadline,
};

use super::helpers::*;

// ---------------------------------------------------------------------
// Headline
// ---------------------------------------------------------------------

pub(super) fn build_headline(
    pre_move_pos: &Position,
    best: &MoveAnalysis,
    user: &MoveAnalysis,
    verdict: MoveVerdict,
    root_stm: Color,
    reveal_best_moves: bool,
) -> RetrospectiveHeadline {
    let user_san = san::format(pre_move_pos, user.mv);
    let user_is_sharp = matches!(
        (verdict, user.surprise(root_stm)),
        (
            MoveVerdict::Best | MoveVerdict::Good,
            Some(SurpriseKind::LooksBadButGood)
        )
    );
    let san_annotation = sharp_or_verdict_annotation(verdict, user_is_sharp);
    let verdict_label = verdict_label(verdict);
    let verdict_sentiment = verdict_sentiment(verdict);
    let user_score = format_score_pawns(user.score);

    // Best-move reveal is opt-in. When off, the four "what the engine
    // would have played" fields stay `None` so renderers naturally
    // skip them — telling the student the answer trains memorisation,
    // not the understanding the per-category cards below are designed
    // to build.
    let mut best_san = None;
    let mut best_score = None;
    let mut gap = None;
    let mut best_move_annotation = None;
    if reveal_best_moves && best.mv != user.mv {
        let san = san::format(pre_move_pos, best.mv);
        best_score = Some(format_score_pawns(best.score));
        gap = Some(format_delta_pawns(user.score.0 - best.score.0));
        best_move_annotation = Some(BoardAnnotation::Arrow {
            from: best.mv.from(),
            to: best.mv.to(),
            kind: AnnotationKind::BestMove,
        });
        best_san = Some(san);
    }

    let note = match verdict {
        MoveVerdict::BestAvailable => Some(format!(
            "Position was already lost ({}).",
            format_score_pawns(best.score)
        )),
        _ if user_is_sharp => Some(
            "Well spotted — this looks risky at first glance, but the longer line pays off."
                .to_string(),
        ),
        _ => surprise_note(verdict, user.surprise(root_stm)),
    };

    RetrospectiveHeadline {
        user_san,
        san_annotation,
        verdict_label,
        verdict_sentiment,
        user_score,
        best_san,
        best_score,
        gap,
        note,
        best_move_annotation,
    }
}

