use std::sync::Arc;

use chess_tutor_ui::event::Event;
use chess_tutor_ui::Session;
use eframe::egui;

mod draw;

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
        Box::new(|cc| {
            // The session spawns a worker thread that needs a "wake
            // the UI" callback. egui::Context::request_repaint is the
            // egui-native idiom; we capture the Context in a closure
            // so the shared layer never sees egui types directly.
            let ctx = cc.egui_ctx.clone();
            let repaint = Arc::new(move || ctx.request_repaint());
            Ok(Box::new(App {
                session: Session::new(repaint),
            }))
        }),
    )
}

/// Desktop-side `eframe::App`. Thin wrapper around the platform-
/// agnostic [`Session`]; the only desktop-flavoured concerns are
/// egui panels and the `eframe::App` trait impl.
struct App {
    session: Session,
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.session.poll_worker();

        // Renderers push intents into this buffer; the session drains
        // it after rendering finishes so mutations don't fight egui's
        // borrow of `self` inside the panel closures.
        let mut events: Vec<Event> = Vec::new();

        egui::TopBottomPanel::top("topbar").show(ctx, |ui| {
            draw::top_bar::draw(ui, &self.session.build_top_bar_view(), &mut events);
        });
        egui::SidePanel::left("evalbar")
            .resizable(false)
            .exact_width(56.0)
            .show(ctx, |ui| {
                draw::eval_bar::draw(ui, &self.session.build_eval_bar_view());
            });
        egui::SidePanel::right("sidebar")
            .resizable(false)
            // exact_width (not default_width) pins the column: long card
            // headings wrap within it instead of stretching the panel and
            // squeezing the board — and a width egui grew on a prior frame
            // can't stick, since the range is clamped to [320, 320].
            .exact_width(320.0)
            .show(ctx, |ui| {
                // Big action bar pinned to the bottom of the right column
                // (chess.com idiom). Reserve it first via a bottom-up
                // layout, then the side-panel content fills the space above.
                egui::TopBottomPanel::bottom("action_bar")
                    .resizable(false)
                    .show_inside(ui, |ui| {
                        ui.add_space(6.0);
                        draw::action_bar::draw(
                            ui,
                            &self.session.build_action_bar_view(),
                            &mut events,
                        );
                        ui.add_space(6.0);
                    });
                draw::side_panel::draw(ui, &self.session.build_side_panel_view(), &mut events);
            });
        egui::CentralPanel::default().show(ctx, |ui| {
            // Opponent strip pinned above the board (chess.com idiom).
            // Reserve it first via a top panel so the board below fills
            // the remaining height — no dark band underneath it.
            egui::TopBottomPanel::top("bot_strip")
                .resizable(false)
                .show_inside(ui, |ui| {
                    draw::bot_strip::draw(ui, &self.session.build_bot_strip_view());
                });
            draw::board::draw(ui, &self.session.build_board_view(), &mut events);
        });

        if let Some(dialog) = self.session.build_new_game_dialog_view() {
            draw::dialog::draw(ctx, dialog, &mut events);
        }

        for event in events {
            self.session.dispatch(event);
        }
    }
}
