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

use chess_tutor_teaching::claim::{verdict_claim, Claim};
use chess_tutor_teaching::phrasing::{
    phrase, Locale, Perspective, PhrasingContext, Verbosity,
};

use crate::view::{
    AnnotationKind, BoardAnnotation, RetrospectiveHeadline,
};

use super::helpers::*;

// ---------------------------------------------------------------------
// Headline
// ---------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub(super) fn build_headline(
    pre_move_pos: &Position,
    analyses: &[MoveAnalysis],
    best: &MoveAnalysis,
    user: &MoveAnalysis,
    verdict: MoveVerdict,
    root_stm: Color,
    perspective: Perspective,
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
    let verdict_sentiment = verdict_sentiment(verdict);
    let user_score = format_score_pawns(user.score);

    // Verdict label + verdict-specific note come from the teaching
    // translator: it owns the chess.com tier remap ("Great" / "Brilliant"
    // for an only-good-move sacrifice) and the perspective-correct
    // sentence. `reveal_moves: false` here because the GUI carries the
    // engine-preferred move in its own dedicated fields below (with a
    // board arrow), not in the note text.
    let claim = verdict_claim(pre_move_pos, analyses, best, user, verdict, false);
    let ctx = PhrasingContext {
        perspective,
        locale: Locale::En,
        verbosity: Verbosity::Normal,
        reveal_moves: false,
    };
    // The GUI composes its own headline layout (SAN + label + score on
    // separate widgets), so it reuses only the verdict *word* and the
    // verdict-specific *note* from the translator, not the full sentence.
    let phrasing = phrase(&claim, &ctx);
    let verdict_label = verdict_tier_label_of(&claim);

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

    // Note precedence: the translator's verdict-specific note (lost
    // position / missed material) wins; otherwise the shallow-vs-deep
    // surprise note (a separate, surprise-driven concern the verdict
    // claim doesn't carry) fills in — including the positive-surprise
    // "well spotted" case that drives `user_is_sharp` above. Both come
    // from the shared translator now (`surprise_claim` + `phrase`).
    let note = phrasing
        .detail
        .or_else(|| surprise_note(verdict, user.surprise(root_stm), perspective, root_stm));

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

/// Pull the chess.com tier label out of an already-built verdict claim.
fn verdict_tier_label_of(claim: &Claim) -> String {
    match claim {
        Claim::Verdict {
            verdict,
            only_good_move,
            sacrifice,
            ..
        } => chess_tutor_teaching::phrasing::verdict_tier_label(
            *verdict,
            *only_good_move,
            *sacrifice,
        )
        .to_string(),
        _ => unreachable!("verdict_claim always returns Claim::Verdict"),
    }
}

