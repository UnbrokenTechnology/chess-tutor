use eframe::egui;

use chess_tutor_ui::event::Event;
use chess_tutor_ui::view::{
    HintPanelState, HintPanelView, MoveListView, RetrospectiveBody, RetrospectiveCategory,
    RetrospectiveHeadline, RetrospectiveItem, RetrospectiveKind, RetrospectivePanelView,
    RetrospectiveViewModel, Sentiment, SidePanelBody, SidePanelView,
};

pub(crate) fn draw(ui: &mut egui::Ui, view: &SidePanelView, events: &mut Vec<Event>) {
    ui.heading("Moves");
    ui.separator();
    let avail_h = ui.available_height();
    let move_h = (avail_h * 0.40).max(120.0);

    egui::ScrollArea::vertical()
        .id_salt("moves_scroll")
        .stick_to_bottom(view.stick_to_bottom)
        .max_height(move_h)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            draw_move_list(ui, &view.moves, events);
        });

    ui.separator();
    match &view.body {
        SidePanelBody::Hint(hint) => {
            ui.heading("Hint");
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt("hint_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    draw_hint_panel(ui, hint);
                });
        }
        SidePanelBody::Retrospective(retro) => {
            ui.heading("Retrospective");
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt("retro_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    draw_retrospective(ui, retro, events);
                });
        }
    }
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

fn draw_retrospective(
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
                RetrospectiveKind::UserMoveReady { view_model, selected_item } => {
                    draw_retrospective_cards(ui, view_model, *selected_item, events);
                }
                RetrospectiveKind::UserMoveAnalyzing => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("analyzing your move…");
                    });
                }
                RetrospectiveKind::EngineMove {
                    san,
                    eval_pawns,
                    depth,
                    elapsed_ms,
                } => {
                    ui.monospace(format!("Engine played {}", san));
                    ui.monospace(format!(
                        "eval {:+.2}    depth {}    {} ms",
                        eval_pawns, depth, elapsed_ms,
                    ));
                }
                RetrospectiveKind::EngineInfoMissing => {
                    ui.label("(engine info missing)");
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

fn category_glyph(category: RetrospectiveCategory) -> &'static str {
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
        RetrospectiveCategory::Secondary => "…",
    }
}

fn sentiment_color(sentiment: Sentiment) -> egui::Color32 {
    match sentiment {
        Sentiment::Positive => egui::Color32::from_rgb(0x2e, 0x7d, 0x32),
        Sentiment::Negative => egui::Color32::from_rgb(0xc6, 0x28, 0x28),
        Sentiment::Mixed => egui::Color32::from_rgb(0xef, 0x6c, 0x00),
        Sentiment::Neutral => egui::Color32::from_rgb(0x60, 0x60, 0x60),
    }
}

fn draw_hint_panel(ui: &mut egui::Ui, view: &HintPanelView) {
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
