//! Game-review summary widgets (verdict tallies + eval-over-time
//! graph) and the step-through review-mode nav bar (build-order
//! step 6). Pure presentation: every control emits an intent the
//! session owns; no chess logic here.
//!
//! The eval graph is hand-painted (no plotting dependency) — a single
//! polyline over a zero baseline inside a fixed-height band, since the
//! summary lives in the narrow right column.

use eframe::egui;

use chess_tutor_ui::event::Event;
use chess_tutor_ui::view::{EvalSample, ReviewModeView, ReviewNav, ReviewTally, ReviewVerdictTier};

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

/// Per-tier accent colour. Layout-pass only — these reuse the
/// retrospective sentiment hues so the redesign's deliberate colour
/// pass can retune them in one place later.
pub(super) fn verdict_tier_color(tier: ReviewVerdictTier) -> egui::Color32 {
    match tier {
        ReviewVerdictTier::Best => egui::Color32::from_rgb(0x2e, 0x7d, 0x32),
        ReviewVerdictTier::Good => egui::Color32::from_rgb(0x55, 0x8b, 0x2f),
        ReviewVerdictTier::Inaccuracy => egui::Color32::from_rgb(0xf9, 0xa8, 0x25),
        ReviewVerdictTier::Mistake => egui::Color32::from_rgb(0xef, 0x6c, 0x00),
        ReviewVerdictTier::Miss => egui::Color32::from_rgb(0xb3, 0x1c, 0x6a),
        ReviewVerdictTier::Blunder => egui::Color32::from_rgb(0xc6, 0x28, 0x28),
    }
}

/// The verdict tally block: one chip per tier with its count. Zero-count
/// tiers render dimmed so the row layout stays stable.
pub(super) fn draw_verdict_tallies(
    ui: &mut egui::Ui,
    tallies: &[ReviewTally],
    user_move_count: usize,
) {
    ui.label(
        egui::RichText::new(format!("{user_move_count} of your moves"))
            .small()
            .weak(),
    );
    ui.add_space(4.0);
    egui::Grid::new("verdict_tallies")
        .num_columns(2)
        .spacing([10.0, 3.0])
        .show(ui, |ui| {
            for tally in tallies {
                let color = verdict_tier_color(tally.tier);
                let dim = tally.count == 0;
                let glyph = egui::RichText::new(verdict_tier_glyph(tally.tier))
                    .monospace()
                    .strong()
                    .color(if dim { color.gamma_multiply(0.4) } else { color });
                let label = egui::RichText::new(format!("{} {}", tally.label, tally.count))
                    .strong()
                    .color(if dim {
                        ui.visuals().weak_text_color()
                    } else {
                        color
                    });
                ui.label(glyph);
                ui.label(label);
                ui.end_row();
            }
        });
}

/// Hand-painted eval-over-time line. White-advantage above the centre
/// line, black-advantage below. Fixed height; spans the column width.
pub(super) fn draw_eval_graph(ui: &mut egui::Ui, series: &[EvalSample]) {
    let height = 90.0;
    let (rect, _resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::hover());
    let painter = ui.painter_at(rect);

    // Background + zero baseline.
    painter.rect_filled(rect, 4.0, egui::Color32::from_gray(28));
    let mid_y = rect.center().y;
    painter.line_segment(
        [
            egui::pos2(rect.left(), mid_y),
            egui::pos2(rect.right(), mid_y),
        ],
        egui::Stroke::new(1.0, egui::Color32::from_gray(70)),
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
            egui::Stroke::new(1.8, egui::Color32::from_rgb(0x8a, 0xb4, 0xf8)),
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
    let accent = egui::Color32::from_rgb(0xb8, 0x55, 0x00); // amber, matches review header
    let bg = egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 22);
    egui::Frame::group(ui.style())
        .stroke(egui::Stroke::new(1.0, accent))
        .fill(bg)
        .inner_margin(egui::Margin::same(6.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "Move {} of {}",
                        view.current_ply, view.total_plies
                    ))
                    .small()
                    .strong(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button(egui::RichText::new("Summary").small())
                        .on_hover_text("Back to the game-review summary")
                        .clicked()
                    {
                        events.push(Event::OpenGameReview);
                    }
                });
            });
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                let btn = |label: &str| {
                    egui::Button::new(egui::RichText::new(label).size(18.0))
                        .min_size(egui::vec2(40.0, 36.0))
                };
                // ⏮ restart
                if ui
                    .add_enabled(view.can_step_back, btn("\u{23ee}"))
                    .on_hover_text("First move")
                    .clicked()
                {
                    events.push(Event::ReviewNav(ReviewNav::Restart));
                }
                // ◀ back
                if ui
                    .add_enabled(view.can_step_back, btn("\u{25c0}"))
                    .on_hover_text("Previous move")
                    .clicked()
                {
                    events.push(Event::ReviewNav(ReviewNav::Back));
                }
                // ▶ / ⏸ autoplay toggle
                let play_glyph = if view.autoplay { "\u{23f8}" } else { "\u{25b6}" };
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
                // ▷ forward
                if ui
                    .add_enabled(view.can_step_forward, btn("\u{25b6}\u{25b6}"))
                    .on_hover_text("Next move")
                    .clicked()
                {
                    events.push(Event::ReviewNav(ReviewNav::Forward));
                }
                // ⏭ end
                if ui
                    .add_enabled(view.can_step_forward, btn("\u{23ed}"))
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
