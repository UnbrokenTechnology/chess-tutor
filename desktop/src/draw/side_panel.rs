use eframe::egui;

use crate::event::Event;
use crate::view::{
    HintPanelState, HintPanelView, MoveListView, RetrospectiveBody, RetrospectiveKind,
    RetrospectivePanelView, SidePanelBody, SidePanelView,
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
                    draw_retrospective(ui, retro);
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

fn draw_retrospective(ui: &mut egui::Ui, view: &RetrospectivePanelView) {
    if let Some(end) = view.game_outcome {
        ui.colored_label(egui::Color32::from_rgb(0xb8, 0x55, 0x00), end);
        ui.separator();
    }
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
                RetrospectiveKind::UserMoveText(text) => {
                    ui.monospace(text);
                }
                RetrospectiveKind::UserMoveEmpty => {
                    ui.label("(no analysis text)");
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
