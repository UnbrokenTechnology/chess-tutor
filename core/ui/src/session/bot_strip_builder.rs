//! Build the [`crate::view::BotStripView`] — the opponent strip above
//! the board. Split out of `view_builders.rs` because the
//! captured-material diff has enough logic (per-type counting against
//! the standard array) to warrant its own focused file + sibling test.

use super::*;

use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::{Color, Piece, PieceType};

use crate::view::{BotHandicap, BotStripView, PlayerStripView};

/// Standard starting count of each non-king piece type, in
/// heaviest-first display order. Used to derive how many of each piece
/// a side has lost (captured count = standard − live count).
const STANDARD_COUNTS: [(PieceType, u32); 5] = [
    (PieceType::Queen, 1),
    (PieceType::Rook, 2),
    (PieceType::Bishop, 2),
    (PieceType::Knight, 2),
    (PieceType::Pawn, 8),
];

impl Session {
    /// Build the opponent strip shown above the board. When the
    /// engine plays a side, that side is the bot and the strip frames
    /// it; in two-human / self-play modes there's no single "bot", so
    /// we frame the side *not* to move at the live position as the
    /// nominal opponent (keeps the strip populated without claiming a
    /// bot identity it doesn't have).
    pub fn build_bot_strip_view(&self) -> BotStripView {
        let bot_color = match self.engine_plays {
            EngineMode::Side(c) => c,
            EngineMode::None | EngineMode::Both => !self.user_color(),
        };

        let handicaps = bot_handicaps(&self.opponent);
        let (captured, point_advantage) = captured_diff(&self.position, bot_color);

        BotStripView {
            name: "Bot".to_string(),
            strength_label: format!("depth {}", self.depth),
            handicaps,
            captured,
            point_advantage,
        }
    }

    /// Build the player strip shown *below* the board — the user's own
    /// captured pieces and point lead (mirror of the bot strip).
    pub fn build_player_strip_view(&self) -> PlayerStripView {
        let (captured, point_advantage) = captured_diff(&self.position, self.user_color());
        PlayerStripView {
            captured,
            point_advantage,
        }
    }
}

/// Translate the opponent profile's active knobs into the structured
/// [`BotHandicap`] list. Order is fixed (perception, variety, mask)
/// so the strip reads consistently game to game. Returns empty when
/// the bot plays at full strength.
fn bot_handicaps(opponent: &chess_tutor_engine::opponent::OpponentProfile) -> Vec<BotHandicap> {
    let mut out = Vec::new();
    if opponent.perception < 1.0 {
        out.push(BotHandicap::Perception(opponent.perception));
    }
    if opponent.noise.avg_move_rank > 1.0 {
        out.push(BotHandicap::Variety(opponent.noise.avg_move_rank));
    }
    if !opponent.eval_mask.is_empty() {
        out.push(BotHandicap::EvalMask(opponent.eval_mask.disabled_iter().count()));
    }
    out
}

/// Compute the pieces `bot_color` has captured (i.e. the opponent's
/// missing pieces) and the bot's net classical-point lead.
///
/// Captured list is heaviest-first; the point advantage is the bot's
/// material minus the user's material, both summed over classical
/// piece values — positive when the bot is ahead.
fn captured_diff(pos: &Position, bot_color: Color) -> (Vec<Piece>, i32) {
    let user_color = !bot_color;
    let mut captured = Vec::new();
    for (pt, standard) in STANDARD_COUNTS {
        let lost = standard.saturating_sub(pos.count(user_color, pt));
        for _ in 0..lost {
            captured.push(Piece::new(user_color, pt));
        }
    }

    let bot_material = side_material(pos, bot_color);
    let user_material = side_material(pos, user_color);
    (captured, bot_material - user_material)
}

/// Sum of classical point values of `color`'s non-king pieces.
fn side_material(pos: &Position, color: Color) -> i32 {
    STANDARD_COUNTS
        .iter()
        .map(|&(pt, _)| pos.count(color, pt) as i32 * pt.classical_points() as i32)
        .sum()
}

#[cfg(test)]
#[path = "bot_strip_builder_tests.rs"]
mod tests;
