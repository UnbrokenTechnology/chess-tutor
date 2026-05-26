use eframe::egui;

use chess_tutor_ui::event::Event;
use chess_tutor_ui::view::TopBarView;

pub(crate) fn draw(ui: &mut egui::Ui, view: &TopBarView, events: &mut Vec<Event>) {
    ui.horizontal(|ui| {
        if ui.button("New Game").clicked() {
            events.push(Event::RequestNewGame);
        }
        if ui
            .add_enabled(view.can_takeback, egui::Button::new("Takeback"))
            .clicked()
        {
            events.push(Event::Takeback);
        }
        if ui.button("Flip Board").clicked() {
            events.push(Event::FlipBoard);
        }
        let hint_label = if view.hint_open { "Hide Hint" } else { "Hint" };
        if ui
            .add_enabled(view.hint_button_enabled, egui::Button::new(hint_label))
            .clicked()
        {
            events.push(Event::ToggleHint);
        }
        let review_label = if view.review_open {
            "Close Review"
        } else {
            "Review Game"
        };
        if ui
            .add_enabled(view.review_button_enabled, egui::Button::new(review_label))
            .clicked()
        {
            events.push(if view.review_open {
                Event::CloseGameReview
            } else {
                Event::OpenGameReview
            });
        }
        if !view.viewing_live && ui.button("▶ Live").clicked() {
            events.push(Event::JumpToLive);
        }
        ui.separator();
        ui.label("Depth:");
        let mut depth = view.depth;
        if ui
            .add(egui::DragValue::new(&mut depth).range(1..=20))
            .changed()
        {
            events.push(Event::ChangeDepth(depth));
        }
        ui.separator();
        if view.engine_thinking {
            ui.spinner();
            ui.label("engine thinking…");
        } else if let Some(end) = view.game_outcome {
            ui.colored_label(egui::Color32::from_rgb(0xb8, 0x55, 0x00), end);
        }
    });
}
