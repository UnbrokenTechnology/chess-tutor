//! The Hint pop-over (PLAN build-order step 4 / §"coaching/hint model").
//!
//! A dismissible floating panel of *what to notice* in the current
//! position — opened by the Hint button, optionally auto-opened each
//! move (auto-coach). It names patterns and squares but **never the
//! move** (the opposite of chess.com's answer-flashing Hint), and it
//! floats *over* the right-column feedback zone so the backward-looking
//! retrospective and this forward-looking coaching can coexist instead
//! of fighting for one slot.
//!
//! Content is [`chess_tutor_ui::view::HintPopoverView`] (a
//! `build_coaching_view` snapshot). Dismissing emits
//! [`chess_tutor_ui::event::Event::ToggleHint`] — the same intent the
//! Hint button toggles.

use eframe::egui;

use chess_tutor_ui::event::Event;
use chess_tutor_ui::view::{CoachingItem, HintPopoverView};

use crate::draw::side_panel::cards::{category_glyph, sentiment_color};

/// Teal accent — the forward-looking "before your move" colour, kept
/// deliberately distinct from the backward-looking retrospective's
/// indigo so a glance tells the student which way the panel points.
const POPOVER_ACCENT: egui::Color32 = egui::Color32::from_rgb(0x00, 0x83, 0x77);

/// Render the Hint pop-over as a floating, dismissible egui window
/// centred over the board area. A no-op when `view` is `None` (closed).
pub(crate) fn draw(ctx: &egui::Context, view: &HintPopoverView, events: &mut Vec<Event>) {
    let mut open = true;
    egui::Window::new(
        egui::RichText::new("\u{1f50e}  What to notice — before you move")
            .strong()
            .size(16.0)
            .color(POPOVER_ACCENT),
    )
    .id(egui::Id::new("hint_popover"))
    .collapsible(false)
    .resizable(false)
    .open(&mut open)
    .anchor(egui::Align2::CENTER_TOP, [0.0, 80.0])
    .default_width(380.0)
    .show(ctx, |ui| {
        ui.set_max_width(420.0);
        draw_body(ui, view, events);
    });

    // The window's own close (×) button cleared `open`; translate that
    // into the dismiss intent so the session flips `hint_open` off.
    if !open {
        events.push(Event::ToggleHint);
    }
}

fn draw_body(ui: &mut egui::Ui, view: &HintPopoverView, events: &mut Vec<Event>) {
    if view.view_model.items.is_empty() {
        ui.label(
            egui::RichText::new(
                "Nothing jumping out right now. Look at piece activity, pawn \
                 chains, and which squares each side wants — then pick your move.",
            )
            .italics(),
        );
    } else {
        // Cap the height so a long list scrolls rather than growing the
        // pop-over past the board.
        egui::ScrollArea::vertical()
            .id_salt("hint_popover_scroll")
            .max_height(360.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                draw_items(ui, view);
            });
    }
    ui.add_space(8.0);
    ui.separator();
    ui.vertical_centered(|ui| {
        if ui
            .button(egui::RichText::new("Got it").size(15.0).strong())
            .clicked()
        {
            events.push(Event::ToggleHint);
        }
    });
}

/// Paint the leading (non-demoted) cards, then — when the tactical-mode
/// gate fired — the demoted positional notes under a muted fold. Mirrors
/// the prior coaching-panel treatment; only the host surface changed.
fn draw_items(ui: &mut egui::Ui, view: &HintPopoverView) {
    for item in view.view_model.items.iter().filter(|it| !it.demoted) {
        draw_item(ui, item);
        ui.add_space(8.0);
    }
    let demoted: Vec<&CoachingItem> =
        view.view_model.items.iter().filter(|it| it.demoted).collect();
    if !demoted.is_empty() {
        ui.add_space(4.0);
        egui::CollapsingHeader::new(
            egui::RichText::new("Quiet-position notes — not the priority right now")
                .small()
                .weak(),
        )
        .default_open(false)
        .show(ui, |ui| {
            for item in demoted {
                draw_item(ui, item);
                ui.add_space(8.0);
            }
        });
    }
}

fn draw_item(ui: &mut egui::Ui, item: &CoachingItem) {
    let accent = sentiment_color(item.sentiment);
    let bg = egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 28);
    egui::Frame::group(ui.style())
        .stroke(egui::Stroke::new(1.5, accent))
        .fill(bg)
        .inner_margin(egui::Margin::same(10.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(category_glyph(item.category)).size(16.0));
                // Wrap the heading instead of letting a long title
                // stretch the pop-over.
                ui.add(
                    egui::Label::new(egui::RichText::new(&item.heading).strong().size(15.0)).wrap(),
                );
            });
            if !item.summary.is_empty() {
                ui.add_space(2.0);
                ui.label(egui::RichText::new(&item.summary).weak());
            }
            if !item.detail.is_empty() {
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(2.0);
                ui.label(&item.detail);
            }
        });
}
