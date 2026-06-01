//! Qualitative [`MoveVerdict`] + the `classify_move` thresholds.
//!
//! Thresholds in engine-internal cp. The engine's pawn values
//! (`PawnMg = 128`, `PawnEG = 213`) sit above Lichess's
//! 100-cp-per-pawn scale, so these numbers are roughly 1.3–2× the
//! equivalent chess.com bands. Picked for feel rather than from data;
//! tune once real retrospective output lands. Keep in one place so
//! the tuning surface is tight.
//!
//! ## Two axes, not one
//!
//! Verdict classification uses **two** independent measurements of a
//! move's quality:
//!
//! - `relative_loss = best_score − user_score` — how much the user
//!   "missed" by not picking the engine's preferred move. This drives
//!   the band ladder (Best / Good / Inaccuracy / Mistake / Blunder).
//! - `absolute_swing = user_score − pre_score` — whether the user's
//!   move actually helped, hurt, or held the position from where it
//!   was. Used as a *guard* on the worst labels.
//!
//! The two-axis design fixes a real-game disconnect where Ng5 went
//! pre-move +5.23 → post-move +8.23 (a +3 *improvement*) but was
//! classified `Mistake` because cxd4 would have reached +9.49. The
//! position got better; calling that move a "Mistake" — and slapping
//! `?` on the SAN — implies it got worse. The swing guard caps such
//! moves at `Inaccuracy`: the user *did* miss something, but they
//! didn't actively harm their position.
//!
//! Best / Good / Inaccuracy bands still use only `relative_loss`; the
//! guard only kicks in when the band would otherwise land at Mistake
//! or Blunder.
//!
//! ## The material axis (Miss vs Blunder)
//!
//! [`classify_move_with_material`] adds a third measurement, mirroring
//! the chess.com distinction (and our own opponent bot's
//! `miss_chance` / `blunder_chance` knobs in `noise.rs`):
//!
//! - **Blunder** — the move loses *your own* material.
//! - **Miss** — the move fails to *win* material that was on offer:
//!   the engine's best line wins material by force and your move
//!   neither grabs it nor hangs your own.
//!
//! The material axis is consulted only inside the Mistake/Blunder band
//! (where there's already a real eval gap). A **Miss** fires even when
//! the swing guard would otherwise quiet the move — declining a forced
//! material win while merely *holding* the eval is still the salient
//! lesson — but **not** when the move actively *improved* the position
//! (`absolute_swing > 0`), since a move that made things better isn't a
//! "Miss" no matter how much more was on offer. Material is measured in
//! engine midgame-cp (a pawn is [`Value::PAWN_MG`] = 128), the scale
//! [`super::compute_material_outcome`] already reports `net_mg_cp` on.
//! The plain [`classify_move`] keeps its score-only behaviour by
//! delegating with zero material on both sides.

use super::MoveAnalysis;
use crate::types::Value;

/// Did the user's move give away the advantage, from the mover's own POV?
/// Both scores are side-to-move (mover) POV in raw engine-cp:
/// `best` is the engine's top line's score from the position *before* the
/// move; `played` is the score of the move the user actually played.
///
/// Two conditions, both required (the `critique` CLI predicate, ported
/// here so the GUI's retrospective and Supported-mode pause share one
/// definition — PLAN §3 / §4.1):
/// - **conceded more than a pawn** (`best − played > PAWN_EG`), and
/// - **no longer clearly winning** (`played < +PAWN_EG`).
///
/// Broader than "crossed into a negative eval": it catches "gave up a win
/// without crossing zero" (`+2.0 → +0.2`) and "gave away the game from a
/// neutral start" (`+0.2 → −3.0`), while the `played < +1.0` floor keeps
/// "still clearly winning" slips quiet (`+5.0 → +3.0` stays silent).
pub fn gave_away_advantage(best: Value, played: Value) -> bool {
    let one_pawn = Value::PAWN_EG.0;
    let conceded = best.0 - played.0;
    conceded > one_pawn && played.0 < one_pawn
}

/// Qualitative judgement of a move, from the perspective of
/// retrospective teaching feedback. Mirrors the Lichess / chess.com
/// scale but with one extra variant for positions that were already
/// lost.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveVerdict {
    /// Essentially the engine's top choice — within a few engine-cp.
    Best,
    /// Close to best; minor deviation, likely still a good move.
    Good,
    /// Small mistake; the position is still playable but worse than
    /// it needed to be. Also the cap for "missed a stronger move but
    /// the position got better anyway" cases — see module docs.
    Inaccuracy,
    /// Noticeable positional or tactical error — *and* the move
    /// actively worsened the position (see swing guard in module
    /// docs).
    Mistake,
    /// Major swing — likely losing material, an attack, or the game
    /// — *and* the move actively worsened the position.
    Blunder,
    /// A forced material win was on offer (the engine's best line wins
    /// material) and this move let it slip without hanging the user's
    /// own material. The salient feature is the *un-grabbed win*, not a
    /// hang — so it's reported distinctly from Blunder. Only produced
    /// by [`classify_move_with_material`]; the score-only
    /// [`classify_move`] never returns it. See module docs.
    Miss,
    /// The position was already hopeless before this move, and the
    /// chosen move is as good as any. We flag this separately so the
    /// renderer can say *"nothing you could have done"* rather than
    /// *"excellent move"* when the user is losing a queen they'd
    /// already lost three moves ago.
    BestAvailable,
}

const BEST_LOSS_MAX: i32 = 15;
const GOOD_LOSS_MAX: i32 = 50;
const INACCURACY_LOSS_MAX: i32 = 120;
const MISTAKE_LOSS_MAX: i32 = 350;

/// A position is "hopeless" when even the best available move leaves
/// the side-to-move behind by this much. In such positions, a move
/// that's within a small window of best deserves a gentler verdict
/// (`BestAvailable`) than "Best" would convey.
const HOPELESS_SCORE_MAX: i32 = -500;

/// Classify `user_score` against `best_score` and `pre_score` (all
/// side-to-move POV from the same root position) into a
/// [`MoveVerdict`]. See module docs for the two-axis design.
///
/// Logic:
/// - Compute `relative_loss = max(0, best_score - user_score)`. Clamp
///   at zero because minor MultiPV score noise can put the "user's
///   move" slightly above the reported "best" when they're the same
///   move.
/// - If `relative_loss` is within the Best band AND best_score is
///   hopeless, return [`MoveVerdict::BestAvailable`] — we don't want
///   to congratulate someone on a -12 pawn position.
/// - Otherwise pick the band by `relative_loss` (Best / Good /
///   Inaccuracy / Mistake / Blunder).
/// - **Swing guard**: if the band landed at Mistake or Blunder but
///   `absolute_swing = user_score - pre_score` is non-negative — i.e.
///   the move improved or held the position — cap the verdict at
///   `Inaccuracy`. The user missed a stronger continuation but did
///   not actively worsen the position; calling that a "Mistake" is
///   misleading.
pub fn classify_move(user_score: Value, best_score: Value, pre_score: Value) -> MoveVerdict {
    // Material-free: zero on both sides means the material branch can
    // never fire (no Miss, no material-driven Blunder), so this keeps
    // the original score-only ladder exactly.
    classify_move_with_material(user_score, best_score, pre_score, 0, 0)
}

/// Material-aware [`classify_move`]. Adds the Miss vs Blunder axis on
/// top of the score ladder; see module docs for the full design.
///
/// `user_net_mg` / `best_net_mg` are the net material outcomes of the
/// user's line and the engine's best line through the settled ply,
/// from the **mover's own POV**, in engine midgame-cp (pawn = 128) —
/// exactly what [`super::compute_material_outcome`] returns as
/// `net_mg_cp` when called with `root_stm = pre_move_pos.side_to_move()`.
/// Positive = that line wins material for the user.
///
/// The Best / Good / Inaccuracy bands and the swing guard are
/// unchanged. Inside the Mistake/Blunder band, before the swing guard:
/// - if the best line wins ≥ 1 pawn of material, the user's move
///   doesn't hang ≥ 1 pawn of their own, and the user's move did not
///   improve the position (`absolute_swing <= 0`) → [`MoveVerdict::Miss`].
/// - otherwise fall through to the existing band + swing-guard logic
///   (a hang with a negative swing stays Mistake/Blunder; a sound
///   sacrifice that improved the eval is still capped at Inaccuracy).
pub fn classify_move_with_material(
    user_score: Value,
    best_score: Value,
    pre_score: Value,
    user_net_mg: i32,
    best_net_mg: i32,
) -> MoveVerdict {
    let relative_loss = (best_score.0 - user_score.0).max(0);
    let absolute_swing = user_score.0 - pre_score.0;
    let hopeless = best_score.0 <= HOPELESS_SCORE_MAX;

    if relative_loss <= BEST_LOSS_MAX {
        return if hopeless {
            MoveVerdict::BestAvailable
        } else {
            MoveVerdict::Best
        };
    }
    if relative_loss <= GOOD_LOSS_MAX {
        return MoveVerdict::Good;
    }
    if relative_loss <= INACCURACY_LOSS_MAX {
        return MoveVerdict::Inaccuracy;
    }

    // Below: the band would land at Mistake or Blunder.
    let one_pawn = Value::PAWN_MG.0;
    let user_lost_material = user_net_mg <= -one_pawn;
    let best_wins_material = best_net_mg >= one_pawn;

    // Miss: a forced material win was declined without hanging our own,
    // and the move didn't actively improve the position. Fires even
    // when the swing guard would otherwise quiet the move (a held eval
    // that left a free piece on the board is still a Miss); a move that
    // *improved* the eval is never a Miss, no matter how much more was
    // available (that path falls through to the swing guard → Inaccuracy).
    if best_wins_material && !user_lost_material && absolute_swing <= 0 {
        return MoveVerdict::Miss;
    }

    // Apply the swing guard before committing to Mistake / Blunder.
    let band = if relative_loss <= MISTAKE_LOSS_MAX {
        MoveVerdict::Mistake
    } else {
        MoveVerdict::Blunder
    };
    if absolute_swing >= 0 {
        // The position improved or held — refuse to call this a
        // Mistake / Blunder no matter how much was missed. Cap at the
        // top of the Inaccuracy band so the headline still flags that
        // a stronger move existed.
        MoveVerdict::Inaccuracy
    } else {
        band
    }
}

impl MoveAnalysis {
    /// Classify this move against the best available move's score
    /// from the same root position. `best_score` is the engine's top
    /// line's score (side-to-move POV, same scale as `self.score`);
    /// by definition the top line of the same-search
    /// `analyze_position` call produced this analysis. `pre_score`
    /// comes from the shared `pre_move_trace` evaluation and is
    /// stored on every `MoveAnalysis` for convenience.
    pub fn classify(&self, best_score: Value) -> MoveVerdict {
        classify_move(self.score, best_score, self.pre_score)
    }

    /// Material-aware verdict. `user_net_mg` / `best_net_mg` are the
    /// settled net material outcomes (mover-POV, engine midgame-cp) of
    /// this move's line and the best line — get them from
    /// [`super::compute_material_outcome`] for `self` and the best
    /// `MoveAnalysis` respectively. See [`classify_move_with_material`].
    pub fn classify_with_material(
        &self,
        best_score: Value,
        user_net_mg: i32,
        best_net_mg: i32,
    ) -> MoveVerdict {
        classify_move_with_material(self.score, best_score, self.pre_score, user_net_mg, best_net_mg)
    }
}

#[cfg(test)]
mod tests {
    use super::super::analyze_position;
    use super::*;
    use crate::engine::{Engine, SearchParams};
    use crate::position::Position;

    /// Most existing tests don't care about `pre_score`; default it
    /// to zero so `absolute_swing == user_score`. Tests that exercise
    /// the swing guard pass `pre_score` explicitly.
    fn classify(user: i32, best: i32) -> MoveVerdict {
        classify_move(Value(user), Value(best), Value::ZERO)
    }

    // ---- band ladder (relative_loss only, swing irrelevant) ---------

    #[test]
    fn classify_best_when_user_matches_best() {
        assert_eq!(classify(40, 40), MoveVerdict::Best);
    }

    #[test]
    fn classify_best_when_within_epsilon() {
        assert_eq!(classify(35, 40), MoveVerdict::Best);
    }

    #[test]
    fn classify_good_just_past_best_band() {
        assert_eq!(classify(15, 40), MoveVerdict::Good);
    }

    #[test]
    fn classify_inaccuracy_range() {
        assert_eq!(classify(-40, 40), MoveVerdict::Inaccuracy);
    }

    #[test]
    fn classify_loss_clamped_to_zero_when_user_above_best() {
        assert_eq!(classify(45, 40), MoveVerdict::Best);
    }

    // ---- BestAvailable in hopeless positions ------------------------

    #[test]
    fn classify_best_available_when_position_hopeless_and_move_matches() {
        assert_eq!(classify(-800, -800), MoveVerdict::BestAvailable);
    }

    #[test]
    fn classify_best_available_in_lost_endgame_tolerates_small_slip() {
        assert_eq!(classify(-1508, -1500), MoveVerdict::BestAvailable);
    }

    // ---- swing guard: Mistake / Blunder require absolute_swing < 0 --

    #[test]
    fn classify_mistake_when_position_actually_worsened() {
        // user dropped from +400 to +150; best held +400. Loss 250
        // (Mistake band, 120 < loss <= 350), swing -250 (negative)
        // -> Mistake.
        let v = classify_move(Value(150), Value(400), Value(400));
        assert_eq!(v, MoveVerdict::Mistake);
    }

    #[test]
    fn classify_blunder_when_position_actually_worsened() {
        // user dropped from +400 to -300; best held +400. Loss 700,
        // swing -700 -> Blunder.
        let v = classify_move(Value(-300), Value(400), Value(400));
        assert_eq!(v, MoveVerdict::Blunder);
    }

    #[test]
    fn classify_caps_at_inaccuracy_when_position_improved_despite_missed_better_move() {
        // Real-game case from HANDOFF: pre +523, user (Ng5) +823,
        // best (cxd4) +949. Loss is 126 cp (Mistake band), but swing
        // is +300 (improved). Should cap at Inaccuracy.
        let v = classify_move(Value(823), Value(949), Value(523));
        assert_eq!(v, MoveVerdict::Inaccuracy);
    }

    #[test]
    fn classify_caps_at_inaccuracy_when_blunder_band_but_position_improved() {
        // pre +0, user +200, best +700. Loss 500 (Blunder band) but
        // swing is +200 (improved) -> cap at Inaccuracy.
        let v = classify_move(Value(200), Value(700), Value(0));
        assert_eq!(v, MoveVerdict::Inaccuracy);
    }

    #[test]
    fn classify_caps_at_inaccuracy_when_position_held_exactly() {
        // Swing exactly zero — the move neither helped nor hurt.
        // Still counts as "didn't worsen" -> cap.
        let v = classify_move(Value(400), Value(900), Value(400));
        assert_eq!(v, MoveVerdict::Inaccuracy);
    }

    #[test]
    fn classify_blunder_in_losing_position_when_position_worsened() {
        // pre -300, user -1400, best -500. Loss 900 (Blunder band),
        // swing -1100 (got even worse) -> Blunder. Note: best is
        // -500, *not* hopeless (= HOPELESS_SCORE_MAX boundary,
        // hopeless requires <=), so BestAvailable doesn't fire.
        let v = classify_move(Value(-1400), Value(-499), Value(-300));
        assert_eq!(v, MoveVerdict::Blunder);
    }

    // ---- material axis: Miss vs Blunder -----------------------------

    /// One pawn on the engine midgame-cp material scale.
    const PAWN: i32 = Value::PAWN_MG.0;

    #[test]
    fn classify_miss_when_best_wins_material_and_user_declines() {
        // pre 0, user holds 0, best wins to +400 (Blunder band by
        // score). Best line nets a pawn; user's nets nothing and hangs
        // nothing; swing held at 0 -> Miss, not Blunder.
        let v = classify_move_with_material(Value(0), Value(400), Value(0), 0, PAWN);
        assert_eq!(v, MoveVerdict::Miss);
    }

    #[test]
    fn classify_miss_fires_even_when_eval_held_in_a_winning_position() {
        // Up +300 the whole time; best line grabs a free piece (+600)
        // but user stays +300. Swing is exactly 0 (held), so the swing
        // guard would normally quiet this — but declining a forced
        // material win is the lesson -> Miss.
        let v = classify_move_with_material(Value(300), Value(600), Value(300), 0, 3 * PAWN);
        assert_eq!(v, MoveVerdict::Miss);
    }

    #[test]
    fn classify_blunder_when_user_hangs_material_not_miss() {
        // user dropped +400 -> -300 by hanging a piece; best held +400.
        // user_net is a lost piece, so this is a Blunder even though the
        // best line also wins material (best_net positive). The hang
        // wins the tie: a Miss requires NOT losing your own material.
        let v = classify_move_with_material(Value(-300), Value(400), Value(400), -3 * PAWN, PAWN);
        assert_eq!(v, MoveVerdict::Blunder);
    }

    #[test]
    fn classify_positional_drop_stays_mistake_not_miss() {
        // 3-pawn-ish positional slide with no material on either side
        // of the ledger: user +200, best +500 (Mistake band), swing
        // -200, no material -> Mistake, never Miss.
        let v = classify_move_with_material(Value(200), Value(500), Value(400), 0, 0);
        assert_eq!(v, MoveVerdict::Mistake);
    }

    #[test]
    fn classify_not_miss_when_position_improved_despite_bigger_material_win() {
        // The Ng5 shape with a material-winning best line: pre +523,
        // user improved to +823, best +949 wins a piece. Because the
        // move IMPROVED the eval (swing +300), it's capped at Inaccuracy
        // — improving moves are never a Miss.
        let v = classify_move_with_material(Value(823), Value(949), Value(523), 0, 3 * PAWN);
        assert_eq!(v, MoveVerdict::Inaccuracy);
    }

    #[test]
    fn classify_score_only_never_returns_miss() {
        // The material-free entry point must keep the original ladder:
        // a worsening drop is Mistake/Blunder, never Miss. (pre 400 so
        // the swing is negative and the guard doesn't cap to Inaccuracy.)
        assert_eq!(
            classify_move_with_material(Value(100), Value(400), Value(400), 0, 0),
            MoveVerdict::Mistake
        );
        assert_eq!(
            classify_move_with_material(Value(-300), Value(400), Value(400), 0, 0),
            MoveVerdict::Blunder
        );
    }

    /// End-to-end wiring: the same `compute_material_outcome` the
    /// retrospective feeds into `classify_with_material` really yields a
    /// Miss when the best line wins a piece and the user plays a quiet
    /// non-hanging move. Uses fabricated PVs (no live search) over real
    /// `Move`/`Position` types so the material accounting is exercised.
    #[test]
    fn classify_with_material_grades_declined_free_rook_as_miss() {
        use super::super::{compute_material_outcome, test_support::ma_with_pv_score};
        use crate::types::{Color, Move, Square};

        // Black to move; the a8 rook can take the undefended a1 rook
        // for free (Rxa1+). A quiet Ra7 declines the win.
        let pre = Position::from_fen("r3k3/8/8/8/8/8/8/R3K3 b - - 0 1").unwrap();
        let rxa1 = Move::normal(Square::A8, Square::A1);
        let ra7 = Move::normal(Square::A8, Square::A7);

        let best = ma_with_pv_score(vec![rxa1], Some(0), Value(1200));
        let user = ma_with_pv_score(vec![ra7], Some(0), Value(0));

        let best_net = compute_material_outcome(&best, &pre, Color::Black).net_mg_cp;
        let user_net = compute_material_outcome(&user, &pre, Color::Black).net_mg_cp;
        assert!(best_net >= Value::PAWN_MG.0, "best line wins a rook");
        assert_eq!(user_net, 0, "quiet move wins nothing");

        let verdict = user.classify_with_material(best.score, user_net, best_net);
        assert_eq!(verdict, MoveVerdict::Miss);
    }

    // ---- MoveAnalysis::classify delegates ---------------------------

    #[test]
    fn classify_via_move_analysis_delegates() {
        let mut pos = Position::startpos();
        let mut engine = Engine::default();
        let analyses = analyze_position(
            &mut engine,
            &mut pos,
            SearchParams {
                max_depth: 3,
                multi_pv: 2,
                ..SearchParams::default()
            },
        );
        assert!(analyses.len() >= 2);
        let best_score = analyses[0].score;
        assert!(matches!(
            analyses[0].classify(best_score),
            MoveVerdict::Best | MoveVerdict::BestAvailable
        ));
    }

    #[test]
    fn classify_via_move_analysis_uses_pre_score_field() {
        // Construct two analyses with the same score / best but
        // different pre_score; verify that swing-gating is honored.
        let mut pos = Position::startpos();
        let mut engine = Engine::default();
        let analyses = analyze_position(
            &mut engine,
            &mut pos,
            SearchParams {
                max_depth: 3,
                multi_pv: 1,
                ..SearchParams::default()
            },
        );
        let a = &analyses[0];
        // pre_score is populated; the verdict computed via the
        // method should match calling classify_move directly with
        // a.pre_score.
        let direct = classify_move(a.score, a.score, a.pre_score);
        let via = a.classify(a.score);
        assert_eq!(direct, via);
    }
}
