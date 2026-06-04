//! Right-column rendering: the FEEDBACK zone (backward-looking
//! retrospective / review / intervention) as the primary citizen on
//! top, a compact move-list zone at the bottom. Forward-looking
//! coaching is NOT here — it pops over (`draw::hint_popover`) so the two
//! never fight for one slot (PLAN §"coaching/hint model").
//!
//! The always-on learning-mode picker and board-overlay toggle block
//! that build-order step 3 stripped off this panel now live in their
//! true home: the pre-game Start/Options screen (`draw::dialog`) and the
//! mid-game ⚙ settings surface (`draw::settings`), both built on the
//! shared `draw::options` widgets. The learning toggles (Support /
//! auto-coach / reveal-best-move) and the overlay toggles are edited
//! there; this panel stays purely backward-looking. `SidePanelView`
//! still carries `.learning` / `.active_overlays` for any renderer that
//! wants to *reflect* the current state inline, but the *controls* are
//! off the play surface (decision #2).

use eframe::egui;

use crate::draw::theme;
use chess_tutor_ui::event::Event;
use chess_tutor_ui::view::{
    GameReviewView, InterventionAction, InterventionPanelKind, InterventionPanelView, MoveListView,
    ReviewVerdictTier, SidePanelBody, SidePanelView,
};

pub(crate) mod cards;
use cards::draw_retrospective;

mod review;
use review::{draw_eval_graph, draw_review_mode_bar, draw_verdict_tallies, verdict_tier_glyph};

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

    // Review-mode nav bar (step 6): big step-through controls pinned
    // just below the title, above the feedback zone. Only present while
    // the session is in `Reviewing`.
    if let Some(review) = &view.review_mode {
        draw_review_mode_bar(ui, review, events);
        ui.add_space(4.0);
    }

    // Feedback zone fills the remaining space above the move list.
    match &view.body {
        SidePanelBody::Intervention(prompt) => {
            draw_panel_header(
                ui,
                egui_phosphor::regular::PAUSE,
                "Pause — on your move",
                "Your move triggered something worth a look before you continue.",
                theme::BAD,
            );
            egui::ScrollArea::vertical()
                .id_salt("intervention_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    draw_intervention_panel(ui, prompt, events);
                });
        }
        SidePanelBody::Retrospective(retro) => {
            // Backward-looking. The temporally-explicit title + distinct
            // colour is what stops the student confusing this with the
            // forward-looking coaching panel (they share this slot but
            // never render at once). In review mode the header is dropped:
            // the nav bar already frames the surface as review, "After
            // your move" is self-evident when stepping moves, and the
            // recovered vertical space goes to the lesson.
            let in_review = view.review_mode.is_some();
            if !in_review {
                draw_panel_header(
                    ui,
                    egui_phosphor::regular::CLOCK_COUNTER_CLOCKWISE,
                    "After your move",
                    "What the move you just played changed — looking back.",
                    theme::RETRO,
                );
            }
            egui::ScrollArea::vertical()
                .id_salt("retro_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    draw_retrospective(ui, retro, in_review, events);
                });
        }
    }
}

/// A colour-coded, temporally-explicit *title* for each side-panel body.
/// Styled as a section heading (accent glyph + title, subtitle, rule) —
/// deliberately NOT a bordered/filled card, since it labels the zone
/// rather than being content within it.
fn draw_panel_header(
    ui: &mut egui::Ui,
    icon: &str,
    title: &str,
    subtitle: &str,
    accent: egui::Color32,
) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        if !icon.is_empty() {
            ui.label(crate::draw::icon::icon(icon).size(16.0).color(accent));
        }
        ui.label(
            egui::RichText::new(title)
                .strong()
                .size(17.0)
                .color(accent),
        );
    });
    if !subtitle.is_empty() {
        // Explanatory zone subtitle — keep it at the muted-but-legible
        // token rather than `.weak()`, which fades too far for this
        // body-length text the student is meant to read.
        ui.label(egui::RichText::new(subtitle).small().color(theme::TEXT_MUTED));
    }
    ui.add_space(3.0);
    ui.separator();
    ui.add_space(4.0);
}

/// The game-review **summary** as an on-demand modal popover: outcome
/// line, the White/Black verdict table, and the eval-over-time graph.
/// Floats over the board (opened from the action-bar "Summary" button)
/// so the step-through panel stays put. The window's close button
/// dismisses it without leaving review.
///
/// No significant-moments list: the move list already flags those moves
/// (red rating glyph), so the old list was redundant and lived in a
/// cramped scroll area. Navigation is via the move list and the nav bar.
pub(crate) fn draw_summary_modal(
    ctx: &egui::Context,
    view: &GameReviewView,
    events: &mut Vec<Event>,
) {
    let mut open = true;
    egui::Window::new(crate::draw::icon::icon_label(
        egui_phosphor::regular::CLIPBOARD_TEXT,
        "Game review",
        16.0,
    ))
    .collapsible(false)
    .resizable(false)
    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
    .default_width(440.0)
    .open(&mut open)
    .show(ctx, |ui| {
        if let Some(end) = view.game_outcome {
            ui.colored_label(theme::OUTCOME, end);
            ui.separator();
        }

        // White/Black verdict table (Best → Blunder), the student's
        // column highlighted.
        draw_verdict_tallies(ui, &view.tallies, view.user_is_white);

        // Eval-over-time graph.
        if view.eval_series.len() >= 2 {
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Evaluation over time").small().weak());
            ui.add_space(2.0);
            draw_eval_graph(ui, &view.eval_series);
        }
    });
    if !open {
        events.push(Event::CloseReviewSummary);
    }
}

fn draw_intervention_panel(
    ui: &mut egui::Ui,
    view: &InterventionPanelView,
    events: &mut Vec<Event>,
) {
    let accent = match view.kind {
        InterventionPanelKind::BlunderSafety => theme::BAD,
        InterventionPanelKind::TeachingMoment => theme::CAUTION,
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

/// One move-list cell: the SAN as a selectable label, prefixed in
/// review mode by the small chess.com-style rating glyph. Returns
/// `true` when clicked. The glyph and the label share one cell so the
/// grid column count is unchanged.
fn draw_move_cell(
    ui: &mut egui::Ui,
    san: &str,
    selected: bool,
    rating: Option<ReviewVerdictTier>,
) -> bool {
    let mut text = egui::RichText::new(san).monospace();
    if let Some(tier) = rating {
        // Prepend the glyph + colour the whole cell by tier so the
        // rating reads at a glance without widening the column.
        text = egui::RichText::new(format!("{} {}", verdict_tier_glyph(tier), san))
            .monospace()
            .color(theme::verdict_tier_color(tier));
    }
    ui.add(egui::SelectableLabel::new(selected, text)).clicked()
}

fn draw_move_list(ui: &mut egui::Ui, view: &MoveListView, events: &mut Vec<Event>) {
    egui::Grid::new("moves_grid")
        .num_columns(3)
        .spacing([12.0, 4.0])
        .min_col_width(30.0)
        .show(ui, |ui| {
            for row in &view.rows {
                ui.monospace(format!("{}.", row.move_pair_idx));
                if draw_move_cell(ui, &row.white.san, row.white.selected, row.white.rating) {
                    events.push(Event::ViewHistoryIndex(Some(row.white.history_index)));
                }
                if let Some(black) = &row.black {
                    if draw_move_cell(ui, &black.san, black.selected, black.rating) {
                        events.push(Event::ViewHistoryIndex(Some(black.history_index)));
                    }
                } else {
                    ui.label("");
                }
                ui.end_row();
            }
        });
}

