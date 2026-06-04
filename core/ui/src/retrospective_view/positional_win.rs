//! Positional-win card builder — the sound-sacrifice justification.
//!
//! When the engine's best move ends **down material** yet the search
//! rates it at least equal, this card explains the *positional*
//! compensation in static terms: "you give up R for P, but a bare king
//! under your Q+R (king danger swings hard) makes it worth it." It fills
//! the gap where a material-losing best move would otherwise render only
//! as a misleading "you lost a point" Material card.
//!
//! The detection gate and the compensating-term computation (diff the
//! baseline trace vs the forcing-tail climax over every non-`Material*`
//! term, taper at the climax phase, take the dominant mover-favourable
//! swing) live in [`positional_win_claim`]; this builder owns only the
//! *structured* card surface — sentiment and the board annotations that
//! paint the king-danger / trapped-rook story.
//!
//! Split out of `retrospective_view`; the orchestrator
//! ([`super::build_retrospective_view`]) assembles the cards.

use chess_tutor_engine::analysis::{trapped_cages, MoveAnalysis, TermId};
use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::{Color, File, PieceType, Rank, Square};

use chess_tutor_teaching::claim::{forcing_tail_climax, positional_win_claim, Claim};
use chess_tutor_teaching::phrasing::{phrase, Locale, Perspective, PhrasingContext, Verbosity};

use crate::view::{
    AnnotationKind, BoardAnnotation, RetrospectiveCategory, RetrospectiveItem, Sentiment,
};

/// Build the positional-win card for the engine's best move, or `None`
/// when the sound-sacrifice gate doesn't hold. `perspective` selects
/// "you" vs "they"; the sacrifice is always a *strength* of the moving
/// side, so the card is `Positive` from the player's POV when the player
/// moved and `Negative` when the opponent did.
///
/// When the dominant compensating term is king danger, the card paints
/// the *climax* position's enemy king ring plus an arrow from each
/// attacker bearing on it — the "bare king under your heavy pieces"
/// receipt. When it's a trapped rook, the frozen rook's square is
/// highlighted. The climax position is reached by walking the best PV
/// through its settled ply.
pub(super) fn build_positional_win_item(
    best: &MoveAnalysis,
    pre_move_pos: &Position,
    root_stm: Color,
    perspective: Perspective,
) -> Option<(RetrospectiveItem, TermId)> {
    let claim = positional_win_claim(best, pre_move_pos, root_stm)?;
    let ctx = PhrasingContext {
        perspective,
        locale: Locale::En,
        verbosity: Verbosity::Normal,
        reveal_moves: false,
    };
    let phrasing = phrase(&claim, &ctx);

    let Claim::PositionalWin {
        sacrificed_points,
        dominant_term,
        ..
    } = claim
    else {
        unreachable!("positional_win_claim always returns Claim::PositionalWin");
    };

    // The sacrifice is the moving side's resource. It's good for the user
    // when the user moved (Player), bad when the opponent found it
    // (Opponent) — same flip as every mover-relative card.
    let sentiment = match perspective {
        Perspective::Player => Sentiment::Positive,
        Perspective::Opponent => Sentiment::Negative,
    };

    // Annotations illustrate the climax (post-forcing-tail) position: that
    // is where the compensating term peaks (the bare king, the frozen
    // rook). Walk the best PV through its settled ply.
    let climax = climax_position(best, pre_move_pos);
    let annotations = annotations_for(dominant_term, &climax, root_stm);

    let item = RetrospectiveItem {
        category: RetrospectiveCategory::PositionalWin,
        heading: phrasing.summary,
        summary: format!(
            "down {} of material, compensated by {}",
            describe_points(-sacrificed_points),
            dominant_term.pretty_label()
        ),
        detail: phrasing.detail.unwrap_or_default(),
        score_delta_pawns: None,
        sentiment,
        annotations,
    };
    Some((item, dominant_term))
}

/// The climax position — `pre_move_pos` advanced through the best PV's
/// forcing-tail prefix (the same endpoint [`positional_win_claim`] reads
/// its compensation at), so the annotations paint the board the card's
/// "why" describes (the bare king mid-hunt, not the converted endgame).
fn climax_position(best: &MoveAnalysis, pre_move_pos: &Position) -> Position {
    let climax_idx = forcing_tail_climax(pre_move_pos, &best.pv)
        .min(best.ply_traces.len().saturating_sub(1));
    let mut pos = pre_move_pos.clone();
    for &mv in best.pv.iter().take(climax_idx + 1) {
        pos.do_move(mv);
    }
    pos
}

/// Board annotations for the compensating term, painted on the climax
/// position. King danger → the enemy king's ring + attacker arrows; a
/// trapped rook → a bad-piece highlight on the frozen rook. Everything
/// else carries no spatial story (the prose stands alone).
fn annotations_for(term: TermId, climax: &Position, root_stm: Color) -> Vec<BoardAnnotation> {
    let mut anns = Vec::new();
    match term {
        TermId::KingDanger | TermId::KingFlankAttacks => {
            // The mover is attacking the *opponent's* king: paint that
            // king's square plus its adjacent ring (the "bare king under
            // heavy fire" receipt). The ring is computed locally — the
            // king square and its up-to-8 neighbours — so the card carries
            // no dependency on the king-safety evaluator internals.
            let enemy = !root_stm;
            let king_sq = climax.king_square(enemy);
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
        TermId::PiecesTrappedRook => {
            // The frozen rook belongs to the opponent (the mover's gain).
            for (sq, _) in trapped_cages(climax, !root_stm) {
                if climax
                    .piece_on(sq)
                    .is_some_and(|p| p.kind() == PieceType::Rook)
                {
                    anns.push(BoardAnnotation::SquareHighlight {
                        square: sq,
                        kind: AnnotationKind::BadPiece,
                    });
                }
            }
        }
        _ => {}
    }
    anns
}

/// The up-to-8 squares adjacent to `king_sq` — the king's ring,
/// excluding the king square itself. Computed by file/rank arithmetic so
/// it has no dependency on the engine's king-safety internals.
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

/// Terse material-deficit phrase for the card's stat summary — "a pawn"
/// for 1, "N points" otherwise. `points` is the (positive) magnitude.
fn describe_points(points: i32) -> String {
    match points {
        n if n <= 1 => "a pawn".to_string(),
        n => format!("{n} points"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chess_tutor_engine::engine::{Engine, SearchParams};

    /// Position 2 from the case study: White is up a clean exchange-plus
    /// but jammed; the sound rook sac `Rxe7+` ends White *down a point*
    /// yet at least equal because the king-danger term explodes. The
    /// positional-win card must fire, naming king safety as the
    /// compensation.
    #[test]
    fn case_study_position_2_fires_naming_king_danger() {
        let fen = "4kb1Q/rp2pp2/p2p2p1/1b6/8/3P4/PP3K1P/4R1R1 w - - 0 1";
        let mut pos = Position::from_fen(fen).unwrap();
        let root_stm = pos.side_to_move();
        let mut engine = Engine::default();
        let analyses = chess_tutor_engine::analysis::analyze_position(
            &mut engine,
            &mut pos,
            SearchParams {
                max_depth: 12,
                multi_pv: 2,
                ..SearchParams::default()
            },
        );
        let pre = Position::from_fen(fen).unwrap();
        let best = &analyses[0];
        let (item, term) = build_positional_win_item(best, &pre, root_stm, Perspective::Player)
            .expect("the sound rook sac should fire the positional-win card");
        assert_eq!(term, TermId::KingDanger);
        // King safety is the headline compensation, per the case study
        // (king danger +286 → +3211 mover-POV mg).
        assert!(
            item.summary.contains("king safety"),
            "expected king safety as the dominant term, got: {}",
            item.summary
        );
        assert_eq!(item.sentiment, Sentiment::Positive);
        // The card paints the climax king-ring receipt.
        assert!(
            item.annotations.iter().any(|a| matches!(
                a,
                BoardAnnotation::SquareHighlight {
                    kind: AnnotationKind::KingRing,
                    ..
                }
            )),
            "expected a king-ring highlight, got {:?}",
            item.annotations
        );
    }

    /// A routine winning capture is NOT a sacrifice, so the card must
    /// stay silent — the gate is `is_sacrifice`, not "best move loses a
    /// material card." White just grabs a free queen with `Qxd8`.
    #[test]
    fn routine_winning_capture_does_not_fire() {
        // White to move, can capture the undefended black queen on d8.
        let fen = "3qk3/8/8/8/8/8/8/3QK3 w - - 0 1";
        let mut pos = Position::from_fen(fen).unwrap();
        let root_stm = pos.side_to_move();
        let mut engine = Engine::default();
        let analyses = chess_tutor_engine::analysis::analyze_position(
            &mut engine,
            &mut pos,
            SearchParams {
                max_depth: 8,
                multi_pv: 2,
                ..SearchParams::default()
            },
        );
        let pre = Position::from_fen(fen).unwrap();
        let best = &analyses[0];
        assert!(
            build_positional_win_item(best, &pre, root_stm, Perspective::Player).is_none(),
            "a routine winning capture must not fire the sacrifice-justification card"
        );
    }
}
