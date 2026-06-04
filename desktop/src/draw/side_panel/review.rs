//! Game-review summary widgets (verdict tallies + eval-over-time
//! graph) and the step-through review-mode nav bar (build-order
//! step 6). Pure presentation: every control emits an intent the
//! session owns; no chess logic here.
//!
//! The eval graph is hand-painted (no plotting dependency) — a single
//! polyline over a zero baseline inside a fixed-height band, since the
//! summary lives in the narrow right column.

use eframe::egui;

use crate::draw::theme;
use chess_tutor_ui::event::Event;
use chess_tutor_ui::view::{EvalSample, ReviewModeView, ReviewNav, ReviewTallyRow, ReviewVerdictTier};

/// Small chess.com-style glyph for a verdict tier. ASCII-leaning so it
/// renders on egui's default font without tofu; the colour (not the
/// glyph) carries most of the signal.
pub(super) fn verdict_tier_glyph(tier: ReviewVerdictTier) -> &'static str {
    match tier {
        ReviewVerdictTier::Best => "\u{2713}",       // ✓
        ReviewVerdictTier::Good => "\u{2022}",       // •
        ReviewVerdictTier::Inaccuracy => "?!",
        ReviewVerdictTier::Mistake => "?",
        ReviewVerdictTier::Miss => "\u{00d7}",       // ×
        ReviewVerdictTier::Blunder => "??",
    }
}

/// The verdict tally table: one row per tier, with a White and a Black
/// count column. The column for the side the student played is
/// highlighted (header marked "(you)" + bold counts). A row with no
/// moves on either side renders dimmed so the table layout stays stable.
pub(super) fn draw_verdict_tallies(
    ui: &mut egui::Ui,
    tallies: &[ReviewTallyRow],
    user_is_white: bool,
) {
    egui::Grid::new("verdict_tallies")
        .num_columns(3)
        .spacing([18.0, 4.0])
        .min_col_width(44.0)
        .show(ui, |ui| {
            // Header row: blank label cell, then the two side columns.
            ui.label("");
            draw_side_header(ui, "White", user_is_white);
            draw_side_header(ui, "Black", !user_is_white);
            ui.end_row();

            for row in tallies {
                let color = theme::verdict_tier_color(row.tier);
                let dim = row.white == 0 && row.black == 0;
                let glyph_color = if dim { color.gamma_multiply(0.4) } else { color };
                // Label cell: tier glyph + name.
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(verdict_tier_glyph(row.tier))
                            .monospace()
                            .strong()
                            .color(glyph_color),
                    );
                    ui.label(egui::RichText::new(row.label).color(glyph_color));
                });
                draw_count_cell(ui, row.white, color, user_is_white, dim);
                draw_count_cell(ui, row.black, color, !user_is_white, dim);
                ui.end_row();
            }
        });
}

/// A column header ("White" / "Black"), marked "(you)" and brightened
/// for the side the student played.
fn draw_side_header(ui: &mut egui::Ui, side: &str, is_user: bool) {
    let text = if is_user {
        format!("{side} (you)")
    } else {
        side.to_string()
    };
    ui.label(
        egui::RichText::new(text)
            .small()
            .strong()
            .color(if is_user {
                theme::TEXT
            } else {
                theme::TEXT_MUTED
            }),
    );
}

/// One per-side count cell. Zero counts dim; the student's column is
/// bold and tier-coloured, the opponent's is muted.
fn draw_count_cell(
    ui: &mut egui::Ui,
    count: usize,
    color: egui::Color32,
    is_user: bool,
    dim_row: bool,
) {
    let mut text = egui::RichText::new(count.to_string()).monospace();
    text = if count == 0 || dim_row {
        text.color(ui.visuals().weak_text_color())
    } else if is_user {
        text.strong().color(color)
    } else {
        text.color(theme::TEXT_MUTED)
    };
    ui.label(text);
}

/// Hand-painted eval-over-time line. White-advantage above the centre
/// line, black-advantage below. Fixed height; spans the column width.
pub(super) fn draw_eval_graph(ui: &mut egui::Ui, series: &[EvalSample]) {
    let height = 90.0;
    let (rect, _resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::hover());
    let painter = ui.painter_at(rect);

    // Background + zero baseline.
    painter.rect_filled(rect, 4.0, theme::GRAPH_BG);
    let mid_y = rect.center().y;
    painter.line_segment(
        [
            egui::pos2(rect.left(), mid_y),
            egui::pos2(rect.right(), mid_y),
        ],
        egui::Stroke::new(1.0, theme::GRAPH_BASELINE),
    );

    // Symmetric vertical scale clamped to the data's peak magnitude
    // (min 2 pawns so a quiet game still reads), so the curve fills the
    // band instead of hugging the baseline.
    let peak = series
        .iter()
        .map(|s| s.pawns.abs())
        .fold(2.0_f32, f32::max);
    let n = series.len().max(2);
    let x_at = |i: usize| -> f32 {
        rect.left() + (i as f32 / (n - 1) as f32) * rect.width()
    };
    let y_at = |pawns: f32| -> f32 {
        // +pawns (white better) -> up (smaller y).
        mid_y - (pawns / peak) * (rect.height() / 2.0 - 4.0)
    };

    let points: Vec<egui::Pos2> = series
        .iter()
        .enumerate()
        .map(|(i, s)| egui::pos2(x_at(i), y_at(s.pawns)))
        .collect();
    if points.len() >= 2 {
        painter.add(egui::Shape::line(
            points,
            egui::Stroke::new(1.8, theme::GRAPH_LINE),
        ));
    }
}

/// The review-mode nav bar: restart / back / forward / end, an autoplay
/// toggle, a "move N of M" readout, and a "Back to summary" affordance.
pub(super) fn draw_review_mode_bar(
    ui: &mut egui::Ui,
    view: &ReviewModeView,
    events: &mut Vec<Event>,
) {
    let accent = theme::OUTCOME; // amber, matches review header
    let bg = egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 22);
    egui::Frame::group(ui.style())
        .stroke(egui::Stroke::new(1.0, accent))
        .fill(bg)
        .inner_margin(egui::Margin::same(6.0))
        .show(ui, |ui| {
            // Just the nav buttons now — the "Move N of M" readout (the
            // move list highlights the position) and the "Summary" button
            // (moved to the action bar) were removed to recover vertical
            // space for the lesson.
            ui.horizontal(|ui| {
                // Stretch the five buttons to fill the panel width so the
                // bar reads as a single full-width control rather than a
                // narrow left-aligned cluster with dead space to its right.
                const N: f32 = 5.0;
                let spacing = ui.spacing().item_spacing.x;
                let btn_w = ((ui.available_width() - spacing * (N - 1.0)) / N).max(40.0);
                let btn = |glyph: &str| {
                    egui::Button::new(crate::draw::icon::icon(glyph).size(18.0))
                        .min_size(egui::vec2(btn_w, 36.0))
                };
                // skip-to-first
                if ui
                    .add_enabled(view.can_step_back, btn(egui_phosphor::regular::SKIP_BACK))
                    .on_hover_text("First move")
                    .clicked()
                {
                    events.push(Event::ReviewNav(ReviewNav::Restart));
                }
                // step back
                if ui
                    .add_enabled(view.can_step_back, btn(egui_phosphor::regular::CARET_LEFT))
                    .on_hover_text("Previous move")
                    .clicked()
                {
                    events.push(Event::ReviewNav(ReviewNav::Back));
                }
                // play / pause autoplay toggle
                let play_glyph = if view.autoplay {
                    egui_phosphor::regular::PAUSE
                } else {
                    egui_phosphor::regular::PLAY
                };
                if ui
                    .add_enabled(
                        view.can_step_forward || view.autoplay,
                        btn(play_glyph),
                    )
                    .on_hover_text("Autoplay")
                    .clicked()
                {
                    events.push(Event::ToggleReviewAutoplay);
                }
                // step forward
                if ui
                    .add_enabled(
                        view.can_step_forward,
                        btn(egui_phosphor::regular::CARET_RIGHT),
                    )
                    .on_hover_text("Next move")
                    .clicked()
                {
                    events.push(Event::ReviewNav(ReviewNav::Forward));
                }
                // skip-to-last
                if ui
                    .add_enabled(
                        view.can_step_forward,
                        btn(egui_phosphor::regular::SKIP_FORWARD),
                    )
                    .on_hover_text("Last move")
                    .clicked()
                {
                    events.push(Event::ReviewNav(ReviewNav::End));
                }
            });
        });

    // Drive autoplay: while running, emit a `Forward` step roughly once
    // per `AUTOPLAY_INTERVAL`. We accumulate frame dt in egui's per-id
    // memory so the cadence is wall-clock-stable regardless of frame
    // rate, and keep requesting repaints so the loop ticks even when the
    // UI is otherwise idle. The session halts autoplay at the last move,
    // so this self-limits (`can_step_forward` goes false).
    if view.autoplay && view.can_step_forward {
        const AUTOPLAY_INTERVAL: f32 = 0.9;
        let acc_id = egui::Id::new("review_autoplay_acc");
        let dt = ui.input(|i| i.unstable_dt);
        let mut acc = ui.ctx().memory(|m| m.data.get_temp(acc_id).unwrap_or(0.0_f32));
        acc += dt;
        if acc >= AUTOPLAY_INTERVAL {
            acc = 0.0;
            events.push(Event::ReviewNav(ReviewNav::Forward));
        }
        ui.ctx().memory_mut(|m| m.data.insert_temp(acc_id, acc));
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f32(AUTOPLAY_INTERVAL));
    } else {
        // Reset the accumulator so a fresh autoplay run starts clean.
        let acc_id = egui::Id::new("review_autoplay_acc");
        ui.ctx().memory_mut(|m| m.data.insert_temp(acc_id, 0.0_f32));
    }
}
