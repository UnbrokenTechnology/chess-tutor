//! UCI protocol shim — exposes a dial-configured bot as a UCI engine so
//! the offline ELO-calibration harness (fastchess gauntlets vs Maia) can
//! drive it. **Measurement/test only; the product never enters this
//! path** — it's the harness's bridge from our depth-budget engine to a
//! standard UCI match runner.
//!
//! Move selection mirrors the play worker (`core/ui/src/worker.rs`):
//! generate legal moves → `engine.search` at the configured depth and
//! [`NoiseProfile::effective_multi_pv`] → `noise::pick` → emit
//! `bestmove`. The same [`EvalMask`] / [`NoiseProfile`] dials a human
//! game uses apply here, so a config measured by the harness is the
//! config the product would ship.
//!
//! Determinism: the bot's randomness is seeded per *game*, not per
//! process. A `ucinewgame` bumps a counter, and the per-game seed is
//! `base_seed` mixed with that counter — so one `--seed` replays an
//! entire multi-day run bit-for-bit while individual games still differ
//! (the determinism contract from CLAUDE.md, carried into the harness).
//!
//! Protocol subset (all fastchess needs): `uci` → id + `uciok`;
//! `isready` → `readyok`; `ucinewgame` → reset + reseed; `position
//! [startpos | fen <FEN>] [moves …]`; `go [depth N]` → `bestmove`;
//! `quit`. Time-control tokens on `go` are ignored — we always search to
//! the configured depth (or an explicit `go depth N`), which is what
//! makes per-config strength reproducible. fastchess still imposes a
//! wall-clock timeout, which a fixed-depth search clears easily.

use std::io::{self, BufRead, Write};

use anyhow::Result;

use chess_tutor_engine::endgame::EndgameSkill;
use chess_tutor_engine::engine::{Engine, SearchParams};
use chess_tutor_engine::noise::{self, NoisePick};
use chess_tutor_engine::opponent::{EvalMask, NoiseProfile};
use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::{Move, Value};

use crate::uci;
use crate::units;

/// Resolved bot configuration for one shim invocation. Built from the
/// `chess-tutor uci` CLI flags in `main.rs`, mirroring the way the
/// `play` subcommand assembles an `OpponentProfile`.
pub struct UciConfig {
    /// Iterative-deepening depth the bot searches to per move (unless a
    /// `go depth N` overrides it). This is the engine-strength *floor*
    /// dial; the move-distribution dials in `noise` do the human-like
    /// reshaping (see PLAN-elo-calibration.md).
    pub depth: u32,
    /// Lazy-SMP threads. Default 1 keeps each move bit-deterministic so
    /// a `--seed` replay is exact.
    pub threads: usize,
    /// Base seed for per-game randomness. Mixed with the `ucinewgame`
    /// counter to produce each game's seed.
    pub base_seed: u64,
    /// Evaluation categories the bot is blind to (knowledge-gap dial).
    pub eval_mask: EvalMask,
    /// Quiescence horizon cap (tactical-vision dial). `None` = full
    /// vision; `Some(0)` = tactically blind (hangs pieces).
    pub qsearch_max_plies: Option<u32>,
    /// Endgame-book skill tier (technique dial). [`EndgameSkill::Full`] =
    /// all books; lower tiers withhold the harder mates so the bot
    /// misplays endgames (and queens instead of underpromoting).
    pub endgame_skill: EndgameSkill,
    /// Move-visibility dial (geometric-blindness lever): `1.0` = sees
    /// every move; lower prunes geometrically subtle moves from the
    /// bot's search (see `chess_tutor_engine::visibility`).
    pub perception: f32,
    /// Move-sampling dials (variety / blunder / miss / wild).
    pub noise: NoiseProfile,
}

/// Run the UCI read-eval-print loop on stdin/stdout until EOF or `quit`.
pub fn run(cfg: UciConfig) -> Result<()> {
    // One persistent play engine: TT and history accumulate across the
    // moves of a game (what makes a bot stronger deeper into a game),
    // cleared by `new_game()` on each `ucinewgame`.
    let mut engine = Engine::default();
    let mut pos = Position::startpos();
    // Pre-root repetition keys for the current root (excludes the root
    // itself) so the bot's search sees threefold draws correctly.
    let mut history: Vec<u64> = Vec::new();
    let mut ply: u64 = 0;
    // Destination of the last move applied by the latest `position`
    // command — the perception filter's root attention locus.
    let mut last_move_to: Option<chess_tutor_engine::types::Square> = None;
    let mut game_index: u64 = 0;
    let mut game_seed = mix_seed(cfg.base_seed, game_index);

    // Surface the resolved config on stderr (stdout is the UCI channel)
    // so harness logs record exactly what was measured.
    eprintln!(
        "uci-shim: depth={} qsearch_depth={:?} endgame_skill={:?} perception={} threads={} base_seed={} eval_mask_disabled=[{}] noise={{rank={}, guaranteed_mate_in={}}}",
        cfg.depth,
        cfg.qsearch_max_plies,
        cfg.endgame_skill,
        cfg.perception,
        cfg.threads,
        cfg.base_seed,
        cfg.eval_mask
            .disabled_iter()
            .map(|c| c.slug())
            .collect::<Vec<_>>()
            .join(","),
        cfg.noise.avg_move_rank,
        cfg.noise.guaranteed_mate_in,
    );

    let stdin = io::stdin();
    let mut out = io::stdout().lock();

    let mut input = String::new();
    loop {
        input.clear();
        if stdin.lock().read_line(&mut input)? == 0 {
            break; // EOF
        }
        let line = input.trim();
        if line.is_empty() {
            continue;
        }
        let cmd = line.split_whitespace().next().unwrap_or("");
        match cmd {
            "uci" => {
                writeln!(out, "id name chess-tutor-bot")?;
                writeln!(out, "id author chess-tutor")?;
                // No tunable UCI options: every dial is fixed at launch
                // via CLI flags so a config is a single immutable unit.
                writeln!(out, "uciok")?;
                out.flush()?;
            }
            "isready" => {
                writeln!(out, "readyok")?;
                out.flush()?;
            }
            "ucinewgame" => {
                engine.new_game();
                game_index += 1;
                game_seed = mix_seed(cfg.base_seed, game_index);
                pos = Position::startpos();
                history.clear();
                ply = 0;
                last_move_to = None;
            }
            "position" => match build_position(line) {
                Ok((p, h, n, last)) => {
                    pos = p;
                    history = h;
                    ply = n;
                    last_move_to = last;
                }
                Err(e) => eprintln!("uci-shim: bad position command: {e}"),
            },
            "go" => {
                let depth = parse_go_depth(line).unwrap_or(cfg.depth).max(1);
                match choose_move(
                    &mut engine,
                    &mut pos,
                    depth,
                    &cfg,
                    &history,
                    game_seed,
                    ply,
                    last_move_to,
                ) {
                    Some(choice) => {
                        // Emit one `info` line so fastchess has a score for
                        // PGN annotation and eval-based resign/draw
                        // adjudication (which keeps a long unattended run's
                        // games short). Score is the chosen move's line, in
                        // UCI convention: side-to-move-POV, conventional
                        // cp (pawn = 100) / `mate N`.
                        let pv: String = choice
                            .pv
                            .iter()
                            .map(|m| uci::format(*m))
                            .collect::<Vec<_>>()
                            .join(" ");
                        writeln!(
                            out,
                            "info depth {} score {} pv {}",
                            choice.depth,
                            uci_score(choice.score),
                            pv,
                        )?;
                        writeln!(out, "bestmove {}", uci::format(choice.mv))?;
                    }
                    // No legal move (terminal). fastchess adjudicates the
                    // result from the position; the null reply is a guard.
                    None => writeln!(out, "bestmove 0000")?,
                }
                out.flush()?;
            }
            // We search synchronously to completion, so there is nothing
            // to interrupt; ignore. `setoption` likewise has no tunables.
            "stop" | "setoption" | "debug" | "ponderhit" => {}
            "quit" => break,
            _ => {} // ignore anything unrecognised (UCI is lenient)
        }
    }
    Ok(())
}

/// The bot's chosen move plus the score/depth/PV to report over UCI.
struct MoveChoice {
    mv: Move,
    /// Side-to-move-POV engine score of the reported line.
    score: Value,
    depth: u32,
    pv: Vec<Move>,
}

/// Pick the bot's move for the current root, mirroring the play worker's
/// search → `noise::pick` → move pipeline, and carry back the line's
/// score/PV for the UCI `info` string.
#[allow(clippy::too_many_arguments)]
fn choose_move(
    engine: &mut Engine,
    pos: &mut Position,
    depth: u32,
    cfg: &UciConfig,
    history: &[u64],
    seed: u64,
    ply: u64,
    last_move_to: Option<chess_tutor_engine::types::Square>,
) -> Option<MoveChoice> {
    let params = SearchParams {
        max_depth: depth,
        max_nodes: None,
        max_time: None,
        multi_pv: cfg.noise.effective_multi_pv(),
        game_history: history.to_vec(),
        force_include: Vec::new(),
        verbose_progress: false,
        threads: cfg.threads.max(1),
        eval_mask: cfg.eval_mask,
        qsearch_max_plies: cfg.qsearch_max_plies,
        endgame_skill: cfg.endgame_skill,
        // Per-game seed (not the base seed) so blind spots are stable
        // within a game but vary across the run's games — mirroring
        // how the noise rolls vary.
        perception: if cfg.perception < 1.0 {
            Some(chess_tutor_engine::visibility::PerceptionParams {
                level: cfg.perception,
                seed,
                last_move_to,
                exempt_root_checks: cfg.noise.guaranteed_mate_in >= 1,
            })
        } else {
            None
        },
    };
    let lines = engine.search(pos, params);
    // The noise pick (engine-best / variety) yields a ranked line;
    // report that line's own score and PV.
    match noise::pick(&cfg.noise, seed, ply, pos, &lines) {
        NoisePick::Line(idx) => {
            let line = lines.get(idx).or_else(|| lines.first())?;
            Some(MoveChoice {
                mv: *line.pv.first()?,
                score: line.score,
                depth: line.depth,
                pv: line.pv.clone(),
            })
        }
    }
}

/// Format a side-to-move-POV [`Value`] as a UCI `score` token: `mate N`
/// (signed full-moves; positive = stm mates) or `cp C` in conventional
/// centipawns (pawn = 100), converting from our PawnEG=213 engine scale.
fn uci_score(v: Value) -> String {
    if units::is_mate_score(v) {
        let plies = Value::MATE.0 - v.0.abs();
        let moves = (plies + 1) / 2;
        let signed = if v.0 >= 0 { moves } else { -moves };
        format!("mate {signed}")
    } else {
        format!(
            "cp {}",
            units::engine_cp_to_conventional_cp(v).round() as i32
        )
    }
}

/// Parse a UCI `position` command into the root position, its pre-root
/// repetition-key history (excluding the root), and the half-move count
/// (ply) used to seed move-by-move noise.
fn build_position(
    line: &str,
) -> Result<(
    Position,
    Vec<u64>,
    u64,
    Option<chess_tutor_engine::types::Square>,
)> {
    let rest = line.strip_prefix("position").unwrap_or(line).trim();
    // FEN fields never contain the literal "moves", so a plain find is
    // safe to split the position spec from the move list.
    let (spec, moves_str) = match rest.find("moves") {
        Some(i) => (rest[..i].trim(), rest[i + "moves".len()..].trim()),
        None => (rest, ""),
    };

    let mut pos = if spec.is_empty() || spec == "startpos" {
        Position::startpos()
    } else if let Some(fen) = spec.strip_prefix("fen") {
        Position::from_fen(fen.trim())
            .map_err(|e| anyhow::anyhow!("invalid FEN {:?}: {e}", fen.trim()))?
    } else {
        anyhow::bail!("unrecognised position spec {spec:?} (expected `startpos` or `fen <FEN>`)");
    };

    let mut history = Vec::new();
    let mut ply = 0u64;
    let mut last_move_to = None;
    for mv_str in moves_str.split_whitespace() {
        // Key of the position *before* this move joins the pre-root
        // history; the root (after the last move) is deliberately omitted.
        history.push(pos.key());
        let mv = uci::parse(&mut pos, mv_str)
            .map_err(|e| anyhow::anyhow!("bad move {mv_str:?}: {e}"))?;
        pos.do_move(mv);
        last_move_to = Some(mv.to());
        ply += 1;
    }
    Ok((pos, history, ply, last_move_to))
}

/// Extract the depth from a `go depth N` command, if present. Any other
/// `go` form (time controls, `infinite`, `nodes`) returns `None` and the
/// caller falls back to the configured depth — we never time-budget.
fn parse_go_depth(line: &str) -> Option<u32> {
    let mut it = line.split_whitespace();
    while let Some(tok) = it.next() {
        if tok == "depth" {
            return it.next().and_then(|d| d.parse().ok());
        }
    }
    None
}

/// SplitMix64 step mixing the base seed with the per-game counter, so
/// each game in a run gets a distinct-yet-reproducible seed.
fn mix_seed(base: u64, game_index: u64) -> u64 {
    let mut x = base
        .wrapping_add(game_index.wrapping_mul(0x9E37_79B9_7F4A_7C15))
        .wrapping_add(0xD1B5_4A32_D192_ED03);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    x
}

#[cfg(test)]
#[path = "uci_shim_tests.rs"]
mod tests;
