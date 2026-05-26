//! Platform-agnostic session state and game logic.
//!
//! Owns the live position, history, viewing index, opponent profile,
//! hint state, and the channel pair used to talk to the worker.
//! Renderers consume [`crate::view`] descriptors built by the
//! `build_*_view` methods and feed user intents back via
//! [`Session::dispatch`].

use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

use chess_tutor_engine::book::BookCursor;
use chess_tutor_engine::opponent::OpponentProfile;
use chess_tutor_engine::position::Position;
use chess_tutor_engine::traps::PendingTrap;
use chess_tutor_engine::types::{Move, Square};

use crate::learning_mode::{
    LearningPreferences, PendingIntervention,
};
use crate::worker::{WorkerJob, WorkerResult};

/// Renderer-supplied "wake up" callback. The worker thread calls this
/// after sending a result to nudge the renderer's event loop:
/// `egui::Context::request_repaint` for desktop, a native run-loop
/// post for iOS / Android, a no-op for headless CLI consumers.
pub type RepaintFn = Arc<dyn Fn() + Send + Sync>;

pub(crate) const ENGINE_TURN_NODE_CAP: u64 = 5_000_000;
pub(crate) const HINT_MULTI_PV: usize = 3;
/// Engine-play depth — what the bot uses to pick its own moves.
pub(crate) const DEFAULT_DEPTH: u32 = 10;
/// Analytical depth for retrospective / hint / analyze paths. Kept
/// deeper than [`DEFAULT_DEPTH`] so the student's feedback is a
/// stronger reference than the bot they're playing — at d=10 we
/// observed verdict flips on common opening positions (e.g. 1.e4 e5
/// 2.Nf3 reads "inaccuracy" at d=10 but "best" at d=12). Independent
/// of bot-play depth so a weakened bot can still give strong
/// teaching feedback. UI exposure deferred — for now the New Game
/// dialog only tunes engine depth.
pub(crate) const ANALYTICAL_DEPTH: u32 = 12;

pub struct Session {
    pub(crate) position: Position,
    /// Snapshot of the position the current game started from. Lets
    /// renderers compute the pre-move position for any history index
    /// (`history[i-1].position_after`, or `start_position` when
    /// `i == 0`) without storing one per entry.
    pub(crate) start_position: Position,
    pub(crate) position_keys: Vec<u64>,
    pub(crate) history: Vec<HistoryEntry>,
    pub(crate) selected: Option<Square>,
    pub(crate) legal_from_selected: Vec<Move>,
    pub(crate) flipped: bool,

    pub(crate) engine_plays: EngineMode,
    pub(crate) depth: u32,
    /// Depth used by auto-retrospective worker jobs. Defaults to
    /// [`ANALYTICAL_DEPTH`]; CLI consumers tweak via
    /// [`Session::set_retrospective_depth`] for `--retrospective-depth`.
    pub(crate) retrospective_depth: u32,
    /// When `true`, Session writes book-pick / opening-seed / noise-
    /// pick events to stderr. Defaults to `true` for the desktop's
    /// "shell window is the de facto session log" model; CLI consumers
    /// set this to `false` and surface the same data through their own
    /// stdout output.
    pub(crate) log_to_stderr: bool,
    /// When `true`, every user move triggers an auto-retrospective
    /// search via the worker. Defaults to `true` for desktop; CLI
    /// callers that run their own retrospective set this to `false`
    /// to avoid the redundant search.
    pub(crate) auto_retrospective: bool,

    pub(crate) worker_tx: Sender<WorkerJob>,
    pub(crate) worker_rx: Receiver<WorkerResult>,
    /// Bumped on cancel events (NewGame, Takeback). Worker results
    /// with a stale `gen` are dropped on arrival.
    pub(crate) gen: u64,
    pub(crate) engine_thinking: bool,

    /// `None` = following live play; `Some(i)` = viewing the position
    /// after `history[i]`.
    pub(crate) viewing_index: Option<usize>,

    /// `Some` while the New Game dialog is open. The form holds the
    /// in-flight color / FEN / depth choices; `try_start_from_form`
    /// validates and either applies (closing the dialog) or sets
    /// `form.error` and keeps it open.
    pub(crate) new_game_form: Option<NewGameForm>,

    /// `true` while the Hint panel is showing (replacing the
    /// retrospective panel). Toggled by the Hint button; auto-closed
    /// on next move, takeback, and new game.
    pub(crate) hint_open: bool,
    /// `true` while a Hint Analyze job is in flight. Distinct from
    /// `hint_open` because the panel may be open showing stale results
    /// while we wait for fresh ones.
    pub(crate) hint_thinking: bool,
    /// Latest analyze result. Tagged with the position key it was
    /// computed for so stale arrivals can be discarded.
    pub(crate) hint_result: Option<HintResult>,

    /// Bot personality / variability for the current game. Reseeded
    /// on every New Game; the play loop reads `book` to pick an
    /// opening line and consults [`Self::book_cursor`] to follow it.
    pub(crate) opponent: OpponentProfile,
    /// Holds the allowed-openings list and seed for this game. The
    /// cursor is stateless — peek(history) re-derives the matching
    /// set each call — so this stays `Some` for the entire game
    /// whenever a book was configured. It's only `None` when the
    /// profile started with [`BookSelection::None`] or the game
    /// started from a custom FEN where the book doesn't apply.
    pub(crate) book_cursor: Option<BookCursor>,
    /// `true` once we've printed the "out of book" line during the
    /// current streak of out-of-book bot turns. Reset on new game
    /// AND on takeback, so the announcement re-prints if the user
    /// takes back and either deviates a second time or returns to
    /// in-book history and then deviates again. Without this dedup
    /// we'd repeat the message on every bot turn out of book; without
    /// the takeback reset, takeback couldn't restore book play.
    pub(crate) book_out_announced: bool,
    /// True until the user clicks Start in the New Game dialog for
    /// the first time. While true the dialog hides its Cancel button
    /// — there's no game in progress to cancel back to, so the only
    /// path forward is to commit a configuration.
    pub(crate) first_launch: bool,

    /// `Some` while the user has clicked a pawn onto the promotion
    /// rank and we're waiting for them to choose which piece to
    /// promote to. Carries the four candidate promotion moves (Q / R /
    /// B / N variants of the same from→to). Cleared on pick, off-board
    /// click, or any state-changing action (new game, takeback).
    pub(crate) pending_promotion: Option<PendingPromotion>,

    /// Live trap-library cursor. `Some` between a trap firing and
    /// the refutation tree reaching a terminal node; `None`
    /// otherwise. Renderers query [`Self::pending_trap`] /
    /// [`Self::trap_threats`] to surface active traps and pre-move
    /// warnings.
    pub(crate) pending_trap: Option<PendingTrap>,

    /// Currently-selected retrospective card for the panel entry the
    /// user is viewing. `Some((history_index, item_index))` while a
    /// card is selected; `None` when nothing's selected (or when the
    /// user has navigated to a different move). Drives which board
    /// annotations the next [`Self::build_board_view`] surfaces.
    pub(crate) selected_retrospective: Option<(usize, usize)>,

    /// When `true`, the retrospective view surfaces every non-zero
    /// per-piece-type mobility shift and every residual term in
    /// "Other shifts" (no cumulative-prefix filter). When `false`
    /// (default), mobility uses a 50 cp floor per piece type and
    /// "Other shifts" caps at the 50%-coverage prefix. Sticky across
    /// moves so the student can opt in once and keep the wider view.
    pub(crate) show_all_signals: bool,

    /// User-toggled board overlays — each [`crate::view::OverlayKind`]
    /// paints its own annotations onto the live board. Sticky across
    /// moves; not persisted to disk yet.
    pub(crate) active_overlays: std::collections::HashSet<crate::view::OverlayKind>,

    /// Learning-mode preferences (assistance level, mistake handling,
    /// blunder safety, whether engine-preferred moves are revealed).
    /// Defaults match the "Practicing" preset: silent during play,
    /// retrospective only, no best-move reveal. The intervention path
    /// reads these on every move-related event; the retrospective
    /// builder reads `reveal_best_moves` per frame.
    pub(crate) learning: LearningPreferences,

    /// Set after a user move when the engine classifier said an
    /// intervention is warranted *and* the user's preferences want
    /// to be paused for it. While `Some`, the engine reply is held
    /// (no `WorkerJob::Search` queued) and the side panel renders
    /// the intervention prompt instead of the retrospective. Cleared
    /// by any of the intervention-response events (continue, reveal,
    /// take-back).
    pub(crate) pending_intervention: Option<PendingIntervention>,

    /// `true` between `apply_user_move` and the matching retrospective
    /// arrival when we deferred the engine search waiting for the
    /// classifier to weigh in. Without this flag we'd never queue the
    /// engine search in the "user is in intervention mode, but the
    /// move turned out Fine" case. Cleared as soon as the
    /// classifier decision lands.
    pub(crate) awaiting_intervention_decision: bool,

    /// `true` while the user has opened the post-game review surface.
    /// Renderers swap the side panel's body to the review when set.
    /// Auto-closed on takeback / new game so the user isn't left on a
    /// stale list.
    pub(crate) game_review_open: bool,
}

mod event_dispatch;
mod lifecycle;
mod moves;
mod queries;
mod types;
mod view_builders;
mod worker;

// Re-exports. Types are pub (external `chess_tutor_ui::session::X` API);
// the free-fn modules are glob-re-exported crate-internal so sibling
// submodules resolve their helpers via `use super::*` (globs survive
// `cargo fix`, unlike explicit lists used only through the glob).
pub use types::*;
pub(crate) use lifecycle::*;
