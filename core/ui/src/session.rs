//! Platform-agnostic session state and game logic.
//!
//! Owns the live position, history, viewing index, opponent profile,
//! hint state, and the channel pair used to talk to the worker.
//! Renderers consume [`crate::view`] descriptors built by the
//! `build_*_view` methods and feed user intents back via
//! [`Session::dispatch`].

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use chess_tutor_engine::book::BookCursor;
use chess_tutor_engine::engine::SearchParams;
use chess_tutor_engine::movegen::legal_moves_vec;
use chess_tutor_engine::opponent::{EvalMask, NoiseProfile, OpponentProfile};
use chess_tutor_engine::position::{Position, StateInfo};
use chess_tutor_engine::san;
use chess_tutor_engine::traps::{self, PendingTrap, TrapEvent, TrapHit, TrapThreatened};
use chess_tutor_engine::types::{Color, Move, MoveKind, PieceType, Square, Value};

use crate::event::Event;
use crate::learning_mode::{
    build_intervention_panel, gating_config_for, intervention_required, LearningPreferences,
    LearningPreset, MistakeHandling, PendingIntervention,
};
use crate::view::{
    BoardView, CoachingPanelView, EvalBarView, HintEntryView, HintPanelState, HintPanelView,
    MoveListCell, MoveListRow, MoveListView, NewGameDialogView, PromotionPickerView,
    RetrospectiveBody, RetrospectiveKind, RetrospectivePanelView, SidePanelBody, SidePanelView,
    TopBarView,
};
use crate::worker::{worker_loop, NoisePickInfo, WorkerJob, WorkerResult};

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

pub struct EngineInfo {
    pub score_white_pov: Value,
    pub depth: u32,
    pub elapsed: Duration,
    /// Total nodes searched for this engine move. Populated for the
    /// CLI's per-move output; the GUI ignores it.
    pub nodes: u64,
    /// Mega-nodes per second. Same source as `nodes` —
    /// `engine.last_nps() / 1e6`.
    pub nps_m: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorChoice {
    White,
    Black,
    Random,
    Both,
}

pub struct NewGameForm {
    pub color: ColorChoice,
    pub fen: String,
    pub depth: u32,
    /// Bot move-sampling knobs. Persists across New Game clicks so the
    /// user can tune incrementally between games without losing prior
    /// settings.
    pub noise: NoiseProfile,
    /// Eval categories the bot is blind to. Same persistence rule.
    pub eval_mask: EvalMask,
    pub error: Option<String>,
}

impl NewGameForm {
    /// Pre-populate from the live game so the dialog reflects what
    /// the user is currently playing against — encourages incremental
    /// tweaking rather than rebuilding settings from scratch every
    /// time they click New Game.
    fn from_current(session: &Session) -> Self {
        Self {
            color: match session.engine_plays {
                EngineMode::Side(Color::Black) => ColorChoice::White,
                EngineMode::Side(Color::White) => ColorChoice::Black,
                // Both EngineMode::None (user plays both) and
                // EngineMode::Both (engine self-play) land here. The
                // GUI dialog has no self-play radio; Both is the
                // closest match.
                EngineMode::None | EngineMode::Both => ColorChoice::Both,
            },
            fen: String::new(),
            depth: session.depth,
            noise: session.opponent.noise.clone(),
            eval_mask: session.opponent.eval_mask,
            error: None,
        }
    }

    /// Defaults for the first-launch dialog — same shape as
    /// [`Self::from_current`] would produce for a freshly constructed
    /// [`Session`], but without needing one to exist yet.
    fn initial() -> Self {
        Self {
            color: ColorChoice::White,
            fen: String::new(),
            depth: DEFAULT_DEPTH,
            noise: NoiseProfile::default(),
            eval_mask: EvalMask::EMPTY,
            error: None,
        }
    }
}

pub struct HistoryEntry {
    pub mv: Move,
    pub state: StateInfo,
    pub san: String,
    pub moved_by: Color,
    pub position_after: Position,
    /// Filled for moves the user made when auto-retrospective is on
    /// and the worker has returned the analysis. Carries raw data so
    /// each renderer formats with its own [`NarrationOptions`] (and a
    /// future GUI can ignore the text entirely and draw arrows /
    /// highlights from the per-term deltas).
    pub retrospective: Option<RetrospectiveResult>,
    /// Filled for moves the engine made. Carries score / depth / time.
    pub engine_info: Option<EngineInfo>,
    /// `Some` when noise drove the bot off the engine's preferred
    /// move. Both desktop and CLI surface this — desktop logs it to
    /// stderr; CLI prints it inline with the played-move line.
    pub noise_pick: Option<crate::worker::NoisePickInfo>,

    // ---- Trap library bookkeeping ----
    /// Snapshot of Session's `pending_trap` *before* this move was
    /// applied. Used by [`Session::dispatch`] takeback to roll the
    /// trap cursor back in lockstep with the position.
    pub pending_trap_before: Option<PendingTrap>,
    /// Move-by-move events the trap cursor emitted as this move was
    /// applied (`PunisherExecuted`, `DefenderInTree`, etc.). Empty
    /// when no trap was active. Renderers iterate to print prose
    /// (CLI) or surface badges (future GUI).
    pub trap_events: Vec<TrapEvent>,
    /// `Some` when this move triggered a *new* trap (i.e. the
    /// opponent walked into a known refutation). Distinct from
    /// `trap_events`: that field narrates the continuation of an
    /// already-active trap; this one marks the trigger move itself.
    pub trap_hit: Option<TrapHit>,
}

/// Raw retrospective output for one user move. The worker computes
/// `analyses` via `analyze_position` (which is what the narration
/// crate consumes) plus the timing surface the CLI reports. Each
/// renderer formats text from these on demand — the worker does no
/// prose formatting itself.
#[derive(Clone)]
pub struct RetrospectiveResult {
    pub user_move: Move,
    pub analyses: Vec<chess_tutor_engine::analysis::MoveAnalysis>,
    pub elapsed: Duration,
    pub nodes: u64,
    pub nps_m: f64,
}

/// Result of a synchronous [`Session::run_analysis`] call — raw
/// analyses + the perf surface CLI's REPL prints. The CLI's REPL
/// `search` and `analyze` commands both consume this; they differ
/// only in how they format the contents (one PV table vs. per-term
/// breakdown).
#[derive(Default)]
pub struct AnalysisOutcome {
    pub analyses: Vec<chess_tutor_engine::analysis::MoveAnalysis>,
    pub elapsed: Duration,
    pub nodes: u64,
    pub nps_m: f64,
}

/// Which side(s) the engine plays in the current game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineMode {
    /// Neither side is the engine — user controls both colours.
    None,
    /// Engine plays the given colour; user plays the other.
    Side(Color),
    /// Engine plays both sides (self-play). User never moves.
    Both,
}

impl EngineMode {
    /// True when `side` is whose move it is and the engine should pick it.
    pub fn is_engine_turn(self, side: Color) -> bool {
        match self {
            EngineMode::None => false,
            EngineMode::Side(c) => side == c,
            EngineMode::Both => true,
        }
    }
}

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

pub(crate) struct PendingPromotion {
    /// Promotion-rank square — target of every candidate, and the
    /// anchor for the picker stack.
    pub(crate) to: Square,
    /// The four legal promotion moves with shared `from` / `to`. Order
    /// is Q, R, B, N to match the on-screen stack.
    pub(crate) candidates: [Move; 4],
}

pub(crate) struct HintResult {
    /// Position the analyses are *for* — needed to format SAN of
    /// candidate moves and PV plies on render. Identification of
    /// which position this corresponds to happens at arrival time
    /// (via `for_key` matching `self.position.key()`); once stored
    /// the position itself carries everything the panel needs.
    pub(crate) pos: Position,
    pub(crate) analyses: Vec<chess_tutor_engine::analysis::MoveAnalysis>,
}

impl Session {
    pub fn new(repaint: RepaintFn) -> Self {
        let (job_tx, job_rx) = mpsc::channel::<WorkerJob>();
        let (result_tx, result_rx) = mpsc::channel::<WorkerResult>();
        thread::spawn(move || worker_loop(job_rx, result_tx, repaint));

        // First-launch behaviour: open the New Game dialog
        // immediately so the user picks difficulty / colour before
        // the bot makes a move. The board still renders behind the
        // modal, but `engine_plays = None` keeps the engine idle
        // until Start commits the configuration.
        let position = Position::startpos();
        let position_keys = vec![position.key()];
        Self {
            start_position: position.clone(),
            position,
            position_keys,
            history: Vec::new(),
            selected: None,
            legal_from_selected: Vec::new(),
            flipped: false,
            engine_plays: EngineMode::None,
            depth: DEFAULT_DEPTH,
            retrospective_depth: ANALYTICAL_DEPTH,
            log_to_stderr: true,
            auto_retrospective: true,
            worker_tx: job_tx,
            worker_rx: result_rx,
            gen: 0,
            engine_thinking: false,
            viewing_index: None,
            new_game_form: Some(NewGameForm::initial()),
            hint_open: false,
            hint_thinking: false,
            hint_result: None,
            opponent: OpponentProfile::new_random(),
            book_cursor: None,
            book_out_announced: false,
            first_launch: true,
            pending_promotion: None,
            pending_trap: None,
            selected_retrospective: None,
            show_all_signals: false,
            active_overlays: std::collections::HashSet::new(),
            learning: LearningPreferences::default(),
            pending_intervention: None,
            awaiting_intervention_decision: false,
            game_review_open: false,
        }
    }

    /// Start a fresh game directly, bypassing the New Game dialog.
    /// Used by CLI / headless callers; the desktop goes through
    /// [`Self::try_start_from_form`] via the dialog widget.
    pub fn start_game(
        &mut self,
        position: Position,
        engine_plays: EngineMode,
        depth: u32,
        opponent: OpponentProfile,
    ) {
        self.new_game_form = None;
        self.first_launch = false;
        self.gen = self.gen.wrapping_add(1);
        self.engine_thinking = false;
        self.position_keys = vec![position.key()];
        self.start_position = position.clone();
        self.position = position;
        self.history.clear();
        self.deselect();
        self.viewing_index = None;
        self.engine_plays = engine_plays;
        self.depth = depth;
        self.opponent = opponent;
        self.book_cursor = BookCursor::new(&self.opponent, &self.position);
        self.book_out_announced = false;
        self.pending_trap = None;
        self.selected_retrospective = None;
        self.pending_intervention = None;
        self.awaiting_intervention_decision = false;
        self.game_review_open = false;
        if self.log_to_stderr {
            log_new_game_intro(&self.opponent);
        }
        self.close_hint();
        let _ = self.worker_tx.send(WorkerJob::NewGame);
        self.maybe_queue_engine_search();
    }

    /// Toggle Session's stderr logging of book picks / opening seed /
    /// noise-pick events. Defaults to `true` (desktop's "shell window
    /// is the de facto log" model); CLI sets `false` and surfaces the
    /// same data through its own stdout output.
    pub fn set_log_to_stderr(&mut self, enabled: bool) {
        self.log_to_stderr = enabled;
    }

    /// Toggle the auto-retrospective worker job. Defaults to `true`;
    /// CLI callers that run their own retrospective set this to
    /// `false` so [`Self::apply_user_move`] doesn't queue a redundant
    /// search.
    pub fn set_auto_retrospective(&mut self, enabled: bool) {
        self.auto_retrospective = enabled;
    }

    /// Current auto-retrospective state. CLI's REPL `retrospect`
    /// command queries this.
    pub fn auto_retrospective(&self) -> bool {
        self.auto_retrospective
    }

    /// Depth used for auto-retrospective worker jobs. CLI calls this
    /// to honour `--retrospective-depth`; desktop leaves it at the
    /// default [`ANALYTICAL_DEPTH`].
    pub fn set_retrospective_depth(&mut self, depth: u32) {
        self.retrospective_depth = depth;
    }

    /// Position the current game started from. Lets headless callers
    /// reconstruct the pre-move position for any history index when
    /// they need to format a retrospective from raw analyses (the
    /// pre-move pos for `history[i]` is `history[i-1].position_after`,
    /// falling back to this when `i == 0`).
    pub fn start_position(&self) -> &Position {
        &self.start_position
    }

    fn start_new_game(
        &mut self,
        position: Position,
        engine_plays: EngineMode,
        depth: u32,
        noise: NoiseProfile,
        eval_mask: EvalMask,
    ) {
        self.gen = self.gen.wrapping_add(1);
        self.engine_thinking = false;
        self.position_keys = vec![position.key()];
        self.start_position = position.clone();
        self.position = position;
        self.history.clear();
        self.deselect();
        self.viewing_index = None;
        self.engine_plays = engine_plays;
        self.depth = depth;
        // Fresh seed + curated book for the new game; carry over the
        // noise + eval-mask settings the user picked in the dialog.
        self.opponent = OpponentProfile::new_random();
        self.opponent.noise = noise;
        self.opponent.eval_mask = eval_mask;
        self.book_cursor = BookCursor::new(&self.opponent, &self.position);
        self.book_out_announced = false;
        self.pending_trap = None;
        self.selected_retrospective = None;
        self.pending_intervention = None;
        self.awaiting_intervention_decision = false;
        self.game_review_open = false;
        if self.log_to_stderr {
            log_new_game_intro(&self.opponent);
        }
        self.close_hint();
        let _ = self.worker_tx.send(WorkerJob::NewGame);
        self.maybe_queue_engine_search();
    }

    pub(crate) fn close_hint(&mut self) {
        self.hint_open = false;
        self.hint_thinking = false;
        self.hint_result = None;
    }

    pub(crate) fn toggle_hint(&mut self) {
        if self.hint_open {
            self.close_hint();
            return;
        }
        // Open and queue an Analyze job for the current live position.
        self.hint_open = true;
        self.hint_thinking = true;
        self.hint_result = None;
        let _ = self.worker_tx.send(WorkerJob::Analyze {
            pos: Box::new(self.position.clone()),
            // Analytical paths use ANALYTICAL_DEPTH, independent of
            // self.depth (the bot's play depth). See the constant
            // for rationale.
            depth: ANALYTICAL_DEPTH,
            multi_pv: HINT_MULTI_PV,
            game_history: game_history_for_search(&self.position_keys),
            for_key: self.position.key(),
        });
    }

    pub(crate) fn open_new_game_dialog(&mut self) {
        // Idempotent: don't trample unsaved tweaks if the user double-
        // clicks the button or hits it while the dialog is already up.
        if self.new_game_form.is_some() {
            return;
        }
        self.new_game_form = Some(NewGameForm::from_current(self));
    }

    pub(crate) fn try_start_from_form(&mut self) {
        let Some(form) = self.new_game_form.as_mut() else {
            return;
        };
        let position = if form.fen.trim().is_empty() {
            Position::startpos()
        } else {
            match Position::from_fen(form.fen.trim()) {
                Ok(p) => p,
                Err(e) => {
                    form.error = Some(format!("Invalid FEN: {e}"));
                    return;
                }
            }
        };
        let engine_plays = match form.color {
            ColorChoice::White => EngineMode::Side(Color::Black),
            ColorChoice::Black => EngineMode::Side(Color::White),
            ColorChoice::Random => {
                if random_bit() == 0 {
                    EngineMode::Side(Color::Black) // user is white
                } else {
                    EngineMode::Side(Color::White) // user is black
                }
            }
            ColorChoice::Both => EngineMode::None,
        };
        let depth = form.depth;
        let noise = form.noise.clone();
        let eval_mask = form.eval_mask;
        self.new_game_form = None;
        self.first_launch = false;
        self.start_new_game(position, engine_plays, depth, noise, eval_mask);
    }

    pub(crate) fn handle_click(&mut self, sq: Square) {
        // Don't let board clicks fall through when the New Game modal
        // is up — egui Windows don't block clicks below them by
        // default, so without this guard the user could move pieces
        // through the dialog (and at first launch `engine_plays` is
        // None, so `is_users_turn` would say yes).
        if self.new_game_form.is_some() {
            return;
        }
        // Clicks on the board while viewing back snap to live first;
        // the click itself doesn't otherwise act this frame.
        if self.viewing_index.is_some() {
            self.viewing_index = None;
            return;
        }
        if self.engine_thinking || !self.is_users_turn() {
            return;
        }
        if Some(sq) == self.selected {
            self.deselect();
            return;
        }
        if self.selected.is_some() && self.try_move_to(sq) {
            self.maybe_queue_engine_search();
            return;
        }
        self.select(sq);
    }

    pub(crate) fn is_users_turn(&self) -> bool {
        !self.engine_plays.is_engine_turn(self.position.side_to_move())
    }

    fn select(&mut self, sq: Square) {
        match self.position.piece_on(sq) {
            Some(piece) if piece.color() == self.position.side_to_move() => {
                self.selected = Some(sq);
                let mut scratch = self.position.clone();
                self.legal_from_selected = legal_moves_vec(&mut scratch)
                    .into_iter()
                    .filter(|m| m.from() == sq)
                    .collect();
            }
            _ => self.deselect(),
        }
    }

    fn try_move_to(&mut self, target: Square) -> bool {
        let candidates: Vec<Move> = self
            .legal_from_selected
            .iter()
            .copied()
            .filter(|m| m.to() == target)
            .collect();
        if candidates.is_empty() {
            return false;
        }

        // Promotion: legal-move generation produces one move per piece
        // type (Q / R / B / N). Open the picker instead of silently
        // queening — `apply_promotion_choice` will run once the user
        // clicks one of the four pieces.
        if candidates.iter().all(|m| m.kind() == MoveKind::Promotion) {
            if let Some(pending) = build_pending_promotion(&candidates) {
                self.pending_promotion = Some(PendingPromotion {
                    to: target,
                    candidates: pending,
                });
                return true;
            }
        }

        let mv = candidates[0];
        self.apply_user_move(mv);
        true
    }

    /// Apply a user move and queue the engine's reply if it's now the
    /// engine's turn. The convenience entry point for CLI / headless
    /// callers that parse a [`Move`] directly (SAN / UCI input). The
    /// desktop's click path goes through [`Self::apply_user_move`] +
    /// [`Self::maybe_queue_engine_search`] separately because it
    /// re-resolves through [`Event::SelectSquare`].
    pub fn play_user_move(&mut self, mv: Move) {
        self.apply_user_move(mv);
        self.maybe_queue_engine_search();
    }

    /// Finalise a move chosen via the regular click path *or* the
    /// promotion picker. Snapshots pre-move state for the retrospective
    /// job (when [`Self::auto_retrospective`] is set), applies the
    /// move, and clears the hint panel.
    ///
    /// Sets [`Self::awaiting_intervention_decision`] when the user's
    /// preferences want the classifier to run on this move — that
    /// flag causes `maybe_queue_engine_search` to hold the bot
    /// reply until the classifier returns (or the user resolves the
    /// resulting prompt). Without auto-retrospective there's no
    /// classifier to wait for, so the flag stays false.
    pub fn apply_user_move(&mut self, mv: Move) {
        if self.auto_retrospective {
            let pre_move_pos = self.position.clone();
            let pre_move_history = game_history_for_search(&self.position_keys);
            self.apply_move(mv);
            let target_index = self.history.len() - 1;
            if self.intervention_mode_active() {
                self.awaiting_intervention_decision = true;
            }
            let _ = self.worker_tx.send(WorkerJob::Retrospective {
                pre_move_pos: Box::new(pre_move_pos),
                user_move: mv,
                // Independent of self.depth (the bot's play depth) so a
                // weakened bot still gives strong teaching feedback.
                depth: self.retrospective_depth,
                game_history: pre_move_history,
                gen: self.gen,
                target_index,
            });
        } else {
            self.apply_move(mv);
        }
        self.close_hint();
    }

    /// `true` when the user's learning preferences want the engine
    /// classifier to inspect each user move (and pause the game if it
    /// flags one). Both gates — blunder safety and mistake handling —
    /// route through the classifier; we only skip when *neither* is
    /// active.
    fn intervention_mode_active(&self) -> bool {
        !matches!(
            self.learning.mistake_handling,
            MistakeHandling::SilentRetrospective
        ) || matches!(
            self.learning.blunder_safety,
            crate::learning_mode::BlunderSafety::OfferTakeback
        )
    }

    fn apply_move(&mut self, mv: Move) {
        let san_str = san::format(&self.position, mv);
        let moved_by = self.position.side_to_move();

        // ---- Trap bookkeeping, pre-move pass ----
        // Snapshot for undo restore.
        let pending_trap_before = self.pending_trap.clone();
        // Advance the cursor (if any). The pre-move position is what
        // `advance_pending` wants — the cursor was scripted against
        // moves played FROM the position before each ply.
        let mut trap_events = Vec::new();
        if let Some(pending) = self.pending_trap.as_mut() {
            let event = traps::advance_pending(pending, &self.position, mv);
            let terminal = event.is_terminal();
            trap_events.push(event);
            if terminal {
                self.pending_trap = None;
            }
        }
        // Capture pre-move data the post-move scan needs (piece kind
        // can only be read while the source square still has the
        // piece).
        let scan_inputs = self
            .position
            .piece_on(mv.from())
            .map(|piece| (moved_by, piece.kind(), mv.from(), mv.to()));

        let state = self.position.do_move(mv);
        self.position_keys.push(self.position.key());

        // ---- Trap bookkeeping, post-move pass ----
        let mut trap_hit = None;
        if self.pending_trap.is_none() {
            if let Some((mover, piece_kind, from, to)) = scan_inputs {
                if let Some((entry, hit)) =
                    traps::scan_after_move(&self.position, mover, piece_kind, from, to)
                        .into_iter()
                        .next()
                {
                    trap_hit = Some(hit.clone());
                    self.pending_trap = Some(PendingTrap::new(entry, hit));
                }
            }
        }

        self.history.push(HistoryEntry {
            mv,
            state,
            san: san_str,
            moved_by,
            position_after: self.position.clone(),
            retrospective: None,
            engine_info: None,
            noise_pick: None,
            pending_trap_before,
            trap_events,
            trap_hit,
        });
        // No book-cursor advance: BookCursor is stateless and
        // re-derives from history at each peek. Takeback is similarly
        // free of book bookkeeping.
        self.deselect();
    }

    pub(crate) fn takeback(&mut self) {
        if self.engine_thinking {
            self.gen = self.gen.wrapping_add(1);
            self.engine_thinking = false;
        } else {
            // Bump anyway: pending retrospective jobs (which don't
            // toggle engine_thinking) need to be invalidated.
            self.gen = self.gen.wrapping_add(1);
        }
        // Any active or pending intervention referred to the move
        // we're about to undo — drop it so the panel snaps back to
        // the normal retrospective surface.
        self.pending_intervention = None;
        self.awaiting_intervention_decision = false;
        self.game_review_open = false;
        self.viewing_index = None;
        self.close_hint();
        self.undo_one();
        // In user-vs-engine mode, takeback returns to the user's
        // prior turn — undo a second ply if we just landed on the
        // engine's turn. Self-play (Both) and user-plays-both (None)
        // are both happy with a single ply rewind.
        if let EngineMode::Side(eng_color) = self.engine_plays {
            if self.position.side_to_move() == eng_color && !self.history.is_empty() {
                self.undo_one();
            }
        }
        // Re-arm the "out of book" announcement: the user may now
        // be back in book territory (the cursor will re-derive that
        // on its next peek), and either way, if they deviate again
        // they should see the line print again.
        self.book_out_announced = false;
        self.maybe_queue_engine_search();
    }

    fn undo_one(&mut self) {
        if let Some(entry) = self.history.pop() {
            self.position.undo_move(entry.mv, entry.state);
            self.position_keys.pop();
            // Roll the trap cursor back to its pre-move snapshot so
            // the refutation tree is walked in lockstep with the
            // position.
            self.pending_trap = entry.pending_trap_before;
            // No book-cursor restore needed — the stateless cursor
            // re-derives from history on the next peek.
            self.deselect();
        }
    }

    pub(crate) fn deselect(&mut self) {
        self.selected = None;
        self.legal_from_selected.clear();
        self.pending_promotion = None;
    }

    pub(crate) fn maybe_queue_engine_search(&mut self) {
        // Loop because in self-play (EngineMode::Both) consecutive book
        // moves can fire synchronously — after each one it's *still*
        // the engine's turn, so we keep playing book moves until we hit
        // an out-of-book position and queue an actual search (or the
        // game ends). For user-vs-engine flows the loop iterates at
        // most once: after one engine ply it's the user's turn and the
        // top-of-loop guard returns.
        loop {
            if self.engine_thinking {
                return;
            }
            // Hold the engine reply while we're either (a) showing an
            // intervention prompt to the user or (b) waiting for the
            // classifier to decide whether one's needed. The
            // intervention-response events and the Retrospective
            // worker arrival path are responsible for re-calling this
            // method once the wait clears.
            if self.pending_intervention.is_some() || self.awaiting_intervention_decision {
                return;
            }
            if !self.engine_plays.is_engine_turn(self.position.side_to_move()) {
                return;
            }
            let mut scratch = self.position.clone();
            if legal_moves_vec(&mut scratch).is_empty() {
                return;
            }
            // Book first: walk allowed openings for any whose stored
            // move-prefix still matches the moves played so far; if any
            // match, play the deterministically-picked next move
            // synchronously and skip the worker round-trip entirely.
            let history_moves: Vec<Move> = self.history.iter().map(|e| e.mv).collect();
            let book_pick = self
                .book_cursor
                .as_ref()
                .and_then(|c| c.peek(&history_moves));
            if let Some(book_pick) = book_pick {
                if self.log_to_stderr {
                    let san_str = san::format(&self.position, book_pick.mv);
                    if let Some(entry) = chess_tutor_engine::openings::entry(book_pick.opening_id) {
                        eprintln!("book: engine plays {} ({} {})", san_str, entry.eco, entry.name);
                    } else {
                        eprintln!("book: engine plays {}", san_str);
                    }
                }
                // A successful book pick clears the "we've announced
                // out-of-book" flag — the user may have taken back to
                // an in-book position, and if they later deviate again
                // we want the announcement to print fresh.
                self.book_out_announced = false;
                self.apply_move(book_pick.mv);
                continue;
            }
            // No book match on this position. Announce once per
            // out-of-book streak — *don't* drop the cursor itself,
            // because a takeback might bring us back into book
            // territory and we need peek to keep working on the next
            // bot turn.
            if self.book_cursor.is_some() && !self.book_out_announced {
                if self.log_to_stderr {
                    eprintln!("out of book — engine now plays from search.");
                }
                self.book_out_announced = true;
            }
            return self.dispatch_engine_search();
        }
    }

    /// Dispatch a [`WorkerJob::Search`] for the current position. The
    /// caller is responsible for checking `engine_thinking` /
    /// `is_engine_turn` first — extracted from
    /// [`Self::maybe_queue_engine_search`] only so the book-pick loop
    /// can fall through to "queue a real search and exit".
    fn dispatch_engine_search(&mut self) {
        let params = SearchParams {
            max_depth: self.depth,
            max_nodes: Some(ENGINE_TURN_NODE_CAP),
            max_time: None,
            // Bot noise widens this beyond 1 when the opponent profile
            // wants alternatives to sample from; off-profile keeps the
            // engine's single-PV fast path.
            multi_pv: self.opponent.noise.effective_multi_pv(),
            game_history: game_history_for_search(&self.position_keys),
            force_include: Vec::new(),
            verbose_progress: false,
            // Engine moves: single-threaded. We're targeting iOS where
            // single-core utilisation is much friendlier to the
            // thermal/battery envelope, and at depth 10 startpos the
            // single-thread search finishes in ~40 ms — perceptually
            // instant. Multi-thread is kept available through the CLI
            // `--threads N` flag for bench / dev work.
            threads: 1,
            // Play engine move — apply the opponent's mid-game eval
            // mask so the bot plays as if blind to the masked
            // categories.
            eval_mask: self.opponent.eval_mask,
        };
        self.engine_thinking = true;
        let _ = self.worker_tx.send(WorkerJob::Search {
            pos: Box::new(self.position.clone()),
            params,
            gen: self.gen,
            noise: self.opponent.noise.clone(),
            seed: self.opponent.seed,
            ply: self.position_keys.len() as u64,
        });
    }

    pub fn poll_worker(&mut self) {
        while let Ok(result) = self.worker_rx.try_recv() {
            self.handle_worker_result(result);
        }
    }

    fn handle_worker_result(&mut self, result: WorkerResult) {
        match result {
            WorkerResult::Search {
                gen,
                mv,
                line,
                noise_pick,
                elapsed,
                nodes,
                nps_m,
            } => {
                if gen != self.gen {
                    return;
                }
                self.engine_thinking = false;
                let Some(mv) = mv else {
                    return;
                };
                if self.log_to_stderr {
                    if let Some(info) = &noise_pick {
                        log_noise_pick_to_stderr(info, &self.position, mv, &self.opponent.noise);
                    }
                }
                let root_stm = self.position.side_to_move();
                self.apply_move(mv);
                // Wild picks have no SearchLine (no search for that
                // exact move); the per-move score badge stays empty.
                if let Some(line) = line {
                    let white_pov = if root_stm == Color::White {
                        line.score
                    } else {
                        -line.score
                    };
                    if let Some(entry) = self.history.last_mut() {
                        entry.engine_info = Some(EngineInfo {
                            score_white_pov: white_pov,
                            depth: line.depth,
                            elapsed,
                            nodes,
                            nps_m,
                        });
                    }
                }
                if let Some(entry) = self.history.last_mut() {
                    entry.noise_pick = noise_pick;
                }
                // Engine just moved — any open Hint was for the prior
                // position, so close it.
                self.close_hint();
                // Self-play (EngineMode::Both) needs us to queue the
                // *next* engine move after each completes; without this
                // the bot freezes after move 1. For EngineMode::Side
                // and EngineMode::None this is a no-op — the post-move
                // side-to-move isn't the engine, so the guard returns
                // immediately.
                self.maybe_queue_engine_search();
            }
            WorkerResult::Retrospective {
                gen,
                target_index,
                user_move,
                analyses,
                elapsed,
                nodes,
                nps_m,
            } => {
                if gen != self.gen {
                    return;
                }
                // Snapshot pre-move position before mutating the entry
                // — we need it for the classifier below and the
                // immutable borrow can't coexist with the later
                // `history.get_mut`.
                let pre_pos = (target_index <= self.history.len())
                    .then(|| self.pre_move_position(target_index));
                if let Some(entry) = self.history.get_mut(target_index) {
                    entry.retrospective = Some(RetrospectiveResult {
                        user_move,
                        analyses: analyses.clone(),
                        elapsed,
                        nodes,
                        nps_m,
                    });
                }
                // If we held the engine reply waiting for the
                // classifier to decide, decide now. The retrospective
                // we just received must be for the *latest* user move
                // — anything else is a stale arrival and we ignore it
                // for intervention purposes (the gen-check above
                // already filtered most of those).
                if self.awaiting_intervention_decision
                    && target_index + 1 == self.history.len()
                {
                    self.awaiting_intervention_decision = false;
                    let assessment = pre_pos.as_ref().map(|pp| {
                        chess_tutor_engine::analysis::classify_user_move(
                            pp,
                            &analyses,
                            user_move,
                            &gating_config_for(self.learning.mistake_handling),
                        )
                    });
                    if let Some(assessment) = assessment {
                        if intervention_required(&assessment, &self.learning) {
                            self.pending_intervention = Some(PendingIntervention {
                                at_history_index: target_index,
                                original_move: user_move,
                                assessment,
                                concept_revealed: false,
                            });
                        }
                    }
                    self.maybe_queue_engine_search();
                }
            }
            WorkerResult::Analyze { for_key, analyses } => {
                if !self.hint_open {
                    return;
                }
                if for_key != self.position.key() {
                    // Stale: hint was issued for a different position
                    // (e.g., user moved while it was queued).
                    return;
                }
                self.hint_thinking = false;
                let _ = for_key;
                self.hint_result = Some(HintResult {
                    pos: self.position.clone(),
                    analyses,
                });
            }
            WorkerResult::AnalyzeSync { .. } => {
                // Synchronous analyses are consumed inline by
                // [`Self::run_analysis`], not via the regular event
                // stream. Any AnalyzeSync result that reaches here is
                // a stale arrival — drop it.
            }
        }
    }

    pub fn game_outcome(&self) -> Option<&'static str> {
        let mut scratch = self.position.clone();
        if legal_moves_vec(&mut scratch).is_empty() {
            return Some(if self.position.in_check() {
                match self.position.side_to_move() {
                    Color::White => "Checkmate — Black wins.",
                    Color::Black => "Checkmate — White wins.",
                }
            } else {
                "Stalemate — draw."
            });
        }
        if self.position.halfmove_clock() >= 100 {
            return Some("Draw — 50-move rule.");
        }
        if threefold_reached(&self.position_keys) {
            return Some("Draw — threefold repetition.");
        }
        if self.position.has_insufficient_material() {
            return Some("Draw — insufficient material.");
        }
        None
    }

    // ---- View helpers ---------------------------------------------------

    pub(crate) fn viewed_entry(&self) -> Option<&HistoryEntry> {
        match self.viewing_index {
            Some(i) => self.history.get(i),
            None => self.history.last(),
        }
    }

    pub(crate) fn viewed_position(&self) -> &Position {
        match self.viewing_index {
            Some(i) => self
                .history
                .get(i)
                .map(|e| &e.position_after)
                .unwrap_or(&self.position),
            None => &self.position,
        }
    }

    pub(crate) fn is_viewing_live(&self) -> bool {
        self.viewing_index.is_none()
    }

    /// The most-recent post-move evaluation (white POV) at or before
    /// the currently viewed history index — used by the eval bar.
    ///
    /// Both engine moves (`engine_info`) and user moves (the
    /// retrospective worker's analysis of the user's chosen move) are
    /// valid sources. Scanning backward picks up the first either-or
    /// hit, so the bar updates on every move that has reached the
    /// analysis stage — not only engine moves.
    ///
    /// When the user is browsing back to a position whose retrospective
    /// hasn't arrived yet, we fall further back to the most recent
    /// pre-existing evaluation. That's an approximation, but it gives
    /// a sensible "trend" view while the worker catches up.
    pub(crate) fn viewed_eval_white_pov(&self) -> Option<Value> {
        let upper = self.viewing_index.map_or(self.history.len(), |i| i + 1);
        self.history[..upper].iter().rev().find_map(entry_eval_white_pov)
    }

    /// Picks the (index, entry) to show in the retrospective panel:
    ///   - Viewing back: the viewed entry.
    ///   - Live: the most recent user-move entry (so the engine's
    ///     reply doesn't bury the analysis of the user's own move).
    pub(crate) fn panel_entry_with_index(&self) -> Option<(usize, &HistoryEntry)> {
        if let Some(i) = self.viewing_index {
            return self.history.get(i).map(|e| (i, e));
        }
        if let Some(found) = self
            .history
            .iter()
            .enumerate()
            .rev()
            .find(|(_, e)| self.is_user_move(e))
        {
            return Some(found);
        }
        self.history
            .last()
            .map(|e| (self.history.len() - 1, e))
    }

    /// Pre-move position for history entry `i` — needed by anything
    /// that wants to format an analysis whose root was the position
    /// the user faced before their move.
    fn pre_move_position(&self, i: usize) -> Position {
        if i == 0 {
            self.start_position.clone()
        } else {
            self.history[i - 1].position_after.clone()
        }
    }

    pub(crate) fn is_user_move(&self, entry: &HistoryEntry) -> bool {
        match self.engine_plays {
            EngineMode::None => true,
            EngineMode::Side(c) => entry.moved_by != c,
            EngineMode::Both => false,
        }
    }

    /// "User's" colour for POV-flipped overlays. When the engine plays
    /// one side, the user is the other; otherwise we fall back to the
    /// side-to-move at the currently-viewed position (the natural POV
    /// for two-human / self-play modes).
    pub(crate) fn user_color(&self) -> Color {
        match self.engine_plays {
            EngineMode::Side(eng) => !eng,
            EngineMode::None | EngineMode::Both => self.viewed_position().side_to_move(),
        }
    }

    // ---- Public accessors (CLI / headless callers) ---------------------

    /// Current live position.
    pub fn position(&self) -> &Position {
        &self.position
    }

    /// Move history, in play order. Engine moves have
    /// [`HistoryEntry::engine_info`] populated; user moves have
    /// [`HistoryEntry::retrospective_text`] (when auto-retrospective
    /// is on).
    pub fn history(&self) -> &[HistoryEntry] {
        &self.history
    }

    /// Opponent profile (book selection, noise, eval mask, seed) for
    /// the current game.
    pub fn opponent(&self) -> &OpponentProfile {
        &self.opponent
    }

    /// Mutable opponent access. Most fields take effect on the next
    /// engine move (noise / eval-mask are read per search). Mutating
    /// `book` mid-game does *not* affect the active book cursor —
    /// that's frozen at game start; the field change applies to
    /// the next game.
    pub fn opponent_mut(&mut self) -> &mut OpponentProfile {
        &mut self.opponent
    }

    /// True between a [`Self::maybe_queue_engine_search`] that
    /// dispatched a worker job and the matching [`WorkerResult`]
    /// arriving. CLI / headless callers use this to decide whether
    /// to block on [`Self::wait_for_worker`] or prompt the user.
    pub fn is_engine_thinking(&self) -> bool {
        self.engine_thinking
    }

    /// Block until the next worker result arrives, process it, then
    /// drain any further results. Companion to [`Self::poll_worker`]
    /// for synchronous callers (CLI). Returns immediately if the
    /// worker channel is disconnected.
    pub fn wait_for_worker(&mut self) {
        if let Ok(result) = self.worker_rx.recv() {
            self.handle_worker_result(result);
        }
        self.poll_worker();
    }

    /// Run an analysis on `pos` with `params`, blocking until the
    /// worker returns. The CLI's REPL `search` and `analyze` commands
    /// use this so they don't need a private engine — the same
    /// analytical worker that powers retrospective and hint paths
    /// handles them too. Other worker results encountered while
    /// waiting are processed normally (engine moves applied to
    /// history, etc.). Returns an empty [`AnalysisOutcome`] if the
    /// worker channel is disconnected.
    pub fn run_analysis(&mut self, pos: Position, params: SearchParams) -> AnalysisOutcome {
        let _ = self.worker_tx.send(WorkerJob::AnalyzeSync {
            pos: Box::new(pos),
            params,
        });
        loop {
            match self.worker_rx.recv() {
                Ok(WorkerResult::AnalyzeSync {
                    analyses,
                    elapsed,
                    nodes,
                    nps_m,
                }) => {
                    return AnalysisOutcome {
                        analyses,
                        elapsed,
                        nodes,
                        nps_m,
                    };
                }
                Ok(other) => self.handle_worker_result(other),
                Err(_) => return AnalysisOutcome::default(),
            }
        }
    }

    /// Engine mode for the current game.
    pub fn engine_plays(&self) -> EngineMode {
        self.engine_plays
    }

    /// Live trap cursor. `Some` between a trap firing and its
    /// refutation tree reaching a terminal node. Renderers use this
    /// to decide whether to suppress pre-move threat warnings (CLI)
    /// or to surface a "trap active" badge (future GUI).
    pub fn pending_trap(&self) -> Option<&PendingTrap> {
        self.pending_trap.as_ref()
    }

    /// Pre-move trap threats for the side currently to move: legal
    /// moves that would walk into a known refutation. Computed fresh
    /// each call (the underlying library scan is cheap). Renderers
    /// typically suppress this when [`Self::pending_trap`] is already
    /// `Some` — a trap mid-refutation is doing its own narration.
    pub fn trap_threats(&self) -> Vec<TrapThreatened> {
        traps::scan_threats(&self.position)
    }

    /// Bot-play depth.
    pub fn depth(&self) -> u32 {
        self.depth
    }

    /// Current learning preferences (assistance level, mistake
    /// handling, blunder safety, reveal-best-moves).
    pub fn learning_preferences(&self) -> &LearningPreferences {
        &self.learning
    }

    /// Replace the full learning preferences bundle. Renderers can
    /// either dispatch [`Event::ApplyLearningPreset`] for the named
    /// presets or call this for per-axis edits.
    pub fn set_learning_preferences(&mut self, prefs: LearningPreferences) {
        self.learning = prefs;
    }

    /// `Some` while an intervention prompt is showing. CLI callers
    /// can inspect this to know the bot reply is being held.
    pub fn pending_intervention(&self) -> Option<&PendingIntervention> {
        self.pending_intervention.as_ref()
    }

    /// Walk every user move's cached retrospective analysis through
    /// the engine classifier and return the ranked list of moments
    /// worth reviewing. Returns `None` when the game has no user
    /// moves whose retrospective has arrived yet.
    ///
    /// Reuses [`crate::learning_mode::gating_config_for`] with the
    /// user's current `mistake_handling` preference so the same gate
    /// drives both the in-game prompt and the post-game review —
    /// switching to "AllMistakes" before opening review surfaces
    /// every non-best move, switching back tightens the list.
    pub fn build_game_review(&self) -> Option<crate::view::GameReviewView> {
        use crate::view::{GameReviewMoment, GameReviewView, ReviewMomentKind};

        let mut moments: Vec<GameReviewMoment> = Vec::new();
        let mut user_move_count: usize = 0;
        let config = gating_config_for(self.learning.mistake_handling);

        for (idx, entry) in self.history.iter().enumerate() {
            if !self.is_user_move(entry) {
                continue;
            }
            user_move_count += 1;
            let Some(retro) = entry.retrospective.as_ref() else {
                // Analysis hasn't arrived yet — skip silently. Most
                // common case is the very-latest move while the worker
                // is still computing.
                continue;
            };
            let pre = self.pre_move_position(idx);
            let assessment = chess_tutor_engine::analysis::classify_user_move(
                &pre,
                &retro.analyses,
                retro.user_move,
                &config,
            );
            let kind = match (&assessment.blunder, &assessment.teaching) {
                (Some(_), Some(_)) => ReviewMomentKind::BlunderWithLesson,
                (Some(_), None) => ReviewMomentKind::Blunder,
                (None, Some(_)) => ReviewMomentKind::TeachingMoment,
                (None, None) => continue,
            };
            let headline = review_headline_for(&assessment);
            let move_pair_number = idx / 2 + 1;
            let side_to_move_label = if entry.moved_by == Color::White {
                "White"
            } else {
                "Black"
            };
            moments.push(GameReviewMoment {
                history_index: idx,
                move_pair_number,
                side_to_move_label,
                san: entry.san.clone(),
                kind,
                headline,
            });
        }

        if user_move_count == 0 {
            return None;
        }
        Some(GameReviewView {
            game_outcome: self.game_outcome(),
            user_move_count,
            moments,
        })
    }

    /// Whether the game-review surface is currently being shown.
    pub fn is_game_review_open(&self) -> bool {
        self.game_review_open
    }

    // ---- Event dispatch ------------------------------------------------

    /// Apply a renderer-emitted intent. Centralising this here keeps
    /// the renderers stateless about *what* an interaction means — the
    /// session resolves all priority rules (cancel ordering, snap-to-
    /// live mapping, etc.).
    pub fn dispatch(&mut self, event: Event) {
        match event {
            Event::SelectSquare(sq) => self.handle_click(sq),
            Event::ConfirmPromotion(mv) => {
                self.pending_promotion = None;
                self.apply_user_move(mv);
                self.maybe_queue_engine_search();
            }
            Event::RequestNewGame => self.open_new_game_dialog(),
            Event::Takeback => self.takeback(),
            Event::FlipBoard => self.flipped = !self.flipped,
            Event::ToggleHint => self.toggle_hint(),
            Event::JumpToLive => self.viewing_index = None,
            Event::ChangeDepth(d) => self.depth = d,
            Event::ViewHistoryIndex(target) => {
                // Clicking the last move in the list means "back to
                // live", not "freeze on the live-equivalent index" —
                // otherwise the user can't distinguish viewing-live
                // from viewing-at-history-end.
                self.viewing_index = match target {
                    Some(i) if i + 1 == self.history.len() => None,
                    other => other,
                };
                // Clear retrospective selection when navigating to
                // a different move — annotations belong to the move
                // they describe, not whatever the user clicks next.
                self.selected_retrospective = None;
            }
            Event::SelectRetrospectiveItem(item_idx) => {
                let Some((entry_idx, _)) = self.panel_entry_with_index() else {
                    return;
                };
                // Toggle: clicking the selected card again deselects.
                self.selected_retrospective =
                    match self.selected_retrospective {
                        Some((h, i)) if h == entry_idx && i == item_idx => None,
                        _ => Some((entry_idx, item_idx)),
                    };
            }
            Event::ToggleShowAllSignals => {
                self.show_all_signals = !self.show_all_signals;
            }
            Event::ToggleOverlay(kind) => {
                if !self.active_overlays.remove(&kind) {
                    self.active_overlays.insert(kind);
                }
            }
            Event::Cancel => self.handle_cancel(),
            Event::ConfirmNewGame => self.try_start_from_form(),
            Event::ResetBotForm => {
                if let Some(f) = self.new_game_form.as_mut() {
                    f.noise = NoiseProfile::default();
                    f.eval_mask = EvalMask::EMPTY;
                }
            }
            Event::ApplyLearningPreset(preset) => {
                // Custom is a no-op when set externally; it just means
                // "the bundle was custom-tuned, don't touch it."
                if !matches!(preset, LearningPreset::Custom) {
                    self.learning = preset.to_preferences();
                }
            }
            Event::SetRevealBestMoves(on) => {
                self.learning.reveal_best_moves = on;
            }
            Event::ContinueDespitePrompt => {
                self.pending_intervention = None;
                self.maybe_queue_engine_search();
            }
            Event::RevealMissedConcept => {
                if let Some(p) = self.pending_intervention.as_mut() {
                    p.concept_revealed = true;
                }
            }
            Event::TakeBackDuringIntervention => {
                self.pending_intervention = None;
                self.awaiting_intervention_decision = false;
                self.takeback();
            }
            Event::OpenGameReview => {
                // Only meaningful when there's at least one user move;
                // for an empty history just leave the regular surface up.
                if self.history.iter().any(|e| self.is_user_move(e)) {
                    self.game_review_open = true;
                    self.close_hint();
                }
            }
            Event::CloseGameReview => {
                self.game_review_open = false;
            }
            Event::JumpToReviewMoment(history_index) => {
                if history_index < self.history.len() {
                    self.viewing_index = Some(history_index);
                    self.selected_retrospective = None;
                    self.game_review_open = false;
                }
            }
        }
    }

    /// Resolve [`Event::Cancel`]: promotion picker > open dialog >
    /// deselect. First-launch dialog is non-cancellable (no game to
    /// fall back to), so it's skipped in the dialog branch.
    fn handle_cancel(&mut self) {
        if self.pending_promotion.is_some() {
            // deselect() clears pending + selection together.
            self.deselect();
            return;
        }
        if self.new_game_form.is_some() && !self.first_launch {
            self.new_game_form = None;
            return;
        }
        self.deselect();
    }

    // ---- View builders -------------------------------------------------

    pub fn build_top_bar_view(&self) -> TopBarView {
        let hint_can_open = self.is_viewing_live()
            && !self.engine_thinking
            && self.is_users_turn()
            && self.game_outcome().is_none();
        let review_button_enabled = self.history.iter().any(|e| {
            self.is_user_move(e) && e.retrospective.is_some()
        });
        TopBarView {
            can_takeback: !self.history.is_empty(),
            hint_open: self.hint_open,
            hint_button_enabled: hint_can_open || self.hint_open,
            viewing_live: self.is_viewing_live(),
            depth: self.depth,
            engine_thinking: self.engine_thinking,
            game_outcome: self.game_outcome(),
            review_open: self.game_review_open,
            review_button_enabled,
        }
    }

    pub fn build_eval_bar_view(&self) -> EvalBarView {
        let score = self.viewed_eval_white_pov();
        let (white_ratio, label) = match score {
            Some(v) if v.abs() >= Value::MATE_IN_MAX_PLY => {
                if v.0 > 0 {
                    (1.0, format!("M{}", (Value::MATE.0 - v.0).max(1)))
                } else {
                    (0.0, format!("-M{}", (Value::MATE.0 + v.0).max(1)))
                }
            }
            Some(v) => {
                let ratio = (v.0 as f32 / EVAL_BAR_SATURATION_CP).clamp(-1.0, 1.0);
                let pawns = v.0 as f32 / Value::PAWN_MG.0 as f32;
                (0.5 + 0.5 * ratio, format!("{:+.2}", pawns))
            }
            None => (0.5, String::from("—")),
        };
        EvalBarView { white_ratio, label }
    }

    pub fn build_board_view(&self) -> BoardView {
        let viewed_pos = self.viewed_position().clone();
        let viewed_mv = self.viewed_entry().map(|e| e.mv);
        let live = self.is_viewing_live();
        let pending_promotion = self.pending_promotion.as_ref().map(|p| {
            PromotionPickerView::compose(
                p.to,
                p.candidates,
                self.position.side_to_move(),
                self.flipped,
            )
        });
        // When browsing back, suppress mouse-state overlays: the
        // selected piece and its legal-move dots belong to the *live*
        // position, not the historical one we're displaying. The
        // BoardCell.selected / move_dot fields stay None.
        let (selected, legals): (Option<Square>, &[Move]) = if live {
            (self.selected, &self.legal_from_selected)
        } else {
            (None, &[])
        };
        let annotations = self.collect_board_annotations();
        BoardView::compose(
            &viewed_pos,
            self.flipped,
            viewed_mv,
            selected,
            legals,
            pending_promotion,
            annotations,
        )
    }

    /// Gather any annotations to draw on the board. Sources:
    /// - Active board overlays (always-on, computed against the
    ///   currently-viewed position).
    /// - The currently-viewed user-move entry's retrospective: best-
    ///   move arrow always shown; the selected card's annotations
    ///   layer on top.
    /// - Future: trap-refutation arrows, pin renderer per HANDOFF-ux.
    fn collect_board_annotations(&self) -> Vec<crate::view::BoardAnnotation> {
        let mut out = Vec::new();

        // Overlays first, so retrospective annotations paint on top.
        if !self.active_overlays.is_empty() {
            crate::overlays_view::push_overlay_annotations(
                &mut out,
                &chess_tutor_engine::analysis::compute_overlays(self.viewed_position()),
                self.user_color(),
                &self.active_overlays,
            );
        }

        let Some((entry_idx, entry)) = self.panel_entry_with_index() else {
            return out;
        };
        if !self.is_user_move(entry) {
            return out;
        }
        let Some(result) = &entry.retrospective else {
            return out;
        };
        let pre = self.pre_move_position(entry_idx);
        let vm = crate::retrospective_view::build_retrospective_view(
            &pre,
            &result.analyses,
            result.user_move,
            self.show_all_signals,
            self.learning.reveal_best_moves,
        );
        if let Some(ann) = vm.headline.best_move_annotation {
            out.push(ann);
        }
        if let Some((selected_entry, item_idx)) = self.selected_retrospective {
            if selected_entry == entry_idx {
                if let Some(item) = vm.items.get(item_idx) {
                    out.extend(item.annotations.iter().copied());
                }
            }
        }
        out
    }

    /// Snapshot of the currently-active overlay set. Renderers consume
    /// this to draw the overlay checkboxes with the right initial
    /// state.
    pub fn active_overlays(&self) -> &std::collections::HashSet<crate::view::OverlayKind> {
        &self.active_overlays
    }

    pub fn build_side_panel_view(&self) -> SidePanelView {
        // Body priority, top to bottom:
        //   Intervention > GameReview (when explicitly opened)
        //     > Hint (when explicitly opened)
        //     > Coaching (live, when assistance = Coached, user's turn,
        //                 viewing live, game in progress)
        //     > Retrospective (the default)
        let body = if let Some(pending) = self.pending_intervention.as_ref() {
            SidePanelBody::Intervention(build_intervention_panel(pending))
        } else if self.game_review_open {
            // build_game_review returns None only when there are no
            // user moves at all — in that case fall back to the
            // regular retrospective so the panel isn't blank.
            match self.build_game_review() {
                Some(review) => SidePanelBody::GameReview(review),
                None => SidePanelBody::Retrospective(self.build_retrospective_view()),
            }
        } else if self.hint_open {
            SidePanelBody::Hint(self.build_hint_panel_view())
        } else if self.coaching_should_show() {
            SidePanelBody::Coaching(self.build_coaching_panel_view())
        } else {
            SidePanelBody::Retrospective(self.build_retrospective_view())
        };
        SidePanelView {
            moves: self.build_move_list_view(),
            body,
            active_overlays: self.active_overlays.clone(),
            learning: self.learning,
            stick_to_bottom: self.is_viewing_live(),
        }
    }

    fn build_move_list_view(&self) -> MoveListView {
        let viewing = self.viewing_index;
        let history_len = self.history.len();
        let rows = (0..history_len.div_ceil(2))
            .map(|pair| {
                let i_white = pair * 2;
                let i_black = i_white + 1;
                let white = MoveListCell {
                    history_index: i_white,
                    san: self.history[i_white].san.clone(),
                    selected: viewing == Some(i_white),
                };
                let black = self.history.get(i_black).map(|e| MoveListCell {
                    history_index: i_black,
                    san: e.san.clone(),
                    selected: viewing == Some(i_black),
                });
                MoveListRow {
                    move_pair_idx: pair + 1,
                    white,
                    black,
                }
            })
            .collect();
        MoveListView { rows }
    }

    fn build_retrospective_view(&self) -> RetrospectivePanelView {
        let game_outcome = self.game_outcome();
        let Some((entry_index, entry)) = self.panel_entry_with_index() else {
            return RetrospectivePanelView {
                game_outcome,
                body: RetrospectiveBody::NoMoves,
                show_all_signals: self.show_all_signals,
            };
        };
        let viewing_back_san = (!self.is_viewing_live()).then(|| entry.san.clone());
        let kind = if self.is_user_move(entry) {
            match &entry.retrospective {
                Some(result) => {
                    let pre = self.pre_move_position(entry_index);
                    let view_model = crate::retrospective_view::build_retrospective_view(
                        &pre,
                        &result.analyses,
                        result.user_move,
                        self.show_all_signals,
                        self.learning.reveal_best_moves,
                    );
                    let selected_item = match self.selected_retrospective {
                        Some((h, i)) if h == entry_index => Some(i),
                        _ => None,
                    };
                    RetrospectiveKind::UserMoveReady {
                        view_model: Box::new(view_model),
                        selected_item,
                    }
                }
                None => RetrospectiveKind::UserMoveAnalyzing,
            }
        } else if let Some(info) = &entry.engine_info {
            RetrospectiveKind::EngineMove {
                san: entry.san.clone(),
                eval_pawns: info.score_white_pov.0 as f32 / Value::PAWN_MG.0 as f32,
                depth: info.depth,
                elapsed_ms: info.elapsed.as_millis(),
            }
        } else {
            RetrospectiveKind::EngineInfoMissing
        };
        RetrospectivePanelView {
            game_outcome,
            body: RetrospectiveBody::Entry {
                viewing_back_san,
                kind,
            },
            show_all_signals: self.show_all_signals,
        }
    }

    /// Conditions for the live coaching panel to appear:
    /// - User explicitly turned it on via `AssistanceLevel::Coached`.
    /// - It's the user's turn (coaching applies to the live position
    ///   the student is about to move from).
    /// - The user is viewing live, not browsing back.
    /// - The game isn't over.
    /// - There's no other higher-priority body active (the caller
    ///   already drained those — this function only sees the lower-
    ///   priority case).
    pub(crate) fn coaching_should_show(&self) -> bool {
        matches!(
            self.learning.assistance,
            crate::learning_mode::AssistanceLevel::Coached
        ) && self.is_viewing_live()
            && self.is_users_turn()
            && self.game_outcome().is_none()
    }

    fn build_coaching_panel_view(&self) -> CoachingPanelView {
        let view_model =
            crate::coaching_view::build_coaching_view(&self.position, self.user_color());
        CoachingPanelView { view_model }
    }

    fn build_hint_panel_view(&self) -> HintPanelView {
        if self.hint_thinking && self.hint_result.is_none() {
            return HintPanelView {
                state: HintPanelState::Loading,
            };
        }
        let Some(result) = &self.hint_result else {
            return HintPanelView {
                state: HintPanelState::NoResult,
            };
        };
        if result.analyses.is_empty() {
            return HintPanelView {
                state: HintPanelState::NoMoves,
            };
        }
        let root_stm = result.pos.side_to_move();
        let entries: Vec<HintEntryView> = result
            .analyses
            .iter()
            .map(|ma| {
                let san = san::format(&result.pos, ma.mv);
                let score_str = format_score_root_pov(ma.score, root_stm);
                let pv_san = pv_to_san(&result.pos, &ma.pv);
                let settle_marker = ma.settled_ply.filter(|&i| i < pv_san.len());
                HintEntryView {
                    san,
                    score_str,
                    depth: ma.depth,
                    pv_san,
                    settle_marker,
                }
            })
            .collect();
        HintPanelView {
            state: HintPanelState::Ready(entries),
        }
    }

    pub fn build_new_game_dialog_view(&mut self) -> Option<NewGameDialogView<'_>> {
        let first_launch = self.first_launch;
        let form = self.new_game_form.as_mut()?;
        Some(NewGameDialogView { form, first_launch })
    }
}

/// Build the short headline shown for a moment in the game review
/// list. Mirrors the in-game prompt phrasing without ever naming the
/// engine's preferred move.
fn review_headline_for(
    assessment: &chess_tutor_engine::analysis::MoveAssessment,
) -> String {
    if let Some(b) = assessment.blunder {
        let pawns = (b.material_loss_cp as f32) / 100.0;
        return match b.lost_piece_square {
            Some(sq) => format!(
                "Material at risk: piece on {} ({:.1} pawns)",
                sq.to_algebraic(),
                pawns
            ),
            None => format!("Material at risk: {:.1} pawns", pawns),
        };
    }
    if let Some(t) = assessment.teaching {
        let (area_a, _) = crate::learning_mode::term_prompt_copy(t.dominant.term);
        return match t.secondary {
            None => format!(
                "Missed point: {} ({:.1} pawns concentrated)",
                area_a,
                (t.dominant.severity_cp as f32) / 100.0
            ),
            Some(secondary) => {
                let (area_b, _) = crate::learning_mode::term_prompt_copy(secondary.term);
                let combined = ((t.dominant.severity_cp + secondary.severity_cp) as f32) / 100.0;
                format!(
                    "Missed points: {} and {} ({:.1} pawns split)",
                    area_a, area_b, combined
                )
            }
        };
    }
    "Significant moment".to_string()
}

/// Saturation point for the eval bar's score→ratio mapping. Used by
/// [`Session::build_eval_bar_view`]; lives at module scope so the only
/// constant referenced by view-building stays adjacent to the
/// session.
const EVAL_BAR_SATURATION_CP: f32 = 1000.0;

/// Pick a post-move evaluation (white POV) off a single
/// [`HistoryEntry`]. Engine moves carry the score directly on
/// `engine_info`; user moves carry it on the retrospective's analysis
/// of the move they actually played. The eval bar walks history
/// backward through this so it updates on every move, not only engine
/// replies.
fn entry_eval_white_pov(e: &HistoryEntry) -> Option<Value> {
    if let Some(info) = &e.engine_info {
        return Some(info.score_white_pov);
    }
    let retro = e.retrospective.as_ref()?;
    let analysis = retro.analyses.iter().find(|a| a.mv == retro.user_move)?;
    Some(if e.moved_by == Color::White {
        analysis.score
    } else {
        -analysis.score
    })
}

pub(crate) fn game_history_for_search(position_keys: &[u64]) -> Vec<u64> {
    if position_keys.is_empty() {
        Vec::new()
    } else {
        position_keys[..position_keys.len() - 1].to_vec()
    }
}

fn random_bit() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
        & 1
}

/// Order the four promotion candidates from `candidates` into a fixed
/// `[Q, R, B, N]` array so the picker overlay can stack them in the
/// same order regardless of the order legal-move generation emitted.
/// Returns `None` if fewer than four pieces are represented (which
/// shouldn't happen for a real promotion but we keep the call site
/// defensive — under-promotion to a non-Q piece is rare in human play
/// but never illegal).
fn build_pending_promotion(candidates: &[Move]) -> Option<[Move; 4]> {
    let pick = |pt: PieceType| -> Option<Move> {
        candidates
            .iter()
            .copied()
            .find(|m| m.kind() == MoveKind::Promotion && m.promoted_to() == pt)
    };
    Some([
        pick(PieceType::Queen)?,
        pick(PieceType::Rook)?,
        pick(PieceType::Bishop)?,
        pick(PieceType::Knight)?,
    ])
}

/// True when the position currently at the back of `position_keys`
/// has appeared three or more times in the run-length-encoded path.
/// Mirrors `core/cli/src/play.rs::threefold_reached`.
fn threefold_reached(position_keys: &[u64]) -> bool {
    let Some(&current) = position_keys.last() else {
        return false;
    };
    position_keys.iter().filter(|&&k| k == current).count() >= 3
}

/// Log the new-game header to stderr: the opponent seed so a varied
/// game can be reproduced, and the picked opening line (if any) so the
/// user knows what they're up against. Sent to stderr — not the GUI —
/// because the desktop hasn't grown a status surface for this yet;
/// the launcher shell window is the de facto session log for now.
fn log_new_game_intro(opponent: &OpponentProfile) {
    eprintln!(
        "opponent seed: {} (record this to replay the game)",
        opponent.seed,
    );
    // No "book: X Y" intro line — with per-ply lookup the engine
    // hasn't committed to any specific opening at game start. The
    // opening that emerges is announced inline on each book move
    // (`book: engine plays X (ECO Name)`).
}

/// Emit a one-line stderr entry describing a noise-driven pick.
/// Extracted from [`Session::handle_worker_result`] so the same
/// log line still fires when `log_to_stderr` is on without
/// inlining the match over every variant in the hot path.
fn log_noise_pick_to_stderr(
    info: &NoisePickInfo,
    pos: &Position,
    mv: Move,
    noise: &NoiseProfile,
) {
    match info {
        NoisePickInfo::Softmax {
            pick_idx,
            num_lines,
            delta_from_top_cp,
        } => {
            eprintln!(
                "noise: softmax picked #{} of {} ({:+} cp from #1)",
                pick_idx + 1,
                num_lines,
                delta_from_top_cp,
            );
        }
        NoisePickInfo::Blunder {
            pick_idx,
            num_lines,
            delta_from_top_cp,
        } => {
            eprintln!(
                "noise: blunder picked #{} of {} ({:+} cp from #1)",
                pick_idx + 1,
                num_lines,
                delta_from_top_cp,
            );
        }
        NoisePickInfo::BlunderSkipped {
            closest_above_loss_cp,
        } => {
            let cap = (noise.blunder_max_loss_cp as f32
                * chess_tutor_engine::noise::BLUNDER_FALLBACK_TOLERANCE)
                as i32;
            eprintln!(
                "noise: blunder roll fired but closest plausible alternative \
                 was -{closest_above_loss_cp} cp (exceeds {}× max-loss = {} cp \
                 cap); bot plays best.",
                chess_tutor_engine::noise::BLUNDER_FALLBACK_TOLERANCE,
                cap,
            );
        }
        NoisePickInfo::Wild {
            engine_top,
            engine_top_score,
        } => {
            eprintln!(
                "noise: wild — bot played {} (engine preferred {} at {} cp)",
                san::format(pos, mv),
                san::format(pos, *engine_top),
                engine_top_score.0,
            );
        }
    }
}

/// Format a score for display in the hint panel. Root-stm POV is
/// the natural reading there ("if you play this, you'll be at
/// +0.30").
fn format_score_root_pov(score: Value, _root_stm: Color) -> String {
    if score.abs() >= Value::MATE_IN_MAX_PLY {
        if score.0 > 0 {
            format!("M{}", (Value::MATE.0 - score.0).max(1))
        } else {
            format!("-M{}", (Value::MATE.0 + score.0).max(1))
        }
    } else {
        let pawns = score.0 as f32 / Value::PAWN_MG.0 as f32;
        format!("{:+.2}", pawns)
    }
}

/// Walk a PV applying moves to a clone of `root` and producing a
/// SAN per ply. Stops on any ply that doesn't apply cleanly
/// (shouldn't happen with a real PV from the engine).
fn pv_to_san(root: &Position, pv: &[Move]) -> Vec<String> {
    let mut out = Vec::with_capacity(pv.len());
    let mut pos = root.clone();
    for mv in pv {
        out.push(san::format(&pos, *mv));
        pos.do_move(*mv);
    }
    out
}
