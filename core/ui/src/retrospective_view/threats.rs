//! Threats card builders (hanging / SEE-losing).
//!
//! The prose (heading + per-piece attacker detail, with the "you" /
//! "they" reframe) is produced by the shared teaching translator
//! ([`chess_tutor_teaching`]) from a [`Claim::Threats`]; this builder
//! owns only the *structured* card surface the translator deliberately
//! doesn't carry — sentiment, the structured summary line, and the
//! per-square board annotations — plus the GUI-specific misleading-hang
//! salience ([`filter_misleading_hangs`]) that needs the realised
//! captures the claim layer doesn't see.
//!
//! Split out of `retrospective_view`; the orchestrator
//! ([`super::build_retrospective_view`]) assembles the cards.

use chess_tutor_engine::analysis::{HangingPiece, ThreatsOutcome};
use chess_tutor_engine::types::Square;

use chess_tutor_teaching::claim::{threats_claim_group, Claim, ThreatKind, ThreatSide, ThreatTarget};
use chess_tutor_teaching::phrasing::{phrase, Locale, Perspective, PhrasingContext, Verbosity};

use crate::view::{
    AnnotationKind, BoardAnnotation, RetrospectiveCategory, RetrospectiveItem, Sentiment,
};

use super::helpers::*;

// ---------------------------------------------------------------------
// Threats
// ---------------------------------------------------------------------

pub(super) fn threats_items_empty(outcome: &ThreatsOutcome) -> bool {
    outcome.ours_hanging.is_empty()
        && outcome.theirs_hanging.is_empty()
        && outcome.ours_see_losing.is_empty()
        && outcome.theirs_see_losing.is_empty()
        && outcome.ours_pressured.is_empty()
        && outcome.theirs_pressured.is_empty()
}

/// Build every threat card for one analysed move. `perspective` selects
/// "you" vs "they" and drives the student-POV sentiment colour.
pub(super) fn build_threat_items(
    outcome: &ThreatsOutcome,
    user_captures_by_square: &[(Square, u8)],
    perspective: Perspective,
) -> Vec<RetrospectiveItem> {
    let ctx = PhrasingContext {
        perspective,
        locale: Locale::En,
        verbosity: Verbosity::Normal,
        reveal_moves: false,
    };
    let mut items = Vec::new();

    // Highest-value guaranteed counter-threat — used to suppress
    // ours_hanging entries whose loss is irrelevant because the
    // opponent has a bigger problem on the board. Computed once
    // across both guaranteed-hanging and guaranteed-SEE-losing lists
    // since either qualifies as a winning counter-threat.
    let max_counter_threat_points = max_target_points(&outcome.theirs_hanging_guaranteed)
        .max(max_target_points(&outcome.theirs_see_losing_guaranteed));

    // Our hanging pieces — filter for misleading entries (planned
    // recapture; compensating counter-attack). See
    // [`filter_misleading_hangs`].
    let ours_hanging_filtered = filter_misleading_hangs(
        &outcome.ours_hanging,
        user_captures_by_square,
        max_counter_threat_points,
    );
    push_threat_card(
        &mut items,
        &ctx,
        ThreatSide::Mover,
        ThreatKind::Hanging,
        &ours_hanging_filtered,
    );

    // "You can win material" only fires off the *guaranteed* list —
    // entries that survive every legal opponent response. The raw
    // theirs_hanging is a static snapshot and would mis-teach the
    // student about defensible threats (Nf3 attacks e5 but ...Nc6
    // defends, etc.).
    push_threat_card(
        &mut items,
        &ctx,
        ThreatSide::Opponent,
        ThreatKind::Hanging,
        &outcome.theirs_hanging_guaranteed,
    );

    let ours_see_losing_filtered = filter_misleading_hangs(
        &outcome.ours_see_losing,
        user_captures_by_square,
        max_counter_threat_points,
    );
    push_threat_card(
        &mut items,
        &ctx,
        ThreatSide::Mover,
        ThreatKind::SeeLosing,
        &ours_see_losing_filtered,
    );
    push_threat_card(
        &mut items,
        &ctx,
        ThreatSide::Opponent,
        ThreatKind::SeeLosing,
        &outcome.theirs_see_losing_guaranteed,
    );

    items
}

/// Build one threat card from the already-salience-filtered `hangs`,
/// pushing it onto `items` when non-empty. The heading + per-piece
/// detail prose come from the translator (perspective-correct); the
/// structured summary and the board annotations are computed here. The
/// sentiment and the "is the target ours?" annotation flavour are
/// derived from the threatened side *relative to the user* (so an
/// opponent-move retrospective colours the opponent's hanging piece as
/// the student's opportunity), matching the translator's `victim_is_user`
/// reframe exactly.
fn push_threat_card(
    items: &mut Vec<RetrospectiveItem>,
    ctx: &PhrasingContext,
    side: ThreatSide,
    kind: ThreatKind,
    hangs: &[HangingPiece],
) {
    let pieces: Vec<ThreatTarget> = hangs.iter().map(ThreatTarget::from).collect();
    let Some(claim) = threats_claim_group(side, kind, pieces) else {
        return;
    };
    let phrasing = phrase(&claim, ctx);
    let Claim::Threats { pieces, .. } = &claim else {
        unreachable!("threats_claim_group always returns Claim::Threats");
    };

    // The threatened piece belongs to the user when the moving side is the
    // user (Mover + Player) or the non-moving side is the user (Opponent +
    // Opponent) — the same predicate the translator uses for `victim_is_user`.
    // A user piece in danger is Negative (a warning); an opponent piece in
    // danger is Positive (the student's opportunity).
    let target_is_ours =
        (side == ThreatSide::Mover) == (ctx.perspective == Perspective::Player);
    let sentiment = if target_is_ours {
        Sentiment::Negative
    } else {
        Sentiment::Positive
    };

    let summary = if pieces.len() == 1 {
        format!(
            "{} on {}",
            piece_name(pieces[0].location.piece),
            pieces[0].location.square.to_algebraic()
        )
    } else {
        format!("{} pieces", pieces.len())
    };

    let mut annotations = Vec::new();
    for p in pieces {
        for a in &p.attackers {
            annotations.push(BoardAnnotation::Arrow {
                from: a.square,
                to: p.location.square,
                kind: AnnotationKind::Attacker,
            });
        }
        let target_kind = if target_is_ours {
            AnnotationKind::Threat
        } else {
            AnnotationKind::GoodPiece
        };
        annotations.push(BoardAnnotation::SquareHighlight {
            square: p.location.square,
            kind: target_kind,
        });
    }

    items.push(RetrospectiveItem {
        category: RetrospectiveCategory::Threats,
        heading: phrasing.summary,
        summary,
        detail: phrasing.detail.unwrap_or_default(),
        score_delta_pawns: None,
        sentiment,
        annotations,
    });
}

/// Drop hanging-piece entries that would mislead the student. Two
/// suppression cases:
///
/// 1. **Planned recapture**: we just captured a piece of ≥ equal
///    classical point value (P:1, N:3, B:3, R:5, Q:9) on the same
///    square the piece now sits on. The exchange is fair or
///    favorable — calling it a hang frames the student's deliberate
///    trade as a mistake.
///
/// 2. **Compensating counter-attack**: there's a guaranteed
///    higher-value win elsewhere on the board — `counter_threat_pts`
///    is greater than our hanging piece's points. The opponent can't
///    both address the bigger threat and capture our piece — they have
///    to choose the bigger problem, so our piece isn't really
///    hanging. This catches the classic "leave the bishop hanging,
///    threatening the queen" zwischenzug pattern, when the queen
///    is in the *guaranteed* win list (i.e. opponent can't save it
///    even by capturing the bishop, because the bishop wasn't the
///    only attacker).
///
/// Both checks use classical point values, not cp, so the student
/// reasons in the same units the cards present.
pub(super) fn filter_misleading_hangs(
    hangs: &[HangingPiece],
    user_captures_by_square: &[(Square, u8)],
    counter_threat_pts: u8,
) -> Vec<HangingPiece> {
    hangs
        .iter()
        .filter(|h| {
            let our_points = h.location.piece.classical_points();
            // Case 1: planned recapture on the same square.
            let planned_recapture =
                user_captures_by_square.iter().any(|(sq, captured_points)| {
                    *sq == h.location.square && *captured_points >= our_points
                });
            // Case 2: a guaranteed counter-threat of strictly higher
            // value than the hanging piece. "Strictly higher" because
            // an equal-value counter-threat is a wash — opponent
            // could plausibly take ours and accept the loss elsewhere
            // as compensation.
            let compensated_by_counter = counter_threat_pts > our_points;
            !(planned_recapture || compensated_by_counter)
        })
        .cloned()
        .collect()
}

/// Maximum classical point value across a hanging-piece list.
/// Returns 0 when the list is empty — used as the "no
/// counter-threat" sentinel by [`filter_misleading_hangs`].
pub(super) fn max_target_points(hangs: &[HangingPiece]) -> u8 {
    hangs
        .iter()
        .map(|h| h.location.piece.classical_points())
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chess_tutor_engine::analysis::PieceLocation;
    use chess_tutor_engine::types::PieceType;

    fn pl(square: Square, piece: PieceType) -> PieceLocation {
        PieceLocation { square, piece }
    }

    fn hang(square: Square, piece: PieceType, attackers: Vec<PieceLocation>) -> HangingPiece {
        HangingPiece {
            location: pl(square, piece),
            attackers,
        }
    }

    /// A mover-side hang renders the player-perspective warning heading
    /// from the translator, with the attacker geometry in the detail and
    /// matching board annotations.
    #[test]
    fn ours_hanging_card_warns_with_translator_heading() {
        let outcome = ThreatsOutcome {
            ours_hanging: vec![hang(
                Square::D2,
                PieceType::Knight,
                vec![pl(Square::E3, PieceType::Pawn)],
            )],
            ours_hanging_delta: 1,
            theirs_hanging: vec![],
            ours_see_losing: vec![],
            theirs_see_losing: vec![],
            theirs_hanging_guaranteed: vec![],
            theirs_see_losing_guaranteed: vec![],
            ours_pressured: vec![],
            theirs_pressured: vec![],
            theirs_hanging_delta: 0,
            ours_see_losing_delta: 0,
            theirs_see_losing_delta: 0,
            ours_pressured_delta: 0,
            theirs_pressured_delta: 0,
        };
        let items = build_threat_items(&outcome, &[], Perspective::Player);
        let card = items
            .iter()
            .find(|i| i.heading.contains("hanging"))
            .expect("a hanging card");
        assert_eq!(card.heading, "Your knight on d2 is hanging");
        assert_eq!(card.summary, "knight on d2");
        assert_eq!(card.sentiment, Sentiment::Negative);
        assert!(card.detail.contains("attacked by the e3 pawn"), "{}", card.detail);
        // One attacker arrow + one threat highlight.
        assert_eq!(card.annotations.len(), 2);

        // Same outcome, opponent perspective: the *mover* is now the
        // opponent, so their hanging knight is the student's opportunity —
        // positive sentiment + a "You can win material" reframe, never a
        // "Your … is hanging" warning.
        let opp_items = build_threat_items(&outcome, &[], Perspective::Opponent);
        let opp_card = opp_items
            .iter()
            .find(|i| i.heading.contains("hanging") || i.heading.contains("win material"))
            .expect("an opponent-hang card");
        assert!(
            opp_card.heading.starts_with("You can win material"),
            "opponent-move hang must reframe as the student's chance: {}",
            opp_card.heading
        );
        assert_eq!(opp_card.sentiment, Sentiment::Positive);
    }

    /// A guaranteed opponent hang renders the opportunity heading ("You
    /// can win material"), positive sentiment, GoodPiece highlight.
    #[test]
    fn theirs_hanging_card_is_opportunity() {
        let outcome = ThreatsOutcome {
            ours_hanging: vec![],
            ours_hanging_delta: 0,
            theirs_hanging: vec![hang(
                Square::D7,
                PieceType::Bishop,
                vec![pl(Square::E6, PieceType::Pawn)],
            )],
            theirs_hanging_guaranteed: vec![hang(
                Square::D7,
                PieceType::Bishop,
                vec![pl(Square::E6, PieceType::Pawn)],
            )],
            ours_see_losing: vec![],
            theirs_see_losing: vec![],
            theirs_see_losing_guaranteed: vec![],
            ours_pressured: vec![],
            theirs_pressured: vec![],
            theirs_hanging_delta: 1,
            ours_see_losing_delta: 0,
            theirs_see_losing_delta: 0,
            ours_pressured_delta: 0,
            theirs_pressured_delta: 0,
        };
        let items = build_threat_items(&outcome, &[], Perspective::Player);
        let card = items
            .iter()
            .find(|i| i.heading.starts_with("You can win material"))
            .expect("an opportunity card");
        assert_eq!(card.sentiment, Sentiment::Positive);
        assert!(card.heading.contains("bishop on d7"), "{}", card.heading);
        // GoodPiece highlight on the target (the opponent's piece we win).
        assert!(card.annotations.iter().any(|a| matches!(
            a,
            BoardAnnotation::SquareHighlight {
                kind: AnnotationKind::GoodPiece,
                ..
            }
        )));
    }

    #[test]
    fn planned_recapture_is_filtered() {
        let hangs = vec![hang(Square::H6, PieceType::Bishop, vec![pl(Square::G7, PieceType::Pawn)])];
        // We just captured a bishop (3 pts) on h6 — a fair recapture.
        let kept = filter_misleading_hangs(&hangs, &[(Square::H6, 3)], 0);
        assert!(kept.is_empty(), "an equal-value recapture must be filtered");
    }

    #[test]
    fn compensating_counter_threat_filters_lower_value_hang() {
        let hangs = vec![hang(Square::B5, PieceType::Bishop, vec![pl(Square::A6, PieceType::Pawn)])];
        // A guaranteed queen (9 pts) win elsewhere dwarfs our bishop (3).
        let kept = filter_misleading_hangs(&hangs, &[], 9);
        assert!(kept.is_empty(), "a bigger counter-threat must suppress the hang");
    }
}
