use chess_tutor_engine::opponent::{EvalCategory, EvalMask, NoiseProfile};
use eframe::egui;

use crate::session::App;

impl App {
    pub(crate) fn draw_new_game_dialog(&mut self, ctx: &egui::Context) {
        let Some(form) = self.new_game_form.as_mut() else {
            return;
        };
        let first_launch = self.first_launch;
        let mut start = false;
        let mut cancel = false;
        let mut reset_bot = false;

        let title = if first_launch { "Welcome — Set Up Your Game" } else { "New Game" };
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
                        ui.radio_value(&mut form.color, crate::session::ColorChoice::White, "White");
                        ui.radio_value(&mut form.color, crate::session::ColorChoice::Black, "Black");
                        ui.radio_value(&mut form.color, crate::session::ColorChoice::Random, "Random");
                        ui.radio_value(&mut form.color, crate::session::ColorChoice::Both, "Both");
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
                    // Hide Cancel at first launch: there's no game to
                    // cancel back to, the only path forward is Start.
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
            if let Some(f) = self.new_game_form.as_mut() {
                f.noise = NoiseProfile::default();
                f.eval_mask = EvalMask::EMPTY;
            }
        }
        if cancel {
            self.new_game_form = None;
        } else if start {
            self.try_start_from_form();
        }
    }
}

/// Render the six bot-noise sliders. Mutates the profile in place.
/// Kept as a free function so the New Game dialog can borrow `form`
/// fields mutably without fighting the borrow checker over `self`.
fn draw_noise_controls(ui: &mut egui::Ui, noise: &mut NoiseProfile) {
    egui::Grid::new("bot_noise_grid")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            ui.label("Blunder chance:")
                .on_hover_text(
                    "Per-move probability of a deliberate mistake. Blunders are picked \
                     from the engine's top-6; severity controls how bad they are.",
                );
            ui.add(
                egui::Slider::new(&mut noise.blunder_chance, 0.0..=1.0)
                    .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
            );
            ui.end_row();

            ui.label("Blunder min loss (cp):")
                .on_hover_text(
                    "Minimum loss (centipawns vs the engine's #1 move) for an \
                     alternative to count as a blunder. 100 = ~one pawn-down move \
                     the student can plausibly punish.",
                );
            // Slider max tracks the upper threshold so the user can't accidentally
            // set min > max with the controls.
            let cur_max = noise.blunder_max_loss_cp.max(0);
            ui.add(egui::Slider::new(&mut noise.blunder_min_loss_cp, 0..=cur_max.max(1)));
            ui.end_row();

            ui.label("Blunder max loss (cp):")
                .on_hover_text(
                    "Maximum loss (centipawns vs #1) for an alternative to count \
                     as a blunder. Caps how catastrophic blunders can be — 400 = \
                     about an exchange sacrifice, 900 = queen hangs. When no \
                     alternative falls in the [min, max] band, the picker takes \
                     the closest-loss line on each side of the band but excludes \
                     distant outliers, so the bot won't hang a piece if a less-bad \
                     alternative exists.",
                );
            let cur_min = noise.blunder_min_loss_cp.max(0);
            ui.add(egui::Slider::new(&mut noise.blunder_max_loss_cp, cur_min..=2000));
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

            ui.label("Candidate pool:")
                .on_hover_text(
                    "How many top moves the bot samples from under softmax noise. \
                     1 = no sampling (always #1).",
                );
            ui.add(egui::Slider::new(&mut noise.candidate_pool, 1..=10));
            ui.end_row();

            ui.label("Softmax temperature (cp):")
                .on_hover_text(
                    "Flatness of the softmax distribution over the candidate pool. \
                     0 = always #1; higher = more variety among close-scoring moves.",
                );
            ui.add(egui::Slider::new(&mut noise.temperature_cp, 0..=500));
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

/// Render the eight eval-category checkboxes in a 2-column grid.
/// Each toggle simulates an opponent who doesn't understand the
/// corresponding concept (e.g. mask off king-safety for a positionally
/// naive bot).
fn draw_eval_mask_controls(ui: &mut egui::Ui, mask: &mut EvalMask) {
    // Two-column layout to keep the dialog from getting absurdly tall;
    // 8 categories split 4+4.
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
