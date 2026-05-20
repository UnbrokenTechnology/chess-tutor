//! egui-side rendering. Each submodule exposes a `draw` function
//! that paints one slice of the UI from a view descriptor and emits
//! intent events into a shared buffer. No drawing module mutates
//! session state directly; the main loop drains events into
//! [`crate::session::App::dispatch`].
//!
//! Step 3 of the chess-tutor-ui split moves `session.rs`, `worker.rs`,
//! and `view.rs` into a `core/ui` crate. These draw modules stay
//! desktop-flavoured (egui types live here).

pub(crate) mod board;
pub(crate) mod dialog;
pub(crate) mod eval_bar;
pub(crate) mod side_panel;
pub(crate) mod top_bar;
