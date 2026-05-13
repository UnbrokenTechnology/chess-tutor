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

/// User-facing args, mirroring SF11's positional argument order.
pub struct BenchArgs {
    pub tt_mb: usize,
    pub threads: usize,
    pub limit: u64,
    pub fen_file: String,
    pub limit_type: String,
}

pub fn run(args: BenchArgs) -> Result<()> {
    if args.threads != 1 {
        return Err(anyhow!(
            "bench: only single-thread search is supported (got threads={})",
            args.threads
        ));
    }

    let positions = load_positions(&args.fen_file)?;
    if positions.is_empty() {
        return Err(anyhow!("bench: no positions to search"));
    }

    // Build the search-params template once so each position search
    // uses the same limit shape.
    let params_template = build_params(&args.limit_type, args.limit)?;

    println!(
        "bench: {} positions, TT = {} MB, limit = {} {}",
        positions.len(),
        args.tt_mb,
        args.limit,
        args.limit_type,
    );

    let mut engine = Engine::new(args.tt_mb);
    // SF emits a single `ucinewgame` at the start of bench, not
    // between positions — so TT / history learning carries over across
    // the position list. Mirror that here (`new_game` is a no-op on a
    // freshly-built engine but we call it for clarity of intent).
    engine.new_game();

    let started = Instant::now();
    let mut total_nodes: u64 = 0;

    for (i, entry) in positions.iter().enumerate() {
        let mut pos = parse_bench_entry(entry)
            .with_context(|| format!("parsing bench entry {}", i + 1))?;
        let params = params_template.clone();

        // Detect terminal positions (no legal moves) up front so the
        // bench row reports `(checkmate)` / `(stalemate)` rather than
        // a misleading `0 nodes 0 ms 0.00 Mnps`. SF's bench shows the
        // same positions as `bestmove (none)`; both contribute zero to
        // the aggregate totals, but the human reader shouldn't have to
        // squint at the zeros to figure out why.
        if legal_moves_vec(&mut pos).is_empty() {
            let label = if pos.in_check() { "checkmate" } else { "stalemate" };
            println!(
                "  {:>2}/{}  (terminal — {})",
                i + 1,
                positions.len(),
                label,
            );
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

fn build_params(limit_type: &str, limit: u64) -> Result<SearchParams> {
    let mut p = SearchParams {
        max_depth: 1,
        max_nodes: None,
        max_time: None,
        multi_pv: 1,
        game_history: Vec::new(),
        force_include: Vec::new(),
        verbose_progress: false,
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

