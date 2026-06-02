use std::sync::Arc;

use chess_tutor_ui::event::Event;
use chess_tutor_ui::Session;
use eframe::egui;

mod draw;

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            // 16:9 default (looks good full-screen / on an iPad in
            // landscape). A square board can't fill 16:9, so the eval
            // bar + board are drawn as one centered unit and the leftover
            // width splits between a slightly-wider right panel and small
            // symmetric margins. Resizable — this is just the default.
            .with_inner_size([1280.0, 720.0])
            .with_min_inner_size([1024.0, 640.0])
            .with_title("Chess Tutor"),
        ..Default::default()
    };
    eframe::run_native(
        "Chess Tutor",
        native_options,
        Box::new(|cc| {
            // Bundled Inter UI font + a broad symbol fallback so arrows /
            // geometric / media glyphs render instead of tofu boxes.
            install_fonts(&cc.egui_ctx);
            // SVG image loader (egui_extras) for the cburnett board pieces.
            egui_extras::install_image_loaders(&cc.egui_ctx);
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

/// Install the app's fonts: **Inter** (bundled) as the primary UI font,
/// plus a wide-coverage **symbol fallback** (Windows' Segoe UI Symbol) so
/// glyphs egui's bundled fonts lack (flip arrows, expander triangles,
/// media controls, chess symbols, …) render rather than showing as tofu.
/// egui's bundled Hack stays the monospace font (SANs / scores read well
/// fixed-width). Genuine colour emoji are avoided in the UI instead — egui
/// can't render their COLR layers — so a monochrome symbol fallback is all
/// we need. The Segoe path is Windows-only; absent it, we degrade to
/// egui's defaults.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    // Inter — bundled; highest-priority proportional font.
    fonts.font_data.insert(
        "inter".to_owned(),
        egui::FontData::from_static(include_bytes!("../assets/fonts/Inter.ttf")).into(),
    );
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "inter".to_owned());

    // Segoe UI Symbol — lowest-priority fallback for both families.
    if let Ok(bytes) = std::fs::read(r"C:\Windows\Fonts\seguisym.ttf") {
        fonts
            .font_data
            .insert("symbol_fallback".to_owned(), egui::FontData::from_owned(bytes).into());
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .push("symbol_fallback".to_owned());
        }
    }

    ctx.set_fonts(fonts);
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
        egui::SidePanel::right("sidebar")
            .resizable(false)
            // exact_width (not default_width) pins the column so long card
            // headings wrap within it instead of stretching the panel. 420
            // (up from 320) soaks up some of 16:9's extra width — more
            // room for the deep eval breakdowns — leaving the rest as small
            // symmetric margins around the centered board.
            .exact_width(420.0)
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
            // Player strip below the board — the user's own captured
            // pieces (chess.com idiom: opponent above, you below).
            egui::TopBottomPanel::bottom("player_strip")
                .resizable(false)
                .show_inside(ui, |ui| {
                    draw::player_strip::draw(ui, &self.session.build_player_strip_view());
                });

            // The eval bar + board are one centered unit: the eval bar
            // always hugs the board, and 16:9's leftover width splits into
            // symmetric side margins (rather than detaching a far-left
            // eval bar from a centered board). The eval bar is opt-out
            // (Start/Options + ⚙) — when hidden it claims no width.
            // A thin bar (chess.com-like), not a chunky 56px gutter.
            const EVAL_W: f32 = 30.0;
            const EVAL_GAP: f32 = 6.0;
            let show_eval = self.session.eval_bar_visible();
            let lead = if show_eval { EVAL_W + EVAL_GAP } else { 0.0 };
            let area = ui.available_rect_before_wrap();
            let board_size = (area.width() - lead).min(area.height()).max(0.0);
            let unit_w = lead + board_size;
            let left = area.left() + ((area.width() - unit_w) * 0.5).max(0.0);
            let top = area.top() + ((area.height() - board_size) * 0.5).max(0.0);
            if show_eval {
                let eval_rect = egui::Rect::from_min_size(
                    egui::pos2(left, top),
                    egui::vec2(EVAL_W, board_size),
                );
                draw::eval_bar::draw(ui, eval_rect, &self.session.build_eval_bar_view());
            }
            let board_rect = egui::Rect::from_min_size(
                egui::pos2(left + lead, top),
                egui::vec2(board_size, board_size),
            );
            draw::board::draw(ui, board_rect, &self.session.build_board_view(), &mut events);
        });

        // Hint pop-over (PLAN step 4): a floating "what to notice" panel
        // over the board, opened by Hint / auto-coach. Rendered before
        // the new-game dialog so a modal setup dialog still sits on top.
        if let Some(popover) = self.session.build_hint_popover_view() {
            draw::hint_popover::draw(ctx, &popover, &mut events);
        }

        // Mid-game ⚙ settings (decision #2): edits the same options as
        // the Start screen against the live session. Rendered before the
        // new-game dialog so a setup modal still sits on top.
        if let Some(settings) = self.session.build_settings_view() {
            draw::settings::draw(ctx, &settings, &mut events);
        }

        if let Some(dialog) = self.session.build_new_game_dialog_view() {
            draw::dialog::draw(ctx, dialog, &mut events);
        }

        for event in events {
            self.session.dispatch(event);
        }
    }
}
