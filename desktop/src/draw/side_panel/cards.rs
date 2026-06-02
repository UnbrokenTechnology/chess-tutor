//! Retrospective-card rendering, split out of side_panel. Includes the
//! shared category_glyph / sentiment_color helpers, which the hint-pop-over
//! and review cards (in sibling modules) also call.

use eframe::egui;

use chess_tutor_ui::event::Event;
use chess_tutor_ui::view::{
    RetrospectiveBody, RetrospectiveCategory, RetrospectiveHeadline, RetrospectiveItem,
    RetrospectiveKind, RetrospectivePanelView, ReviewPvLine, RetrospectiveViewModel, Sentiment,
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
    match &view.body {
        RetrospectiveBody::NoMoves => {
            ui.label("(no moves yet)");
        }
        RetrospectiveBody::Entry { viewing_back_san, kind } => {
            // Always render this status line (even live) so the layout
            // doesn't shift when you step back into history and a
            // "viewing move:" line appears/disappears.
            match viewing_back_san {
                Some(san) => ui.weak(format!("viewing move: {}", san)),
                None => ui.weak("showing the current position"),
            };
            ui.separator();
            match kind {
                RetrospectiveKind::MoveReady {
                    view_model,
                    selected_item,
                    ..
                } => {
                    // Same cards regardless of who moved — the perspective
                    // ("you" vs "they") is already baked into the prose by
                    // the teaching translator.
                    draw_retrospective_cards(
                        ui,
                        view_model,
                        *selected_item,
                        view.expanded,
                        view.show_all_signals,
                        events,
                    );
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

    // Engine PV / move-vs-move comparison — review-mode only (the
    // session leaves `review_pv == None` during live play, decision #9).
    if let Some(pv) = &view.review_pv {
        ui.add_space(8.0);
        draw_review_pv(ui, pv);
    }
}

/// The review-only move-vs-move comparison: the user's move beside the
/// engine's best line. The answer key chess.com shows in review and we
/// deliberately withhold during play.
fn draw_review_pv(ui: &mut egui::Ui, pv: &ReviewPvLine) {
    let accent = egui::Color32::from_rgb(0x37, 0x6e, 0x37); // calm green
    let bg = egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 24);
    egui::Frame::group(ui.style())
        .stroke(egui::Stroke::new(1.0, accent))
        .fill(bg)
        .inner_margin(egui::Margin::same(8.0))
        .show(ui, |ui| {
            if pv.user_san == pv.best_san {
                ui.label(
                    egui::RichText::new("You found the engine's move.")
                        .strong()
                        .color(accent),
                );
            } else {
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new("You played").small().weak());
                    ui.label(egui::RichText::new(&pv.user_san).monospace().strong());
                });
            }
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new("Engine line").small().weak());
                ui.label(
                    egui::RichText::new(pv.best_line.join(" "))
                        .monospace()
                        .strong(),
                );
            });
        });
}

/// The feedback zone (decision #1): calm by default, deep on demand.
/// Collapsed shows only the one-line verdict headline plus a "why this
/// move?" affordance; expanded reveals the full per-term eval breakdown
/// (the per-signal cards) in place below it.
fn draw_retrospective_cards(
    ui: &mut egui::Ui,
    view_model: &RetrospectiveViewModel,
    selected_item: Option<usize>,
    expanded: bool,
    show_all_signals: bool,
    events: &mut Vec<Event>,
) {
    draw_headline_card(ui, &view_model.headline);
    ui.add_space(6.0);

    let has_detail = !view_model.items.is_empty();
    // Inline affordance that swaps the calm one-liner for the full
    // breakdown. Disabled (greyed) when no per-term signals fired, so the
    // student isn't promised detail that isn't there. Worded neutrally
    // ("move impact", not "why this move?") so it reads sensibly whether
    // the move was best or a blunder.
    let glyph = if expanded { "\u{25be}" } else { "\u{25b8}" }; // ▾ / ▸
    let verb = if expanded { "Hide" } else { "Show" };
    let label = egui::RichText::new(format!("{glyph} {verb} move impact"))
        .strong()
        .size(14.0);
    let resp = ui.add_enabled(has_detail, egui::Button::new(label).frame(false));
    if resp.clicked() {
        events.push(Event::ToggleRetrospectiveDetail);
    }
    if !has_detail {
        ui.add_space(2.0);
        ui.weak(
            egui::RichText::new("(no detailed signals fired for this move)").small(),
        );
        return;
    }
    if !expanded {
        return;
    }

    ui.add_space(6.0);
    // The "show all signals" depth toggle lives with the breakdown it
    // controls — only meaningful once the breakdown is on screen.
    let mut show_all = show_all_signals;
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
    ui.add_space(4.0);
    for (i, item) in view_model.items.iter().enumerate() {
        let is_selected = selected_item == Some(i);
        if draw_item_card(ui, item, is_selected) {
            events.push(Event::SelectRetrospectiveItem(i));
        }
        ui.add_space(6.0);
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
            // Reserve the delta chip flush-right first, then let the
            // heading wrap into the remaining width. A bare label in a
            // horizontal row doesn't wrap, so without this a long heading
            // stretches the (fixed-width) panel and shrinks the board.
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(category_glyph(item.category))
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
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&item.heading).strong().size(14.0),
                            )
                            .wrap(),
                        );
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

pub(crate) fn category_glyph(category: RetrospectiveCategory) -> &'static str {
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
        // ⌂ (house) instead of the 🏰 emoji, which renders as tofu.
        RetrospectiveCategory::Castling => "⌂",
        RetrospectiveCategory::Space => "◫",
        // Star for a named tactic (fork / pin / mate / …); distinct
        // from Threats' crossed-swords glyph so the two cards read as
        // different concepts in a glance.
        RetrospectiveCategory::Tactic => "★",
        RetrospectiveCategory::Secondary => "…",
    }
}

pub(crate) fn sentiment_color(sentiment: Sentiment) -> egui::Color32 {
    match sentiment {
        Sentiment::Positive => egui::Color32::from_rgb(0x2e, 0x7d, 0x32),
        Sentiment::Negative => egui::Color32::from_rgb(0xc6, 0x28, 0x28),
        Sentiment::Mixed => egui::Color32::from_rgb(0xef, 0x6c, 0x00),
        Sentiment::Neutral => egui::Color32::from_rgb(0x60, 0x60, 0x60),
    }
}

