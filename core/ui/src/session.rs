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
use chess_tutor_engine::types::{Color, Move, MoveKind, PieceType, Square, Value};

use crate::event::Event;
use crate::view::{
    BoardView, EvalBarView, HintEntryView, HintPanelState, HintPanelView, MoveListCell,
    MoveListRow, MoveListView, NewGameDialogView, PromotionPickerView, RetrospectiveBody,
    RetrospectiveKind, RetrospectivePanelView, SidePanelBody, SidePanelView, TopBarView,
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

pub(crate) struct EngineInfo {
    pub(crate) score_white_pov: Value,
    pub(crate) depth: u32,
    pub(crate) elapsed: Duration,
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
                Some(Color::Black) => ColorChoice::White,
                Some(Color::White) => ColorChoice::Black,
                None => ColorChoice::Both,
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

pub(crate) struct HistoryEntry {
    pub(crate) mv: Move,
    pub(crate) state: StateInfo,
    pub(crate) san: String,
    pub(crate) moved_by: Color,
    pub(crate) position_after: Position,
    /// Filled for moves the user made. `None` while the worker is
    /// still computing; populated when the result arrives.
    pub(crate) retrospective_text: Option<String>,
    /// Filled for moves the engine made. Carries score / depth / time.
    pub(crate) engine_info: Option<EngineInfo>,
}

pub struct Session {
    pub(crate) position: Position,
    pub(crate) position_keys: Vec<u64>,
    pub(crate) history: Vec<HistoryEntry>,
    pub(crate) selected: Option<Square>,
    pub(crate) legal_from_selected: Vec<Move>,
    pub(crate) flipped: bool,

    pub(crate) engine_plays: Option<Color>,
    pub(crate) depth: u32,

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
            position,
            position_keys,
            history: Vec::new(),
            selected: None,
            legal_from_selected: Vec::new(),
            flipped: false,
            engine_plays: None,
            depth: DEFAULT_DEPTH,
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
        }
    }

    fn start_new_game(
        &mut self,
        position: Position,
        engine_plays: Option<Color>,
        depth: u32,
        noise: NoiseProfile,
        eval_mask: EvalMask,
    ) {
        self.gen = self.gen.wrapping_add(1);
        self.engine_thinking = false;
        self.position_keys = vec![position.key()];
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
        log_new_game_intro(&self.opponent);
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
            ColorChoice::White => Some(Color::Black),
            ColorChoice::Black => Some(Color::White),
            ColorChoice::Random => {
                if random_bit() == 0 {
                    Some(Color::Black) // user is white
                } else {
                    Some(Color::White) // user is black
                }
            }
            ColorChoice::Both => None,
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
        match self.engine_plays {
            Some(c) => self.position.side_to_move() != c,
            None => true,
        }
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

    /// Finalise a move chosen via the regular click path *or* the
    /// promotion picker. Snapshots pre-move state for the retrospective
    /// job, applies the move, and clears the hint panel.
    pub(crate) fn apply_user_move(&mut self, mv: Move) {
        let pre_move_pos = self.position.clone();
        let pre_move_history = game_history_for_search(&self.position_keys);

        self.apply_move(mv);
        let target_index = self.history.len() - 1;

        let _ = self.worker_tx.send(WorkerJob::Retrospective {
            pre_move_pos: Box::new(pre_move_pos),
            user_move: mv,
            // ANALYTICAL_DEPTH, not self.depth. The student's
            // feedback wants depth even when the bot is weak.
            depth: ANALYTICAL_DEPTH,
            game_history: pre_move_history,
            gen: self.gen,
            target_index,
        });
        self.close_hint();
    }

    fn apply_move(&mut self, mv: Move) {
        let san_str = san::format(&self.position, mv);
        let moved_by = self.position.side_to_move();
        let state = self.position.do_move(mv);
        self.position_keys.push(self.position.key());
        self.history.push(HistoryEntry {
            mv,
            state,
            san: san_str,
            moved_by,
            position_after: self.position.clone(),
            retrospective_text: None,
            engine_info: None,
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
        self.viewing_index = None;
        self.close_hint();
        self.undo_one();
        // In user-vs-engine mode, takeback returns to the user's
        // prior turn — undo a second ply if we just landed on the
        // engine's turn.
        if let Some(eng_color) = self.engine_plays {
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
        if self.engine_thinking {
            return;
        }
        let Some(eng_color) = self.engine_plays else {
            return;
        };
        if self.position.side_to_move() != eng_color {
            return;
        }
        let mut scratch = self.position.clone();
        if legal_moves_vec(&mut scratch).is_empty() {
            return;
        }
        // Book first: walk allowed openings for any whose stored
        // move-prefix still matches the moves played so far; if any
        // match, play the deterministically-picked next move
        // synchronously and skip the worker round-trip entirely. The
        // engine_info field stays None for these moves (no search ran);
        // the move list panel will need to recognise book moves as a
        // separate case once we surface them in the UI.
        let history_moves: Vec<Move> = self.history.iter().map(|e| e.mv).collect();
        let book_pick = self
            .book_cursor
            .as_ref()
            .and_then(|c| c.peek(&history_moves));
        if let Some(book_pick) = book_pick {
            let san_str = san::format(&self.position, book_pick.mv);
            if let Some(entry) = chess_tutor_engine::openings::entry(book_pick.opening_id) {
                eprintln!("book: engine plays {} ({} {})", san_str, entry.eco, entry.name);
            } else {
                eprintln!("book: engine plays {}", san_str);
            }
            // A successful book pick clears the "we've announced
            // out-of-book" flag — the user may have taken back to an
            // in-book position, and if they later deviate again we
            // want the announcement to print fresh.
            self.book_out_announced = false;
            self.apply_move(book_pick.mv);
            return;
        }
        // No book match on this position. Announce once per
        // out-of-book streak — *don't* drop the cursor itself,
        // because a takeback might bring us back into book territory
        // and we need peek to keep working on the next bot turn.
        if self.book_cursor.is_some() && !self.book_out_announced {
            eprintln!("out of book — engine now plays from search.");
            self.book_out_announced = true;
        }
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
            WorkerResult::Search { gen, mv, line, noise_pick, elapsed } => {
                if gen != self.gen {
                    return;
                }
                self.engine_thinking = false;
                let Some(mv) = mv else {
                    return;
                };
                if let Some(info) = &noise_pick {
                    // Log noise-driven picks to stderr so the user can
                    // see when the bot is deliberately off the best
                    // line (otherwise weakened play looks like a bug).
                    // GUI surface for this lives in deferred Phase D
                    // follow-on work.
                    match info {
                        NoisePickInfo::Softmax { pick_idx, num_lines, delta_from_top_cp } => {
                            eprintln!(
                                "noise: softmax picked #{} of {} ({:+} cp from #1)",
                                pick_idx + 1, num_lines, delta_from_top_cp,
                            );
                        }
                        NoisePickInfo::Blunder { pick_idx, num_lines, delta_from_top_cp } => {
                            eprintln!(
                                "noise: blunder picked #{} of {} ({:+} cp from #1)",
                                pick_idx + 1, num_lines, delta_from_top_cp,
                            );
                        }
                        NoisePickInfo::BlunderSkipped { closest_above_loss_cp } => {
                            let cap = (self.opponent.noise.blunder_max_loss_cp as f32
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
                        NoisePickInfo::Wild { engine_top, engine_top_score } => {
                            eprintln!(
                                "noise: wild — bot played {} (engine preferred {} at {} cp)",
                                san::format(&self.position, mv),
                                san::format(&self.position, *engine_top),
                                engine_top_score.0,
                            );
                        }
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
                        });
                    }
                }
                // Engine just moved — any open Hint was for the prior
                // position, so close it.
                self.close_hint();
            }
            WorkerResult::Retrospective {
                gen,
                target_index,
                text,
            } => {
                if gen != self.gen {
                    return;
                }
                if let Some(entry) = self.history.get_mut(target_index) {
                    entry.retrospective_text = Some(text);
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
        }
    }

    pub(crate) fn game_outcome(&self) -> Option<&'static str> {
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

    /// The EngineInfo to display on the eval bar for the position the
    /// user is currently viewing.
    ///
    /// `engine_info` is only populated for moves the engine played, so
    /// when the user is browsing back to a user-move position we scan
    /// backward for the most recent engine evaluation that was
    /// available at that point in the game. That's an approximation —
    /// the true post-user-move eval would require a fresh search per
    /// click — but it's close enough to let the user see the trend
    /// (eval bar drops at the move where it actually dropped, not
    /// always shows the live eval).
    ///
    /// When viewing live (`viewing_index = None`), behaves identically
    /// to the previous `latest_engine_info` — most recent across the
    /// full history.
    pub(crate) fn viewed_engine_info(&self) -> Option<&EngineInfo> {
        let upper = self.viewing_index.map_or(self.history.len(), |i| i + 1);
        self.history[..upper]
            .iter()
            .rev()
            .find_map(|e| e.engine_info.as_ref())
    }

    /// Picks which entry to show in the retrospective panel:
    ///   - Viewing back: the viewed entry.
    ///   - Live: the most recent user-move entry (so the engine's
    ///     reply doesn't bury the analysis of the user's own move).
    pub(crate) fn panel_entry(&self) -> Option<&HistoryEntry> {
        if let Some(i) = self.viewing_index {
            return self.history.get(i);
        }
        self.history
            .iter()
            .rev()
            .find(|e| self.is_user_move(e))
            .or_else(|| self.history.last())
    }

    pub(crate) fn is_user_move(&self, entry: &HistoryEntry) -> bool {
        match self.engine_plays {
            Some(c) => entry.moved_by != c,
            None => true,
        }
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
            }
            Event::Cancel => self.handle_cancel(),
            Event::ConfirmNewGame => self.try_start_from_form(),
            Event::ResetBotForm => {
                if let Some(f) = self.new_game_form.as_mut() {
                    f.noise = NoiseProfile::default();
                    f.eval_mask = EvalMask::EMPTY;
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
        TopBarView {
            can_takeback: !self.history.is_empty(),
            hint_open: self.hint_open,
            hint_button_enabled: hint_can_open || self.hint_open,
            viewing_live: self.is_viewing_live(),
            depth: self.depth,
            engine_thinking: self.engine_thinking,
            game_outcome: self.game_outcome(),
        }
    }

    pub fn build_eval_bar_view(&self) -> EvalBarView {
        let score = self.viewed_engine_info().map(|i| i.score_white_pov);
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
        BoardView::compose(
            &viewed_pos,
            self.flipped,
            viewed_mv,
            selected,
            legals,
            pending_promotion,
        )
    }

    pub fn build_side_panel_view(&self) -> SidePanelView {
        let body = if self.hint_open {
            SidePanelBody::Hint(self.build_hint_panel_view())
        } else {
            SidePanelBody::Retrospective(self.build_retrospective_view())
        };
        SidePanelView {
            moves: self.build_move_list_view(),
            body,
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
        let Some(entry) = self.panel_entry() else {
            return RetrospectivePanelView {
                game_outcome,
                body: RetrospectiveBody::NoMoves,
            };
        };
        let viewing_back_san = (!self.is_viewing_live()).then(|| entry.san.clone());
        let kind = if self.is_user_move(entry) {
            match &entry.retrospective_text {
                Some(text) if !text.is_empty() => RetrospectiveKind::UserMoveText(text.clone()),
                Some(_) => RetrospectiveKind::UserMoveEmpty,
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
        }
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

/// Saturation point for the eval bar's score→ratio mapping. Used by
/// [`Session::build_eval_bar_view`]; lives at module scope so the only
/// constant referenced by view-building stays adjacent to the
/// session.
const EVAL_BAR_SATURATION_CP: f32 = 1000.0;

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
