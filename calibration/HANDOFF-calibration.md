# Calibration harness — internals reference

Durable reference for the offline `dials → Elo` measurement harness. The
**live calibration work** (current grid run + the lookup bake) lives in the
repo-root [`HANDOFF-solver.md`](../HANDOFF-solver.md); the perception-era
ladder findings are in [`HANDOFF-perception.md`](../HANDOFF-perception.md).
This file is just "how the harness is built." For the tooling download list +
Maia anchor findings see [`README.md`](README.md).

---

## What it does

Measures a bot config's strength by playing it against the Maia ladder
(fastchess gauntlet), rating the PGNs with Ordo anchored to measured Maia
ratings, and emitting `config → Elo`. The product goal it feeds: a single
**"opponent Elo" slider** built by inverting the measured surface.

## Stack

fastchess (match runner, UCI-only, ~16-conc) + lc0 + 9 Maia nets (human
anchors, `go nodes 1` = pure policy) + Ordo (rating). All fetched by
`fetch-tools.sh` into `calibration/{tools,nets,books}` (git-ignored). venv at
`.venv` (Python 3.14 + numpy/scipy/sklearn/pandas/matplotlib).

## Modules (`harness/`)

- **`paths.py`** — robust tool/net/book/binary resolution.
- **`engines.py`** — `BotConfig` (every dial → `chess-tutor uci` args) +
  `MaiaEngine` (lc0 + net, `go nodes 1`).
- **`pools.py`** — `opponent_pool()` = 9 Maia + reference rungs (`ref-floor`
  … `ref-d10` ceiling), plus mask groups. The pool is what configs are rated
  against; it must span the configs' range or weak configs go all-loss and
  Ordo can't rate them.
- **`experiment.py`** — the shared driver (`run_and_rate`). **Load-bearing
  design: the seed-swap** — opponents are the fastchess gauntlet *seeds*,
  configs are *non-seeds*, so configs play the pool but NOT each other (they
  connect transitively through it). Keeps games ≈ configs × pool, not
  configs². Auto-sizes batches to the Windows command-line limit
  (`_safe_batch_size`). Batched, **skip-if-complete resume**.
- **`gauntlet.py`** — builds/runs one fastchess gauntlet. Two-sided resign +
  draw adjudication + `-maxmoves` + `-recover`. `pgnout append=false`.
- **`rate.py`** — Ordo wrapper. **Anchoring: `-a`/`-A` (single hard) OR `-y`
  loose multi-anchor — Ordo FORBIDS both.** Loose-anchor file is
  `"Player",Rating,Uncertainty` (3 cols). **Auto-excludes all-win/all-loss
  players** (no finite Elo) via `-x`, iteratively.

**Throughput:** ~49 games/s on a realistic depth/noise mix at concurrency 16.
(Depth-1 opening batches mislead at ~200 g/s — always measure on a mixed-depth
load.)

## Anchoring gotchas (critical, easy to get wrong)

- Only **maia1/maia5/maia9** have *measured* human ratings (Lichess rapid
  ~**1565 / 1680 / 1855**). The other 6 nets have only their band label, and
  the label→measured gap is **non-uniform** (+465 at 1100, ~−45 at 1900) — do
  NOT anchor on band labels. See [`README.md`](README.md) for the full table.
- Single-anchor makes the pool **compress** vs the human scale. **Loose
  multi-anchoring on all 3 measured points fixes it** — production default.
- **Weak rungs float.** A rung that loses ~100% to everything above it floats
  down hundreds of Elo in a sparse pool (info ∝ p(1−p), so lopsided links
  carry ~no scale information). Fix: measure extremes **densely with boundary
  anchors** — a dense self-connected sub-ladder pinned to 2-3 stable rungs.
- **Maia is a noisy ruler:** non-transitive and compressed (~290-Elo measured
  span). Absolute calibration is ±~100 regardless; optimize SHAPE (even
  spacing) and let chess.com feel-tests pin the offset.

## Engine levers (all play-engine-only)

Strict invariant: **analytical engines (retrospective / hint / analyze) NEVER
read these** — they use full strength so teaching judges true best play.
Current lever set (miss%/blunder% were removed — perception subsumes them):

| Lever | What | Where |
|---|---|---|
| **depth** | IDS depth (a high floor; d1 ≈ 1750 no-noise) | `OpponentProfile.depth` |
| **qsearch-depth** | quiescence (tactical-vision) horizon; `Some(0)` blind, `None` full | `SearchParams.qsearch_max_plies` → `search/qsearch.rs` |
| **perception** | move-visibility filter (humanly-missed moves pruned pre-search) | `core/engine/src/visibility.rs` (`//!` for the model) |
| **avg-move-rank** | play the Nth-best ranked move on average (variety) | `noise.rs` |
| **endgame-skill** | tier ladder withholding harder endgame specialists | `endgame/mod.rs` (`//!`) |
| **eval-mask** | 8 positional categories the bot is "blind" to (style, not strength; safety has teeth) | `opponent.rs` |
| **guaranteed-mate-in** | protection floor: always sees mate ≤ N | `noise.rs` |

### Inspecting a config's play
`chess-tutor uci --depth D --qsearch-depth Q [other dials]` then UCI stdin;
or `chess-tutor search "<FEN>" --depth D --qsearch-depth Q --perception P` to
see how a capped bot mis-evaluates a tactic.

## Repo rules carried
- `avg_move_rank` must be a **0.1 multiple** (GUI step); perception on a 0.05
  grid. Never anchor a rung the product can't reproduce.
- Edit `run_*.py` calibration scripts via the **Edit/Write tools only** — a
  heredoc `f.write()` over an existing run script silently failed to persist
  in one session (cause unknown; possibly an open handle from a concurrent
  run). New files wrote fine; overwrites of existing run scripts did not.
- Bench single-threaded; release builds for perf; commit straight to `main`.
