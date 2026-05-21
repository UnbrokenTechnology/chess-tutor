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

use chess_tutor_engine::book::BookCursor;
use chess_tutor_engine::engine::SearchParams;
use chess_tutor_engine::eval::evaluate_with_trace;
use chess_tutor_engine::movegen::legal_moves_vec;
use chess_tutor_engine::openings::{self, OpeningId, OpeningIdentification};
use chess_tutor_engine::opponent::{
    BookSelection, EvalCategory, EvalMask, NoiseProfile, OpponentProfile,
};
use chess_tutor_engine::position::Position;
use chess_tutor_engine::san;
use chess_tutor_engine::traps::{TrapEvent, TrapHit, TrapThreatened};
use chess_tutor_engine::types::{Color, Move, Value};
use chess_tutor_narration::{format_retrospective, NarrationOptions};
use chess_tutor_ui::event::Event;
use chess_tutor_ui::session::{EngineMode, HistoryEntry, RetrospectiveResult};
use chess_tutor_ui::view::BoardView;
use chess_tutor_ui::{NoisePickInfo, Session};

use crate::analysis_report;
use crate::board::{render as render_board, RenderOptions};
use crate::eval_report;
use crate::uci;
use crate::EngineColor;

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

fn print_retrospective(
    out: &mut io::StdoutLock<'_>,
    pre_move_pos: &Position,
    retro: &RetrospectiveResult,
    explain_best: bool,
) -> io::Result<()> {
    let opts = NarrationOptions { explain_best };
    let text = format_retrospective(pre_move_pos, &retro.analyses, retro.user_move, &opts);
    out.write_all(text.as_bytes())?;
    writeln!(
        out,
        "[retrospective] {} ms · {} nodes · {:.2} Mnps",
        retro.elapsed.as_millis(),
        retro.nodes,
        retro.nps_m,
    )
}

fn print_engine_move(out: &mut io::StdoutLock<'_>, entry: &HistoryEntry) -> io::Result<()> {
    let noise_tag = entry
        .noise_pick
        .as_ref()
        .map(|p| format!(" {}", format_noise_tag(p)))
        .unwrap_or_default();
    match &entry.engine_info {
        Some(info) => {
            let score_str = format_score_white_pov(info.score_white_pov, entry.moved_by);
            writeln!(
                out,
                "engine played {} ({}){} at depth {} in {} ms · {} nodes · {:.2} Mnps",
                entry.san,
                score_str,
                noise_tag,
                info.depth,
                info.elapsed.as_millis(),
                info.nodes,
                info.nps_m,
            )?;
        }
        None => {
            writeln!(out, "engine played {} (book move)", entry.san)?;
        }
    }
    Ok(())
}

fn format_noise_tag(info: &NoisePickInfo) -> String {
    match info {
        NoisePickInfo::Softmax {
            pick_idx,
            num_lines,
            delta_from_top_cp,
        } => format!(
            "[noise: softmax #{} of {} ({:+} cp)]",
            pick_idx + 1,
            num_lines,
            delta_from_top_cp,
        ),
        NoisePickInfo::Blunder {
            pick_idx,
            num_lines,
            delta_from_top_cp,
        } => format!(
            "[noise: blunder #{} of {} ({:+} cp)]",
            pick_idx + 1,
            num_lines,
            delta_from_top_cp,
        ),
        NoisePickInfo::BlunderSkipped { closest_above_loss_cp } => format!(
            "[noise: blunder roll skipped — closest above-band line was -{} cp]",
            closest_above_loss_cp,
        ),
        NoisePickInfo::Wild { engine_top, engine_top_score } => format!(
            "[noise: wild — engine preferred {:?} ({:+})]",
            engine_top, engine_top_score.0,
        ),
    }
}

fn format_score_white_pov(white_pov: Value, mover: Color) -> String {
    let from_mover = if mover == Color::White { white_pov } else { -white_pov };
    format_score(from_mover)
}

fn render_current(
    out: &mut io::StdoutLock<'_>,
    pos: &Position,
    history: &[HistoryEntry],
    cfg: &PlayConfig,
    manual_flip: bool,
) -> io::Result<()> {
    let last_move = history.last().map(|h| h.mv);
    let view = BoardView::compose(pos, manual_flip, last_move, None, &[], None, Vec::new());
    writeln!(out)?;
    write!(
        out,
        "{}",
        render_board(
            &view,
            &RenderOptions {
                ascii: cfg.ascii,
                light_mode: cfg.light_mode,
            },
        )
    )?;
    Ok(())
}

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

fn pv_to_san(pos: &Position, pv: &[Move]) -> Vec<String> {
    let mut out = Vec::with_capacity(pv.len());
    let mut scratch = pos.clone();
    for mv in pv {
        out.push(san::format_on(&mut scratch, *mv));
        scratch.do_move(*mv);
    }
    out
}

fn print_search_report(
    out: &mut io::StdoutLock<'_>,
    pos: &Position,
    outcome: &chess_tutor_ui::session::AnalysisOutcome,
) -> io::Result<()> {
    if outcome.analyses.is_empty() {
        writeln!(out, "no legal moves.")?;
        return Ok(());
    }
    if outcome.analyses.len() == 1 {
        let analysis = &outcome.analyses[0];
        let pv_san = pv_to_san(pos, &analysis.pv);
        writeln!(
            out,
            "depth {} | {} | {} ms",
            analysis.depth,
            format_score(analysis.score),
            outcome.elapsed.as_millis(),
        )?;
        writeln!(out, "pv: {}", pv_san.join(" "))?;
        return Ok(());
    }
    writeln!(
        out,
        "depth {} | {} ms | {} lines",
        outcome.analyses[0].depth,
        outcome.elapsed.as_millis(),
        outcome.analyses.len(),
    )?;
    let top_cp = outcome.analyses[0].score.0;
    for (i, analysis) in outcome.analyses.iter().enumerate() {
        let pv_san = pv_to_san(pos, &analysis.pv);
        let delta = analysis.score.0 - top_cp;
        let delta_str = if delta == 0 {
            "(0 cp)".to_string()
        } else {
            format!("({:+} cp)", delta)
        };
        let settled_str = format_settled_suffix(&analysis.pv, analysis.settled_ply);
        writeln!(
            out,
            "  {:>2}. {:>6}   {:<10}  {:<36}  {}",
            i + 1,
            format_score(analysis.score),
            delta_str,
            pv_san.join(" "),
            settled_str,
        )?;
    }
    Ok(())
}

fn print_analyze_report(
    out: &mut io::StdoutLock<'_>,
    pos: &Position,
    outcome: &chess_tutor_ui::session::AnalysisOutcome,
    top_percent: f32,
) -> io::Result<()> {
    if outcome.analyses.is_empty() {
        writeln!(out, "no legal moves.")?;
        return Ok(());
    }
    writeln!(
        out,
        "depth {} | {} ms | {} lines | top {:.0}%",
        outcome.analyses[0].depth,
        outcome.elapsed.as_millis(),
        outcome.analyses.len(),
        top_percent,
    )?;
    write!(
        out,
        "{}",
        analysis_report::render(pos, &outcome.analyses, top_percent),
    )?;
    Ok(())
}

fn format_settled_suffix(pv: &[Move], settled: Option<usize>) -> String {
    match settled {
        None => String::new(),
        Some(_) if pv.is_empty() => String::new(),
        Some(i) if i + 1 == pv.len() => "[settles leaf]".to_string(),
        Some(i) => format!("[settles ply {}]", i + 1),
    }
}

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

#[derive(Debug, PartialEq)]
struct AnalyzeArgs {
    multi_pv: usize,
    top_percent: f32,
}

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
    Ok(AnalyzeArgs { multi_pv, top_percent })
}

fn run_openings_command(
    out: &mut io::StdoutLock<'_>,
    arg: &str,
    allowed: &mut BookSelection,
    pos: &Position,
    history_moves: &[Move],
) -> io::Result<()> {
    let (subverb, subarg) = match arg.split_once(char::is_whitespace) {
        Some((v, a)) => (v.trim(), a.trim()),
        None => (arg.trim(), ""),
    };
    match subverb {
        "" => print_openings_status(out, allowed, history_moves),
        "list" => print_allowed_list(out, allowed),
        "allow" => allow_openings(out, allowed, subarg),
        "deny" => deny_openings(out, allowed, subarg),
        "reset" => {
            *allowed = BookSelection::Allowed(chess_tutor_engine::book::all_ids());
            let count = allowed_count(allowed);
            writeln!(
                out,
                "openings: reset to all theoretical openings ({count} entries; effective next game).",
            )
        }
        "selected" => print_selected(out, pos),
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
    history_moves: &[Move],
) -> io::Result<()> {
    let count = allowed_count(allowed);
    let probe_profile = OpponentProfile {
        seed: 0,
        book: allowed.clone(),
        eval_mask: EvalMask::EMPTY,
        noise: NoiseProfile::default(),
    };
    let probe_pos = Position::startpos();
    let cursor = BookCursor::new(&probe_profile, &probe_pos);
    let in_book = cursor
        .as_ref()
        .and_then(|c| c.peek(history_moves))
        .is_some();
    writeln!(
        out,
        "openings: {count} allowed in book; {} for the next-game profile.",
        if in_book { "in book" } else { "out of book" }
    )?;
    writeln!(
        out,
        "  try: openings list | allow PAT | deny PAT | reset | selected"
    )
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
            None => writeln!(out, "unknown category {subarg:?}; try one of: {}", slug_list()),
        },
        "enable" => match EvalCategory::from_slug(subarg) {
            Some(cat) => {
                mask.enable(cat);
                writeln!(out, "eval-mask: bot now considers {} again.", cat.slug())
            }
            None => writeln!(out, "unknown category {subarg:?}; try one of: {}", slug_list()),
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

fn format_noise_summary(n: &NoiseProfile) -> String {
    if n.is_off() {
        return "off (bot always plays #1)".to_string();
    }
    let max_label = if n.blunder_max_loss_cp >= i32::MAX / 2 {
        "∞".to_string()
    } else {
        format!("{}cp", n.blunder_max_loss_cp)
    };
    format!(
        "pool={} temp={} cp · blunder={:.0}% (loss band {}cp–{}) · wild={:.0}% · guaranteed mate-in {}",
        n.candidate_pool,
        n.temperature_cp,
        n.blunder_chance * 100.0,
        n.blunder_min_loss_cp,
        max_label,
        n.wild_chance * 100.0,
        n.guaranteed_mate_in,
    )
}

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
        "min-loss" | "min_loss" => match subarg.parse::<i32>() {
            Ok(cp) if cp >= 0 && cp <= noise.blunder_max_loss_cp => {
                noise.blunder_min_loss_cp = cp;
                writeln!(out, "noise: blunder min-loss set to {cp} cp.")
            }
            _ => writeln!(
                out,
                "usage: noise min-loss <0..= current max-loss ({} cp)>",
                noise.blunder_max_loss_cp,
            ),
        },
        "max-loss" | "max_loss" => match subarg.parse::<i32>() {
            Ok(cp) if cp >= noise.blunder_min_loss_cp => {
                noise.blunder_max_loss_cp = cp;
                writeln!(out, "noise: blunder max-loss set to {cp} cp.")
            }
            _ => writeln!(
                out,
                "usage: noise max-loss <≥ current min-loss ({} cp)>",
                noise.blunder_min_loss_cp,
            ),
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
            "unknown noise subcommand {other:?} — try: show | pool N | temp CP | blunder F | min-loss CP | max-loss CP | wild F | guarantee N | reset",
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

fn print_selected(out: &mut io::StdoutLock<'_>, pos: &Position) -> io::Result<()> {
    match chess_tutor_engine::openings::identify(pos) {
        Some(id) => writeln!(out, "openings: current position is {} {}", id.eco, id.name),
        None => writeln!(out, "openings: current position is not in the openings database."),
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
    match san::parse(pos, input) {
        Ok(mv) => Ok(mv),
        Err(san_err) => match uci::parse(pos, input) {
            Ok(mv) => Ok(mv),
            Err(uci_err) => Err(format!("not SAN ({san_err}); not UCI ({uci_err})")),
        },
    }
}

fn announce_trap_threats(
    out: &mut io::StdoutLock<'_>,
    threats: &[TrapThreatened],
) -> io::Result<()> {
    for t in threats {
        let pv = t.hit.main_line_san.join(" ");
        writeln!(
            out,
            "warning: {} walks into {} — refutation {} ({:+} cp)",
            t.candidate_san, t.hit.name, pv, t.hit.main_line_gain_cp,
        )?;
    }
    Ok(())
}

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

fn format_score(v: Value) -> String {
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
    writeln!(out, "  eval     per-term evaluation trace for the current position")?;
    writeln!(out, "  search [N]   run the engine; print top N PVs with deltas (default N=1)")?;
    writeln!(out, "  analyze [N] [P]   teaching breakdown: top N PVs with per-term deltas,")?;
    writeln!(out, "                    cumulative coverage P% (default N=3, P=75)")?;
    writeln!(out, "  retrospect [on|off]   toggle automatic post-move verdict (default on)")?;
    writeln!(out, "  explain-best [on|off] narrate why Best moves were best")?;
    writeln!(out, "  openings [list | allow PAT | deny PAT | reset | selected]")?;
    writeln!(out, "                    inspect or edit the opening book (effective next game)")?;
    writeln!(out, "  eval-mask [list | disable CAT | enable CAT | reset]")?;
    writeln!(out, "                    toggle bot's blindness to eval categories")?;
    writeln!(out, "  noise [show | pool N | temp CP | blunder F | wild F | guarantee N | reset]")?;
    writeln!(out, "                    bot move-sampling knobs")?;
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
    fn parse_search_default_to_one() {
        assert_eq!(parse_search_command(""), Ok(1));
        assert_eq!(parse_search_command("   "), Ok(1));
    }

    #[test]
    fn parse_search_accepts_n() {
        assert_eq!(parse_search_command("3"), Ok(3));
        assert_eq!(parse_search_command("  5 "), Ok(5));
    }

    #[test]
    fn parse_search_rejects_zero() {
        assert!(parse_search_command("0").is_err());
    }

    #[test]
    fn parse_analyze_default() {
        assert_eq!(
            parse_analyze_command(""),
            Ok(AnalyzeArgs {
                multi_pv: 3,
                top_percent: 75.0,
            })
        );
    }

    #[test]
    fn parse_analyze_n_p() {
        assert_eq!(
            parse_analyze_command("4 80"),
            Ok(AnalyzeArgs {
                multi_pv: 4,
                top_percent: 80.0,
            })
        );
    }

    #[test]
    fn parse_analyze_rejects_too_many() {
        assert!(parse_analyze_command("1 2 3").is_err());
    }
}
