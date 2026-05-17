use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use chess_tutor_engine::analysis::{analyze_position, MoveAnalysis};
use chess_tutor_engine::book::BookCursor;
use chess_tutor_engine::engine::{Engine, SearchLine, SearchParams};
use chess_tutor_engine::movegen::legal_moves_vec;
use chess_tutor_engine::noise::{self, NoisePick};
use chess_tutor_engine::opponent::{EvalCategory, EvalMask, NoiseProfile, OpponentProfile};
use chess_tutor_engine::position::{Position, StateInfo};
use chess_tutor_engine::san;
use chess_tutor_engine::types::{Color, File, Move, MoveKind, Piece, PieceType, Rank, Square, Value};
use chess_tutor_narration::{format_retrospective, NarrationOptions};
use eframe::egui;

const ENGINE_TURN_NODE_CAP: u64 = 5_000_000;
const RETROSPECTIVE_MULTI_PV: usize = 3;
const HINT_MULTI_PV: usize = 3;
const DEFAULT_DEPTH: u32 = 10;
const EVAL_BAR_SATURATION_CP: f32 = 1000.0;
/// Safety caps for analytical searches that auto-fire (retrospective,
/// hint panel). Without these, pathological positions — notably
/// MultiPV around a found mate — can pin the worker thread for
/// minutes, locking the GUI mid-game. The wall-clock cap is the
/// user-visible guarantee ("retrospective takes max N seconds"); the
/// node cap is a backstop in case the time check is starved by
/// scheduling.
const ANALYSIS_NODE_CAP: u64 = 100_000_000;
const ANALYSIS_TIME_MS: u64 = 10_000;

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 800.0])
            .with_min_inner_size([900.0, 700.0])
            .with_title("Chess Tutor"),
        ..Default::default()
    };
    eframe::run_native(
        "Chess Tutor",
        native_options,
        Box::new(|cc| Ok(Box::new(App::new(cc.egui_ctx.clone())))),
    )
}

// =========================================================================
// Worker thread
// =========================================================================

enum WorkerJob {
    NewGame,
    Search {
        pos: Box<Position>,
        params: SearchParams,
        gen: u64,
        /// Bot noise profile to apply *after* the search returns. The
        /// engine search itself doesn't read this — `params.multi_pv`
        /// is set wide enough by the caller to surface candidates.
        noise: NoiseProfile,
        seed: u64,
        ply: u64,
    },
    Retrospective {
        pre_move_pos: Box<Position>,
        user_move: Move,
        depth: u32,
        game_history: Vec<u64>,
        gen: u64,
        target_index: usize,
    },
    Analyze {
        pos: Box<Position>,
        depth: u32,
        multi_pv: usize,
        game_history: Vec<u64>,
        for_key: u64,
    },
}

enum WorkerResult {
    Search {
        gen: u64,
        /// The move the bot will play, or `None` for terminal
        /// positions (no legal replies).
        mv: Option<Move>,
        /// Search-line context for the chosen move. Present for normal
        /// / softmax / blunder picks; `None` for wild picks because the
        /// engine didn't search the wild move specifically. The GUI's
        /// per-move score/depth display reads this — wild moves end up
        /// without an engine_info badge.
        line: Option<SearchLine>,
        /// Diagnostic info for the move list / debug log when noise
        /// drove the bot off `lines[0]`. `None` on the off-profile /
        /// engine-best hot path.
        noise_pick: Option<NoisePickInfo>,
        elapsed: Duration,
    },
    Retrospective {
        gen: u64,
        target_index: usize,
        text: String,
    },
    Analyze {
        for_key: u64,
        analyses: Vec<MoveAnalysis>,
    },
}

#[derive(Clone, Debug)]
enum NoisePickInfo {
    /// Softmax branch fired — sampled `pick_idx` from the top-K.
    Softmax {
        pick_idx: usize,
        num_lines: usize,
        delta_from_top_cp: i32,
    },
    /// Blunder branch fired — picked a deliberately worse line.
    /// `pick_idx` is always `>= 1`; either a qualifying line or the
    /// worst-available fallback when no line cleared the severity gap.
    Blunder {
        pick_idx: usize,
        num_lines: usize,
        delta_from_top_cp: i32,
    },
    /// Wild branch fired — bot played `mv`; the engine's preferred
    /// move was `engine_top`. The two may coincidentally match.
    Wild {
        engine_top: Move,
        engine_top_score: Value,
    },
}

fn worker_loop(rx: Receiver<WorkerJob>, tx: Sender<WorkerResult>, ctx: egui::Context) {
    // Two engines live in the worker:
    //
    // - `engine` — the play engine. Searches for the bot's move and
    //   accumulates TT / history learning across moves the way SF
    //   does. Persisting state across moves is what makes the bot
    //   stronger over the course of a game.
    //
    // - `analysis_engine` — dedicated to retrospective / hint /
    //   analyze. Its state is cleared via `new_game()` before every
    //   job so the analytical answer for a given position is
    //   bit-identical regardless of session history. **This is
    //   load-bearing for the teaching contract**: same position, same
    //   verdict — across takebacks, across days, across reinstalls.
    //   The prior pattern was `engine.clone()` for each analytical
    //   call, which captured whatever state the play engine had
    //   accumulated and silently produced different verdicts for the
    //   same move depending on what the user had done previously.
    let mut engine = Engine::default();
    let mut analysis_engine = Engine::default();
    while let Ok(job) = rx.recv() {
        match job {
            WorkerJob::NewGame => {
                engine.new_game();
                analysis_engine.new_game();
            }
            WorkerJob::Search { mut pos, params, gen, noise, seed, ply } => {
                // Wild branch needs the legal-move list — generated
                // here so the worker stays self-contained.
                let legal = legal_moves_vec(&mut pos);
                let started = Instant::now();
                let lines = engine.search(&mut pos, params);
                let elapsed = started.elapsed();
                let pick = noise::pick(&noise, seed, ply, &lines, &legal);
                let (mv, line, noise_pick) = match pick {
                    NoisePick::Line(idx) => {
                        let line = lines.get(idx).cloned();
                        let mv = line.as_ref().and_then(|l| l.pv.first().copied());
                        let info = if idx == 0 || lines.is_empty() {
                            None
                        } else {
                            Some(NoisePickInfo::Softmax {
                                pick_idx: idx,
                                num_lines: lines.len(),
                                delta_from_top_cp: lines[idx].score.0 - lines[0].score.0,
                            })
                        };
                        (mv, line, info)
                    }
                    NoisePick::Blunder(idx) => {
                        let line = lines.get(idx).cloned();
                        let mv = line.as_ref().and_then(|l| l.pv.first().copied());
                        let info = lines.get(idx).map(|l| NoisePickInfo::Blunder {
                            pick_idx: idx,
                            num_lines: lines.len(),
                            delta_from_top_cp: l.score.0 - lines[0].score.0,
                        });
                        (mv, line, info)
                    }
                    NoisePick::Wild(wild_mv) => {
                        let info = lines.first().and_then(|top| {
                            top.pv.first().map(|&top_mv| NoisePickInfo::Wild {
                                engine_top: top_mv,
                                engine_top_score: top.score,
                            })
                        });
                        (Some(wild_mv), None, info)
                    }
                };
                let _ = tx.send(WorkerResult::Search { gen, mv, line, noise_pick, elapsed });
                ctx.request_repaint();
            }
            WorkerJob::Retrospective {
                mut pre_move_pos,
                user_move,
                depth,
                game_history,
                gen,
                target_index,
            } => {
                // Clear the analysis engine's TT / history before every
                // retrospective so the result depends only on the
                // position + params, not on session history. (See the
                // worker_loop preamble for the full reasoning — this
                // closes the takeback verdict-flip bug.)
                analysis_engine.new_game();
                let params = SearchParams {
                    max_depth: depth,
                    max_nodes: Some(ANALYSIS_NODE_CAP),
                    max_time: Some(Duration::from_millis(ANALYSIS_TIME_MS)),
                    multi_pv: RETROSPECTIVE_MULTI_PV,
                    game_history,
                    force_include: vec![user_move],
                    verbose_progress: false,
                    // Retrospective is single-threaded for full
                    // determinism. Lazy SMP introduces enough per-run
                    // score variance to flip the same move between
                    // verdicts (e.g. e4 reading as "Best" one run and
                    // "Good" the next, then "Best" again after a
                    // takeback) — a major teaching-tool disconnect
                    // for a student trying to learn what "best" means.
                    // Single-thread gives bit-identical retrospectives
                    // across runs and across takebacks. Cost at the
                    // desktop's default depth=10 is ~60ms vs
                    // multi-thread — well within "feels instant".
                    threads: 1,
                    // Retrospective is analytical — always unbiased
                    // eval, regardless of any mid-game bot mask.
                    eval_mask: chess_tutor_engine::opponent::EvalMask::EMPTY,
                };
                let analyses = analyze_position(&mut analysis_engine, &mut pre_move_pos, params);
                let text = format_retrospective(
                    &pre_move_pos,
                    &analyses,
                    user_move,
                    &NarrationOptions::default(),
                );
                let _ = tx.send(WorkerResult::Retrospective {
                    gen,
                    target_index,
                    text,
                });
                ctx.request_repaint();
            }
            WorkerJob::Analyze {
                mut pos,
                depth,
                multi_pv,
                game_history,
                for_key,
            } => {
                // Same reset-before-use pattern as Retrospective —
                // hint / analyze answer should be deterministic for
                // the position the user is asking about.
                analysis_engine.new_game();
                let params = SearchParams {
                    max_depth: depth,
                    max_nodes: Some(ANALYSIS_NODE_CAP),
                    max_time: Some(Duration::from_millis(ANALYSIS_TIME_MS)),
                    multi_pv,
                    game_history,
                    force_include: Vec::new(),
                    verbose_progress: false,
                    // Hint / analyze: single-threaded for the same
                    // determinism reason as the retrospective. The user
                    // is exploring "what would the engine think about
                    // X" — same question twice should give the same
                    // answer.
                    threads: 1,
                    // Hint panel is analytical — unbiased eval.
                    eval_mask: chess_tutor_engine::opponent::EvalMask::EMPTY,
                };
                let analyses = analyze_position(&mut analysis_engine, &mut pos, params);
                let _ = tx.send(WorkerResult::Analyze { for_key, analyses });
                ctx.request_repaint();
            }
        }
    }
}

// =========================================================================
// App
// =========================================================================

struct EngineInfo {
    score_white_pov: Value,
    depth: u32,
    elapsed: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ColorChoice {
    White,
    Black,
    Random,
    Both,
}

struct NewGameForm {
    color: ColorChoice,
    fen: String,
    depth: u32,
    /// Bot move-sampling knobs. Persists across New Game clicks so the
    /// user can tune incrementally between games without losing prior
    /// settings.
    noise: NoiseProfile,
    /// Eval categories the bot is blind to. Same persistence rule.
    eval_mask: EvalMask,
    error: Option<String>,
}

impl NewGameForm {
    /// Pre-populate from the live game so the dialog reflects what
    /// the user is currently playing against — encourages incremental
    /// tweaking rather than rebuilding settings from scratch every
    /// time they click New Game.
    fn from_current(app: &App) -> Self {
        Self {
            color: match app.engine_plays {
                Some(Color::Black) => ColorChoice::White,
                Some(Color::White) => ColorChoice::Black,
                None => ColorChoice::Both,
            },
            fen: String::new(),
            depth: app.depth,
            noise: app.opponent.noise.clone(),
            eval_mask: app.opponent.eval_mask,
            error: None,
        }
    }

    /// Defaults for the first-launch dialog — same shape as
    /// [`Self::from_current`] would produce for a freshly constructed
    /// [`App`], but without needing one to exist yet.
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

struct HistoryEntry {
    mv: Move,
    state: StateInfo,
    san: String,
    moved_by: Color,
    position_after: Position,
    /// Filled for moves the user made. `None` while the worker is
    /// still computing; populated when the result arrives.
    retrospective_text: Option<String>,
    /// Filled for moves the engine made. Carries score / depth / time.
    engine_info: Option<EngineInfo>,
    /// Snapshot of the opening-book cursor as it was *before* this
    /// move advanced (or dropped) it. On takeback we restore this so
    /// the cursor walks backward with the game — including
    /// resurrecting a cursor the move dropped.
    book_cursor_before: Option<BookCursor>,
}

struct App {
    position: Position,
    position_keys: Vec<u64>,
    history: Vec<HistoryEntry>,
    selected: Option<Square>,
    legal_from_selected: Vec<Move>,
    flipped: bool,

    engine_plays: Option<Color>,
    depth: u32,

    worker_tx: Sender<WorkerJob>,
    worker_rx: Receiver<WorkerResult>,
    /// Bumped on cancel events (NewGame, Takeback). Worker results
    /// with a stale `gen` are dropped on arrival.
    gen: u64,
    engine_thinking: bool,

    /// `None` = following live play; `Some(i)` = viewing the position
    /// after `history[i]`.
    viewing_index: Option<usize>,

    /// `Some` while the New Game dialog is open. The form holds the
    /// in-flight color / FEN / depth choices; `try_start_from_form`
    /// validates and either applies (closing the dialog) or sets
    /// `form.error` and keeps it open.
    new_game_form: Option<NewGameForm>,

    /// `true` while the Hint panel is showing (replacing the
    /// retrospective panel). Toggled by the Hint button; auto-closed
    /// on next move, takeback, and new game.
    hint_open: bool,
    /// `true` while a Hint Analyze job is in flight. Distinct from
    /// `hint_open` because the panel may be open showing stale results
    /// while we wait for fresh ones.
    hint_thinking: bool,
    /// Latest analyze result. Tagged with the position key it was
    /// computed for so stale arrivals can be discarded.
    hint_result: Option<HintResult>,

    /// Bot personality / variability for the current game. Reseeded
    /// on every New Game; the play loop reads `book` to pick an
    /// opening line and consults [`Self::book_cursor`] to follow it.
    opponent: OpponentProfile,
    /// Live opening-book cursor for the current game. `Some` while
    /// the bot is still following the picked line; dropped on the
    /// first deviation, exhaustion, or any takeback that uncovers a
    /// pre-book state.
    book_cursor: Option<BookCursor>,
    /// True until the user clicks Start in the New Game dialog for
    /// the first time. While true the dialog hides its Cancel button
    /// — there's no game in progress to cancel back to, so the only
    /// path forward is to commit a configuration.
    first_launch: bool,

    /// `Some` while the user has clicked a pawn onto the promotion
    /// rank and we're waiting for them to choose which piece to
    /// promote to. Carries the four candidate promotion moves (Q / R /
    /// B / N variants of the same from→to). Cleared on pick, off-board
    /// click, or any state-changing action (new game, takeback).
    pending_promotion: Option<PendingPromotion>,
}

struct PendingPromotion {
    /// Promotion-rank square — target of every candidate, and the
    /// anchor for the picker stack.
    to: Square,
    /// The four legal promotion moves with shared `from` / `to`. Order
    /// is Q, R, B, N to match the on-screen stack.
    candidates: [Move; 4],
}

struct HintResult {
    /// Position the analyses are *for* — needed to format SAN of
    /// candidate moves and PV plies on render. Identification of
    /// which position this corresponds to happens at arrival time
    /// (via `for_key` matching `self.position.key()`); once stored
    /// the position itself carries everything the panel needs.
    pos: Position,
    analyses: Vec<MoveAnalysis>,
}

impl App {
    fn new(ctx: egui::Context) -> Self {
        let (job_tx, job_rx) = mpsc::channel::<WorkerJob>();
        let (result_tx, result_rx) = mpsc::channel::<WorkerResult>();
        thread::spawn(move || worker_loop(job_rx, result_tx, ctx));

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
        self.book_cursor = BookCursor::pick(&self.opponent, &self.position);
        log_new_game_intro(&self.opponent, &self.book_cursor);
        self.close_hint();
        let _ = self.worker_tx.send(WorkerJob::NewGame);
        self.maybe_queue_engine_search();
    }

    fn close_hint(&mut self) {
        self.hint_open = false;
        self.hint_thinking = false;
        self.hint_result = None;
    }

    fn toggle_hint(&mut self) {
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
            depth: self.depth,
            multi_pv: HINT_MULTI_PV,
            game_history: game_history_for_search(&self.position_keys),
            for_key: self.position.key(),
        });
    }

    fn open_new_game_dialog(&mut self) {
        // Idempotent: don't trample unsaved tweaks if the user double-
        // clicks the button or hits it while the dialog is already up.
        if self.new_game_form.is_some() {
            return;
        }
        self.new_game_form = Some(NewGameForm::from_current(self));
    }

    fn try_start_from_form(&mut self) {
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

    fn handle_click(&mut self, sq: Square) {
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

    fn is_users_turn(&self) -> bool {
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
    fn apply_user_move(&mut self, mv: Move) {
        let pre_move_pos = self.position.clone();
        let pre_move_history = game_history_for_search(&self.position_keys);

        self.apply_move(mv);
        let target_index = self.history.len() - 1;

        let _ = self.worker_tx.send(WorkerJob::Retrospective {
            pre_move_pos: Box::new(pre_move_pos),
            user_move: mv,
            depth: self.depth,
            game_history: pre_move_history,
            gen: self.gen,
            target_index,
        });
        self.close_hint();
    }

    fn apply_move(&mut self, mv: Move) {
        let san_str = san::format(&self.position, mv);
        let moved_by = self.position.side_to_move();
        let book_cursor_before = self.book_cursor.clone();
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
            book_cursor_before,
        });
        // Advance / drop the book cursor in the same step that records
        // the move, so the cursor stays consistent with history.
        let dropped = self.book_cursor.as_mut().is_some_and(|c| !c.observe(mv));
        if dropped {
            self.book_cursor = None;
            eprintln!("out of book — engine now plays from search.");
        }
        self.deselect();
    }

    fn takeback(&mut self) {
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
        self.maybe_queue_engine_search();
    }

    fn undo_one(&mut self) {
        if let Some(entry) = self.history.pop() {
            self.position.undo_move(entry.mv, entry.state);
            self.position_keys.pop();
            self.book_cursor = entry.book_cursor_before;
            self.deselect();
        }
    }

    fn deselect(&mut self) {
        self.selected = None;
        self.legal_from_selected.clear();
        self.pending_promotion = None;
    }

    fn maybe_queue_engine_search(&mut self) {
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
        // Book first: if the cursor has a queued move, play it
        // synchronously and skip the worker round-trip entirely. The
        // engine_info field stays None for these moves (no search ran);
        // the move list panel will need to recognise book moves as a
        // separate case once we surface them in the UI.
        if let Some(book_mv) = self.book_cursor.as_ref().and_then(|c| c.peek()) {
            eprintln!("book: engine plays {}", san::format(&self.position, book_mv));
            self.apply_move(book_mv);
            return;
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

    fn poll_worker(&mut self) {
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

    fn game_outcome(&self) -> Option<&'static str> {
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

    fn viewed_entry(&self) -> Option<&HistoryEntry> {
        match self.viewing_index {
            Some(i) => self.history.get(i),
            None => self.history.last(),
        }
    }

    fn viewed_position(&self) -> &Position {
        match self.viewing_index {
            Some(i) => self
                .history
                .get(i)
                .map(|e| &e.position_after)
                .unwrap_or(&self.position),
            None => &self.position,
        }
    }

    fn is_viewing_live(&self) -> bool {
        self.viewing_index.is_none()
    }

    /// The most recent EngineInfo on the live timeline, used to
    /// drive the eval bar regardless of where the user is browsing.
    fn latest_engine_info(&self) -> Option<&EngineInfo> {
        self.history
            .iter()
            .rev()
            .find_map(|e| e.engine_info.as_ref())
    }

    /// Picks which entry to show in the retrospective panel:
    ///   - Viewing back: the viewed entry.
    ///   - Live: the most recent user-move entry (so the engine's
    ///     reply doesn't bury the analysis of the user's own move).
    fn panel_entry(&self) -> Option<&HistoryEntry> {
        if let Some(i) = self.viewing_index {
            return self.history.get(i);
        }
        self.history
            .iter()
            .rev()
            .find(|e| self.is_user_move(e))
            .or_else(|| self.history.last())
    }

    fn is_user_move(&self, entry: &HistoryEntry) -> bool {
        match self.engine_plays {
            Some(c) => entry.moved_by != c,
            None => true,
        }
    }
}

fn game_history_for_search(position_keys: &[u64]) -> Vec<u64> {
    if position_keys.is_empty() {
        Vec::new()
    } else {
        position_keys[..position_keys.len() - 1].to_vec()
    }
}

// =========================================================================
// UI: top-level
// =========================================================================

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_worker();

        egui::TopBottomPanel::top("topbar").show(ctx, |ui| {
            self.draw_top_bar(ui);
        });
        egui::SidePanel::left("evalbar")
            .resizable(false)
            .exact_width(56.0)
            .show(ctx, |ui| {
                self.draw_eval_bar(ui);
            });
        egui::SidePanel::right("sidebar")
            .resizable(false)
            .default_width(320.0)
            .show(ctx, |ui| {
                self.draw_side_panel(ui);
            });
        egui::CentralPanel::default().show(ctx, |ui| {
            self.draw_board(ui);
        });

        if self.new_game_form.is_some() {
            self.draw_new_game_dialog(ctx);
        }
    }
}

// =========================================================================
// UI: panels
// =========================================================================

impl App {
    fn draw_top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("New Game").clicked() {
                self.open_new_game_dialog();
            }
            let can_takeback = !self.history.is_empty();
            if ui
                .add_enabled(can_takeback, egui::Button::new("Takeback"))
                .clicked()
            {
                self.takeback();
            }
            if ui.button("Flip Board").clicked() {
                self.flipped = !self.flipped;
            }
            // Hint is only meaningful while at the live position and
            // it's the user's turn to choose a move. Block the button
            // outside those conditions.
            let hint_enabled = self.is_viewing_live()
                && !self.engine_thinking
                && self.is_users_turn()
                && self.game_outcome().is_none();
            let hint_label = if self.hint_open { "Hide Hint" } else { "Hint" };
            if ui
                .add_enabled(hint_enabled || self.hint_open, egui::Button::new(hint_label))
                .clicked()
            {
                self.toggle_hint();
            }
            if !self.is_viewing_live() && ui.button("▶ Live").clicked() {
                self.viewing_index = None;
            }
            ui.separator();
            ui.label("Depth:");
            ui.add(egui::DragValue::new(&mut self.depth).range(1..=20));
            ui.separator();
            if self.engine_thinking {
                ui.spinner();
                ui.label("engine thinking…");
            } else if let Some(end) = self.game_outcome() {
                ui.colored_label(egui::Color32::from_rgb(0xb8, 0x55, 0x00), end);
            }
        });
    }

    fn draw_eval_bar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width() - 8.0, ui.available_height() - 32.0),
            egui::Sense::hover(),
        );
        let painter = ui.painter_at(rect);

        let white_color = egui::Color32::from_rgb(0xf0, 0xf0, 0xf0);
        let black_color = egui::Color32::from_rgb(0x30, 0x30, 0x30);
        let border = egui::Color32::from_rgb(0x80, 0x80, 0x80);

        let score = self.latest_engine_info().map(|i| i.score_white_pov);
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

        let split_y = rect.max.y - rect.height() * white_ratio;
        let top_rect = egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, split_y));
        let bot_rect = egui::Rect::from_min_max(egui::pos2(rect.min.x, split_y), rect.max);
        painter.rect_filled(top_rect, 0.0, black_color);
        painter.rect_filled(bot_rect, 0.0, white_color);
        painter.rect_stroke(rect, 0.0, egui::Stroke::new(1.0, border));

        ui.add_space(4.0);
        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
            ui.monospace(label);
        });
    }

    fn draw_side_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Moves");
        ui.separator();
        let avail_h = ui.available_height();
        let move_h = (avail_h * 0.40).max(120.0);

        egui::ScrollArea::vertical()
            .id_salt("moves_scroll")
            .stick_to_bottom(self.is_viewing_live())
            .max_height(move_h)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.draw_move_list(ui);
            });

        ui.separator();
        if self.hint_open {
            ui.heading("Hint");
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt("hint_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    self.draw_hint_panel(ui);
                });
        } else {
            ui.heading("Retrospective");
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt("retro_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    self.draw_retrospective(ui);
                });
        }
    }

    fn draw_move_list(&mut self, ui: &mut egui::Ui) {
        let viewing = self.viewing_index;
        let history_len = self.history.len();
        let mut clicked: Option<Option<usize>> = None;

        egui::Grid::new("moves_grid")
            .num_columns(3)
            .spacing([12.0, 4.0])
            .min_col_width(30.0)
            .show(ui, |ui| {
                for move_pair_idx in 0..history_len.div_ceil(2) {
                    let i_white = move_pair_idx * 2;
                    let i_black = i_white + 1;
                    ui.monospace(format!("{}.", move_pair_idx + 1));
                    let entry_w = &self.history[i_white];
                    let selected_w = viewing == Some(i_white);
                    if ui
                        .add(egui::SelectableLabel::new(
                            selected_w,
                            egui::RichText::new(&entry_w.san).monospace(),
                        ))
                        .clicked()
                    {
                        clicked = Some(Some(i_white));
                    }
                    if i_black < history_len {
                        let entry_b = &self.history[i_black];
                        let selected_b = viewing == Some(i_black);
                        if ui
                            .add(egui::SelectableLabel::new(
                                selected_b,
                                egui::RichText::new(&entry_b.san).monospace(),
                            ))
                            .clicked()
                        {
                            clicked = Some(Some(i_black));
                        }
                    } else {
                        ui.label("");
                    }
                    ui.end_row();
                }
            });

        if let Some(target) = clicked {
            // If they clicked the move that's already at the end of
            // the live timeline, treat as "back to live".
            self.viewing_index = match target {
                Some(i) if i + 1 == self.history.len() => None,
                other => other,
            };
        }
    }

    fn draw_retrospective(&self, ui: &mut egui::Ui) {
        if let Some(end) = self.game_outcome() {
            ui.colored_label(egui::Color32::from_rgb(0xb8, 0x55, 0x00), end);
            ui.separator();
        }

        let Some(entry) = self.panel_entry() else {
            ui.label("(no moves yet)");
            return;
        };

        if !self.is_viewing_live() {
            ui.weak(format!("viewing move: {}", entry.san));
            ui.separator();
        }

        let is_user = self.is_user_move(entry);
        if is_user {
            match &entry.retrospective_text {
                Some(text) if !text.is_empty() => {
                    ui.monospace(text);
                }
                Some(_) => {
                    ui.label("(no analysis text)");
                }
                None => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("analyzing your move…");
                    });
                }
            }
        } else if let Some(info) = &entry.engine_info {
            ui.monospace(format!("Engine played {}", entry.san));
            ui.monospace(format!(
                "eval {:+.2}    depth {}    {} ms",
                info.score_white_pov.0 as f32 / Value::PAWN_MG.0 as f32,
                info.depth,
                info.elapsed.as_millis(),
            ));
        } else {
            ui.label("(engine info missing)");
        }
    }

    fn draw_hint_panel(&mut self, ui: &mut egui::Ui) {
        if self.hint_thinking && self.hint_result.is_none() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("analyzing position…");
            });
            return;
        }
        let Some(result) = &self.hint_result else {
            ui.label("(no analysis yet)");
            return;
        };
        if result.analyses.is_empty() {
            ui.label("(no legal moves)");
            return;
        }

        let root_stm = result.pos.side_to_move();
        for (i, ma) in result.analyses.iter().enumerate() {
            ui.add_space(if i == 0 { 0.0 } else { 8.0 });
            let san = san::format(&result.pos, ma.mv);
            let score_str = format_score_root_pov(ma.score, root_stm);
            ui.monospace(format!(
                "{}. {}    {}    depth {}",
                i + 1,
                san,
                score_str,
                ma.depth,
            ));
            let pv_san = pv_to_san(&result.pos, &ma.pv);
            if !pv_san.is_empty() {
                let mut line = pv_san.join(" ");
                if let Some(settled) = ma.settled_ply {
                    if settled < pv_san.len() {
                        line.push_str(&format!("  [settles ply {}]", settled));
                    }
                }
                ui.indent(format!("pv_{i}"), |ui| {
                    ui.weak(egui::RichText::new(line).monospace());
                });
            }
        }
    }

    fn draw_new_game_dialog(&mut self, ctx: &egui::Context) {
        let Some(form) = self.new_game_form.as_mut() else {
            return;
        };
        let first_launch = self.first_launch;
        let mut start = false;
        let mut cancel = false;
        let mut reset_bot = false;

        let title = if first_launch { "Welcome — Set Up Your Game" } else { "New Game" };
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .default_width(420.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().max_height(560.0).show(ui, |ui| {
                    ui.add_space(4.0);
                    ui.label("You play as:");
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut form.color, ColorChoice::White, "White");
                        ui.radio_value(&mut form.color, ColorChoice::Black, "Black");
                        ui.radio_value(&mut form.color, ColorChoice::Random, "Random");
                        ui.radio_value(&mut form.color, ColorChoice::Both, "Both");
                    });
                    ui.add_space(8.0);

                    ui.label("Starting position (FEN, leave empty for startpos):");
                    ui.add(
                        egui::TextEdit::singleline(&mut form.fen)
                            .desired_width(f32::INFINITY)
                            .hint_text("rnbqkbnr/pppppppp/... (optional)"),
                    );
                    if let Some(err) = &form.error {
                        ui.colored_label(egui::Color32::from_rgb(0xc0, 0x40, 0x40), err);
                    }
                    ui.add_space(8.0);

                    ui.horizontal(|ui| {
                        ui.label("Engine depth:");
                        ui.add(egui::Slider::new(&mut form.depth, 1..=20));
                    });

                    ui.add_space(12.0);
                    ui.separator();
                    ui.heading("Bot Difficulty");
                    ui.label(
                        egui::RichText::new(
                            "Tune how the bot plays. Defaults give full-strength play; \
                             raise the mistake knobs for a weaker, more punishable opponent.",
                        )
                        .small()
                        .weak(),
                    );
                    ui.add_space(6.0);

                    draw_noise_controls(ui, &mut form.noise);

                    ui.add_space(8.0);
                    ui.collapsing("Eval mask (advanced) — categories the bot is blind to", |ui| {
                        ui.label(
                            egui::RichText::new(
                                "Toggle off a concept to simulate an opponent who doesn't \
                                 understand it (e.g. mask king-safety to spar against a sub-\
                                 1200 positional player).",
                            )
                            .small()
                            .weak(),
                        );
                        ui.add_space(4.0);
                        draw_eval_mask_controls(ui, &mut form.eval_mask);
                    });

                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Reset bot to defaults").clicked() {
                            reset_bot = true;
                        }
                    });

                    ui.add_space(12.0);
                });

                ui.separator();
                ui.horizontal(|ui| {
                    // Hide Cancel at first launch: there's no game to
                    // cancel back to, the only path forward is Start.
                    if !first_launch && ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    let start_label = if first_launch { "Start Game" } else { "Start" };
                    if ui.button(start_label).clicked() {
                        start = true;
                    }
                });
            });

        if reset_bot {
            if let Some(f) = self.new_game_form.as_mut() {
                f.noise = NoiseProfile::default();
                f.eval_mask = EvalMask::EMPTY;
            }
        }
        if cancel {
            self.new_game_form = None;
        } else if start {
            self.try_start_from_form();
        }
    }

    fn draw_board(&mut self, ui: &mut egui::Ui) {
        let avail = ui.available_size();
        let board_size = avail.x.min(avail.y);
        let cell = board_size / 8.0;
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(board_size, board_size), egui::Sense::click());

        // Escape cancels a pending promotion. Treat like an off-picker
        // click — drop both the promotion state and the selection so
        // the user starts the move from scratch.
        if self.pending_promotion.is_some()
            && ui.input(|i| i.key_pressed(egui::Key::Escape))
        {
            self.deselect();
        }

        let clicked_square = response
            .clicked()
            .then(|| {
                response
                    .interact_pointer_pos()
                    .and_then(|p| pixel_to_square(p - rect.min, cell, self.flipped))
            })
            .flatten();

        let painter = ui.painter_at(rect);

        let light = egui::Color32::from_rgb(0xf0, 0xd9, 0xb5);
        let dark = egui::Color32::from_rgb(0xb5, 0x88, 0x63);
        let last_move_tint = egui::Color32::from_rgba_unmultiplied(0xff, 0xeb, 0x3b, 0x66);
        let selected_tint = egui::Color32::from_rgba_unmultiplied(0xff, 0xb3, 0x00, 0xaa);
        let check_tint = egui::Color32::from_rgba_unmultiplied(0xff, 0x40, 0x40, 0xaa);
        let dot_color = egui::Color32::from_rgba_unmultiplied(0x10, 0x10, 0x10, 0x66);

        let viewed_pos = self.viewed_position().clone();
        let viewed_mv = self.viewed_entry().map(|e| e.mv);
        let king_in_check = viewed_pos
            .in_check()
            .then(|| viewed_pos.king_square(viewed_pos.side_to_move()));
        let live = self.is_viewing_live();

        for display_row in 0..8u8 {
            for display_col in 0..8u8 {
                let (file_idx, rank_idx) = if self.flipped {
                    (7 - display_col, display_row)
                } else {
                    (display_col, 7 - display_row)
                };
                let is_light = (rank_idx + file_idx) % 2 != 0;
                let square_color = if is_light { light } else { dark };
                let top_left = rect.min
                    + egui::vec2(display_col as f32 * cell, display_row as f32 * cell);
                let cell_rect = egui::Rect::from_min_size(top_left, egui::vec2(cell, cell));
                painter.rect_filled(cell_rect, 0.0, square_color);

                let sq = Square::new(
                    File::from_index(file_idx).unwrap(),
                    Rank::from_index(rank_idx).unwrap(),
                );

                if let Some(mv) = viewed_mv {
                    if mv.from() == sq || mv.to() == sq {
                        painter.rect_filled(cell_rect, 0.0, last_move_tint);
                    }
                }
                if live && Some(sq) == self.selected {
                    painter.rect_filled(cell_rect, 0.0, selected_tint);
                }
                if Some(sq) == king_in_check {
                    painter.rect_filled(cell_rect, 0.0, check_tint);
                }

                if let Some(piece) = viewed_pos.piece_on(sq) {
                    painter.text(
                        cell_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        piece_glyph(piece),
                        egui::FontId::proportional(cell * 0.7),
                        egui::Color32::BLACK,
                    );
                }

                if live {
                    if let Some(legal_mv) =
                        self.legal_from_selected.iter().find(|m| m.to() == sq).copied()
                    {
                        if self.position.is_capture(legal_mv) {
                            painter.circle_stroke(
                                cell_rect.center(),
                                cell * 0.42,
                                egui::Stroke::new(cell * 0.06, dot_color),
                            );
                        } else {
                            painter.circle_filled(cell_rect.center(), cell * 0.16, dot_color);
                        }
                    }
                }
            }
        }

        // Promotion picker overlay: a vertical stack of [Q, R, B, N]
        // anchored at the promotion target, paint *after* the regular
        // board so it overdraws any piece on the squares it covers.
        if let Some(pending) = self.pending_promotion.as_ref() {
            let picker_bg = egui::Color32::from_rgb(0xff, 0xff, 0xff);
            let picker_stroke = egui::Stroke::new(2.0, egui::Color32::BLACK);
            let promoter_color = self.position.side_to_move();
            for (i, mv) in pending.candidates.iter().enumerate() {
                let pt = mv.promoted_to();
                let sq = picker_square_at(pending.to, i);
                let (dc, dr) = square_to_display_coords(sq, self.flipped);
                let top_left =
                    rect.min + egui::vec2(dc as f32 * cell, dr as f32 * cell);
                let cell_rect = egui::Rect::from_min_size(top_left, egui::vec2(cell, cell));
                painter.rect_filled(cell_rect, 0.0, picker_bg);
                painter.rect_stroke(cell_rect, 0.0, picker_stroke);
                painter.text(
                    cell_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    piece_glyph(Piece::new(promoter_color, pt)),
                    egui::FontId::proportional(cell * 0.7),
                    egui::Color32::BLACK,
                );
            }
        }

        if let Some(sq) = clicked_square {
            // Promotion picker takes precedence: a click on one of the
            // four picker squares applies that promotion; anything else
            // cancels (deselect drops the pending state too).
            if let Some(pending) = self.pending_promotion.take() {
                let picker_squares: [Square; 4] =
                    std::array::from_fn(|i| picker_square_at(pending.to, i));
                if let Some(idx) = picker_squares.iter().position(|&s| s == sq) {
                    let chosen = pending.candidates[idx];
                    self.apply_user_move(chosen);
                    self.maybe_queue_engine_search();
                } else {
                    // Click landed outside the picker — cancel. We
                    // already `take()`d the pending state, so deselect
                    // just clears the lingering pawn selection.
                    self.deselect();
                }
            } else {
                self.handle_click(sq);
            }
        }
    }
}

// =========================================================================
// Helpers
// =========================================================================

/// Format a score for display in the hint panel. Root-stm POV (the
/// side whose turn it is) is the natural reading there: "if you play
/// this, you'll be at +0.30."
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

/// Log the new-game header to stderr: the opponent seed so a varied
/// game can be reproduced, and the picked opening line (if any) so the
/// Render the six bot-noise sliders. Mutates the profile in place.
/// Kept as a free function so the New Game dialog can borrow `form`
/// fields mutably without fighting the borrow checker over `self`.
fn draw_noise_controls(ui: &mut egui::Ui, noise: &mut NoiseProfile) {
    egui::Grid::new("bot_noise_grid")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            ui.label("Blunder chance:")
                .on_hover_text(
                    "Per-move probability of a deliberate mistake. Blunders are picked \
                     from the engine's top-6; severity controls how bad they are.",
                );
            ui.add(
                egui::Slider::new(&mut noise.blunder_chance, 0.0..=1.0)
                    .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
            );
            ui.end_row();

            ui.label("Blunder severity (cp):")
                .on_hover_text(
                    "Preferred minimum score gap (centipawns) a blunder should achieve. \
                     In quiet positions where no line clears the gate, the bot falls back \
                     to the worst engine-considered move so the position still degrades.",
                );
            ui.add(egui::Slider::new(&mut noise.blunder_severity_cp, 0..=500));
            ui.end_row();

            ui.label("Wild move chance:")
                .on_hover_text(
                    "Per-move probability of picking uniformly from ALL legal moves, \
                     bypassing the search ranking. Beginner-bot territory — the only \
                     branch that can pick moves the engine didn't surface.",
                );
            ui.add(
                egui::Slider::new(&mut noise.wild_chance, 0.0..=1.0)
                    .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
            );
            ui.end_row();

            ui.label("Candidate pool:")
                .on_hover_text(
                    "How many top moves the bot samples from under softmax noise. \
                     1 = no sampling (always #1).",
                );
            ui.add(egui::Slider::new(&mut noise.candidate_pool, 1..=10));
            ui.end_row();

            ui.label("Softmax temperature (cp):")
                .on_hover_text(
                    "Flatness of the softmax distribution over the candidate pool. \
                     0 = always #1; higher = more variety among close-scoring moves.",
                );
            ui.add(egui::Slider::new(&mut noise.temperature_cp, 0..=500));
            ui.end_row();

            ui.label("Guaranteed mate-in:")
                .on_hover_text(
                    "Bot is guaranteed to convert mates of this length or shorter. \
                     1 = mate-in-1 is never thrown away. Set to 0 to allow blundering \
                     any mate.",
                );
            ui.add(egui::Slider::new(&mut noise.guaranteed_mate_in, 0..=10));
            ui.end_row();
        });
}

/// Render the eight eval-category checkboxes in a 2-column grid.
/// Each toggle simulates an opponent who doesn't understand the
/// corresponding concept (e.g. mask off king-safety for a positionally
/// naive bot).
fn draw_eval_mask_controls(ui: &mut egui::Ui, mask: &mut EvalMask) {
    // Two-column layout to keep the dialog from getting absurdly tall;
    // 8 categories split 4+4.
    let half = EvalCategory::ALL.len() / 2;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            for cat in &EvalCategory::ALL[..half] {
                eval_mask_checkbox(ui, mask, *cat);
            }
        });
        ui.vertical(|ui| {
            for cat in &EvalCategory::ALL[half..] {
                eval_mask_checkbox(ui, mask, *cat);
            }
        });
    });
}

fn eval_mask_checkbox(ui: &mut egui::Ui, mask: &mut EvalMask, cat: EvalCategory) {
    let mut disabled = mask.is_disabled(cat);
    if ui.checkbox(&mut disabled, cat.slug()).changed() {
        if disabled {
            mask.disable(cat);
        } else {
            mask.enable(cat);
        }
    }
}

/// user knows what they're up against. Sent to stderr — not the GUI —
/// because the desktop hasn't grown a status surface for this yet;
/// the launcher shell window is the de facto session log for now.
fn log_new_game_intro(opponent: &OpponentProfile, cursor: &Option<BookCursor>) {
    eprintln!(
        "opponent seed: {} (record this to replay the game)",
        opponent.seed,
    );
    if let Some(c) = cursor {
        let entry = c.opening();
        eprintln!("book: {} {}", entry.eco, entry.name);
    }
}

/// Walk a PV applying moves to a clone of `root` and producing a SAN
/// per ply. Stops on any ply that doesn't apply cleanly (shouldn't
/// happen with a real PV from the engine).
fn pv_to_san(root: &Position, pv: &[Move]) -> Vec<String> {
    let mut out = Vec::with_capacity(pv.len());
    let mut pos = root.clone();
    for mv in pv {
        out.push(san::format(&pos, *mv));
        pos.do_move(*mv);
    }
    out
}

fn random_bit() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
        & 1
}

fn pixel_to_square(local: egui::Vec2, cell: f32, flipped: bool) -> Option<Square> {
    let col = (local.x / cell).floor() as i32;
    let row = (local.y / cell).floor() as i32;
    if !(0..8).contains(&col) || !(0..8).contains(&row) {
        return None;
    }
    let (file_idx, rank_idx) = if flipped {
        (7 - col as u8, row as u8)
    } else {
        (col as u8, 7 - row as u8)
    };
    Some(Square::new(
        File::from_index(file_idx).unwrap(),
        Rank::from_index(rank_idx).unwrap(),
    ))
}

/// Display (column, row) for `sq` given board orientation. Mirrors
/// the inverse of [`pixel_to_square`].
fn square_to_display_coords(sq: Square, flipped: bool) -> (u8, u8) {
    let file_idx = sq.file().index() as u8;
    let rank_idx = sq.rank().index() as u8;
    if flipped {
        (7 - file_idx, rank_idx)
    } else {
        (file_idx, 7 - rank_idx)
    }
}

/// The `i`-th square in the promotion picker stack: index 0 = the
/// promotion target itself, then walking back along the file toward
/// the centre of the board. Always returns a valid square because
/// promotions land on rank 0 or rank 7, leaving four ranks of headroom
/// in the relevant direction.
fn picker_square_at(target: Square, i: usize) -> Square {
    let file = target.file();
    let target_rank = target.rank().index() as i8;
    // Promotion target is on rank 8 (idx 7, white promoting) or rank 1
    // (idx 0, black promoting). Walk inward.
    let direction: i8 = if target_rank == 7 { -1 } else { 1 };
    let rank_idx = (target_rank + direction * i as i8) as u8;
    Square::new(file, Rank::from_index(rank_idx).unwrap())
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

fn piece_glyph(piece: Piece) -> &'static str {
    match (piece.color(), piece.kind()) {
        (Color::White, PieceType::King) => "\u{2654}",
        (Color::White, PieceType::Queen) => "\u{2655}",
        (Color::White, PieceType::Rook) => "\u{2656}",
        (Color::White, PieceType::Bishop) => "\u{2657}",
        (Color::White, PieceType::Knight) => "\u{2658}",
        (Color::White, PieceType::Pawn) => "\u{2659}",
        (Color::Black, PieceType::King) => "\u{265A}",
        (Color::Black, PieceType::Queen) => "\u{265B}",
        (Color::Black, PieceType::Rook) => "\u{265C}",
        (Color::Black, PieceType::Bishop) => "\u{265D}",
        (Color::Black, PieceType::Knight) => "\u{265E}",
        (Color::Black, PieceType::Pawn) => "\u{265F}",
    }
}
