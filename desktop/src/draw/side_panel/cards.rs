//! Retrospective-card and hint-panel rendering, split out of side_panel.
//! Includes the shared category_glyph / sentiment_color helpers, which the
//! coaching/review cards in the parent module also call (via super::).

use eframe::egui;

use chess_tutor_ui::event::Event;
use chess_tutor_ui::view::{
    HintPanelState, HintPanelView, RetrospectiveBody, RetrospectiveCategory,
    RetrospectiveHeadline, RetrospectiveItem, RetrospectiveKind, RetrospectivePanelView,
    RetrospectiveViewModel, Sentiment,
};

pub(super) fn draw_retrospective(
    ui: &mut egui::Ui,
    view: &RetrospectivePanelView,
    events: &mut Vec<Event>,
) {
    if let Some(end) = view.game_outcome {
        ui.colored_label(egui::Color32::from_rgb(0xb8, 0x55, 0x00), end);
        ui.separator();
    }
    let mut show_all = view.show_all_signals;
    if ui
        .checkbox(&mut show_all, "Show all signals")
        .on_hover_text(
            "Show every per-piece-type mobility shift and every \
             residual term in \"Other shifts\", instead of just the \
             largest movers.",
        )
        .changed()
    {
        events.push(Event::ToggleShowAllSignals);
    }
    ui.separator();
    match &view.body {
        RetrospectiveBody::NoMoves => {
            ui.label("(no moves yet)");
        }
        RetrospectiveBody::Entry { viewing_back_san, kind } => {
            if let Some(san) = viewing_back_san {
                ui.weak(format!("viewing move: {}", san));
                ui.separator();
            }
            match kind {
                RetrospectiveKind::MoveReady {
                    view_model,
                    selected_item,
                    ..
                } => {
                    // Same cards regardless of who moved — the perspective
                    // ("you" vs "they") is already baked into the prose by
                    // the teaching translator.
                    draw_retrospective_cards(ui, view_model, *selected_item, events);
                }
                RetrospectiveKind::Analyzing => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("analyzing the move…");
                    });
                }
            }
        }
    }
}

fn draw_retrospective_cards(
    ui: &mut egui::Ui,
    view_model: &RetrospectiveViewModel,
    selected_item: Option<usize>,
    events: &mut Vec<Event>,
) {
    draw_headline_card(ui, &view_model.headline);
    ui.add_space(8.0);
    for (i, item) in view_model.items.iter().enumerate() {
        let is_selected = selected_item == Some(i);
        if draw_item_card(ui, item, is_selected) {
            events.push(Event::SelectRetrospectiveItem(i));
        }
        ui.add_space(6.0);
    }
    if view_model.items.is_empty() {
        ui.weak("(no detailed signals fired for this move)");
    }
}

fn draw_headline_card(ui: &mut egui::Ui, h: &RetrospectiveHeadline) {
    let accent = sentiment_color(h.verdict_sentiment);
    let bg = egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 30);

    egui::Frame::group(ui.style())
        .stroke(egui::Stroke::new(1.5, accent))
        .fill(bg)
        .inner_margin(egui::Margin::same(8.0))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new(format!("{}{}", h.user_san, h.san_annotation))
                        .monospace()
                        .strong()
                        .size(16.0),
                );
                ui.label(
                    egui::RichText::new(format!("— {}", h.verdict_label))
                        .color(accent)
                        .strong(),
                );
                ui.label(
                    egui::RichText::new(format!("({})", h.user_score))
                        .monospace()
                        .weak(),
                );
            });
            if let (Some(best_san), Some(best_score)) = (&h.best_san, &h.best_score) {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        egui::RichText::new("Engine preferred")
                            .small()
                            .weak(),
                    );
                    ui.label(
                        egui::RichText::new(best_san)
                            .monospace()
                            .strong(),
                    );
                    ui.label(
                        egui::RichText::new(format!("({})", best_score))
                            .monospace()
                            .weak(),
                    );
                    if let Some(gap) = &h.gap {
                        ui.label(
                            egui::RichText::new(format!("[Δ {}]", gap))
                                .monospace()
                                .small()
                                .weak(),
                        );
                    }
                });
            }
            if let Some(note) = &h.note {
                ui.label(egui::RichText::new(note).italics().small());
            }
        });
}

fn draw_item_card(
    ui: &mut egui::Ui,
    item: &RetrospectiveItem,
    is_selected: bool,
) -> bool {
    let accent = sentiment_color(item.sentiment);
    let bg = if is_selected {
        egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 60)
    } else {
        egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 18)
    };
    let stroke_width = if is_selected { 2.0 } else { 1.0 };

    let frame_resp = egui::Frame::group(ui.style())
        .stroke(egui::Stroke::new(stroke_width, accent))
        .fill(bg)
        .inner_margin(egui::Margin::same(8.0))
        .show(ui, |ui| {
            // Header row: category glyph + heading + optional delta chip.
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(category_glyph(item.category))
                        .size(14.0),
                );
                ui.label(
                    egui::RichText::new(&item.heading)
                        .strong()
                        .size(14.0),
                );
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if let Some(delta) = item.score_delta_pawns {
                            let sign = if delta >= 0.0 { "+" } else { "" };
                            ui.label(
                                egui::RichText::new(format!("{sign}{:.2}", delta))
                                    .monospace()
                                    .small()
                                    .color(accent)
                                    .strong(),
                            );
                        }
                    },
                );
            });
            if !item.summary.is_empty() {
                ui.label(egui::RichText::new(&item.summary).weak().small());
            }
            if is_selected && !item.detail.is_empty() {
                ui.add_space(4.0);
                ui.separator();
                ui.label(egui::RichText::new(&item.detail).small());
            }
        });

    // The whole frame is the click target. Interact at the response
    // rect to capture clicks anywhere on the card.
    let card_rect = frame_resp.response.rect;
    ui.interact(card_rect, ui.id().with(&item.heading), egui::Sense::click())
        .clicked()
}

pub(super) fn category_glyph(category: RetrospectiveCategory) -> &'static str {
    match category {
        RetrospectiveCategory::Material => "♟",
        RetrospectiveCategory::Threats => "⚔",
        RetrospectiveCategory::KingSafety => "♚",
        RetrospectiveCategory::PawnStructure => "⛯",
        RetrospectiveCategory::Mobility => "↔",
        RetrospectiveCategory::PassedPawns => "↑",
        RetrospectiveCategory::PiecePlacement => "◈",
        RetrospectiveCategory::Initiative => "⚡",
        RetrospectiveCategory::BlockedCenter => "▦",
        RetrospectiveCategory::Castling => "🏰",
        RetrospectiveCategory::Space => "◫",
        // Star for a named tactic (fork / pin / mate / …); distinct
        // from Threats' crossed-swords glyph so the two cards read as
        // different concepts in a glance.
        RetrospectiveCategory::Tactic => "★",
        RetrospectiveCategory::Secondary => "…",
    }
}

pub(super) fn sentiment_color(sentiment: Sentiment) -> egui::Color32 {
    match sentiment {
        Sentiment::Positive => egui::Color32::from_rgb(0x2e, 0x7d, 0x32),
        Sentiment::Negative => egui::Color32::from_rgb(0xc6, 0x28, 0x28),
        Sentiment::Mixed => egui::Color32::from_rgb(0xef, 0x6c, 0x00),
        Sentiment::Neutral => egui::Color32::from_rgb(0x60, 0x60, 0x60),
    }
}

pub(super) fn draw_hint_panel(ui: &mut egui::Ui, view: &HintPanelView) {
    match &view.state {
        HintPanelState::Loading => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("analyzing position…");
            });
        }
        HintPanelState::NoResult => {
            ui.label("(no analysis yet)");
        }
        HintPanelState::NoMoves => {
            ui.label("(no legal moves)");
        }
        HintPanelState::Ready(entries) => {
            for (i, e) in entries.iter().enumerate() {
                ui.add_space(if i == 0 { 0.0 } else { 8.0 });
                ui.monospace(format!(
                    "{}. {}    {}    depth {}",
                    i + 1,
                    e.san,
                    e.score_str,
                    e.depth,
                ));
                if !e.pv_san.is_empty() {
                    let mut line = e.pv_san.join(" ");
                    if let Some(settled) = e.settle_marker {
                        line.push_str(&format!("  [settles ply {}]", settled));
                    }
                    ui.indent(format!("pv_{i}"), |ui| {
                        ui.weak(egui::RichText::new(line).monospace());
                    });
                }
            }
        }
    }
}
