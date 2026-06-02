//! Bot strip view descriptor — the opponent strip rendered *above*
//! the board (chess.com idiom; decision #3 in PLAN-ui-redesign.md).
//!
//! There is deliberately **no** matching user strip on the opposite
//! side: the bot's strip doubles as a live reminder of the handicaps
//! in play (blunder %, eval-mask, variety) plus a captured-material
//! diff. The fields here are flat, semantic data — the renderer owns
//! glyphs / formatting. No teaching prose lives here; these are status
//! labels in the same family as the eval-bar label and the move SAN,
//! not Claim-IR prose owned by `core/teaching`.

use chess_tutor_engine::types::Piece;

/// Opponent strip drawn above the board. Carries the bot's identity, a
/// derived strength label, the active handicaps (so the student is
/// always reminded what's been dialled down), and a captured-material
/// diff (the pieces the bot has taken + a signed point lead).
pub struct BotStripView {
    /// Display name for the bot. Today a fixed label; becomes the
    /// selected-bot name once the Start/Options screen (step 5) lands.
    pub name: String,
    /// Short strength descriptor derived from the bot's search depth
    /// (e.g. "depth 10"). Not an ELO — we don't claim one.
    pub strength_label: String,
    /// Handicaps currently in effect. Empty when the bot plays at full
    /// strength (no noise, no eval mask). The renderer formats each
    /// into a chip / inline label.
    pub handicaps: Vec<BotHandicap>,
    /// Pieces the bot has captured from the user, heaviest first. The
    /// renderer paints these as small glyphs next to the strip.
    pub captured: Vec<Piece>,
    /// Net classical-point lead from the **bot's** POV. Positive when
    /// the bot is up material, negative when behind, zero when even.
    /// The renderer shows `+N` only when positive (chess.com idiom:
    /// the lead sits next to whoever holds it).
    pub point_advantage: i32,
}

/// One active bot handicap, structured so the renderer owns the
/// wording (and a future locale can rephrase without touching this
/// layer). Each variant carries the magnitude the renderer formats.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BotHandicap {
    /// Per-move probability the bot deliberately drops material
    /// (chess.com "blunder"). Carries the fraction in `[0,1]`.
    BlunderChance(f32),
    /// Per-move probability the bot fails to capitalise on a
    /// material-winning chance (chess.com "miss"). Fraction in `[0,1]`.
    MissChance(f32),
    /// Move-variety dial — the average rank of the move the bot plays
    /// (`> 1.0` means it doesn't always pick the engine's #1).
    Variety(f32),
    /// The bot's evaluation is blind to `n` named categories
    /// (eval-mask). Carries the count of masked categories.
    EvalMask(usize),
}
