# Handoff: chess-tutor-2

State index for fresh contexts. **Read [`CLAUDE.md`](CLAUDE.md) first** for evergreen guidance (mission, legal/licensing, ground rules); this file and its split-outs are forward-looking only — git history covers what's been built, inline module docs (`//!`) cover design rationale.

## What this app is

A **chess tutor**, not a chess engine. The product surface is move-by-move teaching feedback for ~1200 ELO students climbing toward the 1600+ range. Strength is a means: 2000-ish ELO is enough to pose interesting positions; explainability is the actual product. Three pillars:

1. **The engine** — Stockfish-11 classical port (NNUE banned). 2000 ELO verified empirically. Search has most of the SF11 pruning stack; eval decomposes into 45 named sub-terms keyed by `TermId`, each with mg/eg components and a per-term tapered cp delta the teaching layer reads.
2. **The teaching layer** — [`core/engine/src/analysis/`](core/engine/src/analysis/) — see that module's `//!` for the design principles. Traces every UI claim back to a concrete engine signal: term deltas, structured outcome snapshots, surprise classification, verdict.
3. **The narration crate** (`core/narration/`) — renders structured outcomes into prose. Public surface: `format_retrospective(pre_move_pos, &[MoveAnalysis], user_move, &NarrationOptions) -> String`.

UIs: CLI (`chess-tutor`), egui desktop (`chess-tutor-desktop`), planned Apple + Android. FFI crate (`core/ffi/`) is the prerequisite for the platform apps and doesn't exist yet.

Tests: **822 engine (+4 ignored) + 105 narration + 33 cli + 27 ui = 987 passing**, clippy clean across all targets.

## Status: the engine detour is COMPLETE — teaching UX is the active work

The teaching UX (below) was parked behind a three-part engine detour (the coach could only reason from static eval, so it couldn't honestly say "you missed a tactic" / "you had a mate"). All of it has now landed:

- **SF11 parity** ✅ — engine is functionally faithful to Stockfish 11 (node gap d=14 1.48× / d=20 2.04× SF; two correctness bugs fixed + the SF11 pruning stack landed as balanced bundles). Full historical log: [`parity-audit-log.md`](parity-audit-log.md).
- **Non-functional refactor** ✅ — every source `.rs` ≤ 500 LOC bar documented exceptions (`pawns.rs` 687, one cohesive eval term; data tables); tests in sibling files.
- **lichess tactic-library port** ✅ — the full taxonomy worth porting is engine-available (waves 1–6). Git history carries the wave-by-wave detail; the durable summary is memory `project_tactic_library_reference`.

### Engine-available tactic surface (what the teaching UX consumes)

All in [`core/engine/src/analysis/`](core/engine/src/analysis/); [`tactic_outcome/mod.rs`](core/engine/src/analysis/tactic_outcome/mod.rs) `//!` carries provenance (hand-transliterated from lichess `cook.py` under the idea/expression dichotomy — see CLAUDE.md).

- **`compute_tactic_outcome(best_ma, user_ma, pre_pos, root_stm, prior_move) -> TacticsOutcome`** — three independent `Option<TacticHit>` slots: `user_played_tactic`, `user_missed_tactic` (gated by a don't-nag win% test so it doesn't cry wolf), `user_walked_into`. `prior_move: Option<PriorMove>` feeds the recapture guard (a real retrospective caller passes the opponent's actual prior move).
- **`TacticHit`** = `{ pattern: TacticPattern, mate_pattern: Option<MatePattern>, sacrifice: bool, primary_piece, targets, material_gain, confidence, pv_ply }`. `pattern` names the lesson; `mate_pattern` / `sacrifice` are orthogonal annotations that ride alongside (a fork-into-back-rank-mate is `Fork` + `Some(BackRank)`, exactly as lichess tags both).
- **`TacticPattern`** (each has `heading()`): Fork, HangingCapture, RemovingDefender, TrappedPiece, Pin, Skewer, DiscoveredAttack, DiscoveredCheck, DoubleCheck, Sacrifice, Intermezzo, Deflection, Attraction, Interference, Clearance, XRay, AttackingF2F7, UnderPromotion, Checkmate.
- **`MatePattern`** (terminal-node mates; `heading()` + `surfaced_by_default()` → `true` only for BackRank/Smothered, the rest engine-available but a named-library for later): BackRank, Smothered, Anastasia, Hook, Arabian, Boden, DoubleBishop, Dovetail.
- **`find_overloaded(pos, victim) -> Vec<OverloadedPiece>`** ([`overloading.rs`](core/engine/src/analysis/overloading.rs)) — a *pre-move scan* (strict sole-defender-of-≥2), deliberately **not** in the tactic chain; its own analytical surface (a future coaching card / overlay).
- **Trapped-piece overlay, engine side:** `OverlayData.{white,black}_trapped` + `analysis::trapped_cages(pos, colour)` (per-piece dead-escape "cage").
- **`win_chances(Value) -> f64`** ([`win_chances.rs`](core/engine/src/analysis/win_chances.rs)) — lila cp→win% sigmoid (normalizes our PAWN_EG=213 → conventional pawn=100 first).

### NEXT: surface all of this in the UI (the now-active teaching UX)

None of the engine surface above is wired into the UI yet. The work: a `RetrospectiveCategory::Tactic` card; the `Checkmate`/`MatePattern` mate cards; the flagship **`TrappedPieces` board overlay** (engine side done — see HANDOFF-ux "Trapped-piece overlay"); an overloaded-piece coaching card/overlay; coaching-panel pattern names. Start from **[`HANDOFF-ux.md`](HANDOFF-ux.md)**. The product has three teaching surfaces below, all card-based and all reading the same engine outcomes — merged from `main` while parked, so re-validate against the current tree as you go:

1. **Retrospective panel** — after-the-fact analysis of the user's last move. Cards per signal (material, threats, king safety, mobility, pawn structure, passed pawns, piece placement, secondary, **forced consequences of opponent's best reply**). Best-move reveal is opt-in (`LearningPreferences.reveal_best_moves`, default off).
2. **Coaching panel** (live) — features-to-notice for the position the user is about to move from. Shown when `AssistanceLevel::Coached` is active. Surfaces hanging-piece opportunities (filtered through legal moves so in-check / pinned cases don't lie), en-passant captures, pawn weaknesses on either side, and a "your king is in check" card. Never names a move.
3. **Game Review** (post-game / on-demand) — ranked list of significant moments derived from the same classifier that drives in-game intervention. Click any moment to jump the rest of the UI there.

Plus the **intervention pause**: when `MistakeHandling::TeachingMoments` or blunder safety is on, the engine reply is held after a user move until the classifier decides whether to pause and surface a "you missed something / take back / continue" prompt. Gate is tight by design — single dominant eval-term family + share threshold + position-not-hopeless, so noise / engine subtlety don't interrupt play.

→ **[`HANDOFF-ux.md`](HANDOFF-ux.md)** — teaching layer state, learning-mode design, deferred Phase 2/4/5 work, narration tuning, UX platform tasks, live-play tuning loop. Read this when iterating on teaching UX.

→ **[`HANDOFF-perf.md`](HANDOFF-perf.md)** — engine perf levers tested + reverted, outlier breakdowns, deferred perf/strength opportunities. **Its bench numbers predate the W1 parity audit — [`parity-audit-log.md`](parity-audit-log.md) holds the current figures.** Read only when returning to engine perf / strength work; stable but noisy.

**Open thread for the teaching UX:** persistence design — game history on disk so past games are reviewable across launches and the foundation for drills / per-concept mastery fading. Desktop and mobile storage models differ (filesystem vs platform storage); user erase / clear-history UX needs design before code lands. See HANDOFF-ux's "Persistence (deferred)" section.

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
- **Tactic library (engine)**: [`core/engine/src/analysis/tactic_outcome/mod.rs`](core/engine/src/analysis/tactic_outcome/mod.rs) `//!` (predicate provenance; types + `compute_tactic_outcome`), `detectors.rs` (the per-pattern chain), `mate.rs` (named mates); plus [`overloading.rs`](core/engine/src/analysis/overloading.rs) + [`win_chances.rs`](core/engine/src/analysis/win_chances.rs)
- **Trap library schema + four-gate validator**: [`core/engine/src/traps/mod.rs`](core/engine/src/traps/mod.rs) `//!`
- **Engine public API surface**: [`core/engine/src/engine.rs`](core/engine/src/engine.rs)
- **Search structure + pruning stack**: [`core/engine/src/search/`](core/engine/src/search/) (`mod.rs` `//!` + per-phase files: `negamax`, `pre_loop`, `move_loop`, `move_search`, `loop_helpers`, `qsearch`, `run`, `settled`, `state`)
- **Move picker pipeline**: [`core/engine/src/movepick.rs`](core/engine/src/movepick.rs) `//!`
- **TT layout**: [`core/engine/src/tt.rs`](core/engine/src/tt.rs) `//!`
- **Repo layout, mission, ground rules**: [`CLAUDE.md`](CLAUDE.md)
