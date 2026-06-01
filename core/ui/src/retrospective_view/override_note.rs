//! Static-vs-search override note (PLAN §4.2 — "the hard one").
//!
//! Split out of `retrospective_view`; assembled by
//! [`super::build_retrospective_view`].
//!
//! Some positions are exactly where the per-term ledger *lies*: the
//! recommended move is a **static downgrade** yet a **search upgrade**, and
//! only search rescues the ranking. The
//! [`positional-punish-after-qe6`](teaching-positions/positional-punish-after-qe6.md)
//! case is the regression target — `Ne3` is a ~1.9-pawn static downgrade
//! vs `O-O`, yet a ~0.7–3 pawn search upgrade, because `O-O` allows
//! `…Nxe4` and the king-attack the static eval credits White for is an
//! illusion. When this happens the narration must **say so out loud** and
//! never invent a positional justification: if the GUI ever calls `Ne3`
//! "positionally strong," the layer is lying.
//!
//! The detector compares the *direction* of two measurements between the
//! engine's preferred move (`best`) and the move the user played (`user`):
//!
//! - **term ledger**: each move's post-move static net total, white-POV,
//!   tempo-stripped — `EvalTrace::white_pov_value`. The "static" picture a
//!   one-ply term diff would show.
//! - **search**: each move's full-depth search score (`MoveAnalysis::score`,
//!   root-STM POV).
//!
//! When the two rank the moves in *opposite* directions by a meaningful
//! margin (the ledger prefers the user's move, search prefers the engine's
//! pick), the note fires. It never names a positional virtue for the
//! recommended move — only the honest "the term breakdown would point you
//! the other way; the search overrules it."

use chess_tutor_engine::analysis::MoveAnalysis;
use chess_tutor_engine::types::{Color, Value};

use crate::view::{RetrospectiveCategory, RetrospectiveItem, Sentiment};

/// Minimum disagreement magnitude (engine-internal cp) on *each* axis for
/// the override note to fire. Both the static gap and the search gap must
/// clear this so we don't narrate ledger/search noise as a "lie." 100 cp
/// ≈ a half-pawn on the PawnEG=213 scale; on the positional-punish FEN the
/// static gap is ~1.9 pawns and the search gap ~0.7 pawns, both well clear.
const OVERRIDE_MARGIN_CP: i32 = 100;

/// Build the static-vs-search override note, or `None` when the term
/// ledger and the search agree (the common case — no note).
///
/// `best` / `user` are the engine's preferred move and the user's move
/// from the same root; `root_stm` is the side that moved. Both must carry
/// a post-move trace (`ply_traces[0]`); without it we can't read the
/// static picture and stay silent.
pub(super) fn build_override_note_item(
    best: &MoveAnalysis,
    user: &MoveAnalysis,
    root_stm: Color,
) -> Option<RetrospectiveItem> {
    // Same move → nothing to contrast.
    if best.mv == user.mv {
        return None;
    }
    let best_static = post_move_static_root_pov(best, root_stm)?;
    let user_static = post_move_static_root_pov(user, root_stm)?;

    // Direction on each axis: positive = the engine's preferred move scores
    // higher than the user's. By construction search prefers `best`
    // (search_gap > 0); the override fires only when the *static* ledger
    // disagrees (static_gap < 0 — the ledger ranks the user's move higher).
    let search_gap = best.score.0 - user.score.0;
    let static_gap = best_static.0 - user_static.0;

    if search_gap <= OVERRIDE_MARGIN_CP {
        return None; // search doesn't meaningfully prefer best
    }
    if static_gap >= -OVERRIDE_MARGIN_CP {
        return None; // the ledger agrees (or is close) — no lie to flag
    }

    // The two disagree. Report it honestly and *without* a positional
    // virtue for the recommended move — the static price it pays is real.
    let static_pawns = (-static_gap) as f32 / Value::PAWN_EG.0 as f32;
    let search_pawns = search_gap as f32 / Value::PAWN_EG.0 as f32;
    let detail = format!(
        "Read the term breakdown alone and it would tell you the opposite — your move keeps \
         the prettier static eval (by about {static_pawns:.1} pawns of named terms: king \
         attack, mobility, piece placement). The search overrules it by about {search_pawns:.1} \
         pawns, because those terms are built on something the one-ply breakdown can't see: \
         your move lets the opponent equalise, and the activity the static score is crediting \
         evaporates. This is the case to trust the search over the ledger — the recommended \
         move pays a real, visible positional price to deny the opponent that resource. Don't \
         read it as the prettier positional move; read it as the correctly cautious one."
    );
    Some(RetrospectiveItem {
        category: RetrospectiveCategory::Secondary,
        heading: "The term breakdown is misleading here".to_string(),
        summary: format!(
            "static ledger favours your move (~{static_pawns:.1}), but search favours the \
             other (~{search_pawns:.1})"
        ),
        detail,
        score_delta_pawns: None,
        sentiment: Sentiment::Neutral,
        annotations: Vec::new(),
    })
}

/// The move's post-move static net total, root-STM POV, in engine cp.
/// `ply_traces[0]` is the trace right after the user's move (opponent to
/// move), so the stm at that trace is `!root_stm`; `white_pov_value`
/// strips tempo and orients to white, and we flip to root-STM POV.
fn post_move_static_root_pov(ma: &MoveAnalysis, root_stm: Color) -> Option<Value> {
    let trace = ma.ply_traces.first()?;
    let white_pov = trace.white_pov_value(!root_stm);
    Some(if root_stm == Color::White {
        white_pov
    } else {
        Value(-white_pov.0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chess_tutor_engine::analysis::analyze_position;
    use chess_tutor_engine::engine::{Engine, SearchParams};
    use chess_tutor_engine::position::Position;
    use chess_tutor_engine::san;
    use chess_tutor_engine::types::Move;

    /// The positional-punish-after-qe6 FEN. `Ne3` (search +1.7) is a
    /// ~1.9-pawn static downgrade vs `O-O` (static +1.96). The note must
    /// fire and must never call Ne3 "positionally strong".
    const POSITIONAL_PUNISH_FEN: &str =
        "r1b1kb1r/1p3ppp/p2pqn2/4pNB1/4P3/2N5/PPP2PPP/R2QK2R w KQkq - 0 1";

    fn analyses_for(fen: &str, user_san: &str) -> (Vec<MoveAnalysis>, Move) {
        let mut pos = Position::from_fen(fen).unwrap();
        let user_mv = san::parse(&mut pos, user_san).unwrap();
        let mut pos = Position::from_fen(fen).unwrap();
        let legal = chess_tutor_engine::movegen::legal_moves_vec(&mut pos.clone());
        let mut engine = Engine::new(16);
        let params = SearchParams {
            max_depth: 12,
            multi_pv: legal.len(),
            force_include: vec![user_mv],
            threads: 1,
            ..SearchParams::default()
        };
        (analyze_position(&mut engine, &mut pos, params), user_mv)
    }

    #[test]
    fn fires_on_positional_punish_and_never_calls_recommended_move_strong() {
        let (analyses, user_mv) = analyses_for(POSITIONAL_PUNISH_FEN, "O-O");
        let best = &analyses[0];
        let user = analyses.iter().find(|a| a.mv == user_mv).unwrap();
        // The engine's pick on this FEN is Ne3 (a static downgrade); the
        // user played O-O (the static-pretty move). The override must fire.
        let item = build_override_note_item(best, user, Color::White)
            .expect("static ledger lies here — the override note must fire");
        let blob = format!("{} {} {}", item.heading, item.summary, item.detail);
        assert!(
            !blob.to_lowercase().contains("positionally strong"),
            "must never call the recommended move positionally strong: {blob}"
        );
        assert!(blob.contains("search overrules") || blob.contains("trust the search"));
    }

    #[test]
    fn silent_when_static_and_search_agree() {
        // Start position, depth 6: the top move and a near-best alternative
        // don't have the static-ledger-lies shape, so no note.
        let mut pos = Position::startpos();
        let mut engine = Engine::new(16);
        let analyses = analyze_position(
            &mut engine,
            &mut pos,
            SearchParams {
                max_depth: 6,
                multi_pv: 4,
                threads: 1,
                ..SearchParams::default()
            },
        );
        let best = &analyses[0];
        // A different second move — directions should agree (both small).
        if let Some(other) = analyses.iter().find(|a| a.mv != best.mv) {
            assert!(
                build_override_note_item(best, other, Color::White).is_none(),
                "no static-vs-search lie in the opening — note must stay silent"
            );
        }
    }
}
