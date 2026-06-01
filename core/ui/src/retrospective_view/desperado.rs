//! Desperado-aware material narration (PLAN §4, the safety-net table).
//!
//! The prose (heading + summary + detail, with the "you" / "they"
//! reframe) is produced by the shared teaching translator
//! ([`chess_tutor_teaching`]) from a [`Claim::Desperado`]; the shared
//! salience (the doomed-piece gate, the same-tempo capture-with-check
//! detection via the engine [`find_desperado`]) lives in
//! [`desperado_claim`]. This builder owns only the *structured* card
//! surface the translator deliberately doesn't carry — the category,
//! sentiment, and the recovered-material chip.
//!
//! Split out of `retrospective_view`; assembled by
//! [`super::build_retrospective_view`].
//!
//! When a piece is going to be lost, the material story isn't honest
//! until it accounts for whether that piece can **cash itself for a pawn
//! first** via a forcing in-between (the `Nxg7+` shape). The note
//! narrates "−X becomes −X+pawn because of the desperado," not "you're
//! fine".

use chess_tutor_engine::analysis::MoveAnalysis;
use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::{Color, Value};

use chess_tutor_teaching::claim::{desperado_claim, Claim};
use chess_tutor_teaching::phrasing::{phrase, Locale, Perspective, PhrasingContext, Verbosity};

use crate::view::{RetrospectiveCategory, RetrospectiveItem, Sentiment};

/// Build a desperado card for the user's move, or `None` when the move
/// isn't a same-tempo capture-with-check desperado.
///
/// `perspective` selects "you" vs "they" in the translator's prose. The
/// desperado is the *mover's* resource, so under the Player perspective
/// the recovered material helps the student (Positive); under the
/// Opponent perspective the opponent recovered it (Negative for the
/// student, and the eval chip flips sign).
pub(super) fn build_desperado_item(
    pre_move_pos: &Position,
    user: &MoveAnalysis,
    root_stm: Color,
    perspective: Perspective,
) -> Option<RetrospectiveItem> {
    let claim = desperado_claim(pre_move_pos, user, root_stm)?;
    let ctx = PhrasingContext {
        perspective,
        locale: Locale::En,
        verbosity: Verbosity::Normal,
        reveal_moves: false,
    };
    Some(desperado_item(&claim, &ctx))
}

/// Turn one [`Claim::Desperado`] into a card — prose from the translator,
/// structured surface computed here from the claim payload.
fn desperado_item(claim: &Claim, ctx: &PhrasingContext) -> RetrospectiveItem {
    let phrasing = phrase(claim, ctx);
    let Claim::Desperado { recovered_cp, .. } = claim else {
        unreachable!("desperado_claim always returns Claim::Desperado");
    };
    let recovered_pawns = *recovered_cp as f32 / Value::PAWN_MG.0 as f32;
    let (sentiment, chip) = match ctx.perspective {
        Perspective::Player => (Sentiment::Positive, recovered_pawns),
        Perspective::Opponent => (Sentiment::Negative, -recovered_pawns),
    };
    RetrospectiveItem {
        category: RetrospectiveCategory::Material,
        heading: phrasing.summary,
        summary: format!(
            "the doomed piece cashes ~{recovered_pawns:.0} pawn(s) with check first"
        ),
        detail: phrasing.detail.unwrap_or_default(),
        score_delta_pawns: Some(chip),
        sentiment,
        annotations: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chess_tutor_engine::san;
    use chess_tutor_engine::types::Move;

    /// A position where White's Nf5 is hanging (Black just played …Nxe4
    /// against it conceptually) and `Nxg7+` is a same-tempo desperado.
    /// White to move; the user's line opens with the desperado.
    const DESPERADO_FEN: &str = "r1b1kb1r/1p3ppp/p5pp/4pNB1/4n3/2N5/PPP2PPP/R2Q1RK1 w kq - 0 2";

    fn ma_with_pv(_pre: &Position, pv: Vec<Move>) -> MoveAnalysis {
        MoveAnalysis {
            mv: pv.first().copied().unwrap_or(Move::NONE),
            score: Value::ZERO,
            depth: 1,
            pv,
            ply_traces: Vec::new(),
            settled_ply: Some(0),
            pre_move_trace: chess_tutor_engine::eval::EvalTrace::zero(),
            pre_score: Value::ZERO,
            term_deltas: Vec::new(),
        }
    }

    #[test]
    fn narrates_nxg7_desperado() {
        let mut pre = Position::from_fen(DESPERADO_FEN).unwrap();
        let nxg7 = san::parse(&mut pre, "Nxg7+").unwrap();
        let pre = Position::from_fen(DESPERADO_FEN).unwrap();
        // The forced reply Bxg7 (f8 bishop takes), then a recapture.
        let bxg7 = Move::normal(
            chess_tutor_engine::types::Square::F8,
            chess_tutor_engine::types::Square::G7,
        );
        let user = ma_with_pv(&pre, vec![nxg7, bxg7]);
        let item = build_desperado_item(&pre, &user, Color::White, Perspective::Player)
            .expect("Nxg7+ is a same-tempo capture-with-check desperado");
        assert!(item.heading.contains("Desperado"), "{}", item.heading);
        assert!(item.detail.contains("Nxg7+"), "{}", item.detail);
        assert_eq!(item.sentiment, Sentiment::Positive);
    }

    #[test]
    fn no_note_for_quiet_line() {
        let pre = Position::startpos();
        let user = ma_with_pv(
            &pre,
            vec![Move::normal(
                chess_tutor_engine::types::Square::E2,
                chess_tutor_engine::types::Square::E4,
            )],
        );
        assert!(build_desperado_item(&pre, &user, Color::White, Perspective::Player).is_none());
    }

    /// Regression for the original bug: only the user's ply-0 move is
    /// considered, never a capture-with-check deeper in the PV.
    #[test]
    fn ignores_capture_with_check_deeper_in_pv() {
        use chess_tutor_engine::types::Square;
        let mut pre = Position::from_fen(DESPERADO_FEN).unwrap();
        let nxg7 = san::parse(&mut pre, "Nxg7+").unwrap();
        let pre = Position::from_fen(DESPERADO_FEN).unwrap();
        let a3 = Move::normal(Square::A2, Square::A3);
        let h6 = Move::normal(Square::H7, Square::H6);
        let user = ma_with_pv(&pre, vec![a3, h6, nxg7]);
        assert!(
            build_desperado_item(&pre, &user, Color::White, Perspective::Player).is_none(),
            "must only consider the ply-0 move, not a capture-with-check deeper in the PV",
        );
    }

    /// A move that captures with check but whose piece is NOT doomed is
    /// not a desperado.
    #[test]
    fn no_note_when_moving_piece_is_not_doomed() {
        use chess_tutor_engine::types::Square;
        let pre = Position::from_fen("4k3/8/5p2/3N4/8/8/8/4K3 w - - 0 1").unwrap();
        let nxf6 = Move::normal(Square::D5, Square::F6);
        assert!(pre.is_capture(nxf6) && pre.gives_check(nxf6));
        let user = ma_with_pv(&pre, vec![nxf6]);
        assert!(
            build_desperado_item(&pre, &user, Color::White, Perspective::Player).is_none(),
            "a safe (not SEE-losing) piece making a capture-with-check is not a desperado",
        );
    }
}
