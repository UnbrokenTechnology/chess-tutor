//! The Start / Options screen (PLAN build-order step 5): a proper
//! pre-game setup modal grown out of the old bare new-game dialog.
//! Picks the opponent / strength, exposes an Options block (Eval Bar,
//! Support, Auto-coach, Reveal-best-move, Move-feedback depth) plus the
//! board-overlay toggles, and commits it all with one big **Play**
//! button. **Bot search depth lives in the Opponent-strength section**,
//! not Options — it only affects the opponent's move selection, so it's
//! a strength lever, fixed per game (the mid-game ⚙ gear can't change
//! it). This screen is the *true home* of the learning + overlay config
//! that build-order step 3 stripped off the play surface; the mid-game
//! ⚙ gear (`draw::settings`) edits the same live-changeable set.
//!
//! There is deliberately **no "Engine PV" toggle** — engine best-move
//! lines are review-only (decision #9).

use chess_tutor_engine::opponent::{EvalCategory, EvalMask, NoiseProfile};
use eframe::egui;

use chess_tutor_ui::event::Event;
use chess_tutor_ui::session::ColorChoice;
use chess_tutor_ui::view::NewGameDialogView;

use super::options;

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
        .default_width(460.0)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().max_height(600.0).show(ui, |ui| {
                ui.add_space(4.0);

                // ---- Opponent / colour ----
                ui.label(egui::RichText::new("You play as").size(15.0).strong());
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
                    ui.colored_label(crate::draw::theme::ERROR, err);
                }

                ui.add_space(12.0);
                ui.separator();

                // ---- Options (collapsible to keep the dialog short) ----
                egui::CollapsingHeader::new("Options")
                    .default_open(false)
                    .show(ui, |ui| {
                        draw_play_options(ui, form);

                        ui.add_space(8.0);
                        egui::CollapsingHeader::new("Board overlays")
                            .default_open(false)
                            .show(ui, |ui| {
                                ui.label(
                                    egui::RichText::new(
                                        "Persistent highlights painted on the board — space, \
                                         pins, trapped pieces, attack heatmap, and more.",
                                    )
                                    .small()
                                    .weak(),
                                );
                                ui.add_space(4.0);
                                options::overlay_toggles(ui, &mut form.active_overlays);
                            });
                    });

                ui.add_space(12.0);
                ui.separator();
                ui.heading("Opponent strength");
                ui.label(
                    egui::RichText::new(
                        "Tune how the bot plays. Defaults give full-strength play; \
                         lower the search depth or raise the mistake knobs for a \
                         weaker, more punishable opponent.",
                    )
                    .small()
                    .weak(),
                );
                ui.add_space(6.0);

                // Search depth is the primary strength lever — it only
                // affects the bot's move selection (not retrospective or
                // game review), so it belongs here. Rendered in the same
                // grid as the noise knobs so its label/slider columns line
                // up with them.
                draw_strength_controls(
                    ui,
                    &mut form.depth,
                    &mut form.qsearch_max_plies,
                    &mut form.noise,
                );

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .button(super::icon::icon_label(
                            egui_phosphor::regular::BOOK_OPEN,
                            "Openings…",
                            14.0,
                        ))
                        .on_hover_text("Choose which openings the bot may play")
                        .clicked()
                    {
                        super::opening_picker::open(ui.ctx());
                    }
                    ui.label(
                        egui::RichText::new(super::opening_picker::summary(&form.book))
                            .color(crate::draw::theme::TEXT_MUTED),
                    );
                });

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
                // to, the only path forward is Play.
                if !first_launch && ui.button("Cancel").clicked() {
                    cancel = true;
                }
                // One big, obvious Play button (chess.com idiom).
                let play = egui::Button::new(crate::draw::icon::icon_label(
                    egui_phosphor::regular::PLAY,
                    "Play",
                    18.0,
                ))
                .min_size(egui::vec2(140.0, 40.0));
                if ui.add(play).clicked() {
                    start = true;
                }
            });
        });

    // The opening picker is its own top-level window (drawn outside the
    // auto-sizing New Game modal so it can't balloon the modal off-screen).
    super::opening_picker::draw_window(ctx, &mut form.book);
    if start || cancel {
        super::opening_picker::close(ctx);
    }

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

/// The play-option toggles + depth sliders shared in spirit with the
/// mid-game gear. Mutates the form in place; the form is committed onto
/// the session when the user clicks Play.
fn draw_play_options(ui: &mut egui::Ui, form: &mut chess_tutor_ui::session::NewGameForm) {
    options::toggle_row(
        ui,
        &mut form.show_eval_bar,
        "Eval bar",
        "Show the chess.com-style evaluation bar in the left gutter. \
         Turn it off to play without a constant numeric judgement.",
    );
    options::learning_toggles(ui, &mut form.learning);

    ui.add_space(4.0);
    // Bot search depth is NOT here — it only affects the opponent's move
    // selection, so it lives in the Opponent-strength section. This is
    // the move-feedback (retrospective/review) depth, a genuine generic
    // analysis setting.
    options::depth_row(
        ui,
        &mut form.retrospective_depth,
        "Move-feedback depth",
        "How deeply each of your moves is analysed for the after-move \
         feedback. Higher is more accurate and slower.",
    );
}

/// The opponent-strength sliders: search depth + the six noise knobs,
/// all in one 2-column grid so the label/slider columns align. Free
/// function so it can borrow the two form fields disjointly without
/// fighting the borrow checker over the whole form.
/// Slider position that means "infinite tactical vision" (full quiescence,
/// the form's `None`). Finite caps occupy 0..9; the far-right notch is ∞.
const QSEARCH_INF: u32 = 10;

fn draw_strength_controls(
    ui: &mut egui::Ui,
    depth: &mut u32,
    qsearch: &mut Option<u32>,
    noise: &mut NoiseProfile,
) {
    egui::Grid::new("bot_strength_grid")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            ui.label("Search depth:").on_hover_text(
                "How deeply the bot searches when choosing its move. Higher is \
                 stronger and slower. Only affects the opponent — your move \
                 feedback uses its own depth.",
            );
            ui.add(egui::Slider::new(depth, 1..=20));
            ui.end_row();

            // Tactical vision = the quiescence horizon (how many capture
            // plies the bot resolves before judging on position alone). A
            // plain slider like depth: 0 = blind (hangs pieces), and the
            // far right = ∞ = full tactical sight (the default). `None` on
            // the form is ∞; finite caps are 0..9.
            ui.label("Tactical vision:").on_hover_text(
                "How many capture plies the bot sees before judging on position \
                 alone. Far right (∞) is full sight — normal play. Lower values \
                 hang material like a weaker human; 0 = blind, doesn't even see \
                 the recapture.",
            );
            let mut vision: u32 = qsearch.map_or(QSEARCH_INF, |n| n.min(QSEARCH_INF));
            let resp = ui.add(
                egui::Slider::new(&mut vision, 0..=QSEARCH_INF).custom_formatter(|v, _| {
                    if v >= QSEARCH_INF as f64 {
                        "∞".to_string()
                    } else {
                        format!("{}", v as u32)
                    }
                }),
            );
            if resp.changed() {
                *qsearch = if vision >= QSEARCH_INF {
                    None
                } else {
                    Some(vision)
                };
            }
            ui.end_row();

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
