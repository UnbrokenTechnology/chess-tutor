//! Centralized visual theme — the single source of truth for colors.
//!
//! Renderers map *semantic* tokens (`Sentiment`, verdict tiers, panel
//! kinds) to concrete colors HERE, so the palette is tunable in one place
//! and `core/ui` stays renderer-neutral (it emits semantics, never
//! colors).
//!
//! Style-pass step 1 is a pure relocation: every value below is the
//! pre-existing one pulled out of the scattered `draw::*` modules, so
//! adopting this module changes nothing on screen. Later steps retune it
//! (e.g. step 2 swaps the quality hues to the chess.com verdict palette).
//!
//! Not yet centralized (deliberately, for the later overlay-styling pass):
//! `board.rs`'s per-`AnnotationKind` overlay tint/arrow map and the board
//! square *tints* (last-move / selected / check / move-dot), which use
//! non-const unmultiplied alpha and form their own cohesive overlay color
//! language.

use chess_tutor_ui::view::{ReviewVerdictTier, Sentiment};
use eframe::egui::{self, Color32, FontFamily, FontId, TextStyle};

// === Text (dark theme) ===
// egui's dark defaults sit at grey 140 (labels) / 180 (buttons), which on
// the near-black panels reads as dim grey-on-black. We lift the base text
// to near-white for high contrast; `.weak()` (a 50/50 fade of the base
// toward the panel) then lands above the AA floor instead of below it.
/// Primary text.
pub const TEXT: Color32 = Color32::from_rgb(0xe8, 0xe6, 0xe3);
/// Secondary / caption text. The legibility floor — don't go dimmer for
/// anything a student is expected to read (≈6.4:1 on the dark panels).
pub const TEXT_MUTED: Color32 = Color32::from_rgb(0x9a, 0xa0, 0xa6);

// === Type scale (pt) — accessibility floor is 12; nothing renders below
// it. Mapped onto egui's built-in TextStyles in `apply`. ===
pub const FONT_HEADING: f32 = 18.0;
pub const FONT_BODY: f32 = 14.0;
pub const FONT_MONO: f32 = 13.0;
pub const FONT_CAPTION: f32 = 12.0;

/// Install the app-wide egui style once at startup: the type scale (12 pt
/// accessibility floor — `.small()` text was egui's 9 pt, which the user
/// couldn't read) and high-contrast text on the dark panels. Background
/// fills stay egui's dark defaults; the surface-color repaint is a
/// separate step — this pass is fonts + text contrast only.
pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();

    style.text_styles = [
        (TextStyle::Heading, FontId::new(FONT_HEADING, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(FONT_BODY, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(FONT_BODY, FontFamily::Proportional)),
        (TextStyle::Small, FontId::new(FONT_CAPTION, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(FONT_MONO, FontFamily::Monospace)),
    ]
    .into();

    let v = &mut style.visuals;
    // Bright base text → normal labels read clearly, and `.weak()` (fade of
    // the base toward the panel) stays legible. `.strong()` resolves to the
    // `active` widget color, which egui's dark default already paints white.
    v.widgets.noninteractive.fg_stroke.color = TEXT; // normal labels
    v.widgets.inactive.fg_stroke.color = TEXT; // button text (was grey 180)
    v.widgets.open.fg_stroke.color = TEXT;

    ctx.set_style(style);
}

// === Quality hues — shared by sentiment, verdict tiers, and the
// review/intervention chrome that keys off move quality. (Step 2 retunes
// these to chess.com's verdict palette.) ===
/// Best move / positive sentiment.
pub const GOOD: Color32 = Color32::from_rgb(0x2e, 0x7d, 0x32); // green
/// "Good" verdict — a notch below Best.
pub const GOOD_MUTED: Color32 = Color32::from_rgb(0x55, 0x8b, 0x2f); // olive-green
/// Inaccuracy.
pub const WARN: Color32 = Color32::from_rgb(0xf9, 0xa8, 0x25); // yellow
/// Mistake / mixed sentiment / teaching-moment chrome.
pub const CAUTION: Color32 = Color32::from_rgb(0xef, 0x6c, 0x00); // orange
/// Miss / blunder-with-a-lesson.
pub const MISS: Color32 = Color32::from_rgb(0xb3, 0x1c, 0x6a); // magenta-red
/// Blunder / negative sentiment / pause chrome.
pub const BAD: Color32 = Color32::from_rgb(0xc6, 0x28, 0x28); // red
/// Neutral / no signal.
pub const NEUTRAL: Color32 = Color32::from_rgb(0x60, 0x60, 0x60); // grey
/// Form-validation error text.
pub const ERROR: Color32 = Color32::from_rgb(0xc0, 0x40, 0x40);

// === Panel / surface accents ===
/// Backward-looking retrospective.
pub const RETRO: Color32 = Color32::from_rgb(0x51, 0x39, 0x9a); // indigo
/// Forward-looking coaching (the Hint pop-over).
pub const COACHING: Color32 = Color32::from_rgb(0x00, 0x83, 0x77); // teal
/// Game outcome + game-review surfaces.
pub const OUTCOME: Color32 = Color32::from_rgb(0xb8, 0x55, 0x00); // amber
/// Review-mode engine-PV / move-comparison box.
pub const REVIEW_PV: Color32 = Color32::from_rgb(0x37, 0x6e, 0x37); // calm green

// === Eval bar ===
pub const EVAL_WHITE: Color32 = Color32::from_rgb(0xf0, 0xf0, 0xf0);
pub const EVAL_BLACK: Color32 = Color32::from_rgb(0x30, 0x30, 0x30);
pub const EVAL_BORDER: Color32 = Color32::from_rgb(0x80, 0x80, 0x80);
pub const EVAL_TEXT_ON_LIGHT: Color32 = Color32::from_rgb(0x20, 0x20, 0x20);
pub const EVAL_TEXT_ON_DARK: Color32 = Color32::from_rgb(0xf0, 0xf0, 0xf0);

// === Board squares (opaque; the alpha tints stay in board.rs for now) ===
pub const BOARD_LIGHT: Color32 = Color32::from_rgb(0xf0, 0xd9, 0xb5);
pub const BOARD_DARK: Color32 = Color32::from_rgb(0xb5, 0x88, 0x63);

// === Eval-over-time graph (review summary) ===
pub const GRAPH_BG: Color32 = Color32::from_gray(28);
pub const GRAPH_BASELINE: Color32 = Color32::from_gray(70);
pub const GRAPH_LINE: Color32 = Color32::from_rgb(0x8a, 0xb4, 0xf8);

// === Semantic mappings ===

/// The accent color for a retrospective/coaching card's sentiment.
pub fn sentiment_color(sentiment: Sentiment) -> Color32 {
    match sentiment {
        Sentiment::Positive => GOOD,
        Sentiment::Negative => BAD,
        Sentiment::Mixed => CAUTION,
        Sentiment::Neutral => NEUTRAL,
    }
}

/// The accent color for a game-review verdict tier.
pub fn verdict_tier_color(tier: ReviewVerdictTier) -> Color32 {
    match tier {
        ReviewVerdictTier::Best => GOOD,
        ReviewVerdictTier::Good => GOOD_MUTED,
        ReviewVerdictTier::Inaccuracy => WARN,
        ReviewVerdictTier::Mistake => CAUTION,
        ReviewVerdictTier::Miss => MISS,
        ReviewVerdictTier::Blunder => BAD,
    }
}
