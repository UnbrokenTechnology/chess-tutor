//! Interactive REPL: human vs engine (or human vs human, or engine
//! self-play) with a live ANSI board and a small command vocabulary.
//!
//! Move input accepts both SAN (`Nf3`, `Qxc6`, `O-O`, `e8=Q`) and UCI
//! (`g1f3`, `e2e4`, `e7e8q`). SAN parsing is lenient — missing `x`,
//! stray `+`/`#`, and 0-0 / O-O are all fine.

use std::io::{self, BufRead, Write};
use std::time::{Duration, Instant};

use anyhow::Result;

use chess_tutor_engine::analysis::analyze_position;
use chess_tutor_engine::book::BookCursor;
use chess_tutor_engine::engine::{Engine, SearchParams};
use chess_tutor_engine::eval::evaluate_with_trace;
use chess_tutor_engine::movegen::legal_moves_vec;
use chess_tutor_engine::noise::{self, NoisePick};
use chess_tutor_engine::openings::{self, OpeningId, OpeningIdentification};
use chess_tutor_engine::opponent::{
    BookSelection, EvalCategory, EvalMask, NoiseProfile, OpponentProfile,
};
use chess_tutor_engine::position::Position;
use chess_tutor_engine::san;
use chess_tutor_engine::traps::{self, PendingTrap, TrapEvent, TrapHit};
use chess_tutor_engine::types::{Color, Move, Value};

use crate::analysis_report;
use crate::board::{render as render_board, RenderOptions};
use crate::eval_report;
use crate::retrospective::{self, RetrospectiveConfig};
use crate::uci;
use crate::EngineColor;

/// Per-engine-move node cap for interactive play. At typical speeds
/// (~4 Mnodes/s) this bounds the wait to ~1.3 s even on pathological
/// positions that would otherwise search indefinitely at the nominal
/// depth. Normal middlegame moves complete well under this (~10k–
/// 100k nodes), so it only triggers when something is misbehaving.
const ENGINE_TURN_NODE_CAP: u64 = 5_000_000;

pub struct PlayConfig {
    pub start_fen: Option<String>,
    pub engine_color: EngineColor,
    pub depth: u32,
    pub time_ms: Option<u64>,
    pub ascii: bool,
    pub flip: bool,
    pub light_mode: bool,
    /// Initial value for the runtime `explain-best` flag that the
    /// REPL `explain-best [on|off]` command toggles.
    pub explain_best: bool,
    /// When true, print the current FEN before each side's turn —
    /// for reproducing hangs / bad moves from the same position.
    pub show_fens: bool,
    /// Diagnostic: clear the engine's TT + history before every
    /// engine move. See `--reset-engine-per-move` CLI help for the
    /// rationale.
    pub reset_engine_per_move: bool,
    /// Diagnostic: stream iterative-deepening and root-move progress
    /// from the engine to stderr during each search. Sets
    /// `SearchParams::verbose_progress` on every search we run.
    pub search_progress: bool,
    /// Number of Lazy-SMP search threads for **every** search in this
    /// session — engine moves AND the auto-retrospective. Defaults to
    /// `1` (bit-deterministic across runs and takebacks, which the
    /// teaching tool relies on for "play the same move, get the same
    /// verdict"). Raise to use more cores in benchmarking. REPL
    /// `analyze` / `search` commands stay single-threaded regardless.
    pub threads: usize,
    /// Bot personality / variability toggles for this game. Phase A:
    /// the only populated field is [`OpponentProfile::seed`], logged
    /// at game start. Subsequent phases hook opening books, eval
    /// signal masking, and move noise into this struct.
    pub opponent: OpponentProfile,
}

/// One played ply — enough to undo the move and show what was played.
struct HistoryEntry {
    mv: Move,
    state: chess_tutor_engine::position::StateInfo,
    san: String,
    /// Snapshot of `pending_trap` as it was *before* this move was
    /// applied. On `undo`, we restore this so the trap cursor walks
    /// backward with the game.
    pending_before: Option<PendingTrap>,
    /// Snapshot of the opening-book cursor as it was *before* this
    /// move advanced (or dropped) it. On `undo`, we restore this so
    /// the cursor walks backward with the game — including
    /// resurrecting a cursor the move dropped.
    book_cursor_before: Option<BookCursor>,
}

pub fn play_loop(mut cfg: PlayConfig) -> Result<()> {
    let mut pos = match &cfg.start_fen {
        Some(fen) => Position::from_fen(fen).map_err(|e| anyhow::anyhow!("invalid --fen: {e}"))?,
        None => Position::startpos(),
    };
    let mut engine = Engine::default();
    let mut history: Vec<HistoryEntry> = Vec::new();
    // Every reached position's Zobrist key, starting with the initial
    // position and appending one entry per played move. `position_keys
    // .len() == history.len() + 1` always.
    let mut position_keys: Vec<u64> = vec![pos.key()];
    let mut manual_flip = cfg.flip;
    // Most-recently-announced opening name, if any. Used to print a
    // banner only on transitions — not once per render.
    let mut last_opening: Option<OpeningIdentification> = None;
    // Live trap cursor, when a trap is mid-refutation. `Some` between
    // a trigger firing and the next terminal event; `None` otherwise.
    let mut pending_trap: Option<PendingTrap> = None;
    // When true, every human move triggers an automatic retrospective
    // analysis of the pre-move position: verdict + engine-preferred
    // alternative + dominant term shift. Toggle via `retrospect on/off`.
    let mut retrospect_enabled = true;
    // When true, `Best` verdicts still render the full per-term
    // narration so the student learns *why* their move was best.
    // Default on; flip via REPL `explain-best off` or the CLI
    // `--no-explain-best` startup flag.
    let mut explain_best = cfg.explain_best;

    let stdin = io::stdin();
    let mut out = io::stdout().lock();

    writeln!(
        out,
        "chess-tutor play — enter moves as SAN (e4, Nf3, O-O) or UCI (e2e4)."
    )?;
    writeln!(
        out,
        "commands: moves | eval | search | analyze | retrospect | explain-best | openings | eval-mask | noise | undo | fen | flip | resign | help | quit"
    )?;
    // Log the opponent seed so a varied game can be replayed exactly.
    writeln!(
        out,
        "opponent seed: {} (pass --seed {} to replay this game)",
        cfg.opponent.seed, cfg.opponent.seed,
    )?;
    // Pick an opening line for this game, if the profile allows one
    // and we're starting from startpos. Cursor is mutated as the
    // game progresses; dropped on first deviation.
    let mut book_cursor = BookCursor::pick(&cfg.opponent, &pos);
    if let Some(cursor) = &book_cursor {
        let entry = cursor.opening();
        writeln!(out, "book: {} {}", entry.eco, entry.name)?;
    }
    if !cfg.opponent.eval_mask.is_empty() {
        let disabled: Vec<_> = cfg.opponent.eval_mask.disabled_iter().map(|c| c.slug()).collect();
        writeln!(out, "eval-mask: bot blind to {}", disabled.join(", "))?;
    }
    if !cfg.opponent.noise.is_off() {
        writeln!(out, "noise: {}", format_noise_summary(&cfg.opponent.noise))?;
    }
    // Editable view of the opponent's allowed-openings set. Changes
    // via the `openings allow / deny / reset` commands take effect on
    // the *next* game (the current cursor was already picked above);
    // we still let the user query and edit it so they can shape their
    // practice list while a game is in flight.
    let mut allowed_book: BookSelection = cfg.opponent.book.clone();
    match cfg.engine_color {
        EngineColor::White => writeln!(out, "engine plays white, you play black.")?,
        EngineColor::Black => writeln!(out, "engine plays black, you play white.")?,
        EngineColor::Both => writeln!(out, "engine plays both sides (self-play).")?,
        EngineColor::None => writeln!(out, "engine is idle; you control both sides.")?,
    }

    loop {
        render_current(&mut out, &pos, &history, &cfg, manual_flip)?;
        announce_opening_if_changed(&mut out, &pos, &mut last_opening)?;

        // Terminal-state detection.
        let legal = legal_moves_vec(&mut pos);
        if legal.is_empty() {
            if pos.in_check() {
                let winner = match pos.side_to_move() {
                    Color::White => "black",
                    Color::Black => "white",
                };
                writeln!(out, "checkmate — {winner} wins.")?;
            } else {
                writeln!(out, "stalemate — draw.")?;
            }
            break;
        }
        if pos.halfmove_clock() >= 100 {
            writeln!(out, "draw by 50-move rule.")?;
            break;
        }
        if threefold_reached(&position_keys) {
            writeln!(out, "draw by threefold repetition.")?;
            break;
        }
        if pos.has_insufficient_material() {
            writeln!(out, "draw by insufficient material.")?;
            break;
        }

        let mover = pos.side_to_move();
        let mover_name = match mover {
            Color::White => "white",
            Color::Black => "black",
        };
        writeln!(out, "move {}: {mover_name} to move.", pos.fullmove_number(),)?;
        if cfg.show_fens {
            writeln!(out, "fen: {}", pos.to_fen())?;
        }

        if is_engine_turn(mover, cfg.engine_color) {
            // Book first: if the cursor still has a move queued, play
            // it directly without invoking the search. This is the
            // only place the book overrides search results — see the
            // BookCursor docs for the strict invariant that
            // analytical paths must not consult the book.
            if let Some(book_mv) = book_cursor.as_ref().and_then(|c| c.peek()) {
                let san_text = san::format(&pos, book_mv);
                writeln!(out, "book: engine plays {san_text}")?;
                apply_move_and_scan(
                    &mut out,
                    &mut pos,
                    book_mv,
                    &mut history,
                    &mut position_keys,
                    &mut pending_trap,
                    &mut book_cursor,
                )?;
                continue;
            }
            play_engine_turn(
                &mut out,
                &mut pos,
                &mut engine,
                &cfg,
                &legal,
                &mut history,
                &mut position_keys,
                &mut pending_trap,
                &mut book_cursor,
            )?;
            continue;
        }

        // Don't spam the student with pre-move warnings while they're
        // already mid-trap — the trap cursor is doing the narration.
        if pending_trap.is_none() {
            announce_trap_threats(&mut out, &pos)?;
        }
        write!(out, "> ")?;
        out.flush()?;
        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            break; // EOF
        }
        let cmd = line.trim();
        if cmd.is_empty() {
            continue;
        }
        // Split once on whitespace: the first token is the verb, the
        // rest (if any) is the verb-specific argument. This lets
        // `search` take an optional PV count while keeping simple
        // one-word commands (`quit`, `eval`, `undo`, …) working.
        let (verb, arg) = match cmd.split_once(char::is_whitespace) {
            Some((v, a)) => (v, a.trim()),
            None => (cmd, ""),
        };
        match verb {
            "quit" | "exit" => break,
            "help" | "?" => print_help(&mut out)?,
            "moves" => {
                let sans: Vec<String> = legal.iter().map(|m| san::format(&pos, *m)).collect();
                writeln!(out, "{} legal moves: {}", sans.len(), sans.join(" "))?;
            }
            "eval" => {
                let (_v, trace) = evaluate_with_trace(&pos);
                print!("{}", eval_report::render(&trace));
            }
            "search" => match parse_search_command(arg) {
                // Analytical commands clone the play engine so they
                // inherit its accumulated TT/history (warm cache from
                // prior plies) but don't mutate it. This makes
                // repeated `search` / `analyze` calls deterministic
                // for the same position regardless of session order.
                Ok(multi_pv) => {
                    let mut analysis_engine = engine.clone();
                    run_search_report(
                        &mut out,
                        &mut pos,
                        &mut analysis_engine,
                        &cfg,
                        &position_keys,
                        multi_pv,
                    )?
                }
                Err(e) => writeln!(out, "{}", e)?,
            },
            "analyze" => match parse_analyze_command(arg) {
                Ok(AnalyzeArgs {
                    multi_pv,
                    top_percent,
                }) => {
                    let mut analysis_engine = engine.clone();
                    run_analyze_report(
                        &mut out,
                        &mut pos,
                        &mut analysis_engine,
                        &cfg,
                        &position_keys,
                        multi_pv,
                        top_percent,
                    )?
                }
                Err(e) => writeln!(out, "{}", e)?,
            },
            "retrospect" => match parse_toggle(arg) {
                Ok(Some(on)) => {
                    retrospect_enabled = on;
                    writeln!(
                        out,
                        "retrospective feedback is now {}.",
                        if on { "on" } else { "off" },
                    )?;
                }
                Ok(None) => writeln!(
                    out,
                    "retrospective feedback is {}.",
                    if retrospect_enabled { "on" } else { "off" },
                )?,
                Err(e) => writeln!(out, "{}", e)?,
            },
            "explain-best" => match parse_toggle(arg) {
                Ok(Some(on)) => {
                    explain_best = on;
                    writeln!(
                        out,
                        "explain-best is now {}.",
                        if on { "on" } else { "off" },
                    )?;
                }
                Ok(None) => writeln!(
                    out,
                    "explain-best is {}.",
                    if explain_best { "on" } else { "off" },
                )?,
                Err(e) => writeln!(out, "{}", e)?,
            },
            "openings" => run_openings_command(&mut out, arg, &mut allowed_book, &book_cursor)?,
            "eval-mask" => run_eval_mask_command(&mut out, arg, &mut cfg.opponent.eval_mask)?,
            "noise" => run_noise_command(&mut out, arg, &mut cfg.opponent.noise)?,
            "fen" => writeln!(out, "{}", pos.to_fen())?,
            "flip" => manual_flip = !manual_flip,
            "undo" => match history.pop() {
                Some(entry) => {
                    pos.undo_move(entry.mv, entry.state);
                    position_keys.pop();
                    pending_trap = entry.pending_before;
                    book_cursor = entry.book_cursor_before;
                    writeln!(out, "undid {}", entry.san)?;
                }
                None => writeln!(out, "nothing to undo.")?,
            },
            "resign" => {
                let winner = match mover {
                    Color::White => "black",
                    Color::Black => "white",
                };
                writeln!(out, "you resigned — {winner} wins.")?;
                break;
            }
            input => match parse_user_move(&mut pos, input) {
                Ok(mv) => {
                    // Snapshot pre-move state so retrospective can
                    // analyze the position the user just faced. Cloning
                    // the Position is cheap compared to the search we're
                    // about to run anyway.
                    let pre_move_snapshot = if retrospect_enabled {
                        Some((pos.clone(), game_history_for_search(&position_keys)))
                    } else {
                        None
                    };
                    apply_move_and_scan(
                        &mut out,
                        &mut pos,
                        mv,
                        &mut history,
                        &mut position_keys,
                        &mut pending_trap,
                        &mut book_cursor,
                    )?;
                    if let Some((mut pre_pos, game_hist)) = pre_move_snapshot {
                        let cfg_r = RetrospectiveConfig {
                            max_depth: cfg.depth,
                            max_time_ms: cfg.time_ms,
                            explain_best,
                            // Retrospective inherits whatever the
                            // user picked for `--threads` (default 1).
                            // Same-thread-count as engine moves keeps
                            // the entire CLI flow either fully
                            // bit-deterministic (threads=1) or fully
                            // multi-thread (threads>1); avoids the
                            // confusing middle ground where engine
                            // moves are stable but retrospectives drift.
                            threads: cfg.threads,
                        };
                        // Retrospective is analytical, not a real
                        // move — clone so its TT writes don't bleed
                        // into the engine's actual play state.
                        let mut analysis_engine = engine.clone();
                        retrospective::run_and_render(
                            &mut out,
                            &mut pre_pos,
                            &mut analysis_engine,
                            &cfg_r,
                            game_hist,
                            mv,
                        )?;
                        writeln!(
                            &mut out,
                            "[retrospective] {} ms · {} nodes · {:.2} Mnps",
                            analysis_engine.last_elapsed().as_millis(),
                            analysis_engine.last_nodes(),
                            analysis_engine.last_nps() / 1.0e6,
                        )?;
                    }
                }
                Err(e) => writeln!(out, "rejected: {e}")?,
            },
        }
    }
    Ok(())
}

fn render_current(
    out: &mut io::StdoutLock<'_>,
    pos: &Position,
    history: &[HistoryEntry],
    cfg: &PlayConfig,
    manual_flip: bool,
) -> io::Result<()> {
    let highlight = history
        .last()
        .map(|h| (h.mv.from().to_algebraic(), h.mv.to().to_algebraic()));
    writeln!(out)?;
    write!(
        out,
        "{}",
        render_board(
            &pos.to_fen(),
            &RenderOptions {
                ascii: cfg.ascii,
                flip: manual_flip,
                highlight,
                light_mode: cfg.light_mode,
            },
        )
    )?;
    Ok(())
}

/// If the current position identifies as a different opening than the
/// last one we announced, print a one-liner and update the tracker.
/// Silent when the opening hasn't changed (or has become unknown).
fn announce_opening_if_changed(
    out: &mut io::StdoutLock<'_>,
    pos: &Position,
    last: &mut Option<OpeningIdentification>,
) -> io::Result<()> {
    let current = openings::identify(pos);
    if current != *last {
        if let Some(op) = &current {
            writeln!(out, ">> {}  {}", op.eco, op.name)?;
        }
        *last = current;
    }
    Ok(())
}

fn is_engine_turn(side: Color, engine_color: EngineColor) -> bool {
    match engine_color {
        EngineColor::White => side == Color::White,
        EngineColor::Black => side == Color::Black,
        EngineColor::Both => true,
        EngineColor::None => false,
    }
}

#[allow(clippy::too_many_arguments)] // cohesive runtime-state slices the play loop owns
fn play_engine_turn(
    out: &mut io::StdoutLock<'_>,
    pos: &mut Position,
    engine: &mut Engine,
    cfg: &PlayConfig,
    legal_moves: &[Move],
    history: &mut Vec<HistoryEntry>,
    position_keys: &mut Vec<u64>,
    pending_trap: &mut Option<PendingTrap>,
    book_cursor: &mut Option<BookCursor>,
) -> io::Result<()> {
    if cfg.reset_engine_per_move {
        engine.new_game();
    }
    let effective_multi_pv = cfg.opponent.noise.effective_multi_pv();
    let params = SearchParams {
        max_depth: cfg.depth,
        // Safety cap: positions where alpha-beta pruning degenerates
        // (e.g., late self-play endgames where most lines draw by
        // repetition) can run for minutes at the nominal depth. 5M
        // nodes is ~1.3 s of search at typical speed — plenty for
        // interactive play, and ensures the engine always returns
        // something rather than hanging indefinitely.
        max_nodes: Some(ENGINE_TURN_NODE_CAP),
        max_time: cfg.time_ms.map(Duration::from_millis),
        // Bot noise widens this from 1 when the opponent profile
        // wants alternatives to sample from. With the default
        // (off) profile this stays 1 and the engine keeps its
        // single-PV fast path.
        multi_pv: effective_multi_pv,
        game_history: game_history_for_search(position_keys),
        force_include: Vec::new(),
        verbose_progress: cfg.search_progress,
        threads: cfg.threads,
        // Play engine move — apply the opponent's eval mask so the
        // bot plays "as if blind to" the masked categories. This is
        // the only path that consumes the mask; every analytical
        // construction below uses EvalMask::EMPTY.
        eval_mask: cfg.opponent.eval_mask,
    };
    write!(out, "engine thinking (depth {})... ", cfg.depth)?;
    out.flush()?;
    let started = Instant::now();
    let lines = engine.search(pos, params);
    let elapsed = started.elapsed();
    if lines.is_empty() {
        writeln!(out, "no legal moves.")?;
        return Ok(());
    }
    // Per the strict invariant in `opponent.rs`, only the play search
    // consults the noise profile. The pick is deterministic for a given
    // (seed, ply) — see `noise::pick`.
    let ply = position_keys.len() as u64;
    let pick = noise::pick(&cfg.opponent.noise, cfg.opponent.seed, ply, &lines, legal_moves);
    let (mv, score_label, noise_tag) = match pick {
        NoisePick::Line(idx) => {
            let line = &lines[idx];
            let Some(&mv) = line.pv.first() else {
                writeln!(out, "(search returned empty pv)")?;
                return Ok(());
            };
            // Annotate softmax-sampled picks so the student knows the
            // bot is off the best line. Silent on the common idx == 0
            // (no-branch-fired) path.
            let tag = if idx == 0 {
                String::new()
            } else {
                let delta = line.score.0 - lines[0].score.0;
                format!(" [noise: softmax #{} of {} ({:+} cp)]", idx + 1, lines.len(), delta)
            };
            (mv, format_score(line.score), tag)
        }
        NoisePick::Blunder(idx) => {
            let line = &lines[idx];
            let Some(&mv) = line.pv.first() else {
                writeln!(out, "(search returned empty pv)")?;
                return Ok(());
            };
            let delta = line.score.0 - lines[0].score.0;
            let tag = format!(
                " [noise: blunder #{} of {} ({:+} cp)]",
                idx + 1, lines.len(), delta,
            );
            (mv, format_score(line.score), tag)
        }
        NoisePick::Wild(mv) => {
            // Wild bypassed the engine's ranking. There's no score for
            // the wild move (we didn't search it), so the score column
            // shows the engine's preferred move instead — that's the
            // most useful teaching signal: "I was going to play Nf3
            // (+0.34) but the bot wild-picked Qh5 instead."
            let top_san = lines[0]
                .pv
                .first()
                .map(|m| san::format(pos, *m))
                .unwrap_or_else(|| "?".to_string());
            let tag = format!(
                " [noise: wild — engine preferred {} ({})]",
                top_san,
                format_score(lines[0].score),
            );
            (mv, "wild".to_string(), tag)
        }
    };
    let san_text = san::format(pos, mv);
    let nodes = engine.last_nodes();
    let nps_m = engine.last_nps() / 1.0e6;
    writeln!(
        out,
        "played {} ({}){} in {} ms · {} nodes · {:.2} Mnps",
        san_text,
        score_label,
        noise_tag,
        elapsed.as_millis(),
        nodes,
        nps_m,
    )?;
    apply_move_and_scan(out, pos, mv, history, position_keys, pending_trap, book_cursor)?;
    Ok(())
}

/// Format a PV (vector of moves from `pos`) as space-separated SAN. The
/// position is not mutated.
fn pv_to_san(pos: &Position, pv: &[Move]) -> Vec<String> {
    let mut out = Vec::with_capacity(pv.len());
    let mut scratch = pos.clone();
    for mv in pv {
        out.push(san::format_on(&mut scratch, *mv));
        scratch.do_move(*mv);
    }
    out
}

fn run_search_report(
    out: &mut io::StdoutLock<'_>,
    pos: &mut Position,
    engine: &mut Engine,
    cfg: &PlayConfig,
    position_keys: &[u64],
    multi_pv: usize,
) -> io::Result<()> {
    let effective_multi_pv = multi_pv.max(1);
    let params = SearchParams {
        max_depth: cfg.depth,
        max_nodes: None,
        max_time: cfg.time_ms.map(Duration::from_millis),
        multi_pv: effective_multi_pv,
        game_history: game_history_for_search(position_keys),
        force_include: Vec::new(),
        verbose_progress: false,
        // REPL `search` is analytical — stay single-threaded so
        // repeated invocations on the same position match bit-for-bit.
        threads: 1,
        // Analytical paths always run the unbiased eval so the
        // student sees true best play, regardless of any signal mask
        // the bot is currently using mid-game.
        eval_mask: EvalMask::EMPTY,
    };
    let started = Instant::now();
    let lines = engine.search(pos, params);
    let elapsed = started.elapsed();
    if lines.is_empty() {
        writeln!(out, "no legal moves.")?;
        return Ok(());
    }

    if lines.len() == 1 {
        let line = &lines[0];
        let pv_san = pv_to_san(pos, &line.pv);
        writeln!(
            out,
            "depth {} | {} | {} ms",
            line.depth,
            format_score(line.score),
            elapsed.as_millis()
        )?;
        writeln!(out, "pv: {}", pv_san.join(" "))?;
        return Ok(());
    }

    // Multi-PV output: one row per line with aligned rank / score /
    // delta / PV columns. The top line shows `(0 cp)` in the delta
    // column so the PV column lines up with subsequent rows' PV column.
    writeln!(
        out,
        "depth {} | {} ms | {} lines",
        lines[0].depth,
        elapsed.as_millis(),
        lines.len()
    )?;
    let top_cp = lines[0].score.0;
    for (i, line) in lines.iter().enumerate() {
        let pv_san = pv_to_san(pos, &line.pv);
        let delta = line.score.0 - top_cp;
        let delta_str = if delta == 0 {
            "(0 cp)".to_string()
        } else {
            format!("({:+} cp)", delta)
        };
        let settled_str = format_settled_suffix(&line.pv, line.settled_ply);
        writeln!(
            out,
            "  {:>2}. {:>6}   {:<10}  {:<36}  {}",
            i + 1,
            format_score(line.score),
            delta_str,
            pv_san.join(" "),
            settled_str,
        )?;
    }
    Ok(())
}

/// Run the teaching-analysis pipeline on the current position and
/// render each returned move's per-term delta attribution. Mirrors the
/// one-shot `chess-tutor search --analyze` surface but reuses the
/// REPL's depth / time-budget / repetition-history configuration.
fn run_analyze_report(
    out: &mut io::StdoutLock<'_>,
    pos: &mut Position,
    engine: &mut Engine,
    cfg: &PlayConfig,
    position_keys: &[u64],
    multi_pv: usize,
    top_percent: f32,
) -> io::Result<()> {
    let params = SearchParams {
        max_depth: cfg.depth,
        max_nodes: None,
        max_time: cfg.time_ms.map(Duration::from_millis),
        multi_pv: multi_pv.max(1),
        game_history: game_history_for_search(position_keys),
        force_include: Vec::new(),
        verbose_progress: false,
        // REPL `analyze` is analytical — single-threaded.
        threads: 1,
        // Analytical: always unbiased eval.
        eval_mask: EvalMask::EMPTY,
    };
    let started = Instant::now();
    let analyses = analyze_position(engine, pos, params);
    let elapsed = started.elapsed();
    if analyses.is_empty() {
        writeln!(out, "no legal moves.")?;
        return Ok(());
    }
    writeln!(
        out,
        "depth {} | {} ms | {} lines | top {:.0}%",
        analyses[0].depth,
        elapsed.as_millis(),
        analyses.len(),
        top_percent,
    )?;
    write!(
        out,
        "{}",
        analysis_report::render(pos, &analyses, top_percent)
    )?;
    Ok(())
}

/// Render a `[settles ply N]` / `[settles leaf]` suffix for a PV given
/// its `settled_ply`. Empty string when the PV is empty or no settled
/// index is reported.
fn format_settled_suffix(pv: &[Move], settled: Option<usize>) -> String {
    match settled {
        None => String::new(),
        Some(_) if pv.is_empty() => String::new(),
        Some(i) if i + 1 == pv.len() => "[settles leaf]".to_string(),
        Some(i) => format!("[settles ply {}]", i + 1),
    }
}

/// Parse the REPL `search` command's optional count argument. Returns
/// the PV count to request — default 1 when no arg is given, so
/// `search` matches what the engine would actually play (which uses
/// MultiPV=1). Use `search N` for `N > 1` to see alternatives.
fn parse_search_command(input: &str) -> Result<usize, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(1);
    }
    let n: usize = trimmed
        .parse()
        .map_err(|_| format!("bad count {:?}; expected a positive integer", trimmed))?;
    if n == 0 {
        return Err("count must be at least 1".to_string());
    }
    Ok(n)
}

/// Parsed form of the REPL `analyze` command: optional PV count and
/// optional cumulative-coverage percent.
#[derive(Debug, PartialEq)]
struct AnalyzeArgs {
    multi_pv: usize,
    top_percent: f32,
}

/// Parse the REPL `analyze` command's optional arguments:
///
///     analyze              — default 3 PVs, 75% coverage
///     analyze N            — N PVs, 75% coverage
///     analyze N P          — N PVs, P% coverage
///
/// `P` is a percent in (0, 100]; fractional values are fine (`62.5`).
fn parse_analyze_command(input: &str) -> Result<AnalyzeArgs, String> {
    let mut tokens = input.split_whitespace();
    let first = tokens.next();
    let second = tokens.next();
    if tokens.next().is_some() {
        return Err("too many arguments; usage: analyze [N] [PERCENT]".to_string());
    }

    let multi_pv = match first {
        None => 3,
        Some(tok) => {
            let n: usize = tok
                .parse()
                .map_err(|_| format!("bad count {:?}; expected a positive integer", tok))?;
            if n == 0 {
                return Err("count must be at least 1".to_string());
            }
            n
        }
    };

    let top_percent = match second {
        None => 75.0,
        Some(tok) => {
            let p: f32 = tok
                .parse()
                .map_err(|_| format!("bad percent {:?}; expected a number", tok))?;
            if !(p > 0.0 && p <= 100.0) {
                return Err("percent must be in (0, 100]".to_string());
            }
            p
        }
    };

    Ok(AnalyzeArgs {
        multi_pv,
        top_percent,
    })
}

/// Parse the optional `on` / `off` argument for a toggle command.
/// `Ok(None)` means no argument — caller should render the current
/// state. `Ok(Some(bool))` is an explicit set; `Err` is a bad token.
/// Dispatch for the `openings` REPL command. Edits the allowed-set for
/// the *next* game; queries display the current allowed-set and the
/// live cursor (read-only).
fn run_openings_command(
    out: &mut io::StdoutLock<'_>,
    arg: &str,
    allowed: &mut BookSelection,
    cursor: &Option<BookCursor>,
) -> io::Result<()> {
    let (subverb, subarg) = match arg.split_once(char::is_whitespace) {
        Some((v, a)) => (v.trim(), a.trim()),
        None => (arg.trim(), ""),
    };
    match subverb {
        "" => print_openings_status(out, allowed, cursor),
        "list" => print_allowed_list(out, allowed),
        "allow" => allow_openings(out, allowed, subarg),
        "deny" => deny_openings(out, allowed, subarg),
        "reset" => {
            *allowed = BookSelection::Allowed(chess_tutor_engine::book::curated_default_ids());
            let count = allowed_count(allowed);
            writeln!(
                out,
                "openings: reset to curated default ({count} entries; effective next game).",
            )
        }
        "selected" => print_selected(out, cursor),
        other => writeln!(
            out,
            "unknown openings subcommand {other:?} — try: list | allow PAT | deny PAT | reset | selected",
        ),
    }
}

fn allowed_count(allowed: &BookSelection) -> usize {
    match allowed {
        BookSelection::None => 0,
        BookSelection::Allowed(ids) => ids.len(),
    }
}

fn print_openings_status(
    out: &mut io::StdoutLock<'_>,
    allowed: &BookSelection,
    cursor: &Option<BookCursor>,
) -> io::Result<()> {
    let count = allowed_count(allowed);
    writeln!(out, "openings: {count} allowed in book; {} this game.",
        if cursor.is_some() { "in book" } else { "out of book" })?;
    writeln!(out, "  try: openings list | allow PAT | deny PAT | reset | selected")
}

fn print_allowed_list(out: &mut io::StdoutLock<'_>, allowed: &BookSelection) -> io::Result<()> {
    let ids = match allowed {
        BookSelection::None => &[][..],
        BookSelection::Allowed(v) => v.as_slice(),
    };
    if ids.is_empty() {
        return writeln!(out, "openings: allowed set is empty (engine plays from search).");
    }
    writeln!(out, "openings allowed ({}):", ids.len())?;
    for id in ids {
        if let Some(entry) = openings::entry(*id) {
            writeln!(out, "  {} {}", entry.eco, entry.name)?;
        }
    }
    Ok(())
}

fn allow_openings(
    out: &mut io::StdoutLock<'_>,
    allowed: &mut BookSelection,
    pattern: &str,
) -> io::Result<()> {
    if pattern.is_empty() {
        return writeln!(out, "usage: openings allow <pattern>");
    }
    let matches = openings::find_ids_matching(pattern);
    if matches.is_empty() {
        return writeln!(out, "openings: no opening matches {pattern:?}.");
    }
    let mut current: Vec<OpeningId> = match allowed {
        BookSelection::None => Vec::new(),
        BookSelection::Allowed(v) => std::mem::take(v),
    };
    let before = current.len();
    for id in matches {
        if !current.contains(&id) {
            current.push(id);
        }
    }
    let added = current.len() - before;
    *allowed = BookSelection::Allowed(current);
    writeln!(
        out,
        "openings: added {added} matching {pattern:?} (now {} allowed; effective next game).",
        allowed_count(allowed),
    )
}

fn deny_openings(
    out: &mut io::StdoutLock<'_>,
    allowed: &mut BookSelection,
    pattern: &str,
) -> io::Result<()> {
    if pattern.is_empty() {
        return writeln!(out, "usage: openings deny <pattern>");
    }
    let matches = openings::find_ids_matching(pattern);
    if matches.is_empty() {
        return writeln!(out, "openings: no opening matches {pattern:?}.");
    }
    let ids = match allowed {
        BookSelection::None => return writeln!(out, "openings: book is already off."),
        BookSelection::Allowed(v) => v,
    };
    let before = ids.len();
    ids.retain(|id| !matches.contains(id));
    let removed = before - ids.len();
    writeln!(
        out,
        "openings: removed {removed} matching {pattern:?} (now {} allowed; effective next game).",
        ids.len(),
    )
}

/// Dispatch for the `eval-mask` REPL command. Mutates the
/// [`OpponentProfile::eval_mask`] in place so subsequent engine
/// moves pick up the change without restarting the game.
fn run_eval_mask_command(
    out: &mut io::StdoutLock<'_>,
    arg: &str,
    mask: &mut EvalMask,
) -> io::Result<()> {
    let (subverb, subarg) = match arg.split_once(char::is_whitespace) {
        Some((v, a)) => (v.trim(), a.trim()),
        None => (arg.trim(), ""),
    };
    match subverb {
        "" | "list" => print_eval_mask(out, mask),
        "disable" => match EvalCategory::from_slug(subarg) {
            Some(cat) => {
                mask.disable(cat);
                writeln!(out, "eval-mask: bot now blind to {}.", cat.slug())
            }
            None => writeln!(
                out,
                "unknown category {subarg:?}; try one of: {}",
                slug_list(),
            ),
        },
        "enable" => match EvalCategory::from_slug(subarg) {
            Some(cat) => {
                mask.enable(cat);
                writeln!(out, "eval-mask: bot now considers {} again.", cat.slug())
            }
            None => writeln!(
                out,
                "unknown category {subarg:?}; try one of: {}",
                slug_list(),
            ),
        },
        "reset" => {
            *mask = EvalMask::EMPTY;
            writeln!(out, "eval-mask: all categories re-enabled.")
        }
        other => writeln!(
            out,
            "unknown eval-mask subcommand {other:?} — try: list | disable CAT | enable CAT | reset",
        ),
    }
}

fn slug_list() -> String {
    EvalCategory::ALL
        .iter()
        .map(|c| c.slug())
        .collect::<Vec<_>>()
        .join(", ")
}

/// One-line summary of the active noise knobs — used in the game-start
/// banner and as the default response to `noise` with no argument.
fn format_noise_summary(n: &NoiseProfile) -> String {
    if n.is_off() {
        return "off (bot always plays #1)".to_string();
    }
    format!(
        "pool={} temp={} cp · blunder={:.0}% (severity {}cp) · wild={:.0}% · guaranteed mate-in {}",
        n.candidate_pool,
        n.temperature_cp,
        n.blunder_chance * 100.0,
        n.blunder_severity_cp,
        n.wild_chance * 100.0,
        n.guaranteed_mate_in,
    )
}

/// Dispatch for the `noise` REPL command. Mutates the
/// [`OpponentProfile::noise`] in place so subsequent engine moves pick
/// up the change; the next call to `play_engine_turn` reads the new
/// effective MultiPV automatically.
fn run_noise_command(
    out: &mut io::StdoutLock<'_>,
    arg: &str,
    noise: &mut NoiseProfile,
) -> io::Result<()> {
    let (subverb, subarg) = match arg.split_once(char::is_whitespace) {
        Some((v, a)) => (v.trim(), a.trim()),
        None => (arg.trim(), ""),
    };
    match subverb {
        "" | "show" => writeln!(out, "noise: {}", format_noise_summary(noise)),
        "pool" => match subarg.parse::<usize>() {
            Ok(0) => writeln!(out, "noise: pool must be at least 1."),
            Ok(n) => {
                noise.candidate_pool = n;
                writeln!(out, "noise: pool set to {n} (effective from next engine move).")
            }
            Err(_) => writeln!(out, "usage: noise pool <positive integer>"),
        },
        "temp" => match subarg.parse::<i32>() {
            Ok(cp) => {
                noise.temperature_cp = cp;
                writeln!(out, "noise: temperature set to {cp} cp.")
            }
            Err(_) => writeln!(out, "usage: noise temp <centipawns>"),
        },
        "blunder" => match subarg.parse::<f32>() {
            Ok(p) if (0.0..=1.0).contains(&p) => {
                noise.blunder_chance = p;
                writeln!(out, "noise: blunder chance set to {:.0}%.", p * 100.0)
            }
            _ => writeln!(out, "usage: noise blunder <0.0-1.0>"),
        },
        "wild" => match subarg.parse::<f32>() {
            Ok(p) if (0.0..=1.0).contains(&p) => {
                noise.wild_chance = p;
                writeln!(
                    out,
                    "noise: wild chance set to {:.0}% (uniform pick from all legal moves).",
                    p * 100.0,
                )
            }
            _ => writeln!(out, "usage: noise wild <0.0-1.0>"),
        },
        "severity" => match subarg.parse::<i32>() {
            Ok(cp) if cp >= 0 => {
                noise.blunder_severity_cp = cp;
                writeln!(out, "noise: blunder severity set to {cp} cp.")
            }
            _ => writeln!(out, "usage: noise severity <non-negative centipawns>"),
        },
        "guarantee" => match subarg.parse::<u32>() {
            Ok(n) => {
                noise.guaranteed_mate_in = n;
                writeln!(out, "noise: guaranteed mate-in set to {n} (0 = no mate protected).")
            }
            Err(_) => writeln!(out, "usage: noise guarantee <non-negative integer>"),
        },
        "reset" => {
            *noise = NoiseProfile::default();
            writeln!(out, "noise: reset to off.")
        }
        other => writeln!(
            out,
            "unknown noise subcommand {other:?} — try: show | pool N | temp CP | blunder F | severity CP | wild F | guarantee N | reset",
        ),
    }
}

fn print_eval_mask(out: &mut io::StdoutLock<'_>, mask: &EvalMask) -> io::Result<()> {
    if mask.is_empty() {
        return writeln!(out, "eval-mask: all categories on (bot uses full eval).");
    }
    writeln!(out, "eval-mask:")?;
    for cat in EvalCategory::ALL {
        let state = if mask.is_disabled(cat) { "off" } else { "on " };
        writeln!(out, "  [{state}] {}", cat.slug())?;
    }
    Ok(())
}

fn print_selected(out: &mut io::StdoutLock<'_>, cursor: &Option<BookCursor>) -> io::Result<()> {
    match cursor {
        None => writeln!(out, "openings: out of book this game."),
        Some(c) => {
            let entry = c.opening();
            writeln!(out, "openings: in book — {} {}", entry.eco, entry.name)?;
            // The cursor's next move is shown via the play loop's
            // "book: engine plays X" line when the bot moves, so we
            // don't duplicate it here — keeping the output deterministic
            // regardless of whose turn it is.
            Ok(())
        }
    }
}

fn parse_toggle(input: &str) -> Result<Option<bool>, String> {
    match input.trim() {
        "" => Ok(None),
        "on" | "true" | "1" => Ok(Some(true)),
        "off" | "false" | "0" => Ok(Some(false)),
        other => Err(format!("expected 'on' or 'off', got {:?}", other)),
    }
}

fn parse_user_move(pos: &mut Position, input: &str) -> Result<Move, String> {
    // Try SAN first; fall back to UCI. SAN may fail for things that
    // are parseable as UCI (e.g., `e2e4`), and vice versa.
    match san::parse(pos, input) {
        Ok(mv) => Ok(mv),
        Err(san_err) => match uci::parse(pos, input) {
            Ok(mv) => Ok(mv),
            Err(uci_err) => Err(format!("not SAN ({san_err}); not UCI ({uci_err})")),
        },
    }
}

/// Slice `position_keys` into the form `SearchParams::game_history`
/// expects: every reached key *except* the current one (which the
/// search pushes separately as the root).
fn game_history_for_search(position_keys: &[u64]) -> Vec<u64> {
    if position_keys.is_empty() {
        return Vec::new();
    }
    position_keys[..position_keys.len() - 1].to_vec()
}

fn apply_move(
    pos: &mut Position,
    mv: Move,
    history: &mut Vec<HistoryEntry>,
    position_keys: &mut Vec<u64>,
    pending_before: Option<PendingTrap>,
    book_cursor_before: Option<BookCursor>,
) {
    let san = san::format(pos, mv);
    let state = pos.do_move(mv);
    history.push(HistoryEntry {
        mv,
        state,
        san,
        pending_before,
        book_cursor_before,
    });
    position_keys.push(pos.key());
}

/// [`apply_move`] plus full trap bookkeeping: advance the pending
/// cursor when a trap is live, look for a newly-fired trap on the
/// post-move position, and advance / drop the opening-book cursor.
/// All three pieces of state are snapshotted into the new
/// [`HistoryEntry`] so `undo` rolls them back together.
fn apply_move_and_scan(
    out: &mut io::StdoutLock<'_>,
    pos: &mut Position,
    mv: Move,
    history: &mut Vec<HistoryEntry>,
    position_keys: &mut Vec<u64>,
    pending_trap: &mut Option<PendingTrap>,
    book_cursor: &mut Option<BookCursor>,
) -> io::Result<()> {
    // Snapshot pending-trap and book-cursor BEFORE we advance them, so
    // `undo` restores the world exactly as it was before this move.
    let pending_snapshot = pending_trap.clone();
    let book_snapshot = book_cursor.clone();

    // If a trap is already mid-refutation, advance the cursor using
    // the pre-move position (san::parse needs legal-move context).
    if let Some(pending) = pending_trap.as_mut() {
        let event = traps::advance_pending(pending, pos, mv);
        announce_trap_event(out, &event)?;
        if event.is_terminal() {
            *pending_trap = None;
        }
    }

    // Capture pre-move data the scan_after_move path needs.
    let mover = pos.side_to_move();
    let piece = pos
        .piece_on(mv.from())
        .expect("caller passed a legal move; source square must have a piece");
    let from = mv.from();
    let to = mv.to();

    apply_move(
        pos,
        mv,
        history,
        position_keys,
        pending_snapshot,
        book_snapshot,
    );

    // Only scan for newly-fired traps when nothing is already pending.
    // Overlapping traps are theoretically possible but vanishingly
    // rare in the opening phase where the library lives.
    if pending_trap.is_none() {
        if let Some((entry, hit)) = traps::scan_after_move(pos, mover, piece.kind(), from, to)
            .into_iter()
            .next()
        {
            announce_trap_hit(out, &hit)?;
            *pending_trap = Some(PendingTrap::new(entry, hit));
        }
    }

    // Book bookkeeping: any move (engine book play, engine search,
    // human reply) gets observed. A diverging move drops the cursor;
    // a matching move advances it. Exhausted lines (cursor alive but
    // peek() returned None) also drop here, because observe sees no
    // expected ply to match against.
    let dropped = book_cursor.as_mut().is_some_and(|c| !c.observe(mv));
    if dropped {
        *book_cursor = None;
        writeln!(out, "out of book — engine now plays from search.")?;
    }

    Ok(())
}

/// Print pre-move warnings for every legal move the side-to-move
/// could play that would hand a known trap to the opponent. Silent
/// when no candidate threats exist (the common case).
fn announce_trap_threats(out: &mut io::StdoutLock<'_>, pos: &Position) -> io::Result<()> {
    for t in traps::scan_threats(pos) {
        let pv = t.hit.main_line_san.join(" ");
        writeln!(
            out,
            "warning: {} walks into {} — refutation {} ({:+} cp)",
            t.candidate_san, t.hit.name, pv, t.hit.main_line_gain_cp,
        )?;
    }
    Ok(())
}

/// Print a banner announcing a trap that just became live. The
/// `punisher` side in `hit` is whoever now gets to execute the
/// scripted refutation.
fn announce_trap_hit(out: &mut io::StdoutLock<'_>, hit: &TrapHit) -> io::Result<()> {
    let side = match hit.punisher {
        Color::White => "white",
        Color::Black => "black",
    };
    let pv = hit.main_line_san.join(" ");
    writeln!(
        out,
        ">> {} — {} plays {} ({:+} cp)",
        hit.name, side, pv, hit.main_line_gain_cp,
    )?;
    Ok(())
}

/// Narrate a move-by-move event from the pending-trap cursor.
/// Each variant gets a one-liner that tells the student what just
/// happened relative to the scripted tree.
fn announce_trap_event(out: &mut io::StdoutLock<'_>, event: &TrapEvent) -> io::Result<()> {
    let name = event.trap().name;
    match event {
        TrapEvent::PunisherExecuted { move_san, .. } => {
            writeln!(out, ">> {name}: punisher executes {move_san}")?;
        }
        TrapEvent::PunisherMissed { expected_san, .. } => {
            writeln!(
                out,
                ">> {name}: punisher missed the refutation — expected {expected_san}. \
                 The trap is gone; normal evaluation resumes.",
            )?;
        }
        TrapEvent::DefenderInTree { option, .. } => {
            let label = option.label.unwrap_or("(no commentary)");
            let tag = if option.is_main_defense {
                "defender plays the main line"
            } else {
                "defender walks deeper"
            };
            writeln!(out, ">> {name}: {tag} — {} ({label})", option.san)?;
            if option.punisher_follow_up.is_none() && !option.is_main_defense {
                writeln!(
                    out,
                    "   (library stops tracking here — the continuation is too \
                     position-specific; engine and normal evaluation take over.)",
                )?;
            }
        }
        TrapEvent::DefenderEscaped { .. } => {
            writeln!(
                out,
                ">> {name}: defender stepped out of the scripted line. Normal evaluation resumes.",
            )?;
        }
        TrapEvent::TreeComplete { gain_cp, .. } => {
            let gain = gain_cp.unwrap_or(0);
            writeln!(out, ">> {name}: refutation complete — {gain:+} cp.")?;
        }
    }
    Ok(())
}

/// True when the position currently on the board has appeared at least
/// three times, counting the initial position and every post-move key.
///
/// `position_keys` is expected to hold every reached key in order (the
/// starting position plus one entry per played move), so the current
/// position is always `position_keys.last()`.
fn threefold_reached(position_keys: &[u64]) -> bool {
    let Some(&current) = position_keys.last() else {
        return false;
    };
    position_keys.iter().filter(|&&k| k == current).count() >= 3
}

fn format_score(v: Value) -> String {
    // Mate-distance scores are encoded as MATE - ply. Report them as
    // `#N` / `-#N` (full moves) so the UI reads like a chess app.
    // Regular scores render as pawns (`+0.28`, `-1.05`) which is the
    // form the teaching output uses everywhere.
    let mate = Value::MATE.0;
    let abs = v.0.abs();
    if abs >= mate - Value::MAX_PLY {
        let plies_to_mate = mate - abs;
        let moves = (plies_to_mate + 1) / 2;
        return if v.0 > 0 {
            format!("#{moves}")
        } else {
            format!("-#{moves}")
        };
    }
    format!("{:+.2}", v.0 as f32 / 100.0)
}

fn print_help(out: &mut io::StdoutLock<'_>) -> io::Result<()> {
    writeln!(
        out,
        "move input: SAN (e4, Nf3, O-O, Qxf7#) or UCI (e2e4, g1f3)."
    )?;
    writeln!(out, "commands:")?;
    writeln!(out, "  moves    list every legal move as SAN")?;
    writeln!(
        out,
        "  eval     per-term evaluation trace for the current position"
    )?;
    writeln!(
        out,
        "  search [N]   run the engine; print top N PVs with deltas (default N=1, matching engine play)"
    )?;
    writeln!(
        out,
        "  analyze [N] [P]   teaching breakdown: top N PVs with per-term deltas,"
    )?;
    writeln!(
        out,
        "                    cumulative coverage P% (default N=3, P=75)"
    )?;
    writeln!(
        out,
        "  retrospect [on|off]   toggle automatic post-move verdict (default on)"
    )?;
    writeln!(
        out,
        "  explain-best [on|off] narrate why Best moves were best, not just the headline"
    )?;
    writeln!(
        out,
        "  openings [list | allow PAT | deny PAT | reset | selected]"
    )?;
    writeln!(
        out,
        "                    inspect or edit the opening book; PAT is a case-insensitive substring"
    )?;
    writeln!(
        out,
        "  eval-mask [list | disable CAT | enable CAT | reset]"
    )?;
    writeln!(
        out,
        "                    toggle bot's blindness to eval categories (effective from next engine move)"
    )?;
    writeln!(
        out,
        "  noise [show | pool N | temp CP | blunder F | severity CP | wild F | guarantee N | reset]"
    )?;
    writeln!(
        out,
        "                    bot move-sampling: top-K + softmax temperature, exploitable blunder, wild beginner-bot branch"
    )?;
    writeln!(out, "  undo     take back one ply")?;
    writeln!(out, "  fen      print the current FEN")?;
    writeln!(out, "  flip     flip the board")?;
    writeln!(out, "  resign   resign the game")?;
    writeln!(out, "  help     this message")?;
    writeln!(out, "  quit     exit")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threefold_empty_history_is_not_a_draw() {
        assert!(!threefold_reached(&[]));
    }

    #[test]
    fn threefold_single_visit_is_not_a_draw() {
        // Starting position only; one visit.
        assert!(!threefold_reached(&[0xAAAA_BBBB]));
    }

    #[test]
    fn threefold_second_visit_is_not_yet_a_draw() {
        // Regression: the original implementation fired a draw on the
        // second occurrence of a position (true twofold repetition).
        // The user's winning game ended early because of this.
        let k = 0xDEAD_BEEF_u64;
        let other = 0x1234_5678_u64;
        let keys = vec![k, other, k];
        assert!(!threefold_reached(&keys));
    }

    #[test]
    fn threefold_third_visit_is_a_draw() {
        let k = 0xDEAD_BEEF_u64;
        let other = 0x1234_5678_u64;
        let keys = vec![k, other, k, other, k];
        assert!(threefold_reached(&keys));
    }

    #[test]
    fn threefold_counts_starting_position() {
        // A knight-shuffle cycle can return to the starting position.
        // The starting position counts as the first occurrence, so three
        // visits total (start + two cycles back) draws.
        let start = 0xFFFF_u64;
        let k1 = 0x1111_u64;
        let k2 = 0x2222_u64;
        let k3 = 0x3333_u64;
        // Start → cycle1 → start → cycle2 → start.
        let keys = vec![start, k1, k2, k3, start, k1, k2, k3, start];
        assert!(threefold_reached(&keys));
    }

    #[test]
    fn threefold_end_to_end_via_knight_shuffle() {
        // Nf3 Nf6 Ng1 Ng8 returns both sides to the starting position.
        // Exercises apply_move + threefold_reached together on the real
        // path the REPL walks.
        let mut pos = Position::startpos();
        let mut history: Vec<HistoryEntry> = Vec::new();
        let mut keys: Vec<u64> = vec![pos.key()];

        let cycle = ["g1f3", "g8f6", "f3g1", "f6g8"];
        // First cycle: two visits to the startpos. Still not a draw.
        for m in cycle {
            let mv = crate::uci::parse(&mut pos, m).unwrap();
            apply_move(&mut pos, mv, &mut history, &mut keys, None, None);
        }
        assert!(!threefold_reached(&keys), "second visit must not draw");

        // Second cycle: three visits to the startpos. Draw.
        for m in cycle {
            let mv = crate::uci::parse(&mut pos, m).unwrap();
            apply_move(&mut pos, mv, &mut history, &mut keys, None, None);
        }
        assert!(threefold_reached(&keys), "third visit is threefold");
    }

    // ---- REPL `search [N]` argument parsing --------------------------

    #[test]
    fn parse_search_command_defaults_to_one() {
        // No-arg `search` means "what would the engine play?" — i.e.
        // MultiPV=1, matching `play_engine_turn`'s search params.
        assert_eq!(parse_search_command(""), Ok(1));
        assert_eq!(parse_search_command("   "), Ok(1));
    }

    #[test]
    fn parse_search_command_accepts_positive_integer() {
        assert_eq!(parse_search_command("1"), Ok(1));
        assert_eq!(parse_search_command("5"), Ok(5));
        assert_eq!(parse_search_command("20"), Ok(20));
    }

    #[test]
    fn parse_search_command_rejects_zero() {
        assert!(parse_search_command("0").is_err());
    }

    #[test]
    fn parse_search_command_rejects_garbage() {
        assert!(parse_search_command("abc").is_err());
        assert!(parse_search_command("-3").is_err());
        assert!(parse_search_command("3.5").is_err());
    }

    // ---- REPL `analyze [N] [P]` argument parsing --------------------

    #[test]
    fn parse_analyze_command_defaults_to_three_and_seventy_five() {
        assert_eq!(
            parse_analyze_command(""),
            Ok(AnalyzeArgs {
                multi_pv: 3,
                top_percent: 75.0,
            })
        );
    }

    #[test]
    fn parse_analyze_command_accepts_count_only() {
        assert_eq!(
            parse_analyze_command("5"),
            Ok(AnalyzeArgs {
                multi_pv: 5,
                top_percent: 75.0,
            })
        );
    }

    #[test]
    fn parse_analyze_command_accepts_count_and_percent() {
        assert_eq!(
            parse_analyze_command("4 90"),
            Ok(AnalyzeArgs {
                multi_pv: 4,
                top_percent: 90.0,
            })
        );
    }

    #[test]
    fn parse_analyze_command_accepts_fractional_percent() {
        assert_eq!(
            parse_analyze_command("2 62.5"),
            Ok(AnalyzeArgs {
                multi_pv: 2,
                top_percent: 62.5,
            })
        );
    }

    #[test]
    fn parse_analyze_command_rejects_zero_count() {
        assert!(parse_analyze_command("0").is_err());
    }

    #[test]
    fn parse_analyze_command_rejects_percent_out_of_range() {
        assert!(parse_analyze_command("2 0").is_err());
        assert!(parse_analyze_command("2 150").is_err());
        assert!(parse_analyze_command("2 -10").is_err());
    }

    #[test]
    fn parse_analyze_command_rejects_extra_args() {
        assert!(parse_analyze_command("3 80 extra").is_err());
    }

    #[test]
    fn parse_analyze_command_rejects_garbage() {
        assert!(parse_analyze_command("abc").is_err());
        assert!(parse_analyze_command("2 nope").is_err());
    }

    // ---- parse_toggle ------------------------------------------------

    #[test]
    fn parse_toggle_empty_returns_none() {
        assert_eq!(parse_toggle(""), Ok(None));
        assert_eq!(parse_toggle("   "), Ok(None));
    }

    #[test]
    fn parse_toggle_accepts_on_variants() {
        assert_eq!(parse_toggle("on"), Ok(Some(true)));
        assert_eq!(parse_toggle("true"), Ok(Some(true)));
        assert_eq!(parse_toggle("1"), Ok(Some(true)));
    }

    #[test]
    fn parse_toggle_accepts_off_variants() {
        assert_eq!(parse_toggle("off"), Ok(Some(false)));
        assert_eq!(parse_toggle("false"), Ok(Some(false)));
        assert_eq!(parse_toggle("0"), Ok(Some(false)));
    }

    #[test]
    fn parse_toggle_rejects_garbage() {
        assert!(parse_toggle("maybe").is_err());
        assert!(parse_toggle("yes").is_err());
    }
}
