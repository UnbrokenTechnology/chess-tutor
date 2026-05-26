# Handoff: chess-tutor-2

State index for fresh contexts. **Read [`CLAUDE.md`](CLAUDE.md) first** for evergreen guidance (mission, legal/licensing, ground rules); this file and its split-outs are forward-looking only — git history covers what's been built, inline module docs (`//!`) cover design rationale.

> **[`ROADMAP.md`](ROADMAP.md)** (temporary) — four sequenced big-rock workflows, a detour *out of* the teaching-UX work (the coach couldn't reason from the PV, so the plan is to port lichess tactic detection — which first needs a correct, clean engine). Status: **(1) SF11 parity audit ✅ COMPLETE** (gap closed to ~2× SF — the "~10x" was stale; see [`parity-audit-log.md`](parity-audit-log.md)); **(2) non-functional refactor — NEXT**; (3) lichess tactic-library port; (4) broader lichess feature audit. Order matters; read before starting any of them.

## What this app is

A **chess tutor**, not a chess engine. The product surface is move-by-move teaching feedback for ~1200 ELO students climbing toward the 1600+ range. Strength is a means: 2000-ish ELO is enough to pose interesting positions; explainability is the actual product. Three pillars:

1. **The engine** — Stockfish-11 classical port (NNUE banned). 2000 ELO verified empirically. Search has most of the SF11 pruning stack; eval decomposes into 45 named sub-terms keyed by `TermId`, each with mg/eg components and a per-term tapered cp delta the teaching layer reads.
2. **The teaching layer** — [`core/engine/src/analysis/`](core/engine/src/analysis/) — see that module's `//!` for the design principles. Traces every UI claim back to a concrete engine signal: term deltas, structured outcome snapshots, surprise classification, verdict.
3. **The narration crate** (`core/narration/`) — renders structured outcomes into prose. Public surface: `format_retrospective(pre_move_pos, &[MoveAnalysis], user_move, &NarrationOptions) -> String`.

UIs: CLI (`chess-tutor`), egui desktop (`chess-tutor-desktop`), planned Apple + Android. FFI crate (`core/ffi/`) is the prerequisite for the platform apps and doesn't exist yet.

Tests: **728 engine (+4 ignored) + 105 narration + 33 cli + 27 ui = 893 passing**, clippy clean.

## Current focus: executing the ROADMAP (teaching UX parked)

We are partway through the four-workflow [`ROADMAP.md`](ROADMAP.md) detour. **W1 (SF11 parity audit) is ✅ complete** — done-criteria met (d=14 1.48× SF, d=20 2.04× SF), two correctness bugs fixed plus the SF11 pruning stack landed as balanced bundles; full log in [`parity-audit-log.md`](parity-audit-log.md). **W2 (non-functional refactor) is next** — every `.rs` source file ≤500 LOC, tests to sibling `_tests.rs` files, no logic/perf/test-count change. W3 (lichess tactic port) + W4 (broader audit) follow. The teaching UX is **parked until W4 completes**; the rest of this section is the state to resume from.

### Parked: the teaching UX (resume post-W4)

The product has three teaching surfaces, all card-based and all reading the same engine outcomes. This is the in-progress body that motivated the roadmap (it needs PV-based tactic detection to be honest about "you missed a tactic" / "you had a mate") and was merged from `main` so it doesn't bit-rot:

1. **Retrospective panel** — after-the-fact analysis of the user's last move. Cards per signal (material, threats, king safety, mobility, pawn structure, passed pawns, piece placement, secondary, **forced consequences of opponent's best reply**). Best-move reveal is opt-in (`LearningPreferences.reveal_best_moves`, default off).
2. **Coaching panel** (live) — features-to-notice for the position the user is about to move from. Shown when `AssistanceLevel::Coached` is active. Surfaces hanging-piece opportunities (filtered through legal moves so in-check / pinned cases don't lie), en-passant captures, pawn weaknesses on either side, and a "your king is in check" card. Never names a move.
3. **Game Review** (post-game / on-demand) — ranked list of significant moments derived from the same classifier that drives in-game intervention. Click any moment to jump the rest of the UI there.

Plus the **intervention pause**: when `MistakeHandling::TeachingMoments` or blunder safety is on, the engine reply is held after a user move until the classifier decides whether to pause and surface a "you missed something / take back / continue" prompt. Gate is tight by design — single dominant eval-term family + share threshold + position-not-hopeless, so noise / engine subtlety don't interrupt play.

→ **[`HANDOFF-ux.md`](HANDOFF-ux.md)** — teaching layer state, learning-mode design, deferred Phase 2/4/5 work, narration tuning, UX platform tasks, live-play tuning loop. Read this when iterating on teaching UX.

→ **[`HANDOFF-perf.md`](HANDOFF-perf.md)** — engine perf levers tested + reverted, outlier breakdowns, deferred perf/strength opportunities. **Its bench numbers predate the W1 parity audit — [`parity-audit-log.md`](parity-audit-log.md) holds the current figures.** Read only when returning to engine perf / strength work; stable but noisy.

**Open thread when teaching UX resumes (post-W4):** persistence design — game history on disk so past games are reviewable across launches and the foundation for drills / per-concept mastery fading. Desktop and mobile storage models differ (filesystem vs platform storage); user erase / clear-history UX needs design before code lands. See HANDOFF-ux's "Persistence (deferred)" section.

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
