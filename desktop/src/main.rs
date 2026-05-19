use eframe::egui;

mod draw;
mod session;
mod worker;

use session::App;

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 800.0])
            .with_min_inner_size([900.0, 700.0])
            .with_title("Chess Tutor"),
        ..Default::default()
    };
    eframe::run_native(
        "Chess Tutor",
        native_options,
        Box::new(|cc| Ok(Box::new(App::new(cc.egui_ctx.clone())))),
    )
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_worker();

        egui::TopBottomPanel::top("topbar").show(ctx, |ui| {
            self.draw_top_bar(ui);
        });
        egui::SidePanel::left("evalbar")
            .resizable(false)
            .exact_width(56.0)
            .show(ctx, |ui| {
                self.draw_eval_bar(ui);
            });
        egui::SidePanel::right("sidebar")
            .resizable(false)
            .default_width(320.0)
            .show(ctx, |ui| {
                self.draw_side_panel(ui);
            });
        egui::CentralPanel::default().show(ctx, |ui| {
            self.draw_board(ui);
        });

        if self.new_game_form.is_some() {
            self.draw_new_game_dialog(ctx);
        }
    }
}
