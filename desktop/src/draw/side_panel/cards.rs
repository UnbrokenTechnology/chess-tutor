//! Retrospective-card rendering, split out of side_panel. Includes the
//! shared `category_glyph` helper, which the hint-pop-over also calls.
//! Colors come from `draw::theme` (the single palette source).

use eframe::egui;

use crate::draw::theme;
use chess_tutor_ui::event::Event;
use chess_tutor_ui::view::{
    RetrospectiveBody, RetrospectiveCategory, RetrospectiveHeadline, RetrospectiveItem,
    RetrospectiveKind, RetrospectivePanelView, RetrospectiveViewModel,
};

pub(super) fn draw_retrospective(
    ui: &mut egui::Ui,
    view: &RetrospectivePanelView,
    in_review: bool,
    events: &mut Vec<Event>,
) {
    // In review mode the outcome line is clutter (it's shown in the
    // summary popover and the move list) — suppress it to give the lesson
    // more room.
    if !in_review {
        if let Some(end) = view.game_outcome {
            ui.colored_label(theme::OUTCOME, end);
            ui.separator();
        }
    }
    match &view.body {
        RetrospectiveBody::NoMoves => {
            ui.label("(no moves yet)");
        }
        RetrospectiveBody::Entry { viewing_back_san, kind } => {
            // The "viewing move:" status line keeps the live layout stable
            // when stepping back into history. In review mode it's
            // redundant (the move shows in the headline card and the move
            // list highlights it), so it's dropped to recover space.
            if !in_review {
                match viewing_back_san {
                    Some(san) => ui.weak(format!("viewing move: {}", san)),
                    None => ui.weak("showing the current position"),
                };
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
                    draw_retrospective_cards(
                        ui,
                        view_model,
                        *selected_item,
                        view.expanded,
                        view.show_all_signals,
                        in_review,
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
}

/// The feedback zone (decision #1): calm by default, deep on demand.
/// During live play, collapsed shows only the one-line verdict headline
/// plus a "show move impact" affordance; expanded reveals the full
/// per-term eval breakdown (the per-signal cards) in place below it. In
/// **review mode** the breakdown is the reason you're here, so it's
/// always shown with no toggle.
fn draw_retrospective_cards(
    ui: &mut egui::Ui,
    view_model: &RetrospectiveViewModel,
    selected_item: Option<usize>,
    expanded: bool,
    show_all_signals: bool,
    in_review: bool,
    events: &mut Vec<Event>,
) {
    draw_headline_card(ui, &view_model.headline);
    ui.add_space(6.0);

    let has_detail = !view_model.items.is_empty();
    // In review mode the breakdown is always on (no toggle). During live
    // play it's calm-by-default: a one-line headline plus an inline
    // affordance to expand. Disabled (greyed) when no per-term signals
    // fired, so the student isn't promised detail that isn't there.
    // Worded neutrally ("move impact", not "why this move?") so it reads
    // sensibly whether the move was best or a blunder.
    let expanded = if in_review {
        true
    } else {
        let glyph = if expanded {
            egui_phosphor::regular::CARET_DOWN
        } else {
            egui_phosphor::regular::CARET_RIGHT
        };
        let verb = if expanded { "Hide" } else { "Show" };
        let label = crate::draw::icon::icon_label(glyph, &format!("{verb} move impact"), 14.0);
        let resp = ui.add_enabled(has_detail, egui::Button::new(label).frame(false));
        if resp.clicked() {
            events.push(Event::ToggleRetrospectiveDetail);
        }
        expanded
    };
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

    // Tactical-mode demotion (the keystone teaching principle): when a
    // named tactic drove this move, positional cards are noise until the
    // tactic is resolved — so they collapse under a "Quiet-position notes"
    // divider, with the material / threat / tactic / king-safety cards kept
    // up top. Mirrors the coaching panel's gate. The renderer owns this
    // fold (HANDOFF-ux "card-fold UX"); the engine just supplies the cards.
    let tactical = view_model
        .items
        .iter()
        .any(|it| it.category == RetrospectiveCategory::Tactic);

    let mut draw_card = |ui: &mut egui::Ui, i: usize, item: &RetrospectiveItem| {
        if draw_item_card(ui, item, selected_item == Some(i)) {
            events.push(Event::SelectRetrospectiveItem(i));
        }
        ui.add_space(6.0);
    };

    if !tactical {
        for (i, item) in view_model.items.iter().enumerate() {
            draw_card(ui, i, item);
        }
        return;
    }

    // Primary (tactical / material) cards first, in their natural order.
    for (i, item) in view_model.items.iter().enumerate() {
        if !is_quiet_positional(item.category) {
            draw_card(ui, i, item);
        }
    }
    // Positional cards demoted under a collapsed divider.
    let has_quiet = view_model
        .items
        .iter()
        .any(|it| is_quiet_positional(it.category));
    if has_quiet {
        egui::CollapsingHeader::new(
            egui::RichText::new("Quiet-position notes").strong().size(14.0),
        )
        .default_open(false)
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(
                    "Positional play takes a back seat while a tactic is on the \
                     board — resolve the tactic first.",
                )
                .small()
                .color(theme::TEXT_MUTED),
            );
            ui.add_space(4.0);
            for (i, item) in view_model.items.iter().enumerate() {
                if is_quiet_positional(item.category) {
                    draw_card(ui, i, item);
                }
            }
        });
    }
}

/// Which retrospective categories are "quiet positional" — demoted under
/// the "Quiet-position notes" divider when a tactic drove the move. The
/// tactical/material cards (Material, Threats, Tactic, KingSafety,
/// Initiative) stay primary; everything structural folds away.
fn is_quiet_positional(category: RetrospectiveCategory) -> bool {
    use RetrospectiveCategory as C;
    matches!(
        category,
        C::Mobility
            | C::Space
            | C::PassedPawns
            | C::PiecePlacement
            | C::PawnStructure
            | C::BlockedCenter
            | C::Castling
            | C::Secondary
    )
}

fn draw_headline_card(ui: &mut egui::Ui, h: &RetrospectiveHeadline) {
    let accent = theme::sentiment_color(h.verdict_sentiment);
    let bg = egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 30);

    egui::Frame::group(ui.style())
        .stroke(egui::Stroke::new(1.5, accent))
        .fill(bg)
        .inner_margin(egui::Margin::same(8.0))
        .show(ui, |ui| {
            // Fill the column so the card spans the feedback zone rather
            // than shrinking to its content and leaving dead space to the
            // right.
            ui.set_min_width(ui.available_width());
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
    let accent = theme::sentiment_color(item.sentiment);
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
                ui.label(category_label(item.category, 14.0));
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
        // Phosphor checkerboard for "space" — the territory you already
        // control, a chessboard-patterned grid. A Phosphor PUA glyph, so it
        // MUST render via `category_label` (the icon family); rendering it
        // directly in a Proportional `RichText` lets Inter's PUA shadow it.
        RetrospectiveCategory::Space => egui_phosphor::regular::CHECKERBOARD,
        // Star for a named tactic (fork / pin / mate / …); distinct
        // from Threats' crossed-swords glyph so the two cards read as
        // different concepts in a glance.
        RetrospectiveCategory::Tactic => "★",
        // Flame for a sound positional sacrifice — "burning a piece to
        // light up the enemy king." Distinct from the tactic star.
        RetrospectiveCategory::PositionalWin => "♛",
        // Shield for missed prophylaxis — the defensive move you needed to
        // stop the opponent's punishing line. Reads as "defence," distinct
        // from the offensive tactic star / sacrifice queen.
        RetrospectiveCategory::MissedProphylaxis => "⛨",
        RetrospectiveCategory::Secondary => "…",
    }
}

/// True for categories whose glyph is a Phosphor icon-font codepoint (vs a
/// Unicode chess/concept glyph). These must render through the dedicated
/// icon family so Inter's overlapping PUA glyphs can't shadow them.
fn category_uses_icon_font(category: RetrospectiveCategory) -> bool {
    matches!(category, RetrospectiveCategory::Space)
}

/// A category glyph as styled `RichText` at `size` — the single renderer
/// for category glyphs (card header + hint pop-over). Unicode glyphs use
/// the proportional font; Phosphor-icon categories use the icon family.
pub(crate) fn category_label(category: RetrospectiveCategory, size: f32) -> egui::RichText {
    let glyph = category_glyph(category);
    if category_uses_icon_font(category) {
        crate::draw::icon::icon(glyph).size(size)
    } else {
        egui::RichText::new(glyph).size(size)
    }
}


