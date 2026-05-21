//! Events emitted by renderers, dispatched by the session.
//!
//! Events name *intents*, not input mechanisms — `Cancel` not
//! `EscapePressed`, `RequestNewGame` not `NewGameButtonClicked`. The
//! same enum is consumed by every renderer that links this crate;
//! input-mechanism names would be lies in at least one of those.

use chess_tutor_engine::types::{Move, Square};

#[derive(Clone, Copy, Debug)]
pub enum Event {
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

    // ---- Retrospective panel
    /// User clicked card `i` in the retrospective panel. The session
    /// toggles selection: same index clicked twice deselects. The
    /// selected item's annotations flow into the next [`crate::view::BoardView`].
    SelectRetrospectiveItem(usize),
    /// User toggled the "show all signals" checkbox. When on, the
    /// retrospective surfaces every non-zero mobility shift per piece
    /// type and every residual term in "Other shifts" (no cumulative-
    /// prefix filter). Sticks across moves for the current session.
    ToggleShowAllSignals,

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
