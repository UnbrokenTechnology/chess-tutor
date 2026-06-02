//! Events emitted by renderers, dispatched by the session.
//!
//! Events name *intents*, not input mechanisms — `Cancel` not
//! `EscapePressed`, `RequestNewGame` not `NewGameButtonClicked`. The
//! same enum is consumed by every renderer that links this crate;
//! input-mechanism names would be lies in at least one of those.

use chess_tutor_engine::types::{Move, Square};

use crate::view::OverlayKind;

#[derive(Clone, Copy, Debug)]
pub enum Event {
    // ---- Board interaction
    /// User picked / clicked board square `sq`. Session resolves
    /// whether that means select, move, deselect, or no-op.
    SelectSquare(Square),
    /// User chose one of the four promotion picker entries.
    ConfirmPromotion(Move),

    // ---- Title bar / action bar
    RequestNewGame,
    Takeback,
    FlipBoard,
    ToggleHint,
    /// Open the settings (⚙) surface. The mid-game config entry point
    /// (decision #2). No-op until the settings screen lands in a later
    /// redesign step; the intent is named now so renderers can wire the
    /// gear button without churning the event enum later.
    OpenSettings,
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
    /// User clicked the "why this move?" affordance (or collapsed it
    /// again). Toggles whether the feedback zone shows only the
    /// one-line verdict or expands the full per-term eval breakdown
    /// with deltas in place. Sticky across moves for the session.
    ToggleRetrospectiveDetail,
    /// User toggled the "show all signals" checkbox. When on, the
    /// retrospective surfaces every non-zero mobility shift per piece
    /// type and every residual term in "Other shifts" (no cumulative-
    /// prefix filter). Sticks across moves for the current session.
    ToggleShowAllSignals,
    /// User toggled an overlay checkbox in the side panel. Session
    /// flips the kind's membership in `active_overlays`; the next
    /// [`crate::session::Session::build_board_view`] pulls in the
    /// overlay's annotations.
    ToggleOverlay(OverlayKind),

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

    // ---- Learning-mode preferences
    /// User picked one of the named presets (Practicing / Supported /
    /// Coached). Session applies the preset's full preference bundle.
    ApplyLearningPreset(crate::learning_mode::LearningPreset),
    /// Toggle whether the engine's preferred move is revealed in
    /// retrospective text + arrow.
    SetRevealBestMoves(bool),

    // ---- Active intervention
    /// User dismissed the intervention prompt. Original move stands;
    /// session queues the engine's reply.
    ContinueDespitePrompt,
    /// User clicked "Show me what I missed". Reveals the concept-
    /// level prose without changing game state.
    RevealMissedConcept,
    /// User took back the move that triggered the intervention.
    /// Session undoes the move and clears the prompt; the user is
    /// back on their own turn.
    TakeBackDuringIntervention,

    // ---- Game Review
    /// Open the game-review surface in the side panel — list of
    /// significant moments derived from the assessment classifier.
    /// Available any time there have been user moves; most useful
    /// after a game ends.
    OpenGameReview,
    /// Close the game-review surface; return to the regular
    /// retrospective.
    CloseGameReview,
    /// User clicked a moment in the game review. Session jumps the
    /// view to that history index (sets `viewing_index`) and closes
    /// the review surface so the retrospective for that move shows.
    JumpToReviewMoment(usize),
}
