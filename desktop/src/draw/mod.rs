//! egui-side rendering. Each submodule adds an `impl App` block that
//! paints one slice of the UI from session state. No drawing module
//! mutates non-rendering session state directly; user input is fed
//! back through `App` methods on `session.rs` (`handle_click`,
//! `takeback`, `toggle_hint`, `try_start_from_form`, ...).
//!
//! Step 2 of the chess-tutor-ui split will replace these direct field
//! reads with view-descriptor structs.

pub(crate) mod board;
pub(crate) mod dialog;
pub(crate) mod eval_bar;
pub(crate) mod side_panel;
pub(crate) mod top_bar;
