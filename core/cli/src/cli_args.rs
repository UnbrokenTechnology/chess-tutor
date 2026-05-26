//! Clap command-line argument definitions: the top-level [`Cli`], the
//! [`Command`] subcommand enum, and the [`EngineColor`] value enum. Split
//! out of `main.rs` so the driver logic stays readable; `main.rs` re-exports
//! `EngineColor` at the crate root for `crate::play`.

use clap::{Parser, Subcommand, ValueEnum};

const STARTPOS: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

/// Default analytical depth for the auto-retrospective. Kept
/// deliberately *deeper* than the typical engine-play depth so the
/// retrospective is a stronger reference than the bot the student is
/// playing against. At depth 10 we observed opening-move verdicts
/// that flipped at depth 12 (e.g. 1.e4 e5 2.Nf3 reads as an
/// inaccuracy at d=10 but emerges as best at d=12). Matches
/// `chess_tutor_ui::session::ANALYTICAL_DEPTH`.
const DEFAULT_RETROSPECTIVE_DEPTH: u32 = 12;

#[derive(Parser)]
#[command(
    name = "chess-tutor",
    version,
    about = "Classical chess engine + teaching tool."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Render a FEN as a Unicode/ANSI chess board.
    Board {
        #[arg(default_value = STARTPOS)]
        fen: String,
        #[arg(long)]
        ascii: bool,
        #[arg(long)]
        flip: bool,
        #[arg(long)]
        light_mode: bool,
    },
    /// List every legal move in SAN, one per line.
    Moves {
        #[arg(default_value = STARTPOS)]
        fen: String,
    },
    /// Print the classical-eval per-term trace for a FEN.
    Eval {
        #[arg(default_value = STARTPOS)]
        fen: String,
    },
    /// Identify the opening (ECO code + name) of a position, if known.
    Opening {
        #[arg(default_value = STARTPOS)]
        fen: String,
    },
    /// Run an engine search; print the principal variation and the leaf
    /// [`EvalTrace`]. With `--multi-pv N > 1`, prints N ranked lines
    /// each with its score and the score delta from the top line.
    Search {
        #[arg(default_value = STARTPOS)]
        fen: String,
        /// Maximum iterative-deepening depth (plies).
        #[arg(long, default_value_t = 10)]
        depth: u32,
        /// Stop after this many nodes.
        #[arg(long)]
        nodes: Option<u64>,
        /// Stop after this wall-clock duration (milliseconds).
        #[arg(long)]
        time_ms: Option<u64>,
        /// Return up to this many ranked principal variations (default
        /// 1 = single best line). Only the top line includes the leaf
        /// [`EvalTrace`]; additional lines show PV, score, and the
        /// delta-from-top.
        #[arg(long, default_value_t = 1)]
        multi_pv: usize,
        /// Dump a per-ply trajectory table for each PV: the white-POV,
        /// tempo-free score at each ply along with the delta from the
        /// previous ply. Useful for tuning the settled-ply threshold
        /// and for understanding the ply-to-ply "sawtooth" where each
        /// side's move temporarily shifts the eval before the opponent
        /// responds.
        #[arg(long)]
        debug: bool,
        /// For each returned PV, print the teaching-pipeline term-delta
        /// attribution: what named evaluation terms shifted between the
        /// root position and the "settled" ply of the move's PV, in
        /// tapered engine-cp, sorted by the size of the swing.
        #[arg(long)]
        analyze: bool,
        /// Cumulative `|delta|` coverage percent used by `--analyze` to
        /// pick how many term rows to show per move. 75 = smallest row
        /// prefix whose absolute-delta sum is at least 75% of the
        /// total. Higher values show more detail.
        #[arg(long, default_value_t = 75.0)]
        top_percent: f32,
        /// Number of Lazy-SMP search threads. Default 1 for
        /// reproducible output; raise to use more cores when you
        /// don't need bit-identical results.
        #[arg(long, default_value_t = 1)]
        threads: usize,
        /// Force a move into the MultiPV result. Mirrors the
        /// retrospective's `force_include` so you can reproduce its
        /// pathological positions one-shot. Accepts SAN (`Nf3`,
        /// `Qxe6+`) or UCI (`g1f3`). Repeat the flag to force in
        /// multiple moves.
        #[arg(long = "force-include", value_name = "MOVE")]
        force_include: Vec<String>,
        /// Emit per-depth aspiration / fail-high / fail-low events to
        /// stderr. Useful for diagnosing aspiration blowups and
        /// pathological positions.
        #[arg(long)]
        verbose_progress: bool,
    },
    /// Multi-position search benchmark. Argument order and defaults
    /// mirror Stockfish 11's `bench` command: `tt_mb threads limit
    /// fen_file limit_type`, defaults `16 1 13 default depth`. Output
    /// finishes with an SF-style `Total time / Nodes searched /
    /// Nodes/second` aggregate so the numbers can be compared
    /// apples-to-apples against `stockfish bench`.
    Bench {
        /// Transposition-table size in MB. SF default is 16.
        #[arg(default_value_t = 16)]
        tt_mb: usize,
        /// Number of search threads. Only 1 is supported today (the
        /// engine is single-thread); the arg exists for SF parity.
        #[arg(default_value_t = 1)]
        threads: usize,
        /// Limit value — interpreted by `limit_type`. With the default
        /// `depth`, this is the maximum iterative-deepening depth in
        /// plies (SF default is 13).
        #[arg(default_value_t = 13)]
        limit: u64,
        /// `default` for the built-in 45-position list (mirrored from
        /// SF11), or a path to a file with one bench entry per line
        /// (same `<fen> [moves uci ...]` shape SF accepts).
        #[arg(default_value = "default")]
        fen_file: String,
        /// `depth` (default) or `nodes`. `movetime` / `perft` are not
        /// supported yet.
        #[arg(default_value = "depth")]
        limit_type: String,
        /// Call `engine.new_game()` between every position, clearing
        /// the TT, history, and pawn cache. Off by default to match
        /// SF's behaviour (one `ucinewgame` at the start of bench,
        /// TT carries across positions). Useful for isolating
        /// per-position performance from cross-position TT pollution
        /// — at large TT sizes (e.g. 128 MB), entries from earlier
        /// bench positions can displace deeper entries the later
        /// positions want, causing dramatic per-position regressions
        /// vs. the small-TT case.
        #[arg(long)]
        new_game_between_positions: bool,
        /// TEMPORARY perf-investigation: after each position completes,
        /// print selDepth and a compact per-ply node histogram. Also
        /// enables per-ID-iteration heartbeat output from the search.
        /// Doesn't affect search behaviour, just adds stderr/stdout
        /// output.
        #[arg(long)]
        verbose: bool,
        /// TEMPORARY perf-investigation: comma-separated list of
        /// 1-based position indices to run (e.g. `20,26,40,41`). When
        /// set, only those positions from the FEN list are searched;
        /// others are skipped. Useful for focusing on known-slow FENs
        /// without sitting through the rest. Indexing matches the
        /// bench-output `N/45` numbering.
        #[arg(long)]
        positions: Option<String>,
    },
    /// Measure Lazy-SMP score variance across runs. For each position,
    /// runs `analyze_position` N times with a fresh engine state and
    /// reports how much the same move's score wobbles. Used to
    /// calibrate the [`MoveVerdict`] noise buffer.
    NoiseBench {
        /// Transposition-table size in MB.
        #[arg(long, default_value_t = 16)]
        tt_mb: usize,
        /// Search depth per run. Defaults to the retrospective's
        /// `DEFAULT_DEPTH` (10) so the measurement reflects what users
        /// actually see.
        #[arg(long, default_value_t = 10)]
        depth: u32,
        /// Multi-PV breadth per run. Defaults to the retrospective's
        /// `RETROSPECTIVE_MULTI_PV` (3).
        #[arg(long, default_value_t = 3)]
        multi_pv: usize,
        /// Number of threads. Defaults to 8 — typical Lazy-SMP load on
        /// the desktop's `available_parallelism()` default.
        #[arg(long, default_value_t = 8)]
        threads: usize,
        /// Number of runs per position. Variance estimate improves
        /// with N; 5 is a reasonable starting point.
        #[arg(long, default_value_t = 5)]
        runs: usize,
        /// `default` for the built-in 45-position SF11 set, or a path
        /// to a FEN file (same format as `chess-tutor bench`).
        #[arg(long, default_value = "default")]
        fen_file: String,
    },
    /// Interactive REPL. Human enters SAN or UCI; engine replies on
    /// its turn.
    Play {
        /// Seed from this FEN instead of the start position.
        #[arg(long)]
        fen: Option<String>,
        /// Which side the engine plays.
        #[arg(long, value_enum, default_value_t = EngineColor::Black)]
        engine_color: EngineColor,
        /// Max search depth for the engine when picking its own
        /// moves (plies).
        #[arg(long, default_value_t = 10)]
        depth: u32,
        /// Max search depth for the auto-retrospective analysing the
        /// user's just-played move. Defaults deeper than `--depth`
        /// because at d=10 we observed verdict flips on common
        /// opening positions (1.e4 e5 2.Nf3 reads "inaccuracy" at
        /// d=10, "best" at d=12). Independent of `--depth` so a
        /// weakened bot can still give strong feedback.
        #[arg(long, default_value_t = DEFAULT_RETROSPECTIVE_DEPTH)]
        retrospective_depth: u32,
        /// Engine time cap per move (milliseconds). Omit for pure
        /// depth-capped search.
        #[arg(long)]
        time_ms: Option<u64>,
        #[arg(long)]
        ascii: bool,
        #[arg(long)]
        flip: bool,
        #[arg(long)]
        light_mode: bool,
        /// Suppress the per-term breakdown on `Best` verdicts —
        /// only the congratulatory headline prints. Default behaviour
        /// is to narrate *why* the move was best so the student who
        /// guessed right still learns the reasoning. Toggle at
        /// runtime via the REPL `explain-best` command.
        #[arg(long = "no-explain-best", action = clap::ArgAction::SetTrue)]
        no_explain_best: bool,
        /// When true, print the current FEN before each side's turn.
        /// Useful for debugging — if the engine hangs or plays a bad
        /// move, the last-printed FEN reproduces the position exactly.
        #[arg(long)]
        show_fens: bool,
        /// Number of search threads (Lazy SMP) for **every** search:
        /// engine moves AND the auto-retrospective. Default 1 keeps
        /// every search bit-deterministic across runs and takebacks
        /// — the teaching contract is "same position, same verdict".
        /// Raise it for benchmarking. REPL `search` / `analyze`
        /// commands are always single-threaded.
        #[arg(long, default_value_t = 1)]
        threads: usize,
        /// Seed for the opponent's pseudo-randomness (opening line
        /// pick in Phase B, move sampling in later phases). Default:
        /// random per run, logged at game start. Pass a fixed value
        /// to replay an identical bot game.
        #[arg(long)]
        seed: Option<u64>,
        /// Disable the opening book for this game. Default behaviour
        /// is to pick a random line from the curated default set; pass
        /// this flag to force the engine to search from move 1.
        #[arg(long = "no-book", action = clap::ArgAction::SetTrue)]
        no_book: bool,
        /// Comma-separated list of evaluation categories the bot
        /// should be blind to for this game (e.g.
        /// `--disable-eval king-safety,pawn-structure`). Categories:
        /// pawn-structure | pieces | mobility | king-safety | threats
        /// | passed-pawns | space | initiative. The mid-game REPL
        /// `eval-mask` command can toggle individual categories.
        #[arg(long = "disable-eval", value_name = "CATEGORY[,CATEGORY...]")]
        disable_eval: Option<String>,
        /// How many top search lines the bot may sample from when
        /// softmax noise fires. Default 1 (no sampling — always #1).
        /// Pair with `--noise-temp` to actually pick from the pool;
        /// higher values cost roughly K× the per-move search time.
        #[arg(long = "noise-pool", value_name = "N", default_value_t = 1)]
        noise_pool: usize,
        /// Softmax temperature in centipawns. Default 0 (always pick
        /// #1 even when `--noise-pool > 1`). At 50 a line 50 cp behind
        /// has ~37% the weight of #1; at 200 it has ~78%. Use to dial
        /// up variety among close-scoring moves.
        #[arg(long = "noise-temp", value_name = "CP", default_value_t = 0)]
        noise_temp: i32,
        /// Per-move probability the bot drops a deliberate blunder
        /// (range 0.0–1.0). Default 0.0 (off). When > 0, the search
        /// widens to surface enough worse-than-best alternatives.
        #[arg(long = "blunder-chance", value_name = "P", default_value_t = 0.0)]
        blunder_chance: f32,
        /// Minimum loss (centipawns vs #1) for an alternative line to
        /// count as "in band" for the blunder picker. Default 100 — a
        /// clear pawn-down move the student can plausibly punish.
        #[arg(long = "blunder-min-loss", value_name = "CP", default_value_t = 100)]
        blunder_min_loss: i32,
        /// Maximum loss (centipawns vs #1) for an alternative line to
        /// count as "in band". Default 400 — caps blunders at roughly
        /// an exchange sacrifice; raise to allow more catastrophic
        /// blunders (~900 for queen hangs). When the band is empty
        /// the picker falls back to the closest-loss lines on each
        /// side of the band but excludes distant outliers.
        #[arg(long = "blunder-max-loss", value_name = "CP", default_value_t = 400)]
        blunder_max_loss: i32,
        /// Smallest mate the bot is guaranteed to convert — blunders
        /// are suppressed when `lines[0]` is a mate-in-N for
        /// `N <= guaranteed_mate_in`. Default 1 (mate-in-1 is never
        /// blundered). Set to 0 to allow blunders against any mate.
        #[arg(long = "guaranteed-mate-in", value_name = "N", default_value_t = 1)]
        guaranteed_mate_in: u32,
        /// Per-move probability the bot picks uniformly from ALL legal
        /// moves, bypassing the engine ranking entirely (range
        /// 0.0–1.0). Default 0.0 (off). This is the "beginner bot"
        /// branch — only it can pick moves the engine didn't surface
        /// (e.g. leaving a piece in a pawn's path). Same mate-guard
        /// as `--blunder-chance`.
        #[arg(long = "wild-chance", value_name = "P", default_value_t = 0.0)]
        wild_chance: f32,
    },
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum EngineColor {
    /// Engine plays white; human plays black.
    White,
    /// Engine plays black; human plays white (default).
    Black,
    /// Engine plays both sides (self-play).
    Both,
    /// Neither side is the engine — human controls both. Useful for
    /// exploring positions.
    None,
}
