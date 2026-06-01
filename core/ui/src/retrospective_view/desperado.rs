//! Desperado-aware material narration (PLAN §4, the safety-net table).
//!
//! Split out of `retrospective_view`; assembled by
//! [`super::build_retrospective_view`].
//!
//! When a piece is going to be lost, the material story isn't honest until
//! it accounts for whether that piece can **cash itself for a pawn first**
//! via a forcing in-between. The
//! [`positional-punish-after-qe6`](teaching-positions/positional-punish-after-qe6.md)
//! safety-net table is the case: after `…Nxe4` White's Nf5 is doomed, but
//! `Nxg7+` is a check, so it forces `…Bxg7` and buys the tempo to recapture
//! — the net swings from −1.0 ("down a clean pawn") to 0.0 ("even, the
//! desperado grabbed a pawn on the way down"). The note narrates exactly
//! that: "−X becomes −X+pawn because of the desperado," not "you're fine."
//!
//! ## Scope (the clear case)
//!
//! This walks the user's own PV looking for a **same-tempo
//! capture-with-check by a piece that is itself under attack** — the
//! `Nxg7+` shape. The engine-side [`find_desperado`] does the structural
//! check; here we (a) find such a move in the line and (b) confirm the
//! moving piece was hanging the ply before. A fuller treatment
//! (non-checking zwischenzug desperados, multi-step) is a documented
//! follow-up; see [`find_desperado`]'s boundary note.

use chess_tutor_engine::analysis::{find_desperado, list_hanging, list_see_losing, MoveAnalysis};
use chess_tutor_engine::position::Position;
use chess_tutor_engine::san;
use chess_tutor_engine::types::{Color, Value};

use crate::view::{RetrospectiveCategory, RetrospectiveItem, Sentiment};

/// Build a desperado note for the user's move, or `None` when the move
/// isn't a same-tempo capture-with-check desperado.
///
/// `user` is the user's analysed move; `pre_move_pos` the position it was
/// played from; `root_stm` the user's colour.
///
/// **Only the user's actual move (ply 0) is considered.** The desperado
/// concept is "the piece I am about to lose grabs material on the way
/// down" — it is a property of *the move just played*, not of some capture
/// deep in the engine's continuation. Walking the whole PV (the original
/// bug) fired on ordinary capture-with-check tactics many plies away — e.g.
/// a routine `Nxf6+` recapture six plies into the best line after a quiet
/// `1.e4` — narrating them as desperados that had nothing to do with the
/// user's move. Two gates keep this honest:
///   1. the move is the user's own ply-0 move, and
///   2. the moving piece is *genuinely doomed* — SEE-losing on its origin
///      square (it loses material if it stays), not merely attacked-but-
///      defended (a defended piece making a winning capture-with-check is
///      not "grabbing material on the way down").
pub(super) fn build_desperado_item(
    pre_move_pos: &Position,
    user: &MoveAnalysis,
    root_stm: Color,
) -> Option<RetrospectiveItem> {
    // The user's actual move. (Equivalently `user.pv.first()`, but `mv` is
    // authoritative and doesn't depend on the PV being populated.)
    let mv = user.mv;
    let from = mv.from();

    // Gate 2: the moving piece must be genuinely doomed where it stands.
    // It counts as doomed if it's undefended-and-attacked (`list_hanging`)
    // OR defended-but-SEE-losing (`list_see_losing`) — the two lists are
    // disjoint by construction (see `list_see_losing`'s "don't double-
    // report" note), so we check both. Without this a safe piece playing a
    // winning capture-with-check would be mis-narrated as "grabbing
    // material on the way down".
    let doomed = list_hanging(pre_move_pos, root_stm)
        .iter()
        .chain(list_see_losing(pre_move_pos, root_stm).iter())
        .any(|h| h.location.square == from);
    if !doomed {
        return None;
    }

    let d = find_desperado(pre_move_pos, from, root_stm)?;
    if mv.from() != d.piece || mv.to() != d.captures_on {
        return None;
    }
    let san_str = san::format(pre_move_pos, mv);
    Some(make_item(san_str, d.recovered_cp))
}

fn make_item(san_str: String, recovered_cp: i32) -> RetrospectiveItem {
    let recovered_pawns = recovered_cp as f32 / Value::PAWN_MG.0 as f32;
    RetrospectiveItem {
        category: RetrospectiveCategory::Material,
        heading: format!("Desperado — {san_str} grabs material on the way down"),
        summary: format!("the doomed piece cashes ~{recovered_pawns:.0} pawn(s) with check first"),
        detail: format!(
            "That piece was going to be lost, so before it falls it captures with check ({san_str}). \
             The check must be answered first, which buys the tempo to recover the piece — so \
             instead of losing it for nothing, you trade it off having pocketed a pawn. In the \
             ledger that turns a clean loss into a roughly even one: you go down the piece but \
             collect ~{recovered_pawns:.0} pawn(s) on the way, rather than 'you're fine'."
        ),
        score_delta_pawns: Some(recovered_pawns),
        sentiment: Sentiment::Positive,
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
        let item = build_desperado_item(&pre, &user, Color::White)
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
        assert!(build_desperado_item(&pre, &user, Color::White).is_none());
    }

    /// Regression for the original bug: the builder must consider ONLY the
    /// user's ply-0 move, never a capture-with-check deeper in the PV. Here
    /// the user's move is the quiet `a3`; a real `Nxg7+` desperado sits two
    /// plies later. The old whole-PV walk fired on it (narrating a deep,
    /// unrelated tactic as a desperado for `a3`); the fixed builder returns
    /// `None`.
    #[test]
    fn ignores_capture_with_check_deeper_in_pv() {
        use chess_tutor_engine::types::Square;
        let mut pre = Position::from_fen(DESPERADO_FEN).unwrap();
        let nxg7 = san::parse(&mut pre, "Nxg7+").unwrap();
        let pre = Position::from_fen(DESPERADO_FEN).unwrap();
        // ply 0: quiet a2-a3 (the user's actual move). ply 1: ...h6.
        // ply 2: Nxg7+ (the desperado — still legal, but NOT ply 0).
        let a3 = Move::normal(Square::A2, Square::A3);
        let h6 = Move::normal(Square::H7, Square::H6);
        let user = ma_with_pv(&pre, vec![a3, h6, nxg7]);
        assert!(
            build_desperado_item(&pre, &user, Color::White).is_none(),
            "must only consider the ply-0 move, not a capture-with-check deeper in the PV",
        );
    }

    /// A move that captures with check but whose piece is NOT doomed
    /// (it's safe where it stands) is not a desperado — it's just a strong
    /// move, and narrating it as "grabbing material on the way down" would
    /// be wrong. Here the white knight on d5 is unattacked; `Nxf6+` is a
    /// winning capture-with-check, not a desperado.
    #[test]
    fn no_note_when_moving_piece_is_not_doomed() {
        use chess_tutor_engine::types::Square;
        let pre = Position::from_fen("4k3/8/5p2/3N4/8/8/8/4K3 w - - 0 1").unwrap();
        let nxf6 = Move::normal(Square::D5, Square::F6);
        // Sanity: the move really is a capture-with-check (so only the
        // doomed-gate can be what suppresses it).
        assert!(pre.is_capture(nxf6) && pre.gives_check(nxf6));
        let user = ma_with_pv(&pre, vec![nxf6]);
        assert!(
            build_desperado_item(&pre, &user, Color::White).is_none(),
            "a safe (not SEE-losing) piece making a capture-with-check is not a desperado",
        );
    }
}
