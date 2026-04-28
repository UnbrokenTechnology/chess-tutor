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

use super::MoveAnalysis;
use crate::types::Value;

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

    // Below: the band would land at Mistake or Blunder. Apply the
    // swing guard before committing.
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
