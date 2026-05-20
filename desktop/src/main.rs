use eframe::egui;

mod draw;
mod event;
mod session;
mod view;
mod worker;

use event::Event;
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

        // Renderers push intents into this buffer; the session drains
        // it after rendering finishes so mutations don't fight egui's
        // borrow of `self` inside the panel closures.
        let mut events: Vec<Event> = Vec::new();

        egui::TopBottomPanel::top("topbar").show(ctx, |ui| {
            draw::top_bar::draw(ui, &self.build_top_bar_view(), &mut events);
        });
        egui::SidePanel::left("evalbar")
            .resizable(false)
            .exact_width(56.0)
            .show(ctx, |ui| {
                draw::eval_bar::draw(ui, &self.build_eval_bar_view());
            });
        egui::SidePanel::right("sidebar")
            .resizable(false)
            .default_width(320.0)
            .show(ctx, |ui| {
                draw::side_panel::draw(ui, &self.build_side_panel_view(), &mut events);
            });
        egui::CentralPanel::default().show(ctx, |ui| {
            draw::board::draw(ui, &self.build_board_view(), &mut events);
        });

        if let Some(dialog) = self.build_new_game_dialog_view() {
            draw::dialog::draw(ctx, dialog, &mut events);
        }

        for event in events {
            self.dispatch(event);
        }
    }
}
