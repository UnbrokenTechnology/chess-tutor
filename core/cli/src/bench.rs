//! `chess-tutor bench` — run a multi-position search benchmark and
//! print SF-compatible aggregate node / time / NPS numbers.
//!
//! Designed for apples-to-apples comparison with `stockfish bench
//! <tt_mb> <threads> <limit> [fenFile] [limitType]`: same default
//! position list (mirrored verbatim from [`crate::bench_fens`]), same
//! default limits (16 MB TT, 1 thread, depth 13), same summary footer
//! (`Total time (ms) / Nodes searched / Nodes/second`).
//!
//! What we don't support yet: `threads > 1` (engine is single-thread),
//! `movetime` / `perft` limit types (only `depth` and `nodes`),
//! `fenFile = current` (no UCI session to inherit a position from).

use std::fs;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};

use chess_tutor_engine::engine::{Engine, SearchParams};
use chess_tutor_engine::movegen::legal_moves_vec;

use crate::bench_fens::{parse_bench_entry, DEFAULT_BENCH_FENS};

/// User-facing args, mirroring SF11's positional argument order
/// (`tt_mb threads limit fen_file limit_type`), plus a
/// non-SF-compatible flag for per-position state isolation.
pub struct BenchArgs {
    pub tt_mb: usize,
    pub threads: usize,
    pub limit: u64,
    pub fen_file: String,
    pub limit_type: String,
    /// When `true`, call `engine.new_game()` between every position
    /// so each position is searched from a clean TT / history /
    /// pawn-cache state. Off by default to match SF's bench
    /// (single `ucinewgame` at the start). See [`crate::main`]'s
    /// `Bench::new_game_between_positions` doc-comment for why this
    /// matters at large TT sizes.
    pub new_game_between_positions: bool,
    /// TEMPORARY perf-investigation: enable per-ID heartbeat from the
    /// search and print per-position selDepth + per-ply node histogram.
    pub verbose: bool,
    /// TEMPORARY perf-investigation: if `Some`, only run the listed
    /// (1-based) position indices and skip the rest. Format is the raw
    /// CLI string (e.g. `"20,26,40,41"`); parsed here.
    pub positions: Option<String>,
}

pub fn run(args: BenchArgs) -> Result<()> {
    if args.threads == 0 {
        return Err(anyhow!("bench: threads must be >= 1"));
    }

    let positions = load_positions(&args.fen_file)?;
    if positions.is_empty() {
        return Err(anyhow!("bench: no positions to search"));
    }

    // Optional 1-based whitelist of positions to run. Set to `None`
    // means "run all"; otherwise contains the set of indices that
    // should actually be searched. Indices outside [1..=N] are
    // silently dropped.
    let allowed_indices: Option<std::collections::HashSet<usize>> = match &args.positions {
        Some(s) => Some(parse_position_indices(s)?),
        None => None,
    };

    // Build the search-params template once so each position search
    // uses the same limit shape.
    let mut params_template = build_params(&args.limit_type, args.limit, args.threads)?;
    if args.verbose {
        params_template.verbose_progress = true;
    }

    println!(
        "bench: {} positions, TT = {} MB, limit = {} {}{}",
        positions.len(),
        args.tt_mb,
        args.limit,
        args.limit_type,
        if args.new_game_between_positions {
            ", new-game-between-positions"
        } else {
            ""
        },
    );

    let mut engine = Engine::new(args.tt_mb);
    // SF emits a single `ucinewgame` at the start of bench, not
    // between positions — so TT / history learning carries over across
    // the position list. Mirror that here (`new_game` is a no-op on a
    // freshly-built engine but we call it for clarity of intent).
    // When `new_game_between_positions` is set the per-position loop
    // re-issues the clear; the start-of-bench call is still useful as
    // a no-op intent marker.
    engine.new_game();

    let started = Instant::now();
    let mut total_nodes: u64 = 0;

    for (i, entry) in positions.iter().enumerate() {
        if let Some(set) = allowed_indices.as_ref() {
            if !set.contains(&(i + 1)) {
                continue;
            }
        }
        if args.new_game_between_positions && i > 0 {
            engine.new_game();
        }
        let mut pos =
            parse_bench_entry(entry).with_context(|| format!("parsing bench entry {}", i + 1))?;
        let params = params_template.clone();

        // Detect terminal positions (no legal moves) up front so the
        // bench row reports `(checkmate)` / `(stalemate)` rather than
        // a misleading `0 nodes 0 ms 0.00 Mnps`. SF's bench shows the
        // same positions as `bestmove (none)`; both contribute zero to
        // the aggregate totals, but the human reader shouldn't have to
        // squint at the zeros to figure out why.
        if legal_moves_vec(&mut pos).is_empty() {
            let label = if pos.in_check() {
                "checkmate"
            } else {
                "stalemate"
            };
            println!("  {:>2}/{}  (terminal — {})", i + 1, positions.len(), label,);
            continue;
        }

        let lines = engine.search(&mut pos, params);
        let nodes = engine.last_nodes();
        let elapsed = engine.last_elapsed();
        total_nodes += nodes;

        let nps = if elapsed.as_secs_f64() > 0.0 {
            nodes as f64 / elapsed.as_secs_f64()
        } else {
            0.0
        };

        let depth = lines.first().map(|l| l.depth).unwrap_or(0);
        println!(
            "  {:>2}/{}  depth {:>2}  {:>10} nodes  {:>7} ms  {:>6.2} Mnps",
            i + 1,
            positions.len(),
            depth,
            nodes,
            elapsed.as_millis(),
            nps / 1.0e6,
        );
        if args.verbose {
            print_ply_histogram(engine.last_seldepth(), engine.last_nodes_per_ply());
        }
    }

    let elapsed = started.elapsed().max(Duration::from_millis(1));
    let nps = total_nodes as f64 / elapsed.as_secs_f64();

    println!();
    println!("===========================");
    println!("Total time (ms) : {}", elapsed.as_millis());
    println!("Nodes searched  : {}", total_nodes);
    println!("Nodes/second    : {}", nps as u64);

    Ok(())
}

/// Load bench positions. `"default"` returns the embedded SF11 list;
/// any other value is treated as a path to a file with one bench entry
/// per non-blank line (same `<fen> [moves ...]` shape as SF).
fn load_positions(fen_file: &str) -> Result<Vec<String>> {
    if fen_file == "default" {
        return Ok(DEFAULT_BENCH_FENS.iter().map(|s| s.to_string()).collect());
    }
    if fen_file == "current" {
        return Err(anyhow!(
            "bench: fen_file=current isn't supported (no UCI session to inherit a position from); \
             pass a FEN file path or omit to use the default list"
        ));
    }
    let body = fs::read_to_string(fen_file)
        .with_context(|| format!("reading bench FEN file {:?}", fen_file))?;
    Ok(body
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
        .map(str::to_string)
        .collect())
}

/// Parse the `--positions` CLI string ("1,8,20-22") into the set of
/// 1-based indices to run. Empty entries are tolerated; ranges use the
/// inclusive `start-end` form.
fn parse_position_indices(s: &str) -> Result<std::collections::HashSet<usize>> {
    let mut out = std::collections::HashSet::new();
    for tok in s.split(',') {
        let tok = tok.trim();
        if tok.is_empty() {
            continue;
        }
        if let Some((lo, hi)) = tok.split_once('-') {
            let lo: usize = lo
                .trim()
                .parse()
                .map_err(|_| anyhow!("bench: bad position range {:?}", tok))?;
            let hi: usize = hi
                .trim()
                .parse()
                .map_err(|_| anyhow!("bench: bad position range {:?}", tok))?;
            if lo == 0 || hi < lo {
                return Err(anyhow!("bench: bad position range {:?}", tok));
            }
            for i in lo..=hi {
                out.insert(i);
            }
        } else {
            let i: usize = tok
                .parse()
                .map_err(|_| anyhow!("bench: bad position index {:?}", tok))?;
            if i == 0 {
                return Err(anyhow!("bench: position indices are 1-based; got 0"));
            }
            out.insert(i);
        }
    }
    if out.is_empty() {
        return Err(anyhow!("bench: --positions argument matched no indices"));
    }
    Ok(out)
}

/// Print a compact one-line per-ply node histogram for the most recent
/// search. Trims trailing zeros so a d=14 search that selDepth-stretched
/// to ply 35 prints only the populated buckets, not 246 trailing zeros.
fn print_ply_histogram(seldepth: u32, per_ply: &[u64]) {
    let last_nonzero = per_ply.iter().rposition(|&n| n > 0).unwrap_or(0);
    print!("        seldepth {:>3}  ply nodes:", seldepth);
    for (i, &n) in per_ply.iter().take(last_nonzero + 1).enumerate() {
        print!(" {}={}", i, fmt_compact(n));
    }
    println!();
}

/// Format a node count compactly: 1_234_567 → "1.2M", 12_345 → "12k",
/// 999 → "999". Used by the verbose histogram so a 12-bucket histogram
/// fits on one line.
fn fmt_compact(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.1}G", n as f64 / 1.0e9)
    } else if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1.0e6)
    } else if n >= 1_000 {
        format!("{:.0}k", n as f64 / 1.0e3)
    } else {
        n.to_string()
    }
}

fn build_params(limit_type: &str, limit: u64, threads: usize) -> Result<SearchParams> {
    let mut p = SearchParams {
        max_depth: 1,
        max_nodes: None,
        max_time: None,
        multi_pv: 1,
        game_history: Vec::new(),
        force_include: Vec::new(),
        verbose_progress: false,
        threads,
        eval_mask: chess_tutor_engine::opponent::EvalMask::EMPTY,
        qsearch_max_plies: None,
        endgame_skill: chess_tutor_engine::endgame::EndgameSkill::Full,
        perception: None,
    };
    match limit_type {
        "depth" => {
            p.max_depth = u32::try_from(limit)
                .map_err(|_| anyhow!("bench: depth {} doesn't fit in u32", limit))?;
            if p.max_depth == 0 {
                return Err(anyhow!("bench: depth must be >= 1"));
            }
        }
        "nodes" => {
            p.max_depth = u32::MAX;
            p.max_nodes = Some(limit);
        }
        "movetime" | "perft" | "eval" => {
            return Err(anyhow!(
                "bench: limit_type {:?} isn't supported yet (use 'depth' or 'nodes')",
                limit_type
            ));
        }
        other => return Err(anyhow!("bench: unknown limit_type {:?}", other)),
    }
    Ok(p)
}
