//! egui-side rendering. Each submodule exposes a `draw` function
//! that paints one slice of the UI from a [`chess_tutor_ui::view`]
//! descriptor and emits [`chess_tutor_ui::event::Event`]s into a
//! shared buffer. The desktop `App` newtype in `main.rs` drains
//! events into [`chess_tutor_ui::Session::dispatch`] after each
//! frame.

pub(crate) mod action_bar;
pub(crate) mod board;
pub(crate) mod bot_strip;
pub(crate) mod dialog;
pub(crate) mod eval_bar;
pub(crate) mod hint_popover;
pub(crate) mod options;
pub(crate) mod settings;
pub(crate) mod side_panel;
pub(crate) mod top_bar;
