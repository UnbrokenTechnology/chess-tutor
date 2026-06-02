//! Session lifecycle: construction, game start / reset, the config
//! setters, and the new-game dialog flow.

use super::*;
use std::sync::mpsc::{self};
use std::thread;

use chess_tutor_engine::book::BookCursor;
use chess_tutor_engine::opponent::{EvalMask, NoiseProfile, OpponentProfile};
use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::{Color, Move, MoveKind, PieceType};

use crate::learning_mode::LearningPreferences;
use crate::worker::{worker_loop, WorkerJob, WorkerResult};

pub(crate) fn game_history_for_search(position_keys: &[u64]) -> Vec<u64> {
    if position_keys.is_empty() {
        Vec::new()
    } else {
        position_keys[..position_keys.len() - 1].to_vec()
    }
}

pub(crate) fn random_bit() -> u64 {
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
pub(crate) fn build_pending_promotion(candidates: &[Move]) -> Option<[Move; 4]> {
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
pub(crate) fn threefold_reached(position_keys: &[u64]) -> bool {
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
pub(crate) fn log_new_game_intro(opponent: &OpponentProfile) {
    eprintln!(
        "opponent seed: {} (record this to replay the game)",
        opponent.seed,
    );
    // No "book: X Y" intro line — with per-ply lookup the engine
    // hasn't committed to any specific opening at game start. The
    // opening that emerges is announced inline on each book move
    // (`book: engine plays X (ECO Name)`).
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
            retro_expanded: false,
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
        // Calm default returns on a fresh game — the feedback zone
        // collapses back to the one-line verdict.
        self.retro_expanded = false;
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

    pub(crate) fn start_new_game(
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
        // Calm default returns on a fresh game — the feedback zone
        // collapses back to the one-line verdict.
        self.retro_expanded = false;
        if self.log_to_stderr {
            log_new_game_intro(&self.opponent);
        }
        self.close_hint();
        let _ = self.worker_tx.send(WorkerJob::NewGame);
        self.maybe_queue_engine_search();
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
}
