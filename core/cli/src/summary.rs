//! Position-summary header rendered before every FEN-taking command.
//!
//! See PLAN-cli.md §"The position-summary header" for design and
//! example. The whole point: an agent reading any subcommand's output
//! must never be confused about which side is to move, what the
//! score is, whether it's check, or what opening is being played.
//! Front-loading the same self-describing block on every command makes
//! those non-issues regardless of which command the agent ran.
//!
//! ## Score source
//!
//! For commands that don't run a search (`board`, `moves`, `eval`,
//! everything in Phases B–E except `explain`), the header carries the
//! **static eval** under a `[static]` tag. For `search` / `explain` the
//! tag becomes `[search d=N]` so the agent doesn't confuse a deep
//! search figure with a one-ply static one.

use chess_tutor_engine::analysis::{find_latent_threats, LatentThreat, TriggerShape};
use chess_tutor_engine::eval::evaluate_with_trace;
use chess_tutor_engine::movegen::legal_moves_vec;
use chess_tutor_engine::openings;
use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::{Color, PieceType, Value};
use serde::Serialize;

use crate::piece_fmt::piece_label;
use crate::units::{headline_triple, to_white_pov};

/// Source of the score reported in the summary header. Distinguishes a
/// one-ply static eval from a deep search figure so the agent doesn't
/// over-trust the former.
#[derive(Clone, Copy, Debug)]
pub enum ScoreSource {
    /// One-ply static eval (`[static]` tag).
    Static,
    /// Deep search to the given depth (`[search d=N]` tag).
    Search { depth: u32 },
}

/// The header data as a serializable struct so JSON consumers see the
/// same fields the text rendering shows. Mirrors PLAN-cli.md's worked
/// example shape.
#[derive(Debug, Clone, Serialize)]
pub struct PositionSummary {
    pub fen: String,
    pub to_move: &'static str,
    pub in_check: bool,
    pub material: MaterialBlock,
    pub score: ScoreBlock,
    pub opening: Option<OpeningBlock>,
    pub legal_move_count: usize,
    pub terminal: Option<&'static str>,
    /// Standing (latent) threats the opponent has pre-loaded against the
    /// side to move. This is the header's load-bearing safety net: a
    /// positive→negative eval swing means "you let the opponent do
    /// something devastating", and these are the static fingerprints of
    /// exactly that — a discovered attack / pin / skewer / loose
    /// defender the opponent can cash in if you play a move that doesn't
    /// address it. Check this BEFORE concluding "I have a strong move":
    /// the geometry that looks like *your* opportunity is often *their*
    /// threat aimed the other way down the same line. Empty when none.
    pub danger: Vec<DangerLine>,
}

/// One standing-threat line for the summary header's `danger:` block,
/// resolved to printable labels so both the text and JSON surfaces can
/// use it without touching raw engine square indices.
#[derive(Debug, Clone, Serialize)]
pub struct DangerLine {
    /// Pattern name (`DiscoveredAttack` / `Pin` / `Skewer` /
    /// `RemovingDefender`).
    pub pattern: String,
    /// The piece the threat bears on, e.g. `"Re1"` — *your* piece, the
    /// one about to be won.
    pub target: String,
    /// Classical-points estimate of what the opponent wins (P=1 … Q=9).
    pub min_gain: i32,
    /// Plain-English description of the opponent move that fires it.
    pub trigger: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MaterialBlock {
    pub balance: String,                  // "even" / "white +N" / "black +N"
    pub white_summary: String,            // "Q+2R+B+7P"
    pub black_summary: String,
    pub white_points: u32,
    pub black_points: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScoreBlock {
    /// Score in pawns from white's POV (`+0.85` / `-1.20` / `#3`).
    /// Matches chess.com / lichess / UCI display convention.
    pub pawns_white_pov: String,
    /// Same number as [`Self::pawns_white_pov`] expressed in
    /// conventional centipawns (`+85` / `-120`). Pawn = 100 cp here.
    /// Useful for tooling that wants integer precision without the
    /// pawns decimal.
    pub conv_cp_white_pov: String,
    /// White's win-probability estimate from the lila sigmoid (0..100).
    pub win_pct_white: u8,
    /// Static eval vs. search result.
    pub source: ScoreSourceJson,
    /// **Raw engine-internal cp**, side-to-move-signed, including
    /// tempo. This is the number `chess_tutor_engine::types::Value`
    /// carries throughout the engine (PawnEG = 213 scale). Use this
    /// when comparing against profiling output or search-code
    /// thresholds — *not* when comparing against chess.com.
    pub engine_cp_stm: i32,
    /// Raw engine-internal cp re-signed to white's POV.
    pub engine_cp_white_pov: i32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ScoreSourceJson {
    Static,
    Search { depth: u32 },
}

#[derive(Debug, Clone, Serialize)]
pub struct OpeningBlock {
    pub eco: String,
    pub name: String,
}

/// Build the summary block from a position. If `external_score` is
/// `Some`, that score replaces the static eval — used by `search` /
/// `explain` so the header reflects the deep result they computed.
pub fn build(pos: &Position, source: ScoreSource, external_score: Option<Value>) -> PositionSummary {
    let stm = pos.side_to_move();

    let mut scratch = pos.clone();
    let legal = legal_moves_vec(&mut scratch);
    let legal_move_count = legal.len();
    let in_check = pos.in_check();

    let terminal = match (legal_move_count, in_check) {
        (0, true) => Some("checkmate (side to move is mated)"),
        (0, false) => Some("stalemate"),
        _ => None,
    };

    // Standing latent threats aimed at the side to move. Surfaced in the
    // header of every command so an agent can't reason about the
    // position without first seeing what the opponent has loaded against
    // it. `find_latent_threats(pos, side)` returns the threats whose
    // *target* belongs to `side`, i.e. the threats `side` must defuse.
    // Skipped at terminal nodes (no move to make them matter).
    let danger: Vec<DangerLine> = if terminal.is_some() {
        Vec::new()
    } else {
        find_latent_threats(pos, stm)
            .into_iter()
            .map(|t| build_danger_line(pos, &t))
            .collect()
    };

    // Score: either the explicit override (search) or the static eval.
    let stm_value = match external_score {
        Some(v) => v,
        None => {
            let (v, _trace) = evaluate_with_trace(pos);
            v
        }
    };
    let white_pov_value = to_white_pov(stm_value, stm);
    let (pawns, cp, win_pct) = headline_triple(white_pov_value);

    let source_tag: ScoreSourceJson = match source {
        ScoreSource::Static => ScoreSourceJson::Static,
        ScoreSource::Search { depth } => ScoreSourceJson::Search { depth },
    };

    let score = ScoreBlock {
        pawns_white_pov: pawns,
        conv_cp_white_pov: cp,
        win_pct_white: win_pct,
        source: source_tag,
        engine_cp_stm: stm_value.0,
        engine_cp_white_pov: white_pov_value.0,
    };

    let material = build_material_block(pos);

    let opening = openings::identify(pos).map(|o| OpeningBlock {
        eco: o.eco,
        name: o.name,
    });

    PositionSummary {
        fen: pos.to_fen(),
        to_move: color_name(stm),
        in_check,
        material,
        score,
        opening,
        legal_move_count,
        terminal,
        danger,
    }
}

/// Resolve a [`LatentThreat`] (engine squares) into a printable
/// [`DangerLine`]. The trigger text names the concrete opponent move
/// that fires the threat, mirroring the phrasing the `tactics --latent`
/// surface uses so the two never read inconsistently.
fn build_danger_line(pos: &Position, t: &LatentThreat) -> DangerLine {
    let target = pos
        .piece_on(t.target)
        .map(|p| piece_label(p, t.target))
        .unwrap_or_else(|| t.target.to_algebraic());
    let vehicle = t
        .vehicle
        .and_then(|v| pos.piece_on(v).map(|p| piece_label(p, v)));
    let trigger = match t.trigger_shape {
        TriggerShape::VehicleMoves => format!(
            "any move by {} unmasks the attack",
            vehicle.unwrap_or_else(|| "the blocking piece".to_string()),
        ),
        TriggerShape::VehicleConstrained => format!(
            "{} can't move without exposing it",
            vehicle.unwrap_or_else(|| "the blocking piece".to_string()),
        ),
        TriggerShape::DefenderRemoved { defender } => {
            let d = pos
                .piece_on(defender)
                .map(|p| piece_label(p, defender))
                .unwrap_or_else(|| defender.to_algebraic());
            format!("capturing the defender {d} leaves it unguarded")
        }
    };
    DangerLine {
        pattern: format!("{:?}", t.pattern),
        target,
        min_gain: t.min_gain,
        trigger,
    }
}

/// Multi-line text rendering of the summary. Stable line shape so the
/// agent (and any line-oriented log parsing) can scan it deterministically.
pub fn render_text(summary: &PositionSummary) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    writeln!(out, "position: {}", summary.fen).unwrap();
    writeln!(out, "to move:  {}", summary.to_move).unwrap();
    writeln!(
        out,
        "in check: {}",
        if summary.in_check { "yes" } else { "no" }
    )
    .unwrap();
    writeln!(
        out,
        "material: {}   (W: {} = {}  vs  B: {} = {})",
        summary.material.balance,
        summary.material.white_summary,
        summary.material.white_points,
        summary.material.black_summary,
        summary.material.black_points,
    )
    .unwrap();

    let source_label = match &summary.score.source {
        ScoreSourceJson::Static => "static".to_string(),
        ScoreSourceJson::Search { depth } => format!("search d={depth}"),
    };
    // Headline shows pawns (chess.com-comparable) and engine-cp (what
    // shows up in the engine source and profiler output). Both are
    // explicitly labelled so neither can be misread against the other.
    // Conventional-cp is suppressed in the text headline because it's
    // just `pawns × 100` — see PLAN-cli.md / units.rs gloss.
    writeln!(
        out,
        "score:    {} pawns white-POV  (engine-cp: {} stm; ~{}% win)  [{}]",
        summary.score.pawns_white_pov,
        sign_for_engine_cp_label(summary.score.engine_cp_stm),
        summary.score.win_pct_white,
        source_label,
    )
    .unwrap();

    // Danger block sits directly under the score because the two are
    // causally linked: if the agent plays a move that ignores these
    // standing threats, the score it just read is the score it is about
    // to lose. Silent when there are none, so a clean header still reads
    // clean.
    if !summary.danger.is_empty() {
        writeln!(
            out,
            "danger:   !! {} standing threat(s) against {} (side to move) — a move that ignores these will likely swing the eval against you:",
            summary.danger.len(),
            summary.to_move,
        )
        .unwrap();
        for d in &summary.danger {
            writeln!(
                out,
                "            - opponent's {} on your {} (~{} pts) — fires when {}",
                d.pattern, d.target, d.min_gain, d.trigger,
            )
            .unwrap();
        }
        writeln!(
            out,
            "          (static scan; confirm with `chess-tutor tactics --latent` or `chess-tutor explain`)",
        )
        .unwrap();
    }

    match &summary.opening {
        Some(op) => writeln!(out, "opening:  {}  {}", op.eco, op.name).unwrap(),
        None => writeln!(out, "opening:  (none matched)").unwrap(),
    }
    writeln!(out, "legal:    {} moves", summary.legal_move_count).unwrap();
    if let Some(t) = summary.terminal {
        writeln!(out, "terminal: {}", t).unwrap();
    }
    out
}

fn color_name(c: Color) -> &'static str {
    match c {
        Color::White => "White",
        Color::Black => "Black",
    }
}

/// Render an engine-cp integer with the standard `+N` / `-N` /
/// `#N` notation so the headline reads `engine-cp: +372 stm` /
/// `engine-cp: #3 stm`. Mate scores get the `#N` form for consistency
/// with the pawns column on the same line.
fn sign_for_engine_cp_label(v: i32) -> String {
    use crate::units::format_engine_cp;
    format_engine_cp(chess_tutor_engine::types::Value(v))
}

fn build_material_block(pos: &Position) -> MaterialBlock {
    let white_counts = piece_counts(pos, Color::White);
    let black_counts = piece_counts(pos, Color::Black);
    let white_points = points_total(&white_counts);
    let black_points = points_total(&black_counts);

    let balance = match white_points as i32 - black_points as i32 {
        0 => "even".to_string(),
        d if d > 0 => format!("white +{}", d),
        d => format!("black +{}", -d),
    };

    MaterialBlock {
        balance,
        white_summary: format_piece_summary(&white_counts),
        black_summary: format_piece_summary(&black_counts),
        white_points,
        black_points,
    }
}

/// `[Q, R, B, N, P]` counts for one colour. Kings are excluded — they
/// never differ between sides and add noise to the summary.
fn piece_counts(pos: &Position, c: Color) -> [u32; 5] {
    [
        pos.count(c, PieceType::Queen),
        pos.count(c, PieceType::Rook),
        pos.count(c, PieceType::Bishop),
        pos.count(c, PieceType::Knight),
        pos.count(c, PieceType::Pawn),
    ]
}

fn points_total(counts: &[u32; 5]) -> u32 {
    // Conventional 9/5/3/3/1 scoring — chess.com bar values, not our
    // engine's PAWN_EG=213 ones. The header is for human-style
    // material comparison, not for engine-relative eval.
    counts[0] * 9 + counts[1] * 5 + counts[2] * 3 + counts[3] * 3 + counts[4]
}

fn format_piece_summary(counts: &[u32; 5]) -> String {
    let letters = ["Q", "R", "B", "N", "P"];
    let mut parts: Vec<String> = Vec::new();
    for (i, &n) in counts.iter().enumerate() {
        if n == 0 {
            continue;
        }
        if n == 1 {
            parts.push(letters[i].to_string());
        } else {
            parts.push(format!("{}{}", n, letters[i]));
        }
    }
    if parts.is_empty() {
        // Lone king vs king.
        "K only".to_string()
    } else {
        parts.join("+")
    }
}

#[cfg(test)]
#[path = "summary_tests.rs"]
mod tests;
