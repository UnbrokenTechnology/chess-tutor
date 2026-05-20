//! Events emitted by renderers, dispatched by the session.
//!
//! Events name *intents*, not input mechanisms — `Cancel` not
//! `EscapePressed`, `RequestNewGame` not `NewGameButtonClicked`. The
//! same enum is consumed by the egui renderer today and (after Step
//! 3) by CLI / Apple / Android renderers. Input-mechanism names
//! would be lies in at least one of those.

use chess_tutor_engine::types::{Move, Square};

#[derive(Clone, Copy, Debug)]
pub(crate) enum Event {
    // ---- Board interaction
    /// User picked / clicked board square `sq`. Session resolves
    /// whether that means select, move, deselect, or no-op.
    SelectSquare(Square),
    /// User chose one of the four promotion picker entries.
    ConfirmPromotion(Move),

    // ---- Top bar
    RequestNewGame,
    Takeback,
    FlipBoard,
    ToggleHint,
    /// Jump from "viewing move N" back to live.
    JumpToLive,
    ChangeDepth(u32),

    // ---- Move list
    /// User clicked a move row. `None` is the synthetic "live"
    /// target; `Some(i)` is "view position after history[i]". The
    /// session re-maps "clicked the last move" to `JumpToLive`.
    ViewHistoryIndex(Option<usize>),

    // ---- Global cancel
    /// Generic "back out" intent. Session resolves priority:
    /// pending promotion > open dialog > deselect.
    Cancel,

    // ---- New Game dialog
    /// User clicked Start. Session reads the live form, validates,
    /// and starts the game (or sets `form.error` on failure).
    ConfirmNewGame,
    /// User clicked "Reset bot to defaults" inside the dialog.
    ResetBotForm,
}
