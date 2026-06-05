# HANDOFF — ELO calibration + the tactical-vision engine pivot

Current-state snapshot for the bot-strength / ELO-calibration effort.
Read this + [`PLAN-elo-calibration.md`](../PLAN-elo-calibration.md) +
[`README.md`](README.md) before continuing. Memory file
`project_elo_calibration_harness` has the same story in condensed form.

**Date of this snapshot:** 2026-06-04 (one very long session).

---

## TL;DR — where we are

The goal is a single **"opponent Elo" slider**: drag to a target human
Elo, get a bot config that *plays like* that strength. To build it we
measure `dials → Elo` offline (bot configs vs the Maia ladder via
fastchess + Ordo) and fit a model we can invert.

**What changed this session (the big pivot):** we discovered the engine
was missing the *keystone* low-Elo lever and added it.

- ✅ **`qsearch-depth` lever landed** — a tunable quiescence (tactical-
  vision) horizon. THIS is how you make believable sub-1000 bots.
- ✅ **`wild` lever removed** — random move-picking was a bad,
  high-variance lever; qsearch-depth replaces its purpose.
- ✅ **GUI wired** — New Game dialog has a "Tactical vision" dropdown.
- ✅ **Harness plumbed** for qsearch-depth; pool floor = low-qdepth bots.
- ⏸️ **The full grid run was aborted** mid-flight (it was built on the
  old lever set: had `wild`, no `qsearch-depth`). Data discarded.
- ⬜ **NEXT: the qdepth-driven grid redesign** (user wants to weigh in on
  shape), then the multi-hour run, then the model fit + solver.

---

## The strength model (the mental model that now drives everything)

Human improvement has two distinct phases, and the engine now has a lever
for each:

| Axis | Lever(s) | Human analog |
|---|---|---|
| **Tactical horizon** | **depth × qsearch-depth** | 100→1000 (learn not to hang pieces / see tactics) |
| **Positional sense** | eval masks (8 categories) | 1000→2000 (learn structure, king safety, etc.) |
| **Human error** | miss / blunder(+severity) / avg-move-rank | realistic slips at any level |
| ~~Randomness~~ | ~~wild~~ — **REMOVED** | (humans don't play randomly) |

**Why this was the breakthrough:** every bot previously had *perfect
tactical vision* (full quiescence search resolves all captures at every
leaf — `negamax.rs:47`). So depth-1 already rates ~1800 and the only way
to score below ~1000 was to *force statistically-bad moves* (unrealistic).
Capping qsearch makes a bot **tactically blind** — it can't see that its
piece gets recaptured, so it hangs material like a real beginner. That's
the natural, low-variance, *believable* sub-1000 mechanism.

---

## Engine levers — what exists & where (all play-engine-only)

Strict invariant (like the existing eval-mask): **analytical engines
(retrospective / hint / analyze) NEVER read these** — they use full
strength so teaching feedback judges true best play.

### `qsearch-depth` (NEW — the tactical-vision dial)

- **What:** cap how many plies of captures quiescence search resolves
  before falling back to the static eval. `Some(0)` = tactically blind
  (hangs pieces); `Some(2)` ≈ sees the recapture; `None` = full vision.
- **Code:** `SearchParams.qsearch_max_plies: Option<u32>` →
  `Search.qsearch_cap` (resolved via `QSEARCH_UNBOUNDED` sentinel so the
  full-strength path is byte-identical) → enforced in
  `core/engine/src/search/qsearch.rs` (`if !in_check && depth <=
  -self.qsearch_cap { return best_score }`; never caps in check — must
  still find evasions). Lives on `OpponentProfile.qsearch_max_plies`,
  threaded into the play worker + the UCI shim.
- **CLI:** `--qsearch-depth N` on `chess-tutor uci` (harness) and
  `chess-tutor search` (inspection).
- **GUI:** New Game → "Tactical vision" combo (Full / 6 / 2 / 1 / 0).
- **Validated:** on a position where `Rxd5` looks like a free knight but
  loses to `exd5`, full qsearch scores −0.01 (sees recapture) while
  qdepth-0 scores **+6.45** (blind). And in a real game the d1/q0 bot
  hung its queen by move 9 (`Qxf6??` blind to `Qxf6`) and got mated by
  move 12 — Martin behavior, deterministically.
- **Commits:** `5990349` (lever), `1394d2e` (wild removal).

### Existing dials (unchanged)
- **depth** — IDS depth. A high *floor* (d1 ≈ 1750-1800 no-noise).
- **avg-move-rank** — play the Nth-best ranked move on average (variety).
- **blunder** chance + severity band (pawn/minor/queen via min/max material).
- **miss** chance — decline a forced material win.
- **guaranteed-mate-in** — never blunder mates ≤ N.
- **eval masks** — 8 categories the bot is "blind" to (pawn-structure,
  passed-pawns, space, pieces, mobility, king-safety, threats, initiative).

---

## KEY EXPERIMENTAL FINDINGS (the data that shaped the pivot)

### 1. depth × qsearch-depth Elo (best move, no noise, ~300 games, ±~65)

```
depth |   q0    q1    q2    q6   qinf
------+----------------------------------
 d1   |  879  1504  1666  1768  1800
 d2   | 1590  1626  1683  1788  1729
 d4   | 1812  1901  1957  2097  2045
 d6   | 2070  2277  2232  2328  2461
```
- **qsearch-depth is a clean, monotonic low-end lever** (d1 spans
  879→1800). **Matters most at low depth** (d6 range only ~390) — deep
  full-width search already sees tactics.
- `d2/q2 = 1683` = "positionally sharp, won't hang to recapture, misses
  deeper tactics" — a believable ~1700, *always playing best move*.
- `d4/q0 = 1812` — even modest depth-4 is too strong for a 1200 student;
  beatable bots need d1/d2.
- Some cells non-monotonic (d2 q6>qinf etc.) — within noise.
- Source: `run_qdepth_probe.py` → `runs/qdepth_probe/`.

### 2. Eval masks are a STYLE lever, not a strength lever

Across multiple experiments:
- **Masks barely move Elo** for most categories (≤~150 even at high depth);
  **`safety` (king-safety+threats) is the only one with consistent teeth**
  (~−90 to −230 everywhere).
- **The effect FLIPS by tactical level** (low-band mask experiment,
  `run_lowband_masks.py` → `runs/lowband_masks/`):
  ```
  base (Elo)     pawnspace  activity  safety  initiative
  d1-q0 ( 913)      +233      +243    -189      +42      <- masking HELPS the blind bot
  d2-q0 (1656)     -133      -111    -184      -58      <- masking HURTS the sighted bot
  d2-q2 (1767)      +82       +97     -37      +70
  ```
  **Story:** positional eval only helps a bot that can tactically support
  its plans. A fully-blind bot (d1/q0) chases positional goals (push
  pawns, activity) it can't back up, overextends, and hangs material —
  so masking positional eval makes it play *more solidly* (stronger).
  Give it a little vision (d2/q0) and positional eval becomes an asset.
- **Masks bite harder on tactically-CAPPED bots** at the same depth
  (qdepth probe: pawnspace −184 @ q2 vs −116 @ qinf) — limited calc leans
  more on positional eval. So qdepth + masks **compound**, not redundant.
- **Implication for the backbone:** do NOT rely on masks to make 1000-1500
  bots "positionally weak" — it's unreliable and can backfire. That band
  comes from **qsearch-depth**; masks are flavor (+ `safety` as a real
  handicap). The earlier wild-based mask run (`runs/masks_wild/`) showed
  masks go fully redundant when weakening is random.

### 3. wild was a bad lever
High outcome variance (could stumble into the best move / randomly stomp
you), unrealistic, and its only unique job (reaching moves outside the
top-10 MultiPV, e.g. a move-2 queen hang) is done far better, and
deterministically, by qsearch-depth-0. **Removed entirely.**

### 4. Martin (chess.com's 250 bot) target
`d1/q0 = 879` is "hangs pieces" weak but nowhere near 250. Martin-tier
needs qdepth-0 **stacked with** heavy rank/blunder/miss (e.g.
`d1-q0-r3 = 511`). We have all those levers.

---

## The harness (calibration/ — Python, offline)

**Stack:** fastchess (match runner) + lc0 + 9 Maia nets (human anchors) +
Ordo (rating). All downloaded by `fetch-tools.sh` into
`calibration/{tools,nets,books}` (git-ignored). venv at `.venv`
(Python 3.14 + numpy/scipy/sklearn/pandas/matplotlib).

**Modules (`harness/`):**
- `paths.py` — robust tool/net/book/binary resolution.
- `engines.py` — `BotConfig` (every dial → `chess-tutor uci` args; now has
  `qsearch_depth`, wild removed) + `MaiaEngine` (lc0 + net, `go nodes 1`).
- `pools.py` — `opponent_pool()` = 9 Maia + reference rungs. **Floor bots
  are now low-qdepth "Martins"** (`ref-d1-q0`, `ref-d2-q0`, `ref-d4-q1/q2`)
  replacing the old wild floor. Plus `MASK_GROUPS` (4 thematic groups:
  pawnspace / activity / safety / initiative) and `ALL_MASKS`.
- `experiment.py` — **the shared driver** (`run_and_rate`). THE
  load-bearing design: the **seed-swap** — opponents are the fastchess
  gauntlet *seeds*, configs are *non-seeds*, so configs play the pool but
  NOT each other (they connect through it). Keeps games ≈ configs × pool
  (not configs²). Auto-sizes batches to the Windows command-line limit
  (`_safe_batch_size`). Batched, skip-if-complete resume.
- `gauntlet.py` — builds/runs one fastchess gauntlet. Two-sided resign +
  draw adjudication + `-maxmoves` + `-recover`. `pgnout append=false`.
- `rate.py` — Ordo wrapper. **Anchoring: `-a`/`-A` (single hard) OR `-y`
  loose multi-anchor — Ordo FORBIDS both.** Loose-anchor file is
  `"Player",Rating,Uncertainty` (3 cols). **Auto-excludes all-win/all-loss
  players** (no finite Elo) via `-x`, iteratively.

**Experiment scripts (calibration/):**
- `run_qdepth_probe.py` — depth × qsearch-depth Elo + masks-on-qdepth.
- `run_lowband_masks.py` — masks on low-band (d1/d2 × small qdepth) bots.
- `run_grid.py` / `harness/grid.py` — the OLD move-quality grid (wild
  stripped, but NO qsearch-depth dimension yet). **Superseded — gets
  rebuilt in the redesign.**
- `run_masks.py` — depth-pure + rank mask experiment (older).
- `pilot.py`, `bench_rate.py` — earlier pilots/benchmarks.
- `progress.ps1` — live PowerShell progress watcher (instantaneous rate
  from successive samples; total is grid-specific, re-derive if changed).

**Throughput:** ~49 games/s on a realistic depth/noise mix at concurrency
16 (the depth-1 opening batches mislead at ~200 g/s — always measure on a
mixed-depth load).

### Anchoring gotchas (critical, easy to get wrong)
- Only **maia1/maia5/maia9** have *measured* human ratings (Lichess rapid
  ~**1565 / 1680 / 1855**). The other 6 nets have only their band label.
- The label→measured gap is **non-uniform** (+465 at 1100, ~−45 at 1900) —
  do NOT anchor on band labels.
- Single-anchor on maia-1500 makes our pool **compress** vs the human
  scale (maia-1900 landed 1784 vs measured 1855). **Loose multi-anchoring
  (`rate(loose_anchors=...)`) on all 3 measured points fixes it** — it's
  the production default; the tiny-grid test placed maia-1100/1500/1900 at
  1563/1675/1861 vs measured 1565/1680/1855.
- Still pending: re-check Maia measured ratings (they drift) + decide the
  **lichess→chess.com offset** (a pure post-fit shift; our user is a
  chess.com 1200).

### Agent CLI for inspecting a config's play
`chess-tutor uci --depth D --qsearch-depth Q [other dials]` then UCI
stdin; or `chess-tutor search "<FEN>" --depth D --qsearch-depth Q` to see
how a capped bot mis-evaluates a tactic.

---

## The model-fitting plan (discussed, not yet built)

The levers **interact** (masks help weak bots, hurt strong ones — a
sign-flip), so a linear additive model is wrong. Plan = run THREE fits on
the `config → Elo` table (a few thousand rows; the games only sharpen each
Elo) and compare via cross-validation:
1. **Symbolic regression** (PySR / gplearn) — searches math expressions
   (+,×,/,log,exp,min, powers), returns a *human-readable equation*. The
   tool for the interpretable backbone. (Sign-flip needs an offset like
   `mask·(k−depth)`, not just `mask/depth`.)
2. **Gradient-boosted trees** (sklearn `HistGradientBoosting`) — auto-learns
   all interactions/caps/sign-flips; THE forward model the solver inverts;
   read structure via partial-dependence / SHAP.
3. **Engineered-linear + LASSO** — hand-built candidate features
   (`log depth`, `min(depth,6)`, `mask·depth`, `mask/qdepth`, …); LASSO
   auto-selects; readable coefficients.
Accuracy is **capped by per-config Elo noise** (±~50 at 300 games) — the
games-per-config budget sets the model's best-case RMSE. Saturation
(depth flat above ~6-8) also informs grid *design*: sample densely where
the response moves fast (low depth/qdepth), sparsely where flat (we capped
grid depth at 6). A `fit.py` scaffold (3 models + cross-val + plots) is
proposed but NOT built — offered to fold in.

---

## Backlog ideas

- **Human-perception qsearch filter (great idea, deferred):** instead of a
  *uniform* qdepth cap, gate whether qsearch resolves a capture by its
  GEOMETRY (we have from/to → direction, distance, piece type). Real
  low-Elo biases: diagonal captures (bishop/pawn) harder than orthogonal
  (rook); far captures harder; backward/horizontal harder; knight moves
  harder. Makes weak bots miss the *human-missed* tactics → believable
  FEEL. Determinism-safe via a deterministic visibility threshold OR a
  seeded-per-(position,move) roll (same position → same blind spot).
  FRAMING: a realism/feel lever (which tactics missed), mostly orthogonal
  to Elo — like masks are style not strength. Build AFTER the qdepth grid.
- **Tunable check extensions** (user "MAY want") — secondary tactical-
  horizon knob; deferred until we see if qsearch-depth alone suffices.

---

## NEXT STEPS (in order)

1. **qdepth-driven grid REDESIGN** (user wants to weigh in on shape):
   primary tactical axis = **depth × qsearch-depth** ({0,1,2,6,inf}
   confirmed for qsearch-depth), crossed with the realism dials
   (miss/blunder/avg-rank) and eval masks handled as a separate ceiling
   experiment. Settle exact values + resolution + opponent pool, then
   rebuild `grid.py`/`run_grid.py` around it.
2. **Run the multi-hour grid** on the new lever set (resumable; watch via
   `progress.ps1`).
3. **Fit** (`fit.py`: symbolic + GBT + LASSO, cross-validated) → forward
   model + readable structure.
4. **Constrained solver** honoring user-set bands/binaries (the
   human-realism policy applied at solve time; editable forever).
5. Resolve the Maia-anchor / lichess→chess.com-scale loose ends.

## Commit pointers (this session, on main)
`5990349` qsearch-depth lever · `1394d2e` remove wild · GUI-wire commit ·
`f145d2b`+ harness · qdepth-probe + lowband-mask commits. (See `git log`.)
