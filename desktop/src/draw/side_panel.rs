use chess_tutor_engine::position::Position;
use chess_tutor_engine::san;
use chess_tutor_engine::types::{Color, Move, Value};
use eframe::egui;

use crate::session::App;

impl App {
    pub(crate) fn draw_side_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Moves");
        ui.separator();
        let avail_h = ui.available_height();
        let move_h = (avail_h * 0.40).max(120.0);

        egui::ScrollArea::vertical()
            .id_salt("moves_scroll")
            .stick_to_bottom(self.is_viewing_live())
            .max_height(move_h)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.draw_move_list(ui);
            });

        ui.separator();
        if self.hint_open {
            ui.heading("Hint");
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt("hint_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    self.draw_hint_panel(ui);
                });
        } else {
            ui.heading("Retrospective");
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt("retro_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    self.draw_retrospective(ui);
                });
        }
    }

    fn draw_move_list(&mut self, ui: &mut egui::Ui) {
        let viewing = self.viewing_index;
        let history_len = self.history.len();
        let mut clicked: Option<Option<usize>> = None;

        egui::Grid::new("moves_grid")
            .num_columns(3)
            .spacing([12.0, 4.0])
            .min_col_width(30.0)
            .show(ui, |ui| {
                for move_pair_idx in 0..history_len.div_ceil(2) {
                    let i_white = move_pair_idx * 2;
                    let i_black = i_white + 1;
                    ui.monospace(format!("{}.", move_pair_idx + 1));
                    let entry_w = &self.history[i_white];
                    let selected_w = viewing == Some(i_white);
                    if ui
                        .add(egui::SelectableLabel::new(
                            selected_w,
                            egui::RichText::new(&entry_w.san).monospace(),
                        ))
                        .clicked()
                    {
                        clicked = Some(Some(i_white));
                    }
                    if i_black < history_len {
                        let entry_b = &self.history[i_black];
                        let selected_b = viewing == Some(i_black);
                        if ui
                            .add(egui::SelectableLabel::new(
                                selected_b,
                                egui::RichText::new(&entry_b.san).monospace(),
                            ))
                            .clicked()
                        {
                            clicked = Some(Some(i_black));
                        }
                    } else {
                        ui.label("");
                    }
                    ui.end_row();
                }
            });

        if let Some(target) = clicked {
            // If they clicked the move that's already at the end of
            // the live timeline, treat as "back to live".
            self.viewing_index = match target {
                Some(i) if i + 1 == self.history.len() => None,
                other => other,
            };
        }
    }

    fn draw_retrospective(&self, ui: &mut egui::Ui) {
        if let Some(end) = self.game_outcome() {
            ui.colored_label(egui::Color32::from_rgb(0xb8, 0x55, 0x00), end);
            ui.separator();
        }

        let Some(entry) = self.panel_entry() else {
            ui.label("(no moves yet)");
            return;
        };

        if !self.is_viewing_live() {
            ui.weak(format!("viewing move: {}", entry.san));
            ui.separator();
        }

        let is_user = self.is_user_move(entry);
        if is_user {
            match &entry.retrospective_text {
                Some(text) if !text.is_empty() => {
                    ui.monospace(text);
                }
                Some(_) => {
                    ui.label("(no analysis text)");
                }
                None => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("analyzing your move…");
                    });
                }
            }
        } else if let Some(info) = &entry.engine_info {
            ui.monospace(format!("Engine played {}", entry.san));
            ui.monospace(format!(
                "eval {:+.2}    depth {}    {} ms",
                info.score_white_pov.0 as f32 / Value::PAWN_MG.0 as f32,
                info.depth,
                info.elapsed.as_millis(),
            ));
        } else {
            ui.label("(engine info missing)");
        }
    }

    fn draw_hint_panel(&mut self, ui: &mut egui::Ui) {
        if self.hint_thinking && self.hint_result.is_none() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("analyzing position…");
            });
            return;
        }
        let Some(result) = &self.hint_result else {
            ui.label("(no analysis yet)");
            return;
        };
        if result.analyses.is_empty() {
            ui.label("(no legal moves)");
            return;
        }

        let root_stm = result.pos.side_to_move();
        for (i, ma) in result.analyses.iter().enumerate() {
            ui.add_space(if i == 0 { 0.0 } else { 8.0 });
            let san = san::format(&result.pos, ma.mv);
            let score_str = format_score_root_pov(ma.score, root_stm);
            ui.monospace(format!(
                "{}. {}    {}    depth {}",
                i + 1,
                san,
                score_str,
                ma.depth,
            ));
            let pv_san = pv_to_san(&result.pos, &ma.pv);
            if !pv_san.is_empty() {
                let mut line = pv_san.join(" ");
                if let Some(settled) = ma.settled_ply {
                    if settled < pv_san.len() {
                        line.push_str(&format!("  [settles ply {}]", settled));
                    }
                }
                ui.indent(format!("pv_{i}"), |ui| {
                    ui.weak(egui::RichText::new(line).monospace());
                });
            }
        }
    }
}

/// Format a score for display in the hint panel. Root-stm POV (the
/// side whose turn it is) is the natural reading there: "if you play
/// this, you'll be at +0.30."
fn format_score_root_pov(score: Value, _root_stm: Color) -> String {
    if score.abs() >= Value::MATE_IN_MAX_PLY {
        if score.0 > 0 {
            format!("M{}", (Value::MATE.0 - score.0).max(1))
        } else {
            format!("-M{}", (Value::MATE.0 + score.0).max(1))
        }
    } else {
        let pawns = score.0 as f32 / Value::PAWN_MG.0 as f32;
        format!("{:+.2}", pawns)
    }
}

/// Walk a PV applying moves to a clone of `root` and producing a SAN
/// per ply. Stops on any ply that doesn't apply cleanly (shouldn't
/// happen with a real PV from the engine).
fn pv_to_san(root: &Position, pv: &[Move]) -> Vec<String> {
    let mut out = Vec::with_capacity(pv.len());
    let mut pos = root.clone();
    for mv in pv {
        out.push(san::format(&pos, *mv));
        pos.do_move(*mv);
    }
    out
}
