use chess_tutor_engine::opponent::{EvalCategory, EvalMask, NoiseProfile};
use eframe::egui;

use chess_tutor_ui::event::Event;
use chess_tutor_ui::session::ColorChoice;
use chess_tutor_ui::view::NewGameDialogView;

pub(crate) fn draw(
    ctx: &egui::Context,
    view: NewGameDialogView<'_>,
    events: &mut Vec<Event>,
) {
    let NewGameDialogView { form, first_launch } = view;
    let mut start = false;
    let mut cancel = false;
    let mut reset_bot = false;

    let title = if first_launch {
        "Welcome — Set Up Your Game"
    } else {
        "New Game"
    };
    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .default_width(420.0)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().max_height(560.0).show(ui, |ui| {
                ui.add_space(4.0);
                ui.label("You play as:");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut form.color, ColorChoice::White, "White");
                    ui.radio_value(&mut form.color, ColorChoice::Black, "Black");
                    ui.radio_value(&mut form.color, ColorChoice::Random, "Random");
                    ui.radio_value(&mut form.color, ColorChoice::Both, "Both");
                });
                ui.add_space(8.0);

                ui.label("Starting position (FEN, leave empty for startpos):");
                ui.add(
                    egui::TextEdit::singleline(&mut form.fen)
                        .desired_width(f32::INFINITY)
                        .hint_text("rnbqkbnr/pppppppp/... (optional)"),
                );
                if let Some(err) = &form.error {
                    ui.colored_label(egui::Color32::from_rgb(0xc0, 0x40, 0x40), err);
                }
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    ui.label("Engine depth:");
                    ui.add(egui::Slider::new(&mut form.depth, 1..=20));
                });

                ui.add_space(12.0);
                ui.separator();
                ui.heading("Bot Difficulty");
                ui.label(
                    egui::RichText::new(
                        "Tune how the bot plays. Defaults give full-strength play; \
                         raise the mistake knobs for a weaker, more punishable opponent.",
                    )
                    .small()
                    .weak(),
                );
                ui.add_space(6.0);

                draw_noise_controls(ui, &mut form.noise);

                ui.add_space(8.0);
                ui.collapsing("Eval mask (advanced) — categories the bot is blind to", |ui| {
                    ui.label(
                        egui::RichText::new(
                            "Toggle off a concept to simulate an opponent who doesn't \
                             understand it (e.g. mask king-safety to spar against a sub-\
                             1200 positional player).",
                        )
                        .small()
                        .weak(),
                    );
                    ui.add_space(4.0);
                    draw_eval_mask_controls(ui, &mut form.eval_mask);
                });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Reset bot to defaults").clicked() {
                        reset_bot = true;
                    }
                });

                ui.add_space(12.0);
            });

            ui.separator();
            ui.horizontal(|ui| {
                // Hide Cancel at first launch: no game to fall back
                // to, the only path forward is Start.
                if !first_launch && ui.button("Cancel").clicked() {
                    cancel = true;
                }
                let start_label = if first_launch { "Start Game" } else { "Start" };
                if ui.button(start_label).clicked() {
                    start = true;
                }
            });
        });

    if reset_bot {
        events.push(Event::ResetBotForm);
    }
    if cancel {
        events.push(Event::Cancel);
    }
    if start {
        events.push(Event::ConfirmNewGame);
    }
}

/// Six bot-noise sliders. Free function so it can borrow `noise`
/// without fighting the borrow checker over the whole form.
fn draw_noise_controls(ui: &mut egui::Ui, noise: &mut NoiseProfile) {
    egui::Grid::new("bot_noise_grid")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            ui.label("Blunder chance:")
                .on_hover_text(
                    "Per-move probability of a deliberate blunder — a move that \
                     loses material by force (the bot ends up down material). \
                     Best-effort: only fires when a move hanging material in the \
                     band below is actually available, so it's common in sharp \
                     positions and rare in quiet ones.",
                );
            ui.add(
                egui::Slider::new(&mut noise.blunder_chance, 0.0..=1.0)
                    .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
            );
            ui.end_row();

            ui.label("Miss chance:")
                .on_hover_text(
                    "Per-move probability of a deliberate miss — when a move wins \
                     material by force, the bot declines it and plays the best \
                     move that doesn't (even if that move is itself losing). \
                     Models 'saw the winning tactic, didn't play it.' No effect \
                     when no material-winning move is on the board.",
                );
            ui.add(
                egui::Slider::new(&mut noise.miss_chance, 0.0..=1.0)
                    .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
            );
            ui.end_row();

            // Blunder severity is expressed in points of material (a pawn
            // = 1.0); stored internally as material-centipawns (pawn = 100).
            ui.label("Blunder min material (pts):")
                .on_hover_text(
                    "Smallest amount of material a deliberate blunder must hang, \
                     in points (pawn = 1, minor = 3, rook = 5, queen = 9). 1.0 = \
                     a hung pawn, the lightest punishable mistake.",
                );
            let mut min_pts = noise.blunder_min_material_cp as f32 / 100.0;
            let max_pts = (noise.blunder_max_material_cp.max(0) as f32 / 100.0).max(0.0);
            if ui
                .add(egui::Slider::new(&mut min_pts, 0.0..=max_pts.max(0.5)).step_by(0.5))
                .changed()
            {
                noise.blunder_min_material_cp = (min_pts * 100.0) as i32;
            }
            ui.end_row();

            ui.label("Blunder max material (pts):")
                .on_hover_text(
                    "Largest amount of material a deliberate blunder may hang, in \
                     points. 4.0 caps blunders at roughly a minor-and-pawn / the \
                     exchange so the bot won't gift its queen; raise toward 9.0 \
                     to allow heavier hangs. A hang above this cap is never played.",
                );
            let mut max_pts = noise.blunder_max_material_cp as f32 / 100.0;
            let min_pts2 = (noise.blunder_min_material_cp.max(0) as f32) / 100.0;
            if ui
                .add(egui::Slider::new(&mut max_pts, min_pts2..=12.0).step_by(0.5))
                .changed()
            {
                noise.blunder_max_material_cp = (max_pts * 100.0) as i32;
            }
            ui.end_row();

            ui.label("Wild move chance:")
                .on_hover_text(
                    "Per-move probability of picking uniformly from ALL legal moves, \
                     bypassing the search ranking. Beginner-bot territory — the only \
                     branch that can pick moves the engine didn't surface.",
                );
            ui.add(
                egui::Slider::new(&mut noise.wild_chance, 0.0..=1.0)
                    .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
            );
            ui.end_row();

            ui.label("Average move rank:")
                .on_hover_text(
                    "The bot's variety dial: the average rank of the move it plays. \
                     1.0 = always the engine's best move. Higher plays weaker moves \
                     on average — 3.0 mostly plays the 2nd–4th best — sampled from a \
                     normal distribution around this value.",
                );
            ui.add(
                egui::Slider::new(&mut noise.avg_move_rank, 1.0..=10.0)
                    .step_by(0.5)
                    .custom_formatter(|v, _| format!("{v:.1}")),
            );
            ui.end_row();

            ui.label("Guaranteed mate-in:")
                .on_hover_text(
                    "Bot is guaranteed to convert mates of this length or shorter. \
                     1 = mate-in-1 is never thrown away. Set to 0 to allow blundering \
                     any mate.",
                );
            ui.add(egui::Slider::new(&mut noise.guaranteed_mate_in, 0..=10));
            ui.end_row();
        });
}

/// Eight eval-category checkboxes in a 2-column grid.
fn draw_eval_mask_controls(ui: &mut egui::Ui, mask: &mut EvalMask) {
    let half = EvalCategory::ALL.len() / 2;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            for cat in &EvalCategory::ALL[..half] {
                eval_mask_checkbox(ui, mask, *cat);
            }
        });
        ui.vertical(|ui| {
            for cat in &EvalCategory::ALL[half..] {
                eval_mask_checkbox(ui, mask, *cat);
            }
        });
    });
}

fn eval_mask_checkbox(ui: &mut egui::Ui, mask: &mut EvalMask, cat: EvalCategory) {
    let mut disabled = mask.is_disabled(cat);
    if ui.checkbox(&mut disabled, cat.slug()).changed() {
        if disabled {
            mask.disable(cat);
        } else {
            mask.enable(cat);
        }
    }
}
