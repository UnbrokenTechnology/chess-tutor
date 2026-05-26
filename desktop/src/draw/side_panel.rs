use eframe::egui;

use std::collections::HashSet;

use chess_tutor_ui::event::Event;
use chess_tutor_ui::learning_mode::{
    AssistanceLevel, BlunderSafety, LearningPreferences, LearningPreset, MistakeHandling,
};
use chess_tutor_ui::view::{
    CoachingItem, CoachingPanelView, GameReviewMoment, GameReviewView, HintPanelState,
    HintPanelView, InterventionAction, InterventionPanelKind, InterventionPanelView, MoveListView,
    OverlayKind, RetrospectiveBody, RetrospectiveCategory, RetrospectiveHeadline,
    RetrospectiveItem, RetrospectiveKind, RetrospectivePanelView, RetrospectiveViewModel,
    ReviewMomentKind, Sentiment, SidePanelBody, SidePanelView,
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
    draw_learning_mode_picker(ui, &view.learning, events);
    ui.separator();
    draw_overlay_toggles(ui, &view.active_overlays, events);
    ui.separator();
    match &view.body {
        SidePanelBody::Intervention(prompt) => {
            ui.heading("Pause");
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt("intervention_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    draw_intervention_panel(ui, prompt, events);
                });
        }
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
        SidePanelBody::Coaching(coaching) => {
            ui.heading("Coaching");
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt("coaching_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    draw_coaching_panel(ui, coaching);
                });
        }
        SidePanelBody::GameReview(review) => {
            ui.heading("Game Review");
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt("review_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    draw_game_review(ui, review, events);
                });
        }
    }
}

fn draw_coaching_panel(ui: &mut egui::Ui, view: &CoachingPanelView) {
    // Intro: italics keeps it visually subordinate to cards without
    // dropping to a near-illegible small/weak combo. The whole panel
    // *is* the value here — the student should be able to read every
    // word of it without squinting.
    ui.label(
        egui::RichText::new(
            "Features to notice in this position. No move recommendations — \
             that's your decision.",
        )
        .italics(),
    );
    ui.add_space(8.0);
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
    for item in &view.view_model.items {
        draw_coaching_item(ui, item);
        ui.add_space(8.0);
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
                ui.label(
                    egui::RichText::new(&item.heading)
                        .strong()
                        .size(15.0),
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

fn draw_learning_mode_picker(
    ui: &mut egui::Ui,
    learning: &LearningPreferences,
    events: &mut Vec<Event>,
) {
    let current_preset = preset_of(learning);
    egui::CollapsingHeader::new(format!("Learning: {}", preset_label(current_preset)))
        .default_open(false)
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(
                    "How much help you want during play. Changes apply to the next move.",
                )
                .small()
                .weak(),
            );
            ui.add_space(4.0);
            for preset in [
                LearningPreset::Practicing,
                LearningPreset::Supported,
                LearningPreset::Coached,
            ] {
                let mut selected = current_preset == preset;
                let resp = ui
                    .radio_value(&mut selected, true, preset_label(preset))
                    .on_hover_text(preset_description(preset));
                if resp.clicked() && current_preset != preset {
                    events.push(Event::ApplyLearningPreset(preset));
                }
            }
            if matches!(current_preset, LearningPreset::Custom) {
                ui.label(egui::RichText::new("(custom)").small().weak());
            }
            ui.add_space(6.0);
            let mut reveal = learning.reveal_best_moves;
            let resp = ui
                .checkbox(&mut reveal, "Reveal engine's preferred moves in retrospective")
                .on_hover_text(
                    "Off by default. When on, the retrospective shows the engine's \
                     top choice as a SAN tag and an on-board arrow. Off keeps the \
                     focus on *why* your move was inaccurate without giving away \
                     the answer.",
                );
            if resp.changed() {
                events.push(Event::SetRevealBestMoves(reveal));
            }
        });
}

fn preset_of(prefs: &LearningPreferences) -> LearningPreset {
    let matches_pres = |p: LearningPreset| -> bool {
        let candidate = p.to_preferences();
        prefs.assistance == candidate.assistance
            && prefs.mistake_handling == candidate.mistake_handling
            && prefs.blunder_safety == candidate.blunder_safety
            && prefs.reveal_best_moves == candidate.reveal_best_moves
    };
    if matches_pres(LearningPreset::Practicing) {
        LearningPreset::Practicing
    } else if matches_pres(LearningPreset::Supported) {
        LearningPreset::Supported
    } else if matches_pres(LearningPreset::Coached) {
        LearningPreset::Coached
    } else {
        LearningPreset::Custom
    }
}

fn preset_label(preset: LearningPreset) -> &'static str {
    match preset {
        LearningPreset::Practicing => "Practicing",
        LearningPreset::Supported => "Supported",
        LearningPreset::Coached => "Coached",
        LearningPreset::Custom => "Custom",
    }
}

fn preset_description(preset: LearningPreset) -> &'static str {
    match preset {
        LearningPreset::Practicing => {
            "Silent during play; full retrospective after each move. The strongest \
             mode for transfer to your own games — you make the decisions, then \
             you study what happened."
        }
        LearningPreset::Supported => {
            "Same as Practicing, plus: pauses on detected teaching moments (one \
             concept dominated the swing) and offers a takeback after blunders. \
             Doesn't pause for every non-best move."
        }
        LearningPreset::Coached => {
            "Adds live coaching overlays (features named, never moves) on top of \
             Supported. The most-help mode short of revealing engine moves."
        }
        LearningPreset::Custom => "Custom-tuned combination of the axes.",
    }
}

// Silence unused-import warning until per-axis editor lands; these are
// re-exposed for the upcoming custom-preference UI.
#[allow(dead_code)]
fn _learning_axis_types_in_use() {
    let _: AssistanceLevel = AssistanceLevel::default();
    let _: MistakeHandling = MistakeHandling::default();
    let _: BlunderSafety = BlunderSafety::default();
}

fn draw_overlay_toggles(
    ui: &mut egui::Ui,
    active: &HashSet<OverlayKind>,
    events: &mut Vec<Event>,
) {
    egui::CollapsingHeader::new("Board overlays")
        .default_open(false)
        .show(ui, |ui| {
            for kind in OverlayKind::ALL {
                let mut on = active.contains(&kind);
                let resp = ui
                    .checkbox(&mut on, kind.label())
                    .on_hover_text(kind.description());
                if resp.changed() {
                    events.push(Event::ToggleOverlay(kind));
                }
            }
        });
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
