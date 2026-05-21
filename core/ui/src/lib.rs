//! Platform-agnostic chess-tutor UI layer.
//!
//! Owns session state, game logic, the background search worker, and
//! view descriptors. The concrete renderer (egui desktop, Apple,
//! Android, CLI) is downstream — each consumes [`view`] descriptors,
//! emits [`event::Event`]s, and feeds them back via
//! [`Session::dispatch`].
//!
//! The worker thread wakes the renderer via the [`session::RepaintFn`]
//! callback supplied at construction — desktop closes over
//! `egui::Context::request_repaint`, CLI passes a no-op, mobile shells
//! post to their native run loop. There's no other platform coupling.

pub mod event;
pub mod retrospective_view;
pub mod session;
pub mod view;
mod worker;

pub use retrospective_view::build_retrospective_view;
pub use session::{RepaintFn, Session};
pub use worker::NoisePickInfo;
