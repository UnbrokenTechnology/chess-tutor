//! Session-adjacent value types: engine info, color choice, the new-game
//! form, history entries, retrospective / analysis results, and the
//! engine-mode enum.

use super::*;
use std::time::Duration;

use chess_tutor_engine::opponent::{EvalMask, NoiseProfile};
use chess_tutor_engine::position::{Position, StateInfo};
use chess_tutor_engine::traps::{PendingTrap, TrapEvent, TrapHit};
use chess_tutor_engine::types::{Color, Move, Square, Value};


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
    /// Move-feedback (retrospective) search depth — how deeply each
    /// played move is analysed for the backward-looking feedback zone.
    /// Independent of the bot's play `depth`.
    pub retrospective_depth: u32,
    /// Bot move-sampling knobs. Persists across New Game clicks so the
    /// user can tune incrementally between games without losing prior
    /// settings.
    pub noise: NoiseProfile,
    /// Eval categories the bot is blind to. Same persistence rule.
    pub eval_mask: EvalMask,
    /// Learning-mode preferences set up on the Start screen — the
    /// Support intervention pause, auto-coach, and best-move reveal.
    /// This screen is their true home (PLAN build-order step 5); the
    /// mid-game ⚙ gear edits the same set on the live session.
    pub learning: crate::learning_mode::LearningPreferences,
    /// Board overlays to start the game with. Same true-home rationale
    /// as `learning`.
    pub active_overlays: std::collections::HashSet<crate::view::OverlayKind>,
    /// Whether the chess.com-style eval bar is shown.
    pub show_eval_bar: bool,
    pub error: Option<String>,
}

impl NewGameForm {
    /// Pre-populate from the live game so the dialog reflects what
    /// the user is currently playing against — encourages incremental
    /// tweaking rather than rebuilding settings from scratch every
    /// time they click New Game.
    pub(crate) fn from_current(session: &Session) -> Self {
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
            retrospective_depth: session.retrospective_depth,
            noise: session.opponent.noise.clone(),
            eval_mask: session.opponent.eval_mask,
            learning: session.learning,
            active_overlays: session.active_overlays.clone(),
            show_eval_bar: session.show_eval_bar,
            error: None,
        }
    }

    /// Defaults for the first-launch dialog — same shape as
    /// [`Self::from_current`] would produce for a freshly constructed
    /// [`Session`], but without needing one to exist yet.
    pub(crate) fn initial() -> Self {
        Self {
            color: ColorChoice::White,
            fen: String::new(),
            depth: DEFAULT_DEPTH,
            retrospective_depth: ANALYTICAL_DEPTH,
            noise: NoiseProfile::default(),
            eval_mask: EvalMask::EMPTY,
            learning: crate::learning_mode::LearningPreferences::default(),
            active_overlays: std::collections::HashSet::new(),
            show_eval_bar: true,
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

pub(crate) struct PendingPromotion {
    /// Promotion-rank square — target of every candidate, and the
    /// anchor for the picker stack.
    pub(crate) to: Square,
    /// The four legal promotion moves with shared `from` / `to`. Order
    /// is Q, R, B, N to match the on-screen stack.
    pub(crate) candidates: [Move; 4],
}

