//! Qualitative [`MoveVerdict`] + the `classify_move` thresholds.
//!
//! Thresholds in engine-internal cp. The engine's pawn values
//! (`PawnMg = 128`, `PawnEG = 213`) sit above Lichess's
//! 100-cp-per-pawn scale, so these numbers are roughly 1.3–2× the
//! equivalent chess.com bands. Picked for feel rather than from data;
//! tune once real retrospective output lands. Keep in one place so
//! the tuning surface is tight.

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
    /// it needed to be.
    Inaccuracy,
    /// Noticeable positional or tactical error.
    Mistake,
    /// Major swing — likely losing material, an attack, or the game.
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

/// Classify `user_score` against `best_score` (both side-to-move POV
/// from the same root position) into a [`MoveVerdict`].
///
/// Logic:
/// - Compute `loss = max(0, best_score - user_score)`. Clamp at zero
///   because minor MultiPV score noise can put the "user's move"
///   slightly above the reported "best" when they're the same move.
/// - If `loss` is within the Best band AND best_score is hopeless,
///   the verdict is [`MoveVerdict::BestAvailable`] — we don't want
///   to congratulate someone on a -12 pawn position.
/// - Otherwise walk the bands in order: Best / Good / Inaccuracy /
///   Mistake / Blunder.
pub fn classify_move(user_score: Value, best_score: Value) -> MoveVerdict {
    let loss = (best_score.0 - user_score.0).max(0);
    let hopeless = best_score.0 <= HOPELESS_SCORE_MAX;

    if loss <= BEST_LOSS_MAX {
        return if hopeless {
            MoveVerdict::BestAvailable
        } else {
            MoveVerdict::Best
        };
    }
    if loss <= GOOD_LOSS_MAX {
        return MoveVerdict::Good;
    }
    if loss <= INACCURACY_LOSS_MAX {
        return MoveVerdict::Inaccuracy;
    }
    if loss <= MISTAKE_LOSS_MAX {
        return MoveVerdict::Mistake;
    }
    MoveVerdict::Blunder
}

impl MoveAnalysis {
    /// Classify this move against the best available move's score
    /// from the same root position. `best_score` is the engine's top
    /// line's score (side-to-move POV, same scale as `self.score`);
    /// by definition the top line of the same-search
    /// `analyze_position` call produced this analysis.
    pub fn classify(&self, best_score: Value) -> MoveVerdict {
        classify_move(self.score, best_score)
    }
}

#[cfg(test)]
mod tests {
    use super::super::analyze_position;
    use super::*;
    use crate::engine::{Engine, SearchParams};
    use crate::position::Position;

    #[test]
    fn classify_best_when_user_matches_best() {
        assert_eq!(classify_move(Value(40), Value(40)), MoveVerdict::Best);
    }

    #[test]
    fn classify_best_when_within_epsilon() {
        assert_eq!(classify_move(Value(35), Value(40)), MoveVerdict::Best);
    }

    #[test]
    fn classify_good_just_past_best_band() {
        assert_eq!(classify_move(Value(15), Value(40)), MoveVerdict::Good);
    }

    #[test]
    fn classify_inaccuracy_range() {
        assert_eq!(
            classify_move(Value(-40), Value(40)),
            MoveVerdict::Inaccuracy,
        );
    }

    #[test]
    fn classify_mistake_range() {
        assert_eq!(classify_move(Value(-160), Value(40)), MoveVerdict::Mistake);
    }

    #[test]
    fn classify_blunder_range() {
        assert_eq!(classify_move(Value(-560), Value(40)), MoveVerdict::Blunder);
    }

    #[test]
    fn classify_best_available_when_position_hopeless_and_move_matches() {
        assert_eq!(
            classify_move(Value(-800), Value(-800)),
            MoveVerdict::BestAvailable,
        );
    }

    #[test]
    fn classify_best_available_in_lost_endgame_tolerates_small_slip() {
        assert_eq!(
            classify_move(Value(-1508), Value(-1500)),
            MoveVerdict::BestAvailable,
        );
    }

    #[test]
    fn classify_blunder_even_in_losing_position() {
        assert_eq!(
            classify_move(Value(-1400), Value(-500)),
            MoveVerdict::Blunder,
        );
    }

    #[test]
    fn classify_loss_clamped_to_zero_when_user_above_best() {
        assert_eq!(classify_move(Value(45), Value(40)), MoveVerdict::Best);
    }

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
}
