//! REPL subcommand handlers (openings / eval-mask / noise).
use std::io::{self, Write};

use chess_tutor_engine::book::BookCursor;
use chess_tutor_engine::openings::{self, OpeningId};
use chess_tutor_engine::opponent::{
    BookSelection, EvalCategory, EvalMask, NoiseProfile, OpponentProfile,
};
use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::Move;

use super::*;

pub(super) fn run_openings_command(
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
        ..OpponentProfile::default()
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
        return writeln!(
            out,
            "openings: allowed set is empty (engine plays from search)."
        );
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

pub(super) fn run_eval_mask_command(
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
                slug_list()
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
                slug_list()
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

pub(super) fn format_noise_summary(n: &NoiseProfile) -> String {
    if n.is_off() {
        return "off (bot always plays #1)".to_string();
    }
    format!(
        "avg-rank={:.1} · blunder={:.0}% (hangs {:.1}–{:.1} pts) · miss={:.0}% · guaranteed mate-in {}",
        n.avg_move_rank,
        n.blunder_chance * 100.0,
        n.blunder_min_material_cp as f32 / 100.0,
        n.blunder_max_material_cp as f32 / 100.0,
        n.miss_chance * 100.0,
        n.guaranteed_mate_in,
    )
}

pub(super) fn run_noise_command(
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
        "rank" => match subarg.parse::<f32>() {
            Ok(r) if r >= 1.0 => {
                noise.avg_move_rank = r;
                writeln!(
                    out,
                    "noise: average move rank set to {r:.1} (1.0 = always best; higher = weaker)."
                )
            }
            _ => writeln!(out, "usage: noise rank <>= 1.0>"),
        },
        "blunder" => match subarg.parse::<f32>() {
            Ok(p) if (0.0..=1.0).contains(&p) => {
                noise.blunder_chance = p;
                writeln!(out, "noise: blunder chance set to {:.0}%.", p * 100.0)
            }
            _ => writeln!(out, "usage: noise blunder <0.0-1.0>"),
        },
        "miss" => match subarg.parse::<f32>() {
            Ok(p) if (0.0..=1.0).contains(&p) => {
                noise.miss_chance = p;
                writeln!(
                    out,
                    "noise: miss chance set to {:.0}% (decline a material-winning move when one exists).",
                    p * 100.0,
                )
            }
            _ => writeln!(out, "usage: noise miss <0.0-1.0>"),
        },
        // Material band is in points (a pawn = 1.0); stored as
        // material-cp internally (pawn = 100).
        "min-material" | "min_material" => match subarg.parse::<f32>() {
            Ok(pts) if pts >= 0.0 && (pts * 100.0) as i32 <= noise.blunder_max_material_cp => {
                noise.blunder_min_material_cp = (pts * 100.0) as i32;
                writeln!(out, "noise: blunder min material set to {pts:.1} pts.")
            }
            _ => writeln!(
                out,
                "usage: noise min-material <0..= current max ({:.1} pts)>",
                noise.blunder_max_material_cp as f32 / 100.0,
            ),
        },
        "max-material" | "max_material" => match subarg.parse::<f32>() {
            Ok(pts) if (pts * 100.0) as i32 >= noise.blunder_min_material_cp => {
                noise.blunder_max_material_cp = (pts * 100.0) as i32;
                writeln!(out, "noise: blunder max material set to {pts:.1} pts.")
            }
            _ => writeln!(
                out,
                "usage: noise max-material <≥ current min ({:.1} pts)>",
                noise.blunder_min_material_cp as f32 / 100.0,
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
            "unknown noise subcommand {other:?} — try: show | rank R | blunder F | miss F | min-material PTS | max-material PTS | guarantee N | reset",
        ),
    }
}
