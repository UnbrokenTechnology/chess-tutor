//! Right-column rendering: the FEEDBACK zone (backward-looking
//! retrospective / review) as the primary citizen on top, a compact
//! move-list zone at the bottom.
//!
//! STEP-5 REATTACH NOTE: the always-on learning-mode preset picker and
//! the board-overlay toggle block were removed from this panel in
//! build-order step 3 (they ate permanent play-surface space). They
//! have no temporary home right now — they were *dropped*, not
//! relocated, because the ⚙ settings surface (`Event::OpenSettings`) is
//! still a no-op stub until step 5. Step 5 must rebuild both behind the
//! gear: the preset picker reads `SidePanelView.learning` and emits
//! `Event::ApplyLearningPreset` / `Event::SetRevealBestMoves`; the
//! overlay toggles read `SidePanelView.active_overlays` and emit
//! `Event::ToggleOverlay` per `OverlayKind::ALL`. Both view-model fields
//! are still populated by `build_side_panel_view`, so step 5 only needs
//! a renderer, not a session change. The prior implementations live in
//! this file's git history (`draw_learning_mode_picker` /
//! `draw_overlay_toggles`).

use eframe::egui;

use chess_tutor_ui::event::Event;
use chess_tutor_ui::view::{
    CoachingItem, CoachingPanelView, GameReviewMoment, GameReviewView, InterventionAction,
    InterventionPanelKind, InterventionPanelView, MoveListView, ReviewMomentKind, SidePanelBody,
    SidePanelView,
};

mod cards;
use cards::{category_glyph, draw_hint_panel, draw_retrospective, sentiment_color};

pub(crate) fn draw(ui: &mut egui::Ui, view: &SidePanelView, events: &mut Vec<Event>) {
    // Right column split (build-order step 3): the FEEDBACK zone is the
    // primary citizen, filling the top; the move list is a compact
    // secondary zone reserved at the bottom (above the action bar, which
    // is an outer bottom panel reserved in main.rs). The learning-mode
    // picker + overlay toggles that used to live here moved off the play
    // surface — their home is Options/⚙ (step 5); dropped from the panel
    // for now (see file-level note for what step 5 must reattach).
    egui::TopBottomPanel::bottom("move_list_zone")
        .resizable(false)
        .show_inside(ui, |ui| {
            draw_move_list_zone(ui, &view.moves, view.stick_to_bottom, events);
        });

    // Feedback zone fills the remaining space above the move list.
    match &view.body {
        SidePanelBody::Intervention(prompt) => {
            draw_panel_header(
                ui,
                "\u{23f8}",
                "Pause — on your move",
                "Your move triggered something worth a look before you continue.",
                PANEL_PAUSE,
            );
            egui::ScrollArea::vertical()
                .id_salt("intervention_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    draw_intervention_panel(ui, prompt, events);
                });
        }
        SidePanelBody::Hint(hint) => {
            draw_panel_header(
                ui,
                "\u{1f4a1}",
                "Hint — this position",
                "The engine's top moves from the position in front of you.",
                PANEL_HINT,
            );
            egui::ScrollArea::vertical()
                .id_salt("hint_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    draw_hint_panel(ui, hint);
                });
        }
        SidePanelBody::Retrospective(retro) => {
            // Backward-looking. The temporally-explicit title + distinct
            // colour is what stops the student confusing this with the
            // forward-looking coaching panel (they share this slot but
            // never render at once).
            draw_panel_header(
                ui,
                "\u{1f4cb}",
                "After your move",
                "What the move you just played changed — looking back.",
                PANEL_RETRO,
            );
            egui::ScrollArea::vertical()
                .id_salt("retro_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    draw_retrospective(ui, retro, events);
                });
        }
        SidePanelBody::Coaching(coaching) => {
            // Forward-looking — the deliberate contrast with the
            // retrospective's "After your move" header above.
            draw_panel_header(
                ui,
                "\u{1f50e}",
                "Before your move",
                "What to notice in the position in front of you — you pick the move.",
                PANEL_COACHING,
            );
            egui::ScrollArea::vertical()
                .id_salt("coaching_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    draw_coaching_panel(ui, coaching);
                });
        }
        SidePanelBody::GameReview(review) => {
            draw_panel_header(
                ui,
                "\u{1f4d6}",
                "Game review",
                "The most significant moments across the whole game.",
                PANEL_REVIEW,
            );
            egui::ScrollArea::vertical()
                .id_salt("review_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    draw_game_review(ui, review, events);
                });
        }
    }
}

// Distinct per-panel accent colours. The load-bearing pair is
// Coaching (forward-looking, teal) vs Retrospective (backward-looking,
// indigo): different enough that a glance tells the student whether a card
// is about the move they're *about* to make or the one they *just* made.
const PANEL_COACHING: egui::Color32 = egui::Color32::from_rgb(0x00, 0x83, 0x77); // teal
const PANEL_RETRO: egui::Color32 = egui::Color32::from_rgb(0x51, 0x39, 0x9a); // indigo
const PANEL_HINT: egui::Color32 = egui::Color32::from_rgb(0x15, 0x65, 0xc0); // blue
const PANEL_PAUSE: egui::Color32 = egui::Color32::from_rgb(0xc6, 0x28, 0x28); // red
const PANEL_REVIEW: egui::Color32 = egui::Color32::from_rgb(0xb8, 0x55, 0x00); // amber

/// A colour-coded, temporally-explicit banner that heads each side-panel
/// body. Replaces the bare `ui.heading(...)` so the two look-alike panels
/// (coaching vs retrospective) are instantly distinguishable.
fn draw_panel_header(
    ui: &mut egui::Ui,
    icon: &str,
    title: &str,
    subtitle: &str,
    accent: egui::Color32,
) {
    let bg = egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 38);
    egui::Frame::group(ui.style())
        .fill(bg)
        .stroke(egui::Stroke::new(1.5, accent))
        .inner_margin(egui::Margin::same(8.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(icon).size(18.0));
                ui.label(
                    egui::RichText::new(title)
                        .strong()
                        .size(16.0)
                        .color(accent),
                );
            });
            if !subtitle.is_empty() {
                ui.label(egui::RichText::new(subtitle).small().italics());
            }
        });
    ui.add_space(6.0);
}

fn draw_coaching_panel(ui: &mut egui::Ui, view: &CoachingPanelView) {
    // The colour-coded "Before your move" header (drawn by the caller)
    // already frames this panel as forward-looking and move-free, so no
    // intro line here — straight to the cards.
    if view.view_model.items.is_empty() {
        ui.label(
            egui::RichText::new(
                "Nothing jumping out positionally. Look at piece activity, pawn \
                 chains, and which squares each side wants — then pick your move.",
            )
            .italics(),
        );
        return;
    }
    // Tactical / leading cards first (non-demoted). When the
    // tactical-mode gate fired, the demoted positional cards are
    // collapsed under a muted fold below; otherwise nothing is demoted
    // and everything renders inline as before.
    for item in view.view_model.items.iter().filter(|it| !it.demoted) {
        draw_coaching_item(ui, item);
        ui.add_space(8.0);
    }
    let demoted: Vec<&CoachingItem> =
        view.view_model.items.iter().filter(|it| it.demoted).collect();
    if !demoted.is_empty() {
        ui.add_space(4.0);
        // Minimal first pass at the collapse treatment (the user will
        // refine visuals during GUI testing): a muted, default-collapsed
        // header makes clear these are not the priority right now.
        egui::CollapsingHeader::new(
            egui::RichText::new("Quiet-position notes — not the priority right now")
                .small()
                .weak(),
        )
        .default_open(false)
        .show(ui, |ui| {
            for item in demoted {
                draw_coaching_item(ui, item);
                ui.add_space(8.0);
            }
        });
    }
}

fn draw_coaching_item(ui: &mut egui::Ui, item: &CoachingItem) {
    let accent = sentiment_color(item.sentiment);
    let bg = egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 28);
    egui::Frame::group(ui.style())
        .stroke(egui::Stroke::new(1.5, accent))
        .fill(bg)
        .inner_margin(egui::Margin::same(10.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(category_glyph(item.category))
                        .size(16.0),
                );
                // Wrap the heading within the fixed-width column instead of
                // letting a long title stretch the panel (a bare label in a
                // horizontal row won't wrap).
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(&item.heading).strong().size(15.0),
                    )
                    .wrap(),
                );
            });
            // Summary keeps `.weak()` for hierarchy below the bold
            // heading but drops `.small()` — at small+weak it was a
            // washed-out 10pt grey that fought the eye.
            if !item.summary.is_empty() {
                ui.add_space(2.0);
                ui.label(egui::RichText::new(&item.summary).weak());
            }
            // Detail prose reads at default size — it's the "why" the
            // student needs to absorb. Adding a small vertical buffer
            // before the separator so it doesn't feel cramped.
            if !item.detail.is_empty() {
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(2.0);
                ui.label(&item.detail);
            }
        });
}

fn draw_game_review(
    ui: &mut egui::Ui,
    view: &GameReviewView,
    events: &mut Vec<Event>,
) {
    if let Some(end) = view.game_outcome {
        ui.colored_label(egui::Color32::from_rgb(0xb8, 0x55, 0x00), end);
        ui.separator();
    }
    ui.label(
        egui::RichText::new(format!(
            "{} of {} of your moves flagged.",
            view.moments.len(),
            view.user_move_count
        ))
        .small()
        .weak(),
    );
    if view.moments.is_empty() {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(
                "No significant moments detected. Either you played clean, the \
                 retrospective analyses haven't all arrived yet, or the gating \
                 thresholds skipped your moves. Try changing the learning mode \
                 above (Supported / Coached / All-mistakes) to widen the gate.",
            )
            .small()
            .weak(),
        );
        return;
    }
    ui.add_space(6.0);
    for moment in &view.moments {
        if draw_review_moment(ui, moment) {
            events.push(Event::JumpToReviewMoment(moment.history_index));
        }
        ui.add_space(4.0);
    }
}

fn draw_review_moment(ui: &mut egui::Ui, moment: &GameReviewMoment) -> bool {
    let accent = match moment.kind {
        ReviewMomentKind::Blunder => egui::Color32::from_rgb(0xc6, 0x28, 0x28),
        ReviewMomentKind::TeachingMoment => egui::Color32::from_rgb(0xef, 0x6c, 0x00),
        ReviewMomentKind::BlunderWithLesson => egui::Color32::from_rgb(0xb3, 0x1c, 0x6a),
    };
    let bg = egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 22);
    let frame_resp = egui::Frame::group(ui.style())
        .stroke(egui::Stroke::new(1.0, accent))
        .fill(bg)
        .inner_margin(egui::Margin::same(8.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "{}{}",
                        moment.move_pair_number,
                        if moment.side_to_move_label == "White" { "." } else { "..." }
                    ))
                    .monospace()
                    .small()
                    .weak(),
                );
                ui.label(
                    egui::RichText::new(&moment.san)
                        .monospace()
                        .strong(),
                );
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        ui.label(
                            egui::RichText::new(review_kind_label(moment.kind))
                                .small()
                                .color(accent)
                                .strong(),
                        );
                    },
                );
            });
            ui.label(egui::RichText::new(&moment.headline).small());
        });
    let rect = frame_resp.response.rect;
    ui.interact(
        rect,
        ui.id().with(("review_moment", moment.history_index)),
        egui::Sense::click(),
    )
    .clicked()
}

fn review_kind_label(kind: ReviewMomentKind) -> &'static str {
    match kind {
        ReviewMomentKind::Blunder => "BLUNDER",
        ReviewMomentKind::TeachingMoment => "LESSON",
        ReviewMomentKind::BlunderWithLesson => "BLUNDER + LESSON",
    }
}

fn draw_intervention_panel(
    ui: &mut egui::Ui,
    view: &InterventionPanelView,
    events: &mut Vec<Event>,
) {
    let accent = match view.kind {
        InterventionPanelKind::BlunderSafety => egui::Color32::from_rgb(0xc6, 0x28, 0x28),
        InterventionPanelKind::TeachingMoment => egui::Color32::from_rgb(0xef, 0x6c, 0x00),
    };
    let bg = egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 25);
    egui::Frame::group(ui.style())
        .stroke(egui::Stroke::new(2.0, accent))
        .fill(bg)
        .inner_margin(egui::Margin::same(10.0))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(&view.headline)
                    .strong()
                    .size(15.0)
                    .color(accent),
            );
            if !view.summary.is_empty() {
                ui.add_space(2.0);
                ui.label(egui::RichText::new(&view.summary).small().weak());
            }
            if let Some(concept) = &view.concept {
                ui.add_space(6.0);
                ui.separator();
                ui.label(egui::RichText::new(concept).small());
            }
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                for action in &view.actions {
                    let (label, event) = match action {
                        InterventionAction::TakeBack => match view.kind {
                            InterventionPanelKind::BlunderSafety => {
                                ("Take it back", Event::TakeBackDuringIntervention)
                            }
                            InterventionPanelKind::TeachingMoment => {
                                ("Try a different move", Event::TakeBackDuringIntervention)
                            }
                        },
                        InterventionAction::RevealConcept => {
                            ("Show me what I missed", Event::RevealMissedConcept)
                        }
                        InterventionAction::Continue => {
                            ("Continue", Event::ContinueDespitePrompt)
                        }
                    };
                    if ui.button(label).clicked() {
                        events.push(event);
                    }
                }
            });
        });
}

/// Compact secondary move-list zone at the bottom of the right column
/// (decision #5). A small "Moves" heading, then the grid in a height-
/// capped scroll area so a long game doesn't push the feedback zone
/// off-screen. Sticks to the bottom while following live play.
fn draw_move_list_zone(
    ui: &mut egui::Ui,
    view: &MoveListView,
    stick_to_bottom: bool,
    events: &mut Vec<Event>,
) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new("Moves").strong().small().weak());
    ui.separator();
    // Cap the move list to a compact band so the feedback zone above
    // keeps the lion's share of the column. 150px ≈ 8–9 move pairs;
    // older moves scroll.
    egui::ScrollArea::vertical()
        .id_salt("moves_scroll")
        .stick_to_bottom(stick_to_bottom)
        .max_height(150.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            draw_move_list(ui, view, events);
        });
    ui.add_space(4.0);
}

fn draw_move_list(ui: &mut egui::Ui, view: &MoveListView, events: &mut Vec<Event>) {
    egui::Grid::new("moves_grid")
        .num_columns(3)
        .spacing([12.0, 4.0])
        .min_col_width(30.0)
        .show(ui, |ui| {
            for row in &view.rows {
                ui.monospace(format!("{}.", row.move_pair_idx));
                if ui
                    .add(egui::SelectableLabel::new(
                        row.white.selected,
                        egui::RichText::new(&row.white.san).monospace(),
                    ))
                    .clicked()
                {
                    events.push(Event::ViewHistoryIndex(Some(row.white.history_index)));
                }
                if let Some(black) = &row.black {
                    if ui
                        .add(egui::SelectableLabel::new(
                            black.selected,
                            egui::RichText::new(&black.san).monospace(),
                        ))
                        .clicked()
                    {
                        events.push(Event::ViewHistoryIndex(Some(black.history_index)));
                    }
                } else {
                    ui.label("");
                }
                ui.end_row();
            }
        });
}

