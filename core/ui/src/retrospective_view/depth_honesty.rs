//! Silent-sequencing depth-honesty note (PLAN §4.3).
//!
//! Split out of `retrospective_view`; assembled by
//! [`super::build_retrospective_view`].
//!
//! When a move the engine hates qualifies as **silent sequencing** — the
//! gap is invisible at human depth, large at full depth, and no detector
//! fires (see [`chess_tutor_engine::analysis::is_silent_sequencing`] and
//! the [`silent-sequencing-after-qc8`](teaching-positions/silent-sequencing-after-qc8.md)
//! case) — the retrospective must be **honest about its own limits**: show
//! the visible static-counting facts and say plainly that the reason the
//! move is worse is beyond practical calculation depth. No "blunder"
//! stamp, no fabricated mechanism. Lying about a mechanism is worse than
//! admitting we can't explain it.
//!
//! The note is intentionally narrow: it only fires on a move that already
//! tripped the bad-eval pipeline (`best.mv != user.mv` with a real gap),
//! so the bounded two-depth search inside `is_silent_sequencing` runs
//! rarely.

use chess_tutor_engine::analysis::{is_silent_sequencing, MoveAnalysis, PriorMove};
use chess_tutor_engine::position::Position;

use crate::view::{RetrospectiveCategory, RetrospectiveItem, Sentiment};

/// Build the depth-honesty note, or `None` when the move isn't silent
/// sequencing (the overwhelmingly common case).
///
/// `best` / `user` are the engine's preferred move and the user's move
/// from the same root; `pre_move_pos` is the position they were played
/// from; `prior_move` feeds the detector chain's recapture guard.
pub(super) fn build_depth_honesty_item(
    pre_move_pos: &Position,
    best: &MoveAnalysis,
    user: &MoveAnalysis,
    prior_move: Option<PriorMove>,
) -> Option<RetrospectiveItem> {
    if best.mv == user.mv {
        return None;
    }
    // Deep gap from the already-searched analyses (root-STM POV cp).
    let deep_gap_cp = best.score.0 - user.score.0;
    if !is_silent_sequencing(pre_move_pos, user.mv, best.mv, deep_gap_cp, prior_move) {
        return None;
    }
    Some(RetrospectiveItem {
        category: RetrospectiveCategory::Secondary,
        heading: "No shorter lesson here".to_string(),
        summary: "the engine sees trouble several moves out — beyond practical calculation depth"
            .to_string(),
        detail:
            "The engine evaluates this as worse than its top choice, but the difference doesn't \
             resolve until well past the depth a person can calculate over the board, and no \
             tactic, pin, fork, or loose piece explains it. This isn't a move you should feel \
             you missed — there isn't a shorter, teachable reason. Not every move the engine \
             dislikes has a lesson a human could have used."
                .to_string(),
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
        let item = build_depth_honesty_item(&pre, best, user, None)
            .expect("…Qc8 must produce a depth-honesty note");
        let blob = format!("{} {} {}", item.heading, item.summary, item.detail).to_lowercase();
        // No blunder stamp; no fabricated mechanism.
        assert!(!blob.contains("blunder"), "must not stamp blunder: {blob}");
        assert!(!blob.contains("you walked into"), "no fake walked-into: {blob}");
        assert!(blob.contains("calculation depth"));
    }

    #[test]
    fn silent_when_user_played_best() {
        let (pre, analyses, _) = analyses_for(QC8_FEN, "Be5");
        let best = &analyses[0];
        // Pretend the user played the best move.
        assert!(build_depth_honesty_item(&pre, best, best, None).is_none());
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
        let vm = build_retrospective_view(&pre, &analyses, qc8, false, false, None);
        let has_depth_honesty = vm.items.iter().any(|i| i.heading == "No shorter lesson here");
        assert!(has_depth_honesty, "qc8 view must carry the depth-honesty note");
        // No fabricated mechanism / blunder framing.
        for it in &vm.items {
            let blob = format!("{} {}", it.heading, it.detail).to_lowercase();
            assert!(!blob.contains("you walked into"), "no walked-into card on qc8: {}", it.heading);
        }
    }
}
