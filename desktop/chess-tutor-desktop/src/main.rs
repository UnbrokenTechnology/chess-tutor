//! Chess Tutor desktop — Windows-first, single-binary egui app.
//!
//! Phase 4 shell. Today this just boots an eframe window and holds a [`Game`]
//! from the core; board rendering, input handling, and the feedback panel
//! land alongside the Phase 4 checklist in `PLAN.md`.

use chess_tutor_core::game::{Game, PlayerKind};
use eframe::{egui, App, CreationContext, NativeOptions};

fn main() -> eframe::Result<()> {
    let options = NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Chess Tutor")
            .with_inner_size([1100.0, 760.0])
            .with_min_inner_size([720.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Chess Tutor",
        options,
        Box::new(|cc| Box::new(ChessTutorApp::new(cc))),
    )
}

struct ChessTutorApp {
    game: Game,
}

impl ChessTutorApp {
    fn new(_cc: &CreationContext<'_>) -> Self {
        Self {
            game: Game::new_standard(PlayerKind::Human, PlayerKind::Human),
        }
    }
}

impl App for ChessTutorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Chess Tutor");
                ui.separator();
                if ui.button("New game (H vs H)").clicked() {
                    self.game = Game::new_standard(PlayerKind::Human, PlayerKind::Human);
                }
                if ui.button("New game (H vs Bot)").clicked() {
                    self.game = Game::new_standard(PlayerKind::Human, PlayerKind::Bot);
                }
            });
        });

        egui::SidePanel::right("move_list").min_width(220.0).show(ctx, |ui| {
            ui.heading("Moves");
            for (i, entry) in self.game.history().iter().enumerate() {
                ui.label(format!("{:>3}. {}", i + 1, entry.san));
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Board");
            ui.label("Board rendering lands in Phase 4 — see PLAN.md.");
            ui.label(format!("Side to move: {:?}", self.game.side_to_move()));
            ui.label(format!("Status: {:?}", self.game.status()));
        });
    }
}
