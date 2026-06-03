//! Phosphor icon-font glyphs, rendered through a dedicated font family.
//!
//! Inter (our proportional UI font) maps ~745 Private-Use-Area codepoints
//! to stylistic-alternate glyphs (`G.1`, `diamondwhite_x`, …), and that
//! range overlaps Phosphor's PUA icon codepoints. Because both fonts live
//! in egui's `Proportional` family and Inter has higher priority, Inter
//! *shadows* every colliding Phosphor glyph — the disclosure caret rendered
//! as one of Inter's circled-arrow alternates instead of a triangle.
//!
//! Routing every icon through this dedicated `phosphor` family — which
//! contains only the Phosphor font — sidesteps the collision for all icons,
//! the five we happened to hit and any added later. The family is
//! registered in `main::install_fonts`.

use eframe::egui::{self, Color32, FontFamily, FontId, TextFormat};

/// The font-family key registered for the Phosphor icon font (and nothing
/// else), so Inter's PUA glyphs can never shadow an icon.
pub(crate) fn family() -> FontFamily {
    FontFamily::Name("phosphor".into())
}

/// A standalone icon as `RichText`; the caller chains `.size()` etc. The
/// colour is left unset so the host widget's own (theme / disabled) text
/// colour applies, exactly like a normal label.
pub(crate) fn icon(glyph: &str) -> egui::RichText {
    egui::RichText::new(glyph).family(family())
}

/// An icon followed by proportional text, as one `LayoutJob` — for button
/// labels that pair a glyph with a word ("▸ Show move impact"). Only the
/// glyph uses the Phosphor family; the text stays proportional (Inter).
/// Both sections use [`Color32::PLACEHOLDER`], which the host widget
/// resolves to its own text colour at paint time, so disabled-state graying
/// keeps working.
pub(crate) fn icon_label(glyph: &str, text: &str, size: f32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    job.append(
        glyph,
        0.0,
        TextFormat {
            font_id: FontId::new(size, family()),
            color: Color32::PLACEHOLDER,
            ..Default::default()
        },
    );
    job.append(
        text,
        size * 0.4, // leading gap between glyph and word
        TextFormat {
            font_id: FontId::new(size, FontFamily::Proportional),
            color: Color32::PLACEHOLDER,
            ..Default::default()
        },
    );
    job
}
