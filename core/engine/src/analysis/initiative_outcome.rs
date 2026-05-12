//! [`InitiativeOutcome`] — forcing-hierarchy detection.
//!
//! Codifies the **checks > captures > threats > quiet** forcing
//! hierarchy that 1500+ ELO players carry as a procedural habit.
//! When the user's move added pressure to the opponent's side, this
//! outcome describes the opponent's best one-ply reply (`pv[1]`)
//! relative to that pressure — does the reply outrank the user's
//! threat (a check or capture) or simply address it?
//!
//! Three teaching templates the CLI narrator picks between based on
//! the captured signals:
//!
//! 1. **Reinforcement** — user's threat lands, opponent has no
//!    forcing reply that outranks it. Reinforces the rule by showing
//!    the hierarchy at work.
//! 2. **Refutation** — opponent's reply is check / capture AND the
//!    eval at settled-ply still favours the *opponent*. Teaches the
//!    rule by showing what happens when the user missed a
//!    higher-ranking forcing reply.
//! 3. **Held despite** — opponent's reply is check / capture AND the
//!    eval at settled-ply still favours the *user*. Teaches the
//!    limit of the rule: the hierarchy is a *processing* order, not
//!    a *winning* rule.
//!
//! Doesn't consume any TermId from the fallback line — this is
//! about move-relationship, not eval-term shifts.

use super::{
    compute_king_safety_outcome, compute_threats_outcome, post_user_move, MoveAnalysis,
};
use crate::eval::EvalTrace;
use crate::position::Position;
use crate::san;
use crate::types::{Color, Value};

/// Threshold for any one of the four `theirs_*` threat-category
/// deltas to count as "the user's move created a threat." Set at
/// 1 — any positive delta in any category fires. Real-game tuning
/// may push this higher if template 1 reads as noisy in practice
/// (the brief flagged this knob explicitly).
const THREAT_DELTA_GATE: i32 = 1;

/// Captured signals about the user's move-creating-a-threat
/// relationship to the opponent's best reply. The CLI narrator
/// branches on these to select template 1 / 2 / 3 (see module docs).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InitiativeOutcome {
    /// True when the user's move added meaningful pressure to the
    /// opponent's side (any pre→ply-1 `theirs_*` delta crosses
    /// [`THREAT_DELTA_GATE`] across hanging / SEE-losing / pressured
    /// lists *or* the opponent's king-attackers count).
    pub user_move_was_threat: bool,
    /// True when the opponent's `pv[1]` puts the user's king in
    /// check. Computed by applying `pv[0]` then `pv[1]` to a clone
    /// of `pre_move_pos` and reading `in_check()` (after the reply,
    /// side-to-move is the user — so `in_check()` reports on the
    /// user's king).
    pub opponent_reply_is_check: bool,
    /// True when the opponent's `pv[1]` is a capture. Any capture
    /// counts; the eval-swing test does the gating for "did this
    /// capture matter."
    pub opponent_reply_is_capture: bool,
    /// SAN of the opponent's reply (`pv[1]`), formatted relative to
    /// the position immediately after the user's move. `None` when
    /// the search returned a one-ply PV (terminal after user's
    /// move — mate, stalemate, or shallow search).
    pub opponent_reply_san: Option<String>,
    /// Eval swing from ply 1 (after user's move, before opponent
    /// reply) to settled-ply (after the line settles), in cp from
    /// the **user's** POV. Negative = the line settled worse than
    /// the position looked immediately after the user's move (the
    /// classic refutation case where the opponent's forcing reply
    /// genuinely costs the user material or position).
    pub eval_swing_cp: i32,
    /// True when the eval at settled-ply is in the user's favour
    /// (settled-ply value > 0 in user POV). The narrator's
    /// discriminator between "refutation" (false) and "held despite"
    /// (true) when the opponent's reply is a check or capture.
    pub user_still_favored: bool,
}

/// Build the initiative-outcome snapshot for `ma` against
/// `pre_move_pos`. `root_stm` is the user's side.
pub fn compute_initiative_outcome(
    ma: &MoveAnalysis,
    pre_move_pos: &Position,
    root_stm: Color,
) -> InitiativeOutcome {
    let user_move_was_threat = detect_user_threat(ma, pre_move_pos, root_stm);

    let post_user_pos = post_user_move(pre_move_pos, ma);
    let (opponent_reply_is_check, opponent_reply_is_capture, opponent_reply_san) =
        inspect_opponent_reply(ma, &post_user_pos);

    let (eval_swing_cp, user_still_favored) = compute_eval_swing(ma, root_stm);

    InitiativeOutcome {
        user_move_was_threat,
        opponent_reply_is_check,
        opponent_reply_is_capture,
        opponent_reply_san,
        eval_swing_cp,
        user_still_favored,
    }
}

/// Did the user's move add meaningful pressure to the opponent? We
/// reuse the signals `ThreatsOutcome` + `KingSafetyOutcome` already
/// produce so the gate matches the lists `ThreatsOutcome` would
/// narrate elsewhere — no risk of disagreeing about "is there a
/// threat here?".
fn detect_user_threat(
    ma: &MoveAnalysis,
    pre_move_pos: &Position,
    root_stm: Color,
) -> bool {
    let threats = compute_threats_outcome(ma, pre_move_pos, root_stm);
    let king_safety = compute_king_safety_outcome(ma, pre_move_pos, root_stm);

    threats.theirs_hanging_delta >= THREAT_DELTA_GATE
        || threats.theirs_see_losing_delta >= THREAT_DELTA_GATE
        || threats.theirs_pressured_delta >= THREAT_DELTA_GATE
        || king_safety.theirs_attackers_delta() >= THREAT_DELTA_GATE
}

/// Examine `pv[1]` against the position immediately after the user's
/// move: is it a check / capture, and what's its SAN? Returns
/// `(false, false, None)` when there's no second ply.
fn inspect_opponent_reply(
    ma: &MoveAnalysis,
    post_user_pos: &Position,
) -> (bool, bool, Option<String>) {
    let Some(reply) = ma.pv.get(1).copied() else {
        return (false, false, None);
    };

    // SAN must be formatted from the pre-reply position (it scans
    // for ambiguity, check, etc.) — clone so format_on doesn't
    // mutate our scratch.
    let mut for_san = post_user_pos.clone();
    let san_string = san::format_on(&mut for_san, reply);

    let is_capture = post_user_pos.is_capture(reply);

    let mut after_reply = post_user_pos.clone();
    after_reply.do_move(reply);
    // After the reply, side-to-move is the user — `in_check()`
    // reports on the user's king.
    let is_check = after_reply.in_check();

    (is_check, is_capture, Some(san_string))
}

/// Eval swing ply-1 → settled-ply (user POV) plus a derived
/// "user still favored" flag. Returns `(0, false)` if `ply_traces`
/// is empty (terminal-after-user-move; no eval to read).
fn compute_eval_swing(ma: &MoveAnalysis, root_stm: Color) -> (i32, bool) {
    let Some(p1_trace) = ma.ply_traces.first() else {
        return (0, false);
    };

    // Pick the same trace `MoveAnalysis::diff_trace` would: settled
    // when available, else leaf. The `ply_traces.is_empty()` short
    // circuit above guarantees `last()` is `Some` here.
    let settled_idx = ma
        .settled_ply
        .filter(|i| *i < ma.ply_traces.len())
        .unwrap_or(ma.ply_traces.len() - 1);
    let settled_trace = &ma.ply_traces[settled_idx];

    let p1_user = ply_trace_user_pov(p1_trace, 0, root_stm).0;
    let settled_user = ply_trace_user_pov(settled_trace, settled_idx, root_stm).0;

    let swing = settled_user - p1_user;
    let user_still_favored = settled_user > 0;
    (swing, user_still_favored)
}

/// Convert `ma.ply_traces[ply]` to a Value in the user's
/// (`root_stm`'s) POV. Side-to-move at `ply_traces[i]` follows the
/// `i % 2` parity: ply 0 is after the user's move (opponent to
/// move), ply 1 is after the opponent's reply (user to move), and
/// so on.
fn ply_trace_user_pov(trace: &EvalTrace, ply: usize, root_stm: Color) -> Value {
    let stm_at_eval = if ply % 2 == 0 { !root_stm } else { root_stm };
    let white_pov = trace.white_pov_value(stm_at_eval).0;
    let user_pov = match root_stm {
        Color::White => white_pov,
        Color::Black => -white_pov,
    };
    Value(user_pov)
}

#[cfg(test)]
mod tests {
    use super::super::test_support::ma_with_pv;
    use super::*;
    use crate::engine::{Engine, SearchParams};
    use crate::types::{Move, Square};

    /// Helper: kick off `analyze_position` so we get a real
    /// `MoveAnalysis` (with populated `ply_traces`) for a chosen
    /// position. Tests that need full eval-swing semantics use this;
    /// tests that only need threat detection / SAN can use
    /// `ma_with_pv`.
    fn analyze_first_line(fen: &str, depth: u32) -> (Position, super::super::MoveAnalysis) {
        let mut pos = Position::from_fen(fen).unwrap();
        let pre = pos.clone();
        let mut engine = Engine::default();
        let analyses = super::super::analyze_position(
            &mut engine,
            &mut pos,
            SearchParams {
                max_depth: depth,
                multi_pv: 1,
                ..SearchParams::default()
            },
        );
        (pre, analyses.into_iter().next().unwrap())
    }

    // ---- detect_user_threat -----------------------------------------

    #[test]
    fn quiet_opening_move_is_not_a_threat() {
        let pos = Position::startpos();
        let e4 = Move::normal(Square::E2, Square::E4);
        let ma = ma_with_pv(vec![e4], Some(0));
        let outcome = compute_initiative_outcome(&ma, &pos, Color::White);
        assert!(!outcome.user_move_was_threat);
    }

    #[test]
    fn move_that_attacks_undefended_piece_is_a_threat() {
        // White knight on b1 jumps to c3 attacking the black rook on
        // a4 (undefended). Lots of space for both kings; no other
        // tactics interfering.
        let fen = "4k3/8/8/8/r7/8/8/1N2K3 w - - 0 1";
        let pos = Position::from_fen(fen).unwrap();
        let nc3 = Move::normal(Square::B1, Square::C3);
        let ma = ma_with_pv(vec![nc3], Some(0));
        let outcome = compute_initiative_outcome(&ma, &pos, Color::White);
        assert!(
            outcome.user_move_was_threat,
            "Nc3 attacks undefended a4 rook — should be flagged as threat-creating"
        );
    }

    // ---- inspect_opponent_reply -------------------------------------

    #[test]
    fn opponent_reply_check_flagged_with_san_and_check() {
        // Construct a pv that ends in a check from black. White
        // plays Ra1-a8+ would have been a check, but here we want
        // black's *reply* to white's move to be the check.
        //
        // Setup: white king on g1, white queen on g4. Black king on
        // h8, black rook on h2. White plays Qg4-d4 (quiet); black
        // replies Rh1+ (check on white king).
        let fen = "7k/8/8/8/6Q1/8/7r/6K1 w - - 0 1";
        let pos = Position::from_fen(fen).unwrap();
        let qd4 = Move::normal(Square::G4, Square::D4);
        let rh1 = Move::normal(Square::H2, Square::H1);
        let ma = ma_with_pv(vec![qd4, rh1], Some(1));
        let outcome = compute_initiative_outcome(&ma, &pos, Color::White);
        assert!(outcome.opponent_reply_is_check);
        assert!(!outcome.opponent_reply_is_capture);
        assert_eq!(outcome.opponent_reply_san.as_deref(), Some("Rh1+"));
    }

    #[test]
    fn opponent_reply_capture_flagged_when_taking_a_piece() {
        // We want a clean "opponent captures, not also check" test.
        // White king parked on h1 (off any line the black queen on
        // d4 will attack), white queen hanging on d4, black queen
        // on d8 ready to capture. White plays a quiet pawn push
        // (a2-a3); black replies Qxd4. The black queen on d4
        // doesn't attack h1 (different rank, file, and not on a
        // shared diagonal), so the capture is not also check.
        let fen = "3q3k/8/8/8/3Q4/8/P7/7K w - - 0 1";
        let pos = Position::from_fen(fen).unwrap();
        let a3 = Move::normal(Square::A2, Square::A3);
        let qxd4 = Move::normal(Square::D8, Square::D4);
        let ma = ma_with_pv(vec![a3, qxd4], Some(1));
        let outcome = compute_initiative_outcome(&ma, &pos, Color::White);
        assert!(!outcome.opponent_reply_is_check);
        assert!(outcome.opponent_reply_is_capture);
        assert!(outcome.opponent_reply_san.unwrap().starts_with("Qxd4"));
    }

    #[test]
    fn no_opponent_reply_yields_none_san_and_false_flags() {
        let pos = Position::startpos();
        let e4 = Move::normal(Square::E2, Square::E4);
        let ma = ma_with_pv(vec![e4], Some(0));
        let outcome = compute_initiative_outcome(&ma, &pos, Color::White);
        assert!(!outcome.opponent_reply_is_check);
        assert!(!outcome.opponent_reply_is_capture);
        assert_eq!(outcome.opponent_reply_san, None);
    }

    // ---- compute_eval_swing -----------------------------------------
    //
    // Eval-swing tests rely on real ply_traces, so they call
    // `analyze_position` to get a properly-populated MoveAnalysis.

    #[test]
    fn eval_swing_zero_when_ply_traces_empty() {
        let pos = Position::startpos();
        let e4 = Move::normal(Square::E2, Square::E4);
        let ma = ma_with_pv(vec![e4], Some(0));
        let outcome = compute_initiative_outcome(&ma, &pos, Color::White);
        // ma_with_pv leaves ply_traces empty.
        assert_eq!(outcome.eval_swing_cp, 0);
        assert!(!outcome.user_still_favored);
    }

    #[test]
    fn eval_swing_populated_for_real_search() {
        // Use a heavily one-sided position (white K+Q vs lone black K)
        // so the search score and the settled-ply leaf eval are both
        // robustly positive — the assertion holds without depending on
        // a finely-balanced startpos eval. Validates: the favored flag
        // tracks the settled-ply sign for a real PV trace.
        let (pre, ma) = analyze_first_line("4k3/8/8/8/8/8/8/3QK3 w - - 0 1", 4);
        let outcome = compute_initiative_outcome(&ma, &pre, Color::White);
        assert!(
            outcome.user_still_favored,
            "K+Q vs K must register as user-favored; score={}",
            ma.score.0
        );
    }

    #[test]
    fn eval_swing_signed_from_user_pov_for_black_root_stm() {
        // After 1.e4, black to move. Verify that user_still_favored
        // tracks the signed score from black's POV — if the search
        // returns a negative number for black (white is better),
        // user_still_favored is false.
        let (pre, ma) = analyze_first_line(
            "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1",
            4,
        );
        let outcome = compute_initiative_outcome(&ma, &pre, Color::Black);
        assert_eq!(outcome.user_still_favored, ma.score.0 > 0);
    }
}
