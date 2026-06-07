//! `chess-tutor settled-audit` — TEMPORARY instrumentation for the
//! settled-ply redesign ([`PLAN-perception.md`] step 1; remove with it).
//!
//! Hypothesis under test: `compute_settled_ply`'s backward walk over
//! 25-cp eval deltas drags `settled_ply` to the PV leaf on deep
//! searches (horizon noise always exceeds the threshold near the
//! tail), so the material classifier under the noise miss/blunder
//! branches counts material through the *whole PV* — speculative
//! deep-line trades included — instead of through the tactic's
//! resolution.
//!
//! For every position in the corpus, run a MultiPV search (width
//! defaults to 10, mirroring the play noise path) and for each
//! returned line record:
//!
//! - **where `settled_ply` lands** (distance from the leaf; the
//!   leaf-drag distribution the hypothesis predicts is ~90 % at 0);
//! - **the material delta under today's settled cap** vs **under a
//!   prototype of the proposed `material_settled` semantics** (a
//!   forward event-walk that settles at the first run of
//!   [`QUIET_RUN_LEN`] consecutive non-forcing plies) vs **the full
//!   PV** — and how often the ±1-pawn win/neutral/loss class the
//!   noise branches key on would *flip* under the new semantics;
//! - **the tactic-detector oracle**: when [`find_tactic_in_line`]
//!   names a tactic in the PV, the gap between each settled notion
//!   and the hit's `pv_ply` (the new semantics should land near the
//!   payoff; the old one near the leaf).
//!
//! `--depth` is repeatable (`--depth 8 --depth 12 --depth 16`) so one
//! run shows how leaf-drag scales with search depth.

use anyhow::{anyhow, Context, Result};

use chess_tutor_engine::analysis::{analyze_position, find_tactic_in_line, MoveAnalysis};
use chess_tutor_engine::engine::{Engine, SearchParams};
use chess_tutor_engine::movegen::legal_moves_vec;
use chess_tutor_engine::noise::WIN_MATERIAL_CP;
use chess_tutor_engine::position::Position;
use chess_tutor_engine::search::MATERIAL_QUIET_RUN;
use chess_tutor_engine::san::pv_to_san;
use chess_tutor_engine::types::{Color, Move, MoveKind, PieceType, Value};

use crate::bench_fens::{parse_bench_entry, DEFAULT_BENCH_FENS};

pub struct SettledAuditArgs {
    pub tt_mb: usize,
    /// One aggregate block per depth, in the order given.
    pub depths: Vec<u32>,
    pub multi_pv: usize,
    /// `default` for the 45-position SF11 bench list, or a path to a
    /// FEN file (same format as `chess-tutor bench`).
    pub fen_file: String,
    /// Max classification-flip examples printed in full per depth.
    pub examples: usize,
}

/// The prototype's quiet-run length — now the ENGINE's own constant
/// (the redesign landed; this tool doubles as its regression check:
/// post-redesign, "current" and "prototype" must agree).
const QUIET_RUN_LEN: usize = MATERIAL_QUIET_RUN;

pub fn run(args: SettledAuditArgs) -> Result<()> {
    if args.multi_pv < 1 {
        return Err(anyhow!("settled-audit: --multi-pv must be >= 1"));
    }
    if args.depths.is_empty() {
        return Err(anyhow!("settled-audit: at least one --depth required"));
    }

    let positions = load_positions(&args.fen_file)?;
    if positions.is_empty() {
        return Err(anyhow!("settled-audit: no positions to search"));
    }

    println!(
        "settled-audit: {} positions, TT = {} MB, multi_pv = {}, depths = {:?}",
        positions.len(),
        args.tt_mb,
        args.multi_pv,
        args.depths,
    );

    for &depth in &args.depths {
        println!();
        println!("== depth {depth} ==");
        audit_depth(&args, &positions, depth)?;
    }
    Ok(())
}

/// Everything recorded about one MultiPV line.
struct LineStat {
    pv_len: usize,
    /// `settled_ply` as produced by the search (`None` = no traces).
    settled: Option<usize>,
    /// Material delta (material-cp, root-stm POV) under today's
    /// settled cap — exactly what `noise::line_material_delta_cp`
    /// feeds the miss/blunder branches.
    cur_delta: i32,
    /// Ply of the last forcing event before the prototype's first
    /// quiet run (0 when the line opens quiet).
    proto_settled: usize,
    /// Material delta under the prototype first-resolution walk.
    proto_delta: i32,
    /// Material delta over the whole PV (what the current cap
    /// degenerates to whenever settled lands at the leaf).
    full_delta: i32,
    /// `TacticHit.pv_ply` when the detector chain names a tactic in
    /// this PV.
    tactic_ply: Option<usize>,
    /// Static eval read at the current settled cap's trace (root-stm
    /// POV, engine-cp) — the read-point today's eval-swing consumer
    /// (`initiative_outcome::compute_eval_swing`) lands on. `None`
    /// when the line has no traces.
    eval_cur: Option<i32>,
    /// Static eval read at the prototype settled ply's trace.
    eval_proto: Option<i32>,
    /// Search score (root-stm POV, engine-cp) for the line.
    score: i32,
    /// Whether the PV carries any material event at all (separates
    /// "two read points on a flat line" from a real divergence).
    has_events: bool,
}

/// A line whose ±1-pawn material class differs between the current
/// cap and the prototype — the preview of how the miss/blunder pools
/// would change.
struct FlipExample {
    pos_idx: usize,
    fen: String,
    slot: usize,
    pv_san: String,
    cur: (MatClass, i32, usize),
    proto: (MatClass, i32, usize),
}

fn audit_depth(args: &SettledAuditArgs, positions: &[String], depth: u32) -> Result<()> {
    let mut engine = Engine::new(args.tt_mb);
    let mut stats: Vec<LineStat> = Vec::new();
    let mut flips: Vec<FlipExample> = Vec::new();

    for (i, entry) in positions.iter().enumerate() {
        let mut pos =
            parse_bench_entry(entry).with_context(|| format!("parsing bench entry {}", i + 1))?;
        if legal_moves_vec(&mut pos).is_empty() {
            println!("  {:>2}/{}  (terminal — skipped)", i + 1, positions.len());
            continue;
        }

        engine.new_game();
        let params = SearchParams {
            max_depth: depth,
            multi_pv: args.multi_pv,
            ..SearchParams::default()
        };
        let analyses = analyze_position(&mut engine, &mut pos.clone(), params);

        let mut leaf_count = 0usize;
        let mut flip_count = 0usize;
        for (slot, a) in analyses.iter().enumerate() {
            let Some(stat) = line_stat(&pos, a) else {
                continue; // empty PV (terminal slot) — nothing to audit
            };
            if stat.settled == Some(stat.pv_len.saturating_sub(1)) {
                leaf_count += 1;
            }
            let cur_class = classify(stat.cur_delta);
            let proto_class = classify(stat.proto_delta);
            if cur_class != proto_class {
                flip_count += 1;
                flips.push(FlipExample {
                    pos_idx: i + 1,
                    fen: pos.to_fen(),
                    slot,
                    pv_san: pv_to_san(&pos, &a.pv)
                        .iter()
                        .take(12)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(" "),
                    cur: (cur_class, stat.cur_delta, current_cap(a)),
                    proto: (proto_class, stat.proto_delta, stat.proto_settled),
                });
            }
            stats.push(stat);
        }

        println!(
            "  {:>2}/{}  lines {:>2} | settled-at-leaf {:>2} | class flips {:>2}",
            i + 1,
            positions.len(),
            analyses.len(),
            leaf_count,
            flip_count,
        );
    }

    print_aggregate(&stats);
    print_flips(&flips, args.examples);
    Ok(())
}

/// Compute every recorded metric for one line. `None` when the PV is
/// empty (a terminal MultiPV slot).
fn line_stat(pre_pos: &Position, a: &MoveAnalysis) -> Option<LineStat> {
    if a.pv.is_empty() {
        return None;
    }
    let facts = walk_pv(pre_pos, &a.pv);
    let cur_cap = current_cap(a);
    let cur_delta: i32 = facts[..=cur_cap.min(facts.len() - 1)]
        .iter()
        .map(|f| f.event_cp)
        .sum();
    let full_delta: i32 = facts.iter().map(|f| f.event_cp).sum();
    let (proto_settled, proto_delta) = prototype_first_resolution(&facts);
    let tactic_ply = find_tactic_in_line(pre_pos, &a.pv, pre_pos.side_to_move(), None)
        .map(|hit| hit.pv_ply);
    let root_stm = pre_pos.side_to_move();
    Some(LineStat {
        pv_len: a.pv.len(),
        settled: a.settled_ply,
        cur_delta,
        proto_settled,
        proto_delta,
        full_delta,
        tactic_ply,
        eval_cur: trace_user_pov(a, cur_cap, root_stm),
        eval_proto: trace_user_pov(a, proto_settled, root_stm),
        score: a.score.0,
        has_events: facts.iter().any(|f| f.event_cp != 0),
    })
}

/// Static eval of `ply_traces[ply]` from `root_stm`'s POV, in
/// engine-cp — the same normalization
/// `initiative_outcome::ply_trace_user_pov` applies: side-to-move at
/// trace `i` alternates (`i % 2 == 0` → opponent to move). Clamps the
/// index into the trace range; `None` when the line has no traces.
fn trace_user_pov(a: &MoveAnalysis, ply: usize, root_stm: Color) -> Option<i32> {
    let idx = ply.min(a.ply_traces.len().checked_sub(1)?);
    let stm_at_eval = if idx % 2 == 0 { !root_stm } else { root_stm };
    let white_pov = a.ply_traces[idx].white_pov_value(stm_at_eval).0;
    Some(match root_stm {
        Color::White => white_pov,
        Color::Black => -white_pov,
    })
}

/// True when `score` is in the mate band — excluded from
/// score-vs-eval comparisons (a mate score has no static-eval analog).
fn is_mate_score(score: i32) -> bool {
    score.abs() >= Value::MATE.0 - Value::MAX_PLY
}

/// The inclusive material-walk cap today's consumers derive from
/// `settled_ply` — mirrors `noise::line_material_delta_cp` and
/// `compute_material_outcome`: the settled index when in range, else
/// the PV end.
fn current_cap(a: &MoveAnalysis) -> usize {
    match a.settled_ply {
        Some(idx) if idx < a.pv.len() => idx,
        _ => a.pv.len().saturating_sub(1),
    }
}

/// Per-ply facts from one forward walk of the PV: is the ply forcing
/// (capture / promotion / check), and the signed material event it
/// carries (root-stm POV, material-cp; promotions count the upgrade).
struct PlyFacts {
    forcing: bool,
    event_cp: i32,
}

fn walk_pv(root: &Position, pv: &[Move]) -> Vec<PlyFacts> {
    let root_stm = root.side_to_move();
    let mut scratch = root.clone();
    let mut facts = Vec::with_capacity(pv.len());
    for &mv in pv {
        let mover = scratch
            .piece_on(mv.from())
            .map(|p| p.color())
            .unwrap_or(root_stm);
        let sign = if mover == root_stm { 1 } else { -1 };
        let captured: Option<PieceType> = match mv.kind() {
            MoveKind::Castling => None,
            MoveKind::EnPassant => Some(PieceType::Pawn),
            _ => scratch.piece_on(mv.to()).map(|p| p.kind()),
        };
        let mut event_cp = 0;
        if let Some(pt) = captured {
            event_cp += sign * standard_piece_value_cp(pt);
        }
        if mv.kind() == MoveKind::Promotion {
            event_cp += sign
                * (standard_piece_value_cp(mv.promoted_to())
                    - standard_piece_value_cp(PieceType::Pawn));
        }
        let forcing = captured.is_some()
            || mv.kind() == MoveKind::Promotion
            || scratch.gives_check(mv);
        facts.push(PlyFacts { forcing, event_cp });
        scratch.do_move(mv);
    }
    facts
}

/// Prototype of the proposed `material_settled` semantics
/// (PLAN-perception.md): walk forward, treat captures / promotions /
/// checks as "still resolving", stop at the first run of
/// [`QUIET_RUN_LEN`] consecutive non-forcing plies. Returns the ply
/// of the last forcing event before the stop (0 when the line opens
/// quiet — "settles immediately, banks nothing") and the net
/// material accumulated up to it.
fn prototype_first_resolution(facts: &[PlyFacts]) -> (usize, i32) {
    let mut net = 0;
    let mut last_event = 0usize;
    let mut quiet_run = 0usize;
    for (ply, f) in facts.iter().enumerate() {
        if f.forcing {
            net += f.event_cp;
            last_event = ply;
            quiet_run = 0;
        } else {
            quiet_run += 1;
            if quiet_run >= QUIET_RUN_LEN {
                break;
            }
        }
    }
    (last_event, net)
}

/// The ±1-pawn material class the noise miss/blunder branches key on
/// (`WIN_MATERIAL_CP` is `noise.rs`'s own threshold).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum MatClass {
    Win,
    Neutral,
    Loss,
}

fn classify(delta_cp: i32) -> MatClass {
    if delta_cp >= WIN_MATERIAL_CP {
        MatClass::Win
    } else if delta_cp <= -WIN_MATERIAL_CP {
        MatClass::Loss
    } else {
        MatClass::Neutral
    }
}

impl MatClass {
    fn label(self) -> &'static str {
        match self {
            MatClass::Win => "win",
            MatClass::Neutral => "neutral",
            MatClass::Loss => "loss",
        }
    }
}

/// Standard point value in material-cp (pawn = 100) — same chart
/// `noise.rs` uses (private there; duplicated for this temporary
/// tool).
fn standard_piece_value_cp(pt: PieceType) -> i32 {
    match pt {
        PieceType::Pawn => 100,
        PieceType::Knight | PieceType::Bishop => 300,
        PieceType::Rook => 500,
        PieceType::Queen => 900,
        PieceType::King => 0,
    }
}

fn print_aggregate(stats: &[LineStat]) {
    println!();
    if stats.is_empty() {
        println!("  (no lines to aggregate)");
        return;
    }
    let n = stats.len();

    // PV length distribution.
    let mut lens: Vec<usize> = stats.iter().map(|s| s.pv_len).collect();
    lens.sort_unstable();
    println!(
        "  lines: {}   pv length: p50 {}, p90 {}, max {}",
        n,
        pct(&lens, 0.50),
        pct(&lens, 0.90),
        lens.last().copied().unwrap_or(0),
    );

    // Where settled lands, as distance from the leaf.
    let mut none = 0usize;
    let mut at_zero = 0usize;
    let mut dist_hist = [0usize; 4]; // 0, 1, 2, 3-5; 6+ tracked separately
    let mut dist_far = 0usize;
    for s in stats {
        match s.settled {
            None => none += 1,
            Some(i) => {
                if i == 0 {
                    at_zero += 1;
                }
                let d = s.pv_len.saturating_sub(1).saturating_sub(i);
                match d {
                    0 => dist_hist[0] += 1,
                    1 => dist_hist[1] += 1,
                    2 => dist_hist[2] += 1,
                    3..=5 => dist_hist[3] += 1,
                    _ => dist_far += 1,
                }
            }
        }
    }
    let pc = |c: usize| 100.0 * c as f64 / n as f64;
    println!(
        "  settled distance from leaf: 0 (=leaf) {} ({:.1}%) | 1: {} | 2: {} | 3-5: {} | 6+: {} | none: {}",
        dist_hist[0],
        pc(dist_hist[0]),
        dist_hist[1],
        dist_hist[2],
        dist_hist[3],
        dist_far,
        none,
    );
    println!("  settled at ply 0: {} ({:.1}%)", at_zero, pc(at_zero));

    // Prototype landing spots.
    let mut proto: Vec<usize> = stats.iter().map(|s| s.proto_settled).collect();
    proto.sort_unstable();
    let proto_zero = stats.iter().filter(|s| s.proto_settled == 0).count();
    println!(
        "  prototype settled ply: p50 {}, p90 {}, max {} | at ply 0: {} ({:.1}%)",
        pct(&proto, 0.50),
        pct(&proto, 0.90),
        proto.last().copied().unwrap_or(0),
        proto_zero,
        pc(proto_zero),
    );

    // Material classification: current cap vs prototype vs full PV.
    let count = |f: &dyn Fn(&LineStat) -> i32, class: MatClass| {
        stats.iter().filter(|s| classify(f(s)) == class).count()
    };
    let cur: &dyn Fn(&LineStat) -> i32 = &|s| s.cur_delta;
    let pro: &dyn Fn(&LineStat) -> i32 = &|s| s.proto_delta;
    let ful: &dyn Fn(&LineStat) -> i32 = &|s| s.full_delta;
    println!("  material class (±1 pawn, root-stm POV):");
    println!(
        "    current cap:  win {:>4} | neutral {:>4} | loss {:>4}",
        count(&cur, MatClass::Win),
        count(&cur, MatClass::Neutral),
        count(&cur, MatClass::Loss),
    );
    println!(
        "    prototype:    win {:>4} | neutral {:>4} | loss {:>4}",
        count(&pro, MatClass::Win),
        count(&pro, MatClass::Neutral),
        count(&pro, MatClass::Loss),
    );
    println!(
        "    full PV:      win {:>4} | neutral {:>4} | loss {:>4}",
        count(&ful, MatClass::Win),
        count(&ful, MatClass::Neutral),
        count(&ful, MatClass::Loss),
    );
    let flipped = stats
        .iter()
        .filter(|s| classify(s.cur_delta) != classify(s.proto_delta))
        .count();
    println!(
        "    class flips current→prototype: {} / {} ({:.1}%)",
        flipped,
        n,
        pc(flipped),
    );
    // How often is the current cap indistinguishable from "whole PV"?
    let cur_eq_full = stats.iter().filter(|s| s.cur_delta == s.full_delta).count();
    println!(
        "    current delta == full-PV delta: {} ({:.1}%)  <- degeneration measure",
        cur_eq_full,
        pc(cur_eq_full),
    );

    // Eval read-point comparison: how different is the trace eval the
    // two settled notions land on (the read the eval-swing consumer
    // makes), overall and on lines where material actually moved.
    let eval_gap = |s: &LineStat| -> Option<usize> {
        Some(s.eval_cur?.abs_diff(s.eval_proto?) as usize)
    };
    let mut gaps_all: Vec<usize> = stats.iter().filter_map(eval_gap).collect();
    let mut gaps_events: Vec<usize> = stats
        .iter()
        .filter(|s| s.has_events)
        .filter_map(eval_gap)
        .collect();
    gaps_all.sort_unstable();
    gaps_events.sort_unstable();
    println!("  eval read-point gap |eval(current) − eval(prototype)| (engine-cp, PawnEG=213):");
    println!(
        "    all lines:           p50 {:>4}, p90 {:>4}, max {:>5}   (n = {})",
        pct(&gaps_all, 0.50),
        pct(&gaps_all, 0.90),
        gaps_all.last().copied().unwrap_or(0),
        gaps_all.len(),
    );
    println!(
        "    with material events: p50 {:>4}, p90 {:>4}, max {:>5}   (n = {})",
        pct(&gaps_events, 0.50),
        pct(&gaps_events, 0.90),
        gaps_events.last().copied().unwrap_or(0),
        gaps_events.len(),
    );

    // On tactic-labelled, non-mate lines: which read point better
    // approximates the search score? If the prototype were landing
    // mid-exchange its |score − eval| would blow up (half a hanging
    // queen); if the tactic has resolved by the prototype ply, the
    // static read should already be near the search score.
    let mut score_gap_cur: Vec<usize> = Vec::new();
    let mut score_gap_proto: Vec<usize> = Vec::new();
    for s in stats
        .iter()
        .filter(|s| s.tactic_ply.is_some() && !is_mate_score(s.score))
    {
        if let (Some(ec), Some(ep)) = (s.eval_cur, s.eval_proto) {
            score_gap_cur.push(s.score.abs_diff(ec) as usize);
            score_gap_proto.push(s.score.abs_diff(ep) as usize);
        }
    }
    score_gap_cur.sort_unstable();
    score_gap_proto.sort_unstable();
    println!("  |search score − eval(read point)| on tactic-labelled non-mate lines (n = {}):", score_gap_cur.len());
    println!(
        "    current settled: p50 {:>4}, p90 {:>4}, max {:>5}",
        pct(&score_gap_cur, 0.50),
        pct(&score_gap_cur, 0.90),
        score_gap_cur.last().copied().unwrap_or(0),
    );
    println!(
        "    prototype:       p50 {:>4}, p90 {:>4}, max {:>5}",
        pct(&score_gap_proto, 0.50),
        pct(&score_gap_proto, 0.90),
        score_gap_proto.last().copied().unwrap_or(0),
    );

    // Detector oracle: distance from the named tactic's key-move ply.
    let labelled: Vec<&LineStat> = stats.iter().filter(|s| s.tactic_ply.is_some()).collect();
    if labelled.is_empty() {
        println!("  detector oracle: no tactic-labelled lines");
        return;
    }
    let mut cur_gap: Vec<usize> = Vec::new();
    let mut proto_gap: Vec<usize> = Vec::new();
    for s in &labelled {
        let hit = s.tactic_ply.unwrap();
        let cur = match s.settled {
            Some(i) => i.min(s.pv_len - 1),
            None => s.pv_len - 1,
        };
        cur_gap.push(cur.saturating_sub(hit));
        proto_gap.push(s.proto_settled.saturating_sub(hit));
    }
    cur_gap.sort_unstable();
    proto_gap.sort_unstable();
    println!(
        "  detector oracle ({} tactic-labelled lines), plies past hit.pv_ply:",
        labelled.len(),
    );
    println!(
        "    current settled: p50 {}, p90 {}, max {}",
        pct(&cur_gap, 0.50),
        pct(&cur_gap, 0.90),
        cur_gap.last().copied().unwrap_or(0),
    );
    println!(
        "    prototype:       p50 {}, p90 {}, max {}",
        pct(&proto_gap, 0.50),
        pct(&proto_gap, 0.90),
        proto_gap.last().copied().unwrap_or(0),
    );
}

fn print_flips(flips: &[FlipExample], max: usize) {
    if flips.is_empty() || max == 0 {
        return;
    }
    println!();
    println!(
        "  classification-flip examples ({} of {}):",
        max.min(flips.len()),
        flips.len(),
    );
    for f in flips.iter().take(max) {
        println!(
            "    pos {:>2} slot {:>2}: current {}({:+} cp @ply {}) -> prototype {}({:+} cp @ply {})",
            f.pos_idx,
            f.slot,
            f.cur.0.label(),
            f.cur.1,
            f.cur.2,
            f.proto.0.label(),
            f.proto.1,
            f.proto.2,
        );
        println!("      fen: {}", f.fen);
        println!("      pv:  {}", f.pv_san);
    }
}

/// Nearest-rank percentile over a sorted slice.
fn pct(sorted: &[usize], p: f64) -> usize {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx]
}

fn load_positions(fen_file: &str) -> Result<Vec<String>> {
    if fen_file == "default" {
        return Ok(DEFAULT_BENCH_FENS.iter().map(|s| s.to_string()).collect());
    }
    let contents = std::fs::read_to_string(fen_file)
        .with_context(|| format!("reading FEN file {fen_file:?}"))?;
    Ok(contents
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|s| s.to_string())
        .collect())
}

#[cfg(test)]
#[path = "settled_audit_tests.rs"]
mod tests;
