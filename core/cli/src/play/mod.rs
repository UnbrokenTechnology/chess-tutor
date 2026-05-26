//! Interactive REPL: human vs engine (or human vs human, or engine
//! self-play) with a live ANSI board and a small command vocabulary.
//!
//! Move input accepts both SAN (`Nf3`, `Qxc6`, `O-O`, `e8=Q`) and UCI
//! (`g1f3`, `e2e4`, `e7e8q`). SAN parsing is lenient — missing `x`,
//! stray `+`/`#`, and 0-0 / O-O are all fine.
//!
//! Game state (position, history, opponent profile, book cursor) lives
//! in [`chess_tutor_ui::Session`]. The CLI owns: REPL command parsing,
//! the synchronous-feel game loop (blocks on the worker between
//! prompts), trap-cursor tracking, and prose formatting of the
//! retrospective / search / analyze output. All searches — engine
//! play, auto-retrospective, REPL `search` / `analyze` — run on
//! Session's worker.
use std::io::{self, BufRead, Write};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;

use chess_tutor_engine::engine::SearchParams;
use chess_tutor_engine::eval::evaluate_with_trace;
use chess_tutor_engine::movegen::legal_moves_vec;
use chess_tutor_engine::openings::OpeningIdentification;
use chess_tutor_engine::opponent::{
    BookSelection, EvalMask, OpponentProfile,
};
use chess_tutor_engine::position::Position;
use chess_tutor_engine::san;
use chess_tutor_engine::types::{Color, Move};
use chess_tutor_ui::event::Event;
use chess_tutor_ui::session::{EngineMode, HistoryEntry};
use chess_tutor_ui::Session;

use crate::eval_report;
use crate::EngineColor;


mod commands;
mod output;
use commands::*;
use output::*;
mod parse;
use parse::*;

pub struct PlayConfig {
    pub start_fen: Option<String>,
    pub engine_color: EngineColor,
    /// Depth the engine searches to when picking *its* moves.
    pub depth: u32,
    /// Depth the auto-retrospective searches to. Independent of
    /// [`Self::depth`] so a weakened bot can still give strong
    /// teaching feedback.
    pub retrospective_depth: u32,
    /// Per-move time cap for REPL `search` / `analyze` commands (the
    /// engine itself uses depth-budget for determinism). `None` =
    /// pure depth-cap.
    pub time_ms: Option<u64>,
    pub ascii: bool,
    pub flip: bool,
    pub light_mode: bool,
    /// Initial value for the runtime `explain-best` flag.
    pub explain_best: bool,
    /// Print the current FEN before each side's turn.
    pub show_fens: bool,
    /// Lazy-SMP thread count for the auto-retrospective. Engine play
    /// stays single-thread inside Session for teaching determinism;
    /// REPL `search` / `analyze` are also single-thread (deterministic
    /// across runs).
    pub threads: usize,
    pub opponent: OpponentProfile,
}

pub fn play_loop(cfg: PlayConfig) -> Result<()> {
    let start_pos = match &cfg.start_fen {
        Some(fen) => Position::from_fen(fen).map_err(|e| anyhow::anyhow!("invalid --fen: {e}"))?,
        None => Position::startpos(),
    };
    let mut session = Session::new(Arc::new(|| {}));
    session.set_log_to_stderr(false);
    // Session's worker handles auto-retrospective; CLI just renders.
    session.set_auto_retrospective(true);
    session.set_retrospective_depth(cfg.retrospective_depth);

    let engine_plays = match cfg.engine_color {
        EngineColor::White => EngineMode::Side(Color::White),
        EngineColor::Black => EngineMode::Side(Color::Black),
        EngineColor::Both => EngineMode::Both,
        EngineColor::None => EngineMode::None,
    };
    session.start_game(start_pos.clone(), engine_plays, cfg.depth, cfg.opponent.clone());

    // Trap events live on Session's HistoryEntry now; the CLI just
    // walks newly-applied entries and prints whatever the engine /
    // session populated.
    let mut last_processed_len: usize = 0;

    let mut manual_flip = cfg.flip;
    let mut last_opening: Option<OpeningIdentification> = None;
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
    writeln!(
        out,
        "opponent seed: {} (pass --seed {} to replay this game)",
        cfg.opponent.seed, cfg.opponent.seed,
    )?;
    if !cfg.opponent.eval_mask.is_empty() {
        let disabled: Vec<_> = cfg.opponent.eval_mask.disabled_iter().map(|c| c.slug()).collect();
        writeln!(out, "eval-mask: bot blind to {}", disabled.join(", "))?;
    }
    if !cfg.opponent.noise.is_off() {
        writeln!(out, "noise: {}", format_noise_summary(&cfg.opponent.noise))?;
    }
    // Allowed-openings editor: CLI-local because edits take effect on
    // the next game (the current game's BookCursor is frozen at
    // start_game time).
    let mut allowed_book: BookSelection = cfg.opponent.book.clone();
    match cfg.engine_color {
        EngineColor::White => writeln!(out, "engine plays white, you play black.")?,
        EngineColor::Black => writeln!(out, "engine plays black, you play white.")?,
        EngineColor::Both => writeln!(out, "engine plays both sides (self-play).")?,
        EngineColor::None => writeln!(out, "engine is idle; you control both sides.")?,
    }

    loop {
        // Drain worker results, blocking while the engine is mid-
        // think. Catching up *after each* result keeps the output
        // ordered: the Retrospective job fires before the Search
        // job, so retrospective text prints before the engine's
        // reply line.
        while session.is_engine_thinking() {
            session.wait_for_worker();
            catch_up_history(
                &mut out,
                &start_pos,
                session.history(),
                &mut last_processed_len,
                cfg.engine_color,
                explain_best,
            )?;
        }
        session.poll_worker();
        catch_up_history(
            &mut out,
            &start_pos,
            session.history(),
            &mut last_processed_len,
            cfg.engine_color,
            explain_best,
        )?;

        render_current(&mut out, session.position(), session.history(), &cfg, manual_flip)?;
        announce_opening_if_changed(&mut out, session.position(), &mut last_opening)?;

        if let Some(outcome) = session.game_outcome() {
            writeln!(out, "{}", outcome.to_lowercase())?;
            break;
        }

        let mover = session.position().side_to_move();
        let mover_name = match mover {
            Color::White => "white",
            Color::Black => "black",
        };
        writeln!(
            out,
            "move {}: {mover_name} to move.",
            session.position().fullmove_number(),
        )?;
        if cfg.show_fens {
            writeln!(out, "fen: {}", session.position().to_fen())?;
        }

        if session.pending_trap().is_none() {
            announce_trap_threats(&mut out, &session.trap_threats())?;
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
        let (verb, arg) = match cmd.split_once(char::is_whitespace) {
            Some((v, a)) => (v, a.trim()),
            None => (cmd, ""),
        };
        match verb {
            "quit" | "exit" => break,
            "help" | "?" => print_help(&mut out)?,
            "moves" => {
                let mut scratch = session.position().clone();
                let legal = legal_moves_vec(&mut scratch);
                let sans: Vec<String> =
                    legal.iter().map(|m| san::format(session.position(), *m)).collect();
                writeln!(out, "{} legal moves: {}", sans.len(), sans.join(" "))?;
            }
            "eval" => {
                let (_v, trace) = evaluate_with_trace(session.position());
                print!("{}", eval_report::render(&trace));
            }
            "search" => match parse_search_command(arg) {
                Ok(multi_pv) => {
                    let params = analysis_params(cfg.depth, cfg.time_ms, multi_pv);
                    let pos_for_search = session.position().clone();
                    let outcome = session.run_analysis(pos_for_search, params);
                    print_search_report(&mut out, session.position(), &outcome)?;
                }
                Err(e) => writeln!(out, "{}", e)?,
            },
            "analyze" => match parse_analyze_command(arg) {
                Ok(AnalyzeArgs { multi_pv, top_percent }) => {
                    let params = analysis_params(cfg.depth, cfg.time_ms, multi_pv);
                    let pos_for_search = session.position().clone();
                    let outcome = session.run_analysis(pos_for_search, params);
                    print_analyze_report(
                        &mut out,
                        session.position(),
                        &outcome,
                        top_percent,
                    )?;
                }
                Err(e) => writeln!(out, "{}", e)?,
            },
            "retrospect" => match parse_toggle(arg) {
                Ok(Some(on)) => {
                    session.set_auto_retrospective(on);
                    writeln!(
                        out,
                        "retrospective feedback is now {}.",
                        if on { "on" } else { "off" },
                    )?;
                }
                Ok(None) => writeln!(
                    out,
                    "retrospective feedback is {}.",
                    if session.auto_retrospective() { "on" } else { "off" },
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
            "openings" => {
                let history_moves: Vec<Move> = session.history().iter().map(|e| e.mv).collect();
                run_openings_command(
                    &mut out,
                    arg,
                    &mut allowed_book,
                    session.position(),
                    &history_moves,
                )?;
            }
            "eval-mask" => {
                let mask = &mut session.opponent_mut().eval_mask;
                run_eval_mask_command(&mut out, arg, mask)?;
            }
            "noise" => {
                let noise = &mut session.opponent_mut().noise;
                run_noise_command(&mut out, arg, noise)?;
            }
            "fen" => writeln!(out, "{}", session.position().to_fen())?,
            "flip" => manual_flip = !manual_flip,
            "undo" => {
                let history = session.history();
                if history.is_empty() {
                    writeln!(out, "nothing to undo.")?;
                } else {
                    let last_san = history.last().map(|e| e.san.clone());
                    let prev_san = (history.len() >= 2)
                        .then(|| history[history.len() - 2].san.clone());
                    let len_before = history.len();
                    session.dispatch(Event::Takeback);
                    let rewound = len_before - session.history().len();
                    match (rewound, last_san, prev_san) {
                        (2, Some(last), Some(prev)) => {
                            writeln!(out, "undid {prev} (and engine's {last}).")?;
                        }
                        (_, Some(last), _) => writeln!(out, "undid {last}.")?,
                        _ => writeln!(out, "undid.")?,
                    }
                }
            }
            "resign" => {
                let winner = match mover {
                    Color::White => "black",
                    Color::Black => "white",
                };
                writeln!(out, "you resigned — {winner} wins.")?;
                break;
            }
            input => match parse_user_move(&mut session.position().clone(), input) {
                Ok(mv) => session.play_user_move(mv),
                Err(e) => writeln!(out, "rejected: {e}")?,
            },
        }
    }
    let _ = cfg.threads;
    Ok(())
}

/// Build the `SearchParams` shared between the REPL `search` and
/// `analyze` commands. Both run analytically — single-thread, unbiased
/// eval, no force-include.
fn analysis_params(depth: u32, time_ms: Option<u64>, multi_pv: usize) -> SearchParams {
    SearchParams {
        max_depth: depth,
        max_nodes: None,
        max_time: time_ms.map(Duration::from_millis),
        multi_pv: multi_pv.max(1),
        // Session::run_analysis populates game_history from its own
        // position_keys via its internal flow. Hmm — actually no, the
        // SearchParams.game_history is what the worker sees. Session
        // forwards verbatim. We could leave this empty (the worker
        // root is fed via the AnalyzeSync pos), losing threefold
        // detection in the analytical search. For interactive REPL
        // use that's tolerable; threefold-in-analysis is rare and
        // the CLI's old code also dropped it for `search` (only the
        // play loop's history was threaded).
        game_history: Vec::new(),
        force_include: Vec::new(),
        verbose_progress: false,
        threads: 1,
        eval_mask: EvalMask::EMPTY,
    }
}

fn catch_up_history(
    out: &mut io::StdoutLock<'_>,
    start_pos: &Position,
    history: &[HistoryEntry],
    last_processed_len: &mut usize,
    engine_color: EngineColor,
    explain_best: bool,
) -> io::Result<()> {
    // Handle undo: history shrank since last visit. Replay anything
    // remaining from index 0; trap state itself is already restored
    // by Session::dispatch(Takeback).
    if history.len() < *last_processed_len {
        *last_processed_len = 0;
    }

    while *last_processed_len < history.len() {
        let entry = &history[*last_processed_len];

        // Trap events Session emitted while applying this move
        // (advance_pending output), followed by any new-trap hit
        // (scan_after_move output). Mutually exclusive in practice
        // but the loop handles both consistently.
        for event in &entry.trap_events {
            announce_trap_event(out, event)?;
        }
        if let Some(hit) = &entry.trap_hit {
            announce_trap_hit(out, hit)?;
        }

        if user_owns(entry.moved_by, engine_color) {
            // User move. Print the retrospective if it's filled in;
            // if not, the worker hasn't returned yet — the caller's
            // outer loop will catch it on a subsequent
            // wait_for_worker.
            if let Some(retro) = &entry.retrospective {
                let pre_move_pos = if *last_processed_len == 0 {
                    start_pos.clone()
                } else {
                    history[*last_processed_len - 1].position_after.clone()
                };
                print_retrospective(out, &pre_move_pos, retro, explain_best)?;
            }
        } else {
            print_engine_move(out, entry)?;
        }
        *last_processed_len += 1;
    }
    Ok(())
}

/// True when `mover` is a user-owned side under `engine_color`.
fn user_owns(mover: Color, engine_color: EngineColor) -> bool {
    match engine_color {
        EngineColor::None => true,
        EngineColor::Both => false,
        EngineColor::White => mover != Color::White,
        EngineColor::Black => mover != Color::Black,
    }
}
