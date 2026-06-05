//! REPL output / formatting helpers, split out of play.rs.
use std::io::{self, Write};


use chess_tutor_engine::openings::{self, OpeningIdentification};
use chess_tutor_engine::opponent::{
    EvalCategory, EvalMask,
};
use chess_tutor_engine::position::Position;
use chess_tutor_engine::san;
use chess_tutor_engine::traps::{TrapEvent, TrapHit, TrapThreatened};
use chess_tutor_engine::types::{Color, Move, Value};
use chess_tutor_teaching::phrasing::Perspective;
use chess_tutor_teaching::{format_retrospective, NarrationOptions};
use chess_tutor_ui::session::{HistoryEntry, RetrospectiveResult};
use chess_tutor_ui::view::BoardView;
use chess_tutor_ui::NoisePickInfo;

use crate::analysis_report;
use crate::board::{render as render_board, RenderOptions};

use super::*;

pub(super) fn print_retrospective(
    out: &mut io::StdoutLock<'_>,
    pre_move_pos: &Position,
    retro: &RetrospectiveResult,
    explain_best: bool,
) -> io::Result<()> {
    let opts = NarrationOptions { explain_best };
    // The CLI prints retrospectives for the user's own moves; the engine's
    // moves print via `print_engine_move`. Player perspective here.
    let text = format_retrospective(
        pre_move_pos,
        &retro.analyses,
        retro.user_move,
        &opts,
        Perspective::Player,
    );
    out.write_all(text.as_bytes())?;
    writeln!(
        out,
        "[retrospective] {} ms · {} nodes · {:.2} Mnps",
        retro.elapsed.as_millis(),
        retro.nodes,
        retro.nps_m,
    )
}

pub(super) fn print_engine_move(out: &mut io::StdoutLock<'_>, entry: &HistoryEntry) -> io::Result<()> {
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
        NoisePickInfo::Variety {
            pick_idx,
            num_lines,
        } => format!("[noise: variety #{} of {}]", pick_idx + 1, num_lines),
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
        NoisePickInfo::Miss {
            pick_idx,
            num_lines,
            engine_top,
        } => format!(
            "[noise: miss — declined material-winning {:?}, played #{} of {}]",
            engine_top,
            pick_idx + 1,
            num_lines,
        ),
    }
}

fn format_score_white_pov(white_pov: Value, mover: Color) -> String {
    let from_mover = if mover == Color::White { white_pov } else { -white_pov };
    format_score(from_mover)
}

pub(super) fn render_current(
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

pub(super) fn announce_opening_if_changed(
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

pub(super) fn print_search_report(
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
        let pv_san = san::pv_to_san(pos, &analysis.pv);
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
        let pv_san = san::pv_to_san(pos, &analysis.pv);
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

pub(super) fn print_analyze_report(
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

pub(super) fn print_eval_mask(out: &mut io::StdoutLock<'_>, mask: &EvalMask) -> io::Result<()> {
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

pub(super) fn print_selected(out: &mut io::StdoutLock<'_>, pos: &Position) -> io::Result<()> {
    match chess_tutor_engine::openings::identify(pos) {
        Some(id) => writeln!(out, "openings: current position is {} {}", id.eco, id.name),
        None => writeln!(out, "openings: current position is not in the openings database."),
    }
}

pub(super) fn announce_trap_threats(
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

pub(super) fn announce_trap_hit(out: &mut io::StdoutLock<'_>, hit: &TrapHit) -> io::Result<()> {
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

pub(super) fn announce_trap_event(out: &mut io::StdoutLock<'_>, event: &TrapEvent) -> io::Result<()> {
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

pub(super) fn format_score(v: Value) -> String {
    // Delegate to the shared units helper so the play REPL speaks the
    // same conventional-pawn scale (pawn = PAWN_EG = 213 engine-cp) as
    // every other CLI surface. Dividing by 100 here used to inflate
    // every score ~2.13×.
    crate::units::format_pawns(v)
}

pub(super) fn print_help(out: &mut io::StdoutLock<'_>) -> io::Result<()> {
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
    writeln!(out, "  noise [show | pool N | temp CP | blunder F | guarantee N | reset]")?;
    writeln!(out, "                    bot move-sampling knobs")?;
    writeln!(out, "  undo     take back one ply")?;
    writeln!(out, "  fen      print the current FEN")?;
    writeln!(out, "  flip     flip the board")?;
    writeln!(out, "  resign   resign the game")?;
    writeln!(out, "  help     this message")?;
    writeln!(out, "  quit     exit")?;
    Ok(())
}

