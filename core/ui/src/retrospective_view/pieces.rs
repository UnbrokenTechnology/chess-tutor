//! Piece-placement card builders (one card per sub-signal × side).
//!
//! The prose (heading + detail, with the "you" / "they" reframe and the
//! per-sub-term concept wording) is produced by the shared teaching
//! translator ([`chess_tutor_teaching`]) from a [`Claim::PiecePlacement`];
//! the shared salience (per-sub-term threshold gating, BishopPawns
//! geometry suppression) lives in [`pieces_positional_claims`]. This
//! builder owns only the *structured* card surface the translator
//! deliberately doesn't carry — the sentiment, the score chip, and the
//! capture-aware suppression (which needs the realised capture events the
//! GUI already has in hand).
//!
//! Split out of `retrospective_view`; the orchestrator
//! ([`super::build_retrospective_view`]) assembles the cards.

use chess_tutor_engine::analysis::PiecesPositionalOutcome;
use chess_tutor_engine::types::{Color, PieceType};

use chess_tutor_teaching::claim::{
    pieces_positional_claims, Claim, PlacementCategory, PlacementSide, StructureDirection,
};
use chess_tutor_teaching::phrasing::{phrase, Locale, Perspective, PhrasingContext, Verbosity};

use crate::view::{RetrospectiveCategory, RetrospectiveItem, Sentiment};

/// Capture-aware suppression flags built from the realized capture events,
/// so the per-claim loop can drop cards whose term delta is an artifact
/// of a piece leaving the board (rather than a real repositioning).
#[derive(Copy, Clone, Debug, Default)]
pub(super) struct CaptureSuppression {
    /// `true` when at least one of our minors was captured at ply
    /// ≤ 1. Their average king-distance "improves" purely because a
    /// minor came off the board — no actual repositioning happened.
    /// Drop the ours-side KP card (in either direction).
    pub(super) ours_minor_captured: bool,
    /// Same logic for the opponent's side.
    pub(super) theirs_minor_captured: bool,
    /// `true` when *our* ply-0 move was a capture made *by* a minor.
    /// The minor's "drift away from the king" is what enabled the
    /// capture; framing it as a cost mis-teaches. Drops the
    /// `ours` worsened direction only — improvements (a minor
    /// rallying back to the king) still surface normally.
    pub(super) our_minor_capturing: bool,
    /// `true` when one of our rooks was captured. The trapped-rook
    /// penalty vanishes when the rook leaves the board — but a captured
    /// rook didn't "escape its trap," it died. Drop the ours TrappedRook
    /// card so we don't narrate a capture as an escape.
    pub(super) ours_rook_captured: bool,
    /// Same for the opponent's rook (the "Opponent's rook escaped its
    /// trap" misfire when you just captured it).
    pub(super) theirs_rook_captured: bool,
}

pub(super) fn capture_suppression(
    material: &chess_tutor_engine::analysis::MaterialOutcome,
    root_stm: Color,
) -> CaptureSuppression {
    let mut out = CaptureSuppression::default();
    for ev in material.realized_events() {
        if ev.captured_piece.is_minor() {
            if ev.captor == root_stm {
                // We captured one of their minors.
                out.theirs_minor_captured = true;
            } else {
                // They captured one of ours.
                out.ours_minor_captured = true;
            }
        }
        if ev.captured_piece == PieceType::Rook {
            if ev.captor == root_stm {
                out.theirs_rook_captured = true;
            } else {
                out.ours_rook_captured = true;
            }
        }
        if ev.ply == 0 && ev.captor == root_stm && ev.captor_piece.is_minor() {
            out.our_minor_capturing = true;
        }
    }
    out
}

impl CaptureSuppression {
    /// Whether a piece-placement claim should be dropped as a
    /// capture-artifact rather than a real repositioning. Mirrors the
    /// prior per-sub-term suppression, now keyed off the shared claim's
    /// `(side, category, direction)`.
    fn suppresses(&self, side: PlacementSide, category: PlacementCategory, worsened: bool) -> bool {
        match (side, category) {
            (PlacementSide::Mover, PlacementCategory::KingProtector) => {
                // A captured minor "improves" its mates' king-distance by
                // arithmetic; a minor capturing "drifts" only to make the
                // capture — drop the worsened direction in that case.
                self.ours_minor_captured || (self.our_minor_capturing && worsened)
            }
            (PlacementSide::Opponent, PlacementCategory::KingProtector) => {
                self.theirs_minor_captured
            }
            (PlacementSide::Mover, PlacementCategory::TrappedRook) => self.ours_rook_captured,
            (PlacementSide::Opponent, PlacementCategory::TrappedRook) => self.theirs_rook_captured,
            _ => false,
        }
    }
}

/// Build the piece-placement cards for one analysed move. `perspective`
/// selects "you" vs "they" and drives the student-POV sentiment colour.
pub(super) fn build_pieces_positional_items(
    outcome: &PiecesPositionalOutcome,
    _root_stm: Color,
    kp_supp: CaptureSuppression,
    perspective: Perspective,
) -> Vec<RetrospectiveItem> {
    let ctx = PhrasingContext {
        perspective,
        locale: Locale::En,
        verbosity: Verbosity::Normal,
        reveal_moves: false,
    };
    pieces_positional_claims(outcome)
        .iter()
        .filter_map(|claim| {
            let Claim::PiecePlacement {
                side,
                category,
                direction,
                ..
            } = claim
            else {
                unreachable!("pieces_positional_claims always returns Claim::PiecePlacement");
            };
            let worsened = *direction == StructureDirection::Worsened;
            if kp_supp.suppresses(*side, *category, worsened) {
                return None;
            }
            Some(pieces_item(claim, &ctx))
        })
        .collect()
}

/// Turn one [`Claim::PiecePlacement`] into a card — prose from the
/// translator, structured surface (sentiment, score chip, terse
/// summary) computed here from the claim's payload.
fn pieces_item(claim: &Claim, ctx: &PhrasingContext) -> RetrospectiveItem {
    let phrasing = phrase(claim, ctx);
    let Claim::PiecePlacement {
        side,
        category: _,
        direction,
        delta_mg,
    } = claim
    else {
        unreachable!("pieces_positional_claims always returns Claim::PiecePlacement");
    };

    // The piece is the user's when the moving side is the user
    // (Player + Mover); the player's POV is fixed here.
    let piece_is_user = (*side == PlacementSide::Mover) == (ctx.perspective == Perspective::Player);

    // Sentiment is "good for the user?": improving your own placement
    // is good, improving the opponent's hurts you.
    let sentiment = match (direction, piece_is_user) {
        (StructureDirection::Improved, true) => Sentiment::Positive,
        (StructureDirection::Worsened, true) => Sentiment::Negative,
        (StructureDirection::Improved, false) => Sentiment::Negative,
        (StructureDirection::Worsened, false) => Sentiment::Positive,
    };

    // User-POV score chip: the claim's `delta_mg` is side-relative
    // (positive = that side improved). For the user's own side that maps
    // straight through; for the opponent's it flips.
    let score_delta_mg = if piece_is_user { *delta_mg } else { -*delta_mg };

    // The translator's detail carries a parenthetical raw shift; the
    // card already shows a numeric chip, so strip it for the body.
    let detail = phrasing
        .detail
        .as_deref()
        .map(strip_trailing_shift)
        .unwrap_or_default();

    RetrospectiveItem {
        category: RetrospectiveCategory::PiecePlacement,
        heading: phrasing.summary,
        summary: format!("{:+.2} pawns", score_delta_mg as f32 / 100.0),
        detail,
        score_delta_pawns: Some(score_delta_mg as f32 / 100.0),
        sentiment,
        annotations: Vec::new(),
    }
}

/// Drop the translator's trailing " (+0.30 this side)." shift clause —
/// the card surfaces the same number in its own chip, so the prose body
/// stays a clean concept explanation.
fn strip_trailing_shift(detail: &str) -> String {
    match detail.rfind(" (") {
        Some(idx) if detail.trim_end().ends_with("this side).") => detail[..idx].to_string(),
        _ => detail.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chess_tutor_engine::eval::PiecesBreakdown;
    use chess_tutor_engine::types::Score;

    fn pib_zero() -> PiecesBreakdown {
        PiecesBreakdown {
            outposts: Score::ZERO,
            reachable_outposts: Score::ZERO,
            minor_behind_pawn: Score::ZERO,
            king_protector: Score::ZERO,
            bishop_pawns: Score::ZERO,
            long_diagonal_bishop: Score::ZERO,
            rook_on_queen_file: Score::ZERO,
            rook_on_open_file: Score::ZERO,
            rook_on_semiopen_file: Score::ZERO,
            trapped_rook: Score::ZERO,
            weak_queen: Score::ZERO,
        }
    }

    fn outcome(
        ours_pre: PiecesBreakdown,
        ours_post: PiecesBreakdown,
        theirs_pre: PiecesBreakdown,
        theirs_post: PiecesBreakdown,
    ) -> PiecesPositionalOutcome {
        // Both sides' bishop geometry counted as changed (post != pre on
        // the count), so non-suppression tests see every sub-term fire.
        PiecesPositionalOutcome {
            ours_pre,
            ours_post,
            theirs_pre,
            theirs_post,
            ours_bishop_pawn_count_pre: 0,
            ours_bishop_pawn_count_post: 1,
            theirs_bishop_pawn_count_pre: 0,
            theirs_bishop_pawn_count_post: 1,
        }
    }

    #[test]
    fn our_outpost_claim_is_positive_with_translator_heading() {
        let mut post = pib_zero();
        post.outposts = Score::new(30, 0);
        let o = outcome(pib_zero(), post, pib_zero(), pib_zero());
        let cards = build_pieces_positional_items(&o, Color::White, CaptureSuppression::default(), Perspective::Player);
        let card = cards
            .iter()
            .find(|c| c.heading == "Your knight reached an outpost")
            .expect("an outpost card");
        assert_eq!(card.sentiment, Sentiment::Positive);
        assert_eq!(card.score_delta_pawns, Some(0.30));
        // The shift clause must be stripped from the body.
        assert!(!card.detail.contains("this side"), "body: {}", card.detail);
    }

    #[test]
    fn denying_their_outpost_is_positive_opportunity() {
        // Their outpost bonus dropped (worsened for them) → good for us.
        let mut their_pre = pib_zero();
        their_pre.outposts = Score::new(30, 0);
        let o = outcome(pib_zero(), pib_zero(), their_pre, pib_zero());
        let cards = build_pieces_positional_items(&o, Color::White, CaptureSuppression::default(), Perspective::Player);
        let card = cards
            .iter()
            .find(|c| c.heading == "You denied the opponent's knight an outpost")
            .expect("a denied-outpost card");
        assert_eq!(card.sentiment, Sentiment::Positive);
        // Their −0.30 flips to a +0.30 chip for us.
        assert_eq!(card.score_delta_pawns, Some(0.30));
    }

    #[test]
    fn captured_rook_does_not_narrate_escape() {
        // Our trapped-rook penalty vanished (delta positive = improved),
        // but it's because the rook was captured — suppress the card.
        let mut pre = pib_zero();
        pre.trapped_rook = Score::new(-40, 0);
        let o = outcome(pre, pib_zero(), pib_zero(), pib_zero());
        let supp = CaptureSuppression {
            ours_rook_captured: true,
            ..CaptureSuppression::default()
        };
        let cards = build_pieces_positional_items(&o, Color::White, supp, Perspective::Player);
        assert!(
            !cards.iter().any(|c| c.heading.contains("rook escaped")),
            "captured rook must not read as an escape"
        );
    }

    #[test]
    fn below_threshold_yields_no_card() {
        let mut post = pib_zero();
        post.outposts = Score::new(10, 0);
        let o = outcome(pib_zero(), post, pib_zero(), pib_zero());
        assert!(build_pieces_positional_items(&o, Color::White, CaptureSuppression::default(), Perspective::Player)
            .is_empty());
    }
}
