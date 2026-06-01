//! Loss-of-initiative note (PLAN §4 follow-up — "a tactic doesn't need a
//! name").
//!
//! Split out of `retrospective_view`; assembled by
//! [`super::build_retrospective_view`].
//!
//! Some mistakes aren't a named tactic and aren't a deep silent sequence —
//! they hand the opponent a **run of forcing moves you must answer**, so
//! you spend move after move reacting (retreating, defending, trading)
//! while the opponent improves. The static eval still likes the move
//! (space, attackers near the king); the search rates it a mistake because
//! you've given up the initiative. The regression target is the `e5` push
//! in `rnbqkb1r/pp2npp1/3pp2p/8/2BQP3/5N2/PPP2PPP/RNB2RK1`: Black chases
//! the `Bc4` off the board (`…d5`, `…Bd7`, `…a6`) while developing, and
//! material never changes.
//!
//! Gate (three conditions, deliberately tight to avoid the over-firing the
//! desperado card once had):
//!   1. the move is a real mistake-ish verdict (Inaccuracy / Mistake /
//!      Blunder);
//!   2. it's a static-vs-search *surprise* (`LooksGoodButBad`) — static
//!      thought it was good, search disagrees: the signal that the move is
//!      governed by forcing play, not static features; and
//!   3. [`detect_initiative_loss`] finds an actual forcing run (≥ 2 tempo
//!      hits) in the user's own line — the human-findable mechanism.
//!
//! When all three hold this card explains *why* the static eval and the
//! search disagree (the gap the headline's `surprise_note` only asserts).
//! When (3) fails the move falls through to the depth-honesty note instead
//! ("no shorter lesson") — see [`super::build_depth_honesty_item`].

use chess_tutor_engine::analysis::{
    detect_initiative_loss, is_silent_sequencing, InitiativeLoss, MoveAnalysis, MoveVerdict,
    PriorMove, SurpriseKind,
};
use chess_tutor_engine::position::Position;
use chess_tutor_engine::san;
use chess_tutor_engine::types::Color;

use crate::view::{RetrospectiveCategory, RetrospectiveItem, Sentiment};

/// Build the loss-of-initiative note, or `None` when the move isn't a
/// surprise-mistake explained by a forcing run.
///
/// `best` / `user` are the engine's preferred move and the user's move
/// from the same root; `root_stm` is the side that moved.
pub(super) fn build_initiative_item(
    pre_move_pos: &Position,
    best: &MoveAnalysis,
    user: &MoveAnalysis,
    root_stm: Color,
    prior_move: Option<PriorMove>,
) -> Option<RetrospectiveItem> {
    // Gate 1: a real mistake-ish verdict. (Best/Good/BestAvailable never
    // warrant a "you gave up the initiative" lecture.)
    if !matches!(
        user.classify(best.score),
        MoveVerdict::Inaccuracy | MoveVerdict::Mistake | MoveVerdict::Blunder
    ) {
        return None;
    }
    // Gate 2: static said good, search said bad — the move is being judged
    // by forcing play, not by static features.
    if user.surprise(root_stm) != Some(SurpriseKind::LooksGoodButBad) {
        return None;
    }
    // Gate 3: NOT deep silent sequencing. When the gap only resolves past
    // human calculation depth (the `…Qc8` case — a forced trade-down whose
    // immediate moves *look* like harassment), there's no human-findable
    // lesson and the move routes to the depth-honesty note instead. This is
    // the discriminator between a teachable initiative loss and an
    // engine-only deep sequence.
    let deep_gap_cp = best.score.0 - user.score.0;
    if is_silent_sequencing(pre_move_pos, user.mv, best.mv, deep_gap_cp, prior_move) {
        return None;
    }
    // Gate 4: the opponent's immediate reply actually chases one of our
    // pieces (the human-findable mechanism).
    let loss = detect_initiative_loss(pre_move_pos, &user.pv, root_stm)?;

    let chain = forcing_chain_san(pre_move_pos, user, &loss);
    Some(make_item(&loss, &chain))
}

/// Render the opponent's forcing moves (the threats) as SAN, in order, by
/// re-walking the user's PV so each move is formatted against the board it
/// was played from.
fn forcing_chain_san(pre_move_pos: &Position, user: &MoveAnalysis, loss: &InitiativeLoss) -> Vec<String> {
    // SAN of every ply in the PV, formatted against the board before it.
    let mut board = pre_move_pos.clone();
    let mut per_ply: Vec<String> = Vec::with_capacity(user.pv.len());
    for &mv in &user.pv {
        per_ply.push(san::format(&board, mv));
        board.do_move(mv);
    }
    loss.hits
        .iter()
        .filter_map(|h| per_ply.get(h.ply).cloned())
        .collect()
}

fn make_item(_loss: &InitiativeLoss, chain: &[String]) -> RetrospectiveItem {
    let chain_str = join_with_then(chain);
    // One immediate reply vs. a sustained run — phrase honestly for each.
    let how = if chain.len() <= 1 {
        format!(
            "its best reply, {chain_str}, immediately attacks one of your pieces, so you're \
             reacting from the very first move"
        )
    } else {
        format!(
            "the opponent gets a run of forcing moves you must answer one after another — \
             {chain_str} — so you spend move after move reacting (retreating, defending)"
        )
    };
    let detail = format!(
        "The term breakdown likes this move (more space, your pieces aimed at the enemy king), \
         which is why it looks natural — but that's the static picture. The search rates it worse \
         because the move isn't really positional here, it's tactical: {how}, and your space / \
         attacking chances don't count for much while you're the one reacting. When a move's \
         static eval and its search score disagree this much, the position is being decided by \
         these forcing replies, not by the static features. The fix isn't a single tactic to spot \
         — it's to deny the opponent that initiative (often by keeping the tension and developing) \
         rather than committing to a move that invites it."
    );
    RetrospectiveItem {
        category: RetrospectiveCategory::Initiative,
        heading: "You gave up the initiative".to_string(),
        summary: "looks good statically, but the opponent's reply puts you on the defensive"
            .to_string(),
        detail,
        score_delta_pawns: None,
        sentiment: Sentiment::Negative,
        annotations: Vec::new(),
    }
}

/// Join SAN moves as "a, then b, then c" — reads as a sequence the student
/// can follow.
fn join_with_then(items: &[String]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].clone(),
        _ => items.join(", then "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chess_tutor_engine::analysis::analyze_position;
    use chess_tutor_engine::engine::{Engine, SearchParams};
    use chess_tutor_engine::san;
    use chess_tutor_engine::types::Move;

    /// The `e5`-push regression FEN.
    const E5_FEN: &str = "rnbqkb1r/pp2npp1/3pp2p/8/2BQP3/5N2/PPP2PPP/RNB2RK1 w - - 0 1";

    fn analyses_for(fen: &str, user_san: &str) -> (Vec<MoveAnalysis>, Move) {
        let mut pos = Position::from_fen(fen).unwrap();
        let user_mv = san::parse(&mut pos, user_san).unwrap();
        let mut pos = Position::from_fen(fen).unwrap();
        let mut engine = Engine::new(16);
        // Match the production retrospective config (RETROSPECTIVE_MULTI_PV
        // = 3, depth 12, single thread) so the test exercises the exact PV
        // the GUI would see — the user's move is force-included into the
        // ranked lines.
        let params = SearchParams {
            max_depth: 12,
            multi_pv: 3,
            force_include: vec![user_mv],
            threads: 1,
            ..SearchParams::default()
        };
        (analyze_position(&mut engine, &mut pos, params), user_mv)
    }

    #[test]
    fn fires_on_e5_and_blames_initiative_not_material() {
        let (analyses, user_mv) = analyses_for(E5_FEN, "e5");
        let best = &analyses[0];
        let user = analyses.iter().find(|a| a.mv == user_mv).unwrap();
        let pre = Position::from_fen(E5_FEN).unwrap();
        let item = build_initiative_item(&pre, best, user, Color::White, None)
            .expect("e5 is a surprise-mistake whose opponent reply chases a piece");
        let blob = format!("{} {} {}", item.heading, item.summary, item.detail).to_lowercase();
        // It must blame the initiative, not invent a material loss (e5 holds
        // material even).
        assert!(
            blob.contains("initiative"),
            "should name the initiative: {}",
            item.heading
        );
        assert!(
            !blob.contains("loses material") && !blob.contains("costing you material"),
            "e5 holds material even — must not claim a material loss: {}",
            item.heading
        );
        // And it must name the opponent's forcing reply — the SAN of the
        // immediate reply (pv[1]) that chases our piece. (The exact move
        // depends on the search config's PV, so derive it rather than
        // hard-coding.)
        let mut b = pre.clone();
        b.do_move(user.pv[0]);
        let reply_san = san::format(&b, user.pv[1]);
        assert!(
            item.detail.contains(&reply_san),
            "should name the opponent's forcing reply {reply_san}: {}",
            item.detail
        );
        assert_eq!(item.sentiment, Sentiment::Negative);
    }

    #[test]
    fn silent_on_a_best_move() {
        // The engine's own top move is not a mistake and not a surprise —
        // no initiative lecture.
        let mut pos = Position::from_fen(E5_FEN).unwrap();
        let mut engine = Engine::new(16);
        let analyses = analyze_position(
            &mut engine,
            &mut pos,
            SearchParams {
                max_depth: 12,
                multi_pv: 3,
                threads: 1,
                ..SearchParams::default()
            },
        );
        let best = &analyses[0];
        let pre = Position::from_fen(E5_FEN).unwrap();
        assert!(
            build_initiative_item(&pre, best, best, Color::White, None).is_none(),
            "the best move is neither a mistake nor a surprise — no card"
        );
    }

    /// Calibration bookend (paired with the e5 case): the silent-sequencing
    /// `…Qc8` move must NOT get an initiative card, even though its
    /// immediate `Bd5` reply pressures a piece. Its gap only resolves past
    /// human depth (`is_silent_sequencing` is the discriminator), so it
    /// routes to the depth-honesty note instead — we must not claim an
    /// initiative story for a deep forced trade-down.
    #[test]
    fn stays_silent_on_qc8_silent_sequencing() {
        const QC8_FEN: &str = "1r1q2nr/p3k3/2Bbbpp1/7p/2Q5/8/PPPP1PPP/R1B1R1K1 b - - 0 1";
        let (analyses, user_mv) = analyses_for(QC8_FEN, "Qc8");
        let best = &analyses[0];
        let user = analyses.iter().find(|a| a.mv == user_mv).unwrap();
        let pre = Position::from_fen(QC8_FEN).unwrap();
        assert!(
            build_initiative_item(&pre, best, user, Color::Black, None).is_none(),
            "…Qc8 is silent-sequencing — must route to depth-honesty, not an initiative card"
        );
    }
}
