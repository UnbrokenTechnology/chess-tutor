# Handoff: chess-tutor-2

State index for fresh contexts. **Read [`CLAUDE.md`](CLAUDE.md) first** for evergreen guidance (mission, legal/licensing, ground rules); this file and its split-outs are forward-looking only — git history covers what's been built, inline module docs (`//!`) cover design rationale.

## What this app is

A **chess tutor**, not a chess engine. The product surface is move-by-move teaching feedback for ~1200 ELO students climbing toward the 1600+ range. Strength is a means: 2000-ish ELO is enough to pose interesting positions; explainability is the actual product. Three pillars:

1. **The engine** — Stockfish-11 classical port (NNUE banned). 2000 ELO verified empirically. Search has most of the SF11 pruning stack; eval decomposes into 45 named sub-terms keyed by `TermId`, each with mg/eg components and a per-term tapered cp delta the teaching layer reads.
2. **The teaching layer** — [`core/engine/src/analysis/`](core/engine/src/analysis/) — see that module's `//!` for the design principles. Traces every UI claim back to a concrete engine signal: term deltas, structured outcome snapshots, surprise classification, verdict.
3. **The narration crate** (`core/narration/`) — renders structured outcomes into prose. Public surface: `format_retrospective(pre_move_pos, &[MoveAnalysis], user_move, &NarrationOptions) -> String`.

UIs: CLI (`chess-tutor`), egui desktop (`chess-tutor-desktop`), planned Apple + Android. FFI crate (`core/ffi/`) is the prerequisite for the platform apps and doesn't exist yet.

Tests: **673 engine (+4 ignored) + 105 narration + 49 cli = 827 passing**, clippy clean.

## Currently iterating on: teaching UX

Engine perf is in a good place (sub-300 ms retrospective on hard positions, 43 s for the full d=20 bench at 8 threads). Further perf has diminishing returns relative to the UX work that is now the bottleneck on the actual product.

→ **[`HANDOFF-ux.md`](HANDOFF-ux.md)** — teaching layer state, deferred Phase 2/4/5 work, narration tuning, UX platform tasks, live-play tuning loop. Read this when iterating on teaching UX.

→ **[`HANDOFF-perf.md`](HANDOFF-perf.md)** — current bench numbers, levers tested + reverted, outlier breakdowns, deferred perf opportunities, engine-strength deferred. Read only when returning to engine perf / strength work; it is stable but noisy and would pollute a UX-focused context.

## Build / dev commands

```bash
cargo test --release       # default; debug is 20–200× slower (magic search)
cargo build --release      # → target/release/chess-tutor[-desktop].exe
cargo clippy --all-targets

# Profiling build (release-equivalent + debuginfo for VTune):
cargo build --profile profiling --bin chess-tutor
# → target/profiling/chess-tutor.exe

# Bench (SF11-compatible — `<tt_mb> <threads> <depth> [fen_file] [limit_type]`):
./target/release/chess-tutor bench 16 1 13                              # 1 thread, shared TT
./target/release/chess-tutor bench 128 8 20 default depth --new-game-between-positions  # 8 threads, cold TT
./target/release/chess-tutor bench 16 1 13 path/to/fens.txt             # custom positions

# Play (CLI) and multi-thread retrospective:
./target/release/chess-tutor play --threads 4              # 4 thread engine moves; retrospective uses all cores
./target/release/chess-tutor play --deterministic          # retrospective single-thread (bit-reproducible narration)
```

## Heap allocation policy

Per-search or per-engine allocations are fine. **Per-node allocations are not** — use stack arrays or pool from a thread-local. The `MovePicker` buffer pool (thread-local `Vec<Box<MoveBufs>>`) is the canonical pattern; copy it for any new feature that needs per-call scratch.

## Pointers to inline design briefs

- **Teaching analysis pipeline**: [`core/engine/src/analysis/mod.rs`](core/engine/src/analysis/mod.rs) `//!`
- **Trap library schema + four-gate validator**: [`core/engine/src/traps/mod.rs`](core/engine/src/traps/mod.rs) `//!`
- **Engine public API surface**: [`core/engine/src/engine.rs`](core/engine/src/engine.rs)
- **Search structure + pruning stack**: [`core/engine/src/search.rs`](core/engine/src/search.rs) `//!`
- **Move picker pipeline**: [`core/engine/src/movepick.rs`](core/engine/src/movepick.rs) `//!`
- **TT layout**: [`core/engine/src/tt.rs`](core/engine/src/tt.rs) `//!`
- **Repo layout, mission, ground rules**: [`CLAUDE.md`](CLAUDE.md)
