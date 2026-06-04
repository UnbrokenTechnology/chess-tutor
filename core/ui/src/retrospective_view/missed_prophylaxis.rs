//! Missed-prophylaxis card builder.
//!
//! When the user's move allows a deep punishing line that the engine's
//! best move would have **prevented**, this card surfaces *what they
//! needed to stop and why* — "you needed `Ra8` to stop `Rxe7+`; otherwise
//! king safety collapses." It reframes the flat "ALLOWED, NOT MISSED"
//! detection into a teachable lesson, and **supersedes** a bare
//! ALLOWED-reframe tactic card when prophylaxis is confirmed (the two
//! would otherwise narrate the same swing twice).
//!
//! The detection gate (the explosion scan, the best-line-holds check, and
//! the replay/disambiguation test that distinguishes prophylaxis from a
//! *deferred own-tactic*) lives in [`missed_prophylaxis_claim`]; this
//! builder owns only the structured card surface — sentiment and the
//! board annotations (the punisher's trigger arrow plus, when king safety
//! collapses, the mover's king ring).
//!
//! Split out of `retrospective_view`; the orchestrator
//! ([`super::build_retrospective_view`]) assembles the cards.

use chess_tutor_engine::analysis::{MoveAnalysis, TermId};
use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::{Color, File, Rank, Square};

use chess_tutor_teaching::claim::{
    missed_prophylaxis_claim, prophylaxis_punisher_move, Claim,
};
use chess_tutor_teaching::phrasing::{phrase, Locale, Perspective, PhrasingContext, Verbosity};

use crate::view::{
    AnnotationKind, BoardAnnotation, RetrospectiveCategory, RetrospectiveItem, Sentiment,
};

/// Build the missed-prophylaxis card for the user's move, or `None` when
/// the gate doesn't hold. `perspective` selects "you" vs "they": under
/// `Player` the sub-optimal move is the student's, so the card is a
/// warning (`Negative`); under `Opponent` the opponent left the defence
/// off, so the punisher is *the student's* chance (`Positive`).
///
/// Annotations paint the punisher's trigger arrow (the opponent's reply
/// the user needed to stop) and, when the exploded term is king safety,
/// the mover's king ring — the "your king collapses here" receipt.
///
/// `reveal_best_moves` gates whether the prophylactic move is named in the
/// prose (threaded into the claim builder, same posture as the headline).
pub(super) fn build_missed_prophylaxis_item(
    pre_move_pos: &Position,
    best: &MoveAnalysis,
    user: &MoveAnalysis,
    root_stm: Color,
    reveal_best_moves: bool,
    perspective: Perspective,
) -> Option<RetrospectiveItem> {
    let claim =
        missed_prophylaxis_claim(pre_move_pos, best, user, root_stm, reveal_best_moves)?;
    let ctx = PhrasingContext {
        perspective,
        locale: Locale::En,
        verbosity: Verbosity::Normal,
        reveal_moves: reveal_best_moves,
    };
    let phrasing = phrase(&claim, &ctx);

    let Claim::MissedProphylaxis {
        punisher_san,
        exploded_term,
        ..
    } = &claim
    else {
        unreachable!("missed_prophylaxis_claim always returns Claim::MissedProphylaxis");
    };

    // A missed defence hurts the student when the student moved; under the
    // opponent perspective the opponent skipped it, so the punisher is the
    // student's opportunity — flip the sentiment.
    let sentiment = match perspective {
        Perspective::Player => Sentiment::Negative,
        Perspective::Opponent => Sentiment::Positive,
    };

    let annotations = annotations_for(pre_move_pos, user, root_stm, *exploded_term);

    Some(RetrospectiveItem {
        category: RetrospectiveCategory::MissedProphylaxis,
        heading: phrasing.summary,
        summary: format!("needed to stop {punisher_san}"),
        detail: phrasing.detail.unwrap_or_default(),
        score_delta_pawns: None,
        sentiment,
        annotations,
    })
}

/// Board annotations for the lesson: the punisher's trigger arrow (the
/// opponent's reply, drawn from its origin to its target — a *future* move
/// not yet on the displayed board, so the [`AnnotationKind::TriggerMove`]
/// hue, same as the walked-into-tactic card), plus the mover's king ring
/// when king safety is the term that collapses.
fn annotations_for(
    pre_move_pos: &Position,
    user: &MoveAnalysis,
    root_stm: Color,
    exploded_term: TermId,
) -> Vec<BoardAnnotation> {
    let mut anns = Vec::new();

    // Trigger arrow for the punisher — recover the move from the same
    // explosion scan the claim gated on, so the arrow and the prose name
    // the same move.
    if let Some((_, punisher)) = prophylaxis_punisher_move(pre_move_pos, user, root_stm) {
        if punisher.from() != punisher.to() {
            anns.push(BoardAnnotation::Arrow {
                from: punisher.from(),
                to: punisher.to(),
                kind: AnnotationKind::TriggerMove,
            });
        }
    }

    // King ring when king safety collapses — the term is signed mover-POV,
    // so a king-danger / flank-attack worsening is the *mover's* king under
    // fire. Paint that king and its adjacent ring.
    if matches!(exploded_term, TermId::KingDanger | TermId::KingFlankAttacks) {
        let king_sq = pre_move_pos.king_square(root_stm);
        anns.push(BoardAnnotation::SquareHighlight {
            square: king_sq,
            kind: AnnotationKind::KingRing,
        });
        for sq in king_ring_squares(king_sq) {
            anns.push(BoardAnnotation::SquareHighlight {
                square: sq,
                kind: AnnotationKind::KingRing,
            });
        }
    }

    anns
}

/// The up-to-8 squares adjacent to `king_sq` — the king's ring, excluding
/// the king square itself. Computed by file/rank arithmetic so it carries
/// no dependency on the engine's king-safety internals (mirrors the
/// positional-win builder's helper).
fn king_ring_squares(king_sq: Square) -> Vec<Square> {
    let f = king_sq.file().index() as i32;
    let r = king_sq.rank().index() as i32;
    let mut out = Vec::with_capacity(8);
    for df in -1..=1 {
        for dr in -1..=1 {
            if df == 0 && dr == 0 {
                continue;
            }
            let (nf, nr) = (f + df, r + dr);
            if (0..8).contains(&nf) && (0..8).contains(&nr) {
                if let (Some(file), Some(rank)) =
                    (File::from_index(nf as u8), Rank::from_index(nr as u8))
                {
                    out.push(Square::new(file, rank));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chess_tutor_engine::engine::{Engine, SearchParams};
    use chess_tutor_engine::san;

    /// The case-study transition: after `Rhg1`, Black to move. The user
    /// plays `…Bb5` (removing the lone `…Be6` interposition that refutes
    /// the rook sac), allowing `Rxe7+`. The card must fire — naming `Ra8`
    /// as the prophylaxis (with reveal on) and `Rxe7+` as the punisher.
    #[test]
    fn case_study_after_rhg1_bb5_fires_naming_punisher() {
        let fen = "4kb1Q/rp1bpp2/p2p2p1/8/8/3P4/PP3K1P/4R1R1 b - - 0 1";
        let mut pos = Position::from_fen(fen).unwrap();
        let root_stm = pos.side_to_move();
        let mut pre = Position::from_fen(fen).unwrap();
        let bb5 = san::parse(&mut pre, "Bb5").unwrap();
        let pre = Position::from_fen(fen).unwrap();

        let mut engine = Engine::default();
        let analyses = chess_tutor_engine::analysis::analyze_position(
            &mut engine,
            &mut pos,
            SearchParams {
                max_depth: 12,
                multi_pv: 2,
                force_include: vec![bb5],
                ..SearchParams::default()
            },
        );
        let best = &analyses[0];
        let user = analyses
            .iter()
            .find(|a| a.mv == bb5)
            .expect("force-included Bb5 must be present");
        // Only run the assertion when Bb5 was genuinely sub-optimal at this
        // depth (the case study's premise); if the engine happens to pick
        // Bb5 the structural gate correctly declines and there's nothing to
        // teach.
        if best.mv == bb5 {
            return;
        }

        let item = build_missed_prophylaxis_item(
            &pre,
            best,
            user,
            root_stm,
            true,
            Perspective::Player,
        )
        .expect("…Bb5 allowing Rxe7+ must fire the missed-prophylaxis card");

        assert_eq!(item.category, RetrospectiveCategory::MissedProphylaxis);
        assert_eq!(item.sentiment, Sentiment::Negative);
        // The punisher is named in heading + summary.
        assert!(
            item.heading.contains("Rxe7") || item.summary.contains("Rxe7"),
            "expected the punisher Rxe7+ named, got heading {:?} / summary {:?}",
            item.heading,
            item.summary
        );
        // With reveal on, the prophylactic move Ra8 is named.
        assert!(
            item.heading.contains("Ra8"),
            "expected Ra8 named as the prophylaxis, got {:?}",
            item.heading
        );
        // A trigger arrow for the punisher is painted.
        assert!(
            item.annotations.iter().any(|a| matches!(
                a,
                BoardAnnotation::Arrow {
                    kind: AnnotationKind::TriggerMove,
                    ..
                }
            )),
            "expected a punisher trigger arrow, got {:?}",
            item.annotations
        );
    }

    /// Control: a deferred own-tactic must NOT fire the prophylaxis card.
    /// Position 2 (after `…Bb5`), White to move — the engine's best move
    /// `Rxe7+` is a *deferred own-tactic* (its value lands deep in its own
    /// PV), not prophylaxis. The user playing a quiet alternative may give
    /// away the advantage, but the replay test rejects it: after the best
    /// move, the "punisher" (which is the user's own missed attack) is not
    /// an opponent resource that the best move removed. The card stays
    /// silent (no prophylaxis to teach — Feature 1's framing owns this).
    #[test]
    fn deferred_own_tactic_does_not_fire() {
        let fen = "4kb1Q/rp2pp2/p2p2p1/1b6/8/3P4/PP3K1P/4R1R1 w - - 0 1";
        let mut pos = Position::from_fen(fen).unwrap();
        let root_stm = pos.side_to_move();
        // A quiet, sub-optimal user move (e.g. a3) — the engine prefers the
        // sac Rxe7+. There is no opponent punisher the sac would remove, so
        // the prophylaxis gate must decline.
        let mut pre = Position::from_fen(fen).unwrap();
        let a3 = san::parse(&mut pre, "a3").unwrap();
        let pre = Position::from_fen(fen).unwrap();

        let mut engine = Engine::default();
        let analyses = chess_tutor_engine::analysis::analyze_position(
            &mut engine,
            &mut pos,
            SearchParams {
                max_depth: 12,
                multi_pv: 2,
                force_include: vec![a3],
                ..SearchParams::default()
            },
        );
        let best = &analyses[0];
        let user = analyses.iter().find(|a| a.mv == a3).expect("a3 present");
        assert!(
            build_missed_prophylaxis_item(
                &pre,
                best,
                user,
                root_stm,
                true,
                Perspective::Player,
            )
            .is_none(),
            "a deferred own-tactic must not misfire the prophylaxis card"
        );
    }
}
