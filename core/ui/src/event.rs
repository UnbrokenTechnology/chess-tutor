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
    /// Open the settings (⚙) surface — the mid-game config entry point
    /// (decision #2). Mirrors the pre-game Start/Options screen so the
    /// same options (eval bar, Support, auto-coach, depths, overlays)
    /// can be changed without starting a new game.
    OpenSettings,
    /// Close the mid-game settings surface.
    CloseSettings,
    /// Jump from "viewing move N" back to live.
    JumpToLive,
    ChangeDepth(u32),
    /// Set the move-feedback (retrospective) search depth — how deeply
    /// the engine analyses each played move for the backward-looking
    /// feedback zone. Independent of the bot's play `depth`.
    SetRetrospectiveDepth(u32),
    /// Show or hide the eval bar (chess.com-style left gutter). Some
    /// students prefer to play without a constant numeric judgement.
    SetEvalBarVisible(bool),
    /// Turn the **Support** option on/off — the intervention pause that
    /// stops the game on a detected teaching moment / blunder. On maps
    /// to `MistakeHandling::TeachingMoments` + `BlunderSafety::OfferTakeback`;
    /// off maps to silent-retrospective + no safety net (decision #8).
    SetSupport(bool),
    /// Turn **Auto-coach** on/off — auto-open the Hint pop-over each
    /// move. Sets `LearningPreferences::auto_coach`.
    SetAutoCoach(bool),

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
    /// User clicked a moment in the game-review *summary*. Session
    /// enters step-through review mode at that history index (sets
    /// `viewing_index`), so the move's deep breakdown shows in the
    /// feedback zone.
    JumpToReviewMoment(usize),
    /// User pressed the big **Start Review** button on the summary
    /// screen. Session enters step-through review mode at the first
    /// move (history index 0).
    StartReview,
    /// Step-through review navigation (review-mode only). `Back` /
    /// `Forward` move one ply; `Restart` jumps to the first move;
    /// `End` jumps to the last move. The session clamps at the ends.
    ReviewNav(crate::view::ReviewNav),
    /// Toggle review-mode autoplay. While on, the renderer ticks
    /// [`Event::ReviewNav`]`(Forward)` on a timer until the last move,
    /// where the session reports autoplay as stopped.
    ToggleReviewAutoplay,
}
