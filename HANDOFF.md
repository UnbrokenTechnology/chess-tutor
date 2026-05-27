# Handoff: chess-tutor-2

State index for fresh contexts. **Read [`CLAUDE.md`](CLAUDE.md) first** for evergreen guidance (mission, legal/licensing, ground rules); this file and its split-outs are forward-looking only — git history covers what's been built, inline module docs (`//!`) cover design rationale.

> **[`ROADMAP.md`](ROADMAP.md)** (temporary) — four sequenced big-rock workflows, a detour *out of* the teaching-UX work (the coach couldn't reason from the PV, so the plan is to port lichess tactic detection — which first needs a correct, clean engine). Status: **(1) SF11 parity audit ✅ COMPLETE** (gap closed to ~2× SF — the "~10x" was stale; see [`parity-audit-log.md`](parity-audit-log.md)); **(2) non-functional refactor ✅ COMPLETE** (every source file ≤500 LOC bar documented exceptions; see [`w2-refactor-log.md`](w2-refactor-log.md)); **(3) lichess tactic-library port — 🟡 IN PROGRESS** (Ship 1 *engine* surface landed: `analysis/tactic_outcome.rs`); **(4) broader lichess feature audit — 🟡 research pass ✅ COMPLETE** ([`w4-audit.md`](w4-audit.md): port/reference/skip verdicts + flagship trapped-piece plan + 6-wave engine-availability sequence; implementation is the follow-on). Order matters; read before starting any of them.

## What this app is

A **chess tutor**, not a chess engine. The product surface is move-by-move teaching feedback for ~1200 ELO students climbing toward the 1600+ range. Strength is a means: 2000-ish ELO is enough to pose interesting positions; explainability is the actual product. Three pillars:

1. **The engine** — Stockfish-11 classical port (NNUE banned). 2000 ELO verified empirically. Search has most of the SF11 pruning stack; eval decomposes into 45 named sub-terms keyed by `TermId`, each with mg/eg components and a per-term tapered cp delta the teaching layer reads.
2. **The teaching layer** — [`core/engine/src/analysis/`](core/engine/src/analysis/) — see that module's `//!` for the design principles. Traces every UI claim back to a concrete engine signal: term deltas, structured outcome snapshots, surprise classification, verdict.
3. **The narration crate** (`core/narration/`) — renders structured outcomes into prose. Public surface: `format_retrospective(pre_move_pos, &[MoveAnalysis], user_move, &NarrationOptions) -> String`.

UIs: CLI (`chess-tutor`), egui desktop (`chess-tutor-desktop`), planned Apple + Android. FFI crate (`core/ffi/`) is the prerequisite for the platform apps and doesn't exist yet.

Tests: **802 engine (+4 ignored) + 105 narration + 33 cli + 27 ui = 967 passing**, clippy clean.

## Current focus: executing the ROADMAP (teaching UX parked)

We are through the first two of the four-workflow [`ROADMAP.md`](ROADMAP.md) detour. **W1 (SF11 parity audit) is ✅ complete** — done-criteria met (d=14 1.48× SF, d=20 2.04× SF), two correctness bugs fixed plus the SF11 pruning stack landed as balanced bundles; full log in [`parity-audit-log.md`](parity-audit-log.md). **W2 (non-functional refactor) is ✅ complete** — every source `.rs` file ≤500 LOC bar documented exceptions (`pawns.rs` 687, one cohesive eval term; data tables), tests in sibling files, no logic/perf/test-count change. 18 commits; the final two were the checkpoint files — `search.rs` (decompose `negamax` in place → split into `search/`) and `session.rs` (split into `session/`), engine bench node-neutral (d=14 = 9,739,495), 893 tests, no new clippy warnings; full log in [`w2-refactor-log.md`](w2-refactor-log.md). **W3 (lichess tactic-library port) is 🟡 IN PROGRESS** (W4 broader audit follows) — the teaching UX stays **parked until W4 completes**. See [`HANDOFF-ux.md`](HANDOFF-ux.md) "Tactic library design brief" for W3. **Ship 1's engine surface has landed**: [`core/engine/src/analysis/tactic_outcome.rs`](core/engine/src/analysis/tactic_outcome.rs) — `compute_tactic_outcome(best_ma, user_ma, pre_pos, root_stm, prior_move) -> TacticsOutcome` (played / missed / walked-into slots), with Fork + RemovingDefender + HangingCapture detectors (direct `cook.py` ports, no new search). The hanging-capture recapture false positive is fixed — `prior_move: Option<PriorMove>` ports lichess's `op_capture` guard (a real retrospective caller must pass the opponent's actual prior move). **W4 research pass is ✅ complete** (2026-05-26) — see [`w4-audit.md`](w4-audit.md): every `cook.py` tag / `util.py` primitive / zugzwang / generator / validator / sibling repo classified port-reference-skip, split by teaching value vs. puzzle-bucketing plumbing, with the flagship **trapped-piece** plan (engine port of `is_trapped` + a board overlay — the per-escape-square bad/safe classification is the surfaceable intermediate data, answering memory `project_trapped_piece_visual_goal`) and a 6-wave engine-availability implementation sequence (flagship-first). **W4-impl waves 1–2 — engine side ✅ LANDED (2026-05-27):**
- **Wave 1 (trapped piece):** `is_trapped` ported into [`analysis/tactic_util.rs`](core/engine/src/analysis/tactic_util.rs) (shared lichess-util primitives extracted there); `TacticPattern::TrappedPiece` detector; overlay engine side = `OverlayData.{white,black}_trapped` + `analysis::trapped_cages` (per-piece dead-escape "cage"), via a user-approved null-move turn-flip so a trapped *enemy* piece shows on *your* move.
- **Wave 2 (core-8 completion):** `tactic_outcome.rs` split into a [`analysis/tactic_outcome/`](core/engine/src/analysis/tactic_outcome/) directory (`mod.rs` = types + `compute_tactic_outcome` + material accounting; `detectors.rs` = `detect_line_tactic` + every `detect_*`; `tests.rs`). Added detectors for **Pin, Skewer, DiscoveredAttack, DiscoveredCheck, DoubleCheck** (cook.py ports adapted to single-move framing), appended to the priority chain.
- **Wave 3 (Sacrifice + `win_chances`):** ported `win_chances` (lila cp→win% sigmoid) to [`analysis/win_chances.rs`](core/engine/src/analysis/win_chances.rs) — **normalizes our internal cp (PAWN_EG=213) to conventional pawn=100 before the sigmoid** (constant ported as-is; refit-on-classical-eval is a documented follow-up). Added `TacticHit.sacrifice: bool` + a standalone `TacticPattern::Sacrifice`, and `is_sacrifice` (port of `cook.py:sacrifice`: down ≥2 points by the mover's 2nd move, no opponent-promotion in the line). `compute_tactic_outcome` now reads the line eval: a material-down line that's **sound** (`win_chances(user score) ≥ 0`) surfaces as a *played* `Sacrifice` (when no geometric pattern fires) and **suppresses the spurious `user_walked_into`** — the one-ply-guarantee misfire fix (memory `project_threat_signal_revisit`). Scope was deliberately the tactic layer only; the static `threats_outcome` filter is unchanged (it has no eval access — a search-based hardening there is a separate effort). `compute_tactic_outcome`'s signature is unchanged (it already carried `best_ma`/`user_ma` with `.score`).
- **Wave 4 (second-wave patterns + don't-nag gates):** six **multi-ply** detectors in [`tactic_outcome/detectors.rs`](core/engine/src/analysis/tactic_outcome/detectors.rs) — `TacticPattern::{Intermezzo, Deflection, Attraction, Interference, Clearance, XRay}`, ports of the matching `cook.py` predicates. Unlike waves 1–2 (single key move), these read several plies: a `line_boards` helper replays the `pv` (`boards[i]` = after `pv[0..i]`), mirroring lichess's `mainline`/`parent`/`grandpa` navigation. They resolve at the mover's 2nd move (`pv[2]`) or 3rd (`pv[4]`, e.g. x-ray batteries), bounded to a 5-ply window (`WAVE4_MAX_PLIES`) so a named tactic stays attributable to the user's move; intermezzo additionally uses `prior_move`. Appended to the priority chain after the single-move patterns (the more immediate lesson wins the slot). Fixtures ported verbatim from lichess `tagger/test.py` and tested **per-detector in isolation** ([`detectors_tests.rs`](core/engine/src/analysis/tactic_outcome/detectors_tests.rs)) — the full chain returns one hit, but lichess assigns several tags, so isolation matches its true/false cases. Also added the **don't-nag gates** on `user_missed_tactic` (`missed_tactic_worth_flagging`): suppress unless the best move beats the user's by a real win% gap (`MISS_MIN_WC_GAP=0.15`) and the user isn't already winning (`ALREADY_WINNING_WC=0.80`) — lichess's generator uniqueness/"already winning" gates ported in spirit.

802 engine tests pass, clippy clean. **NEXT is W4-impl wave 5 (mate-pattern library: back-rank + smothered surfaced for 1200s; anastasia/hook/arabian/boden/dovetail engine-available but not surfaced by default — all terminal-node detectors), then wave 6 (optional: attackingF2F7, overloading-from-scratch, under-promotion, analytical-only zugzwang). All UI surfacing — `RetrospectiveCategory::Tactic`, the `TrappedPieces` overlay desktop wiring, coaching-panel names — stays deferred until the engine waves land.**

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
- **Tactic library (W3 Ship 1, engine)**: [`core/engine/src/analysis/tactic_outcome.rs`](core/engine/src/analysis/tactic_outcome.rs) `//!` (predicate provenance + the three detectors)
- **Trap library schema + four-gate validator**: [`core/engine/src/traps/mod.rs`](core/engine/src/traps/mod.rs) `//!`
- **Engine public API surface**: [`core/engine/src/engine.rs`](core/engine/src/engine.rs)
- **Search structure + pruning stack**: [`core/engine/src/search/`](core/engine/src/search/) (`mod.rs` `//!` + per-phase files: `negamax`, `pre_loop`, `move_loop`, `move_search`, `loop_helpers`, `qsearch`, `run`, `settled`, `state`)
- **Move picker pipeline**: [`core/engine/src/movepick.rs`](core/engine/src/movepick.rs) `//!`
- **TT layout**: [`core/engine/src/tt.rs`](core/engine/src/tt.rs) `//!`
- **Repo layout, mission, ground rules**: [`CLAUDE.md`](CLAUDE.md)
