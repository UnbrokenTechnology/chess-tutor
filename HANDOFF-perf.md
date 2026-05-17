# Handoff: chess-tutor-2 — engine perf / strength

Forward-looking engine perf and strength context. **Read this only when returning to perf or strength work.** For current UX-focused iteration see [`HANDOFF-ux.md`](HANDOFF-ux.md); for the project overview and build commands see [`HANDOFF.md`](HANDOFF.md).

## Threading policy — single-thread default (2026-05-16)

**All shipped surfaces (desktop + CLI) default to `threads = 1`.** The Lazy SMP code stays in tree but the multi-thread path is opt-in only (`chess-tutor play --threads N`, `chess-tutor bench <tt> <threads> <depth>`, `chess-tutor noise-bench --threads N`). Engine moves, retrospective, hint panel, and analyze all run single-threaded by default.

Three reasons:

1. **Determinism (the load-bearing one).** Lazy SMP introduces enough per-run score variance that the same move at the same position gets different verdicts across runs and after takebacks — a major teaching disconnect. UX-position noise bench (`chess-tutor noise-bench --fen-file noise_bench_ux_positions.txt --runs 8`) measured p50 = 51 cp / p95 = 121 cp / max = 143 cp same-move score range at 8 threads on typical user-facing positions (openings + quiet middlegames + simple endgames). On the SF11 tactical bench the variance is worse — p95 = 469 cp, with mate-puzzle positions sometimes finding the mate and sometimes returning a non-mate eval at the same depth, swing of ~29000 cp. Widening verdict bands can absorb opening noise but can't fix missed-mate flapping. Single-thread = 0 cp variance, every position, every run.

2. **iOS deployment target.** Single-core utilisation is much friendlier to the iPhone thermal/battery envelope than spinning N cores. Memory savings too — each `WorkerState` carries ~8 MB of cont-history; at 4 threads that's ~32 MB resident.

3. **Cost is small.** At the desktop's default depth = 10, single-thread retrospective is ~120 ms worst case (vs ~60 ms multi-thread) — well inside "feels instant". Engine moves at depth 10 are ~40 ms single-thread. Deeper depths cost more (the user's "1-2 s" memory was depth 20 testing for mobile worst-case modelling).

**Atomics aren't the speed cost they look like in VTune.** Our TT uses `Ordering::Relaxed`, which on x86 and ARM64 compiles to identical machine code as plain loads/stores — zero overhead vs non-atomic. The phantom "atomic_load is a hotspot" attribution in VTune is documented in `tt.rs:88-97`: it was actually function-call overhead from a non-inlined wrapper, fixed by `#[inline(always)]`. Removing the `AtomicU64` types today would produce bit-identical machine code. The multi-thread scaffolding has a real *memory* cost (per-thread `WorkerState`) but not a per-instruction speed cost.

**`chess-tutor noise-bench`** measures Lazy SMP variance for calibration; useful any time we change search parameters or want to re-confirm the single-thread choice. Source: [`core/cli/src/noise_bench.rs`](core/cli/src/noise_bench.rs).

## Engine perf — current state (2026-05-14)

Seven major changes landed today:
1. **Lever 1: universal `moveCountPruning`** tamed the FEN 26 cold d13 cliff (484 M → 226 k).
2. **Lever 2b: SF11 lmrDepth-gated quiet futility** collapsed the residual deep-tail problem at d14 (104 M → 20.5 M aggregate, 5× fewer nodes; FEN 40 alone 22 M → 466 k, 47× faster).
3. **Unified SF11 LMR formula** replaced our `log₂·log₂/2` base with SF11's `int(23.4·ln(i))` table form — direct response to FEN 19 regressing 290× under raw Lever 2b because our smaller `lmrDepth` made the SF11 `< 6` gate fire in nodes SF11 wouldn't fire on. With matched LMR base, the gate behaves as SF11 intended.
4. **SF11 qsearch depth tracking + recapture-only mode** — qsearch now takes `Depth`, decrements by 1 each recursive call (SF11 search.cpp:1522), and at `Depth::QS_RECAPTURES (-5)` the picker filters to moves landing on the parent's to-sq (search.cpp:1459). FEN 19 d=20 391 M → 7.8 M (50×); FEN 41 d=14 16 MB 44 M → 1.45 M (30×).
5. **SF11 aspiration depth-reduction on fail-high** (search.cpp:453) — consecutive fail-highs reduce the re-search depth via `adjusted_depth = max(1, depth - failed_high_cnt)`. FEN 20 d=20 36.8 M → 10 M (3.7×); full d=20 bench 145 s → 116 s. SF11's `21 + |prev|/256` initial delta and `delta + delta/4 + 5` growth both regressed FEN 26 d=13 by 3× on our codebase — kept our existing `delta=17` + `2×` growth; depth-reduction is the only load-bearing piece of the aspiration port.
6. **Lazy SMP multi-threading** — `SearchParams.threads: usize` (default 1) controls how many parallel search threads run. Stockfish-style: main thread does iterative deepening and returns the result; `threads - 1` helper threads run the same loop on per-thread `WorkerState` (history / counter-moves / cont-history / capture-history / pawn-cache) and contribute only via the shared TT. Stop signal is a `Arc<AtomicBool>` set when main thread finishes. `Engine` now holds `Vec<WorkerState>` that grows on demand. CLI `bench <tt> <threads> <depth>` passes the second argument through (was previously rejected); CLI `play --threads N` exposes it for engine-move searches.
7. **Multi-threaded retrospective / hint panel** — retrospective (CLI auto-retrospective + desktop GUI retrospective panel) and the desktop hint panel now default to `available_parallelism()` threads. The teaching output (positional eval term deltas, tactic detection, verdict thresholds, missed-tactic / opponent-tactic enumeration) is robust to the small per-move-score variance Lazy SMP introduces — alternate moves near the best may swap rank between runs but the narrative is the same. `chess-tutor play --deterministic` collapses the retrospective back to single-thread for callers who need bit-identical narration across runs. Multi-PV=3 d=14 on FEN 20 dropped 880 ms → 226 ms (3.9× faster); typical middlegame retrospective ~398 ms → 276 ms (1.4× faster).

### Where we stand vs SF11

| | d13 nodes | d13 time | d14 nodes | d14 time |
|---|---|---|---|---|
| **SF11 (46 FENs, 128 MB shared TT)** | — | — | 6.93 M | 2.2 s |
| **Us pre-Lever-2b (16 MB cold per pos)** | 17.5 M | 5.4 s | 104.2 M | 21.1 s |
| **Us Lever 2b raw (16 MB cold per pos)** | 10.5 M | 4.1 s | 20.5 M | 7.2 s |
| **Us qs-depth + unified-LMR (16 MB cold per pos)** | 8.4 M | 3.8 s | 14.2 M | 6.4 s |
| **Us aspiration-depth-reduce (16 MB cold per pos)** | **8.4 M** | **3.8 s** | **12.0 M** | **5.2 s** |
| **Us aspiration-depth-reduce (128 MB cold per pos)** | — | — | **13.1 M** | **6.5 s** |

Full 45-pos d=20 / 128 MB cold: **115.9 s** (was 145.7 s reported by user pre-aspiration-fix, was multi-hour pre-Lever-1). The d=20 worst-position list has flattened: largest position is now FEN 1 (28.9 M / 17.5 s) at d=20, which is a real-search startpos cost rather than a pathology.

NPS still ~2.0 Mnps single-threaded (vs SF11's 3.1 Mnps). That gap is the remaining 30%-or-so headroom — diffuse across all positions, not concentrated in any one outlier.

**Multi-threaded numbers** (this machine, 24 logical cores, 128 MB shared TT):

| | d=14 bench (45 pos) | d=20 bench (45 pos) |
|---|---|---|
| 1 thread | 6.5 s | 116.8 s |
| 2 threads | 5.1 s (1.27×) | — |
| 4 threads | 3.7 s (1.77×) | 71.1 s (1.64×) |
| 8 threads | 3.1 s (2.11×) | **43.0 s (2.72×)** |

8-thread d=20 is 2.72× faster than 1-thread, and **3.4× faster than the 145.7 s baseline** the user reported before the aspiration fix. Per-position variance is high under Lazy SMP (FEN 20 d=20 at 8 threads ranges 1.7 s – 26 s across 5 runs); aggregate numbers are stable because high and low variance positions average out across the 45-pos set.

### Known residual outliers

The "pathological outlier" class is essentially gone. Per-position d=20 / 128 MB cold (post-aspiration-depth-reduce):

| FEN | Description | d=20 nodes | d=20 time |
|---|---|---|---|
| 1 | startpos | 28.9 M | 17.5 s |
| 41 | K+2R vs K+Q+p | 23.8 M | 9.7 s |
| 19 | K+R race w/ rep | 10.9 M | 3.3 s |
| 12 | middlegame | 10.5 M | 6.5 s |
| 20 | K+Q+4p endgame | 10.0 M | 3.7 s |

These all cluster in the 10–30 M range — healthy deep-search costs, not chain blowups. FEN 1 being the new worst is surprising but it's startpos at d=20, which is inherently expensive (broad PV).

### MultiPV-around-mate pathology (FIXED 2026-05-16)

UX playthrough surfaced a pathology orthogonal to the bench outliers: **MultiPV ≥ 2 on a position where #1 is a forced mate runs unboundedly**. Pinned the desktop GUI mid-game while the retrospective tried to rank the user's move against alternatives.

**Reproducer** (`chess-tutor search "..." --depth N --multi-pv K`):

```
FEN: 4Rb2/p5p1/1p2Q3/2kN2q1/B1p5/8/PPPP1PPP/R1B3K1 w - - 3 24
```

| Config | Pre-fix | Post-fix |
|---|---|---|
| `--multi-pv 1 --depth 10` | 1.8 k nodes / 2 ms | 1.8 k nodes / 2 ms |
| `--multi-pv 2 --depth 10` | hung past 10M-node cap | 22 ms |
| `--multi-pv 3 --depth 10 --force-include Rc8+` | hung past 100M-node cap | 23 ms |

**Root cause** was not the aspiration delta tuning the original theory pointed at. It was a missing SF11 step-13 outer gate on the SEE-pruning-of-losing-captures site at `search.rs:1423`. The other two step-13 prunes (cmp_prune, futility) already carried the `best_score > Value::MATED_IN_MAX_PLY` gate; SEE was the outlier.

The chain:
1. At ply ≥ 1 with the side-to-move in check and only one legal evasion that happens to be a SEE-negative capture (typical in heavy-material winning positions where every black escape lands the king on a square attacked by a white piece), the move is pruned before being searched.
2. `best_score` stays at its initial `-Value::INFINITE` sentinel through the move loop. The `move_count == 0` early-return doesn't fire because move_count *was* incremented (the prune happens after the increment, mirroring SF11). We fall through to the TT save.
3. `value_to_tt(-INFINITE, ply)` produces `-INFINITE - ply`, which `value_from_tt` later reads back as `-INFINITE` at the storage ply. The TT cutoff returns `-INFINITE`; the parent negates to `+INFINITE = 32001`.
4. Up the recursion the chain propagates as alternating ±INFINITE until it reaches the root, where the aspiration loop sees `score = INFINITE = 32001`, fails high, sets `beta = min(score + delta, INFINITE) = INFINITE` (saturated), and re-searches — getting the same INFINITE answer every iteration. Delta doubles unboundedly; adjusted_depth pegs at 1; the loop never converges because beta can't widen past its saturation cap.

SF11 doesn't hit this because step 13 in `evaluate.cpp` has a single outer gate that protects *all* the shallow-depth prunes (CMP, futility, SEE on quiets, SEE on captures). We had ported the gate onto the first three but missed it on the captures branch. With `best_score > MATED_IN_MAX_PLY` added, the first move at every node always reaches the search and updates best_score before any subsequent move can be pruned.

**The fix is two lines**:

```rust
if !is_root
    && is_capture
    && depth <= 6
    && !gives_check
    && best_score > Value::MATED_IN_MAX_PLY        // ← added (SF11 step 13 outer gate)
    && pos.non_pawn_material(us_at_node).0 > 0     // ← added (matches the other two prune sites)
{ ... }
```

Bench impact at d=13 / 16 MB / 1T cold (45-pos): **7.46 M / 3.26 s** (was 8.4 M / 3.8 s, −11% nodes / −14% time). FEN 26 d=13 152 k (was 138 k, +10% within noise). d=20 / 128 MB / 1T cold: **200 M / 104 s** (was 226 M / 116 s, −12% nodes / −10% time). d=20 / 128 MB / 8T: 39–50 s across 3 runs (was 43 s; within Lazy SMP variance). All 827 tests pass, clippy clean.

The original aspiration-delta-tuning theory at the top of this section was wrong; keeping the historical write-up below for posterity since the symptom analysis still applies if a similar mate-related blowup ever returns. The `searchAgainCounter` half-port and the per-iteration depth-cap fallback are no longer needed for this bug. The 100 M / 10 s safety caps in `retrospective.rs` / `desktop/src/main.rs` stay as a backstop for future unknown unknowns.

**Side-band display bug still open:** `format_score` in `core/cli/src/play.rs` and `core/engine/src/types.rs` mate-distance branch displays `#0` for a 1-ply mate. The math `plies_to_mate = mate - abs` and `moves = (plies+1)/2` is right, but the score being formatted is sometimes `MATE` (32000) instead of `MATE - 1`. Unrelated to the pathology — a separate display polish item.

---

**Historical theory (original 2026-05-16 write-up, kept for reference — the actual root cause was different):**

The original theory was that with `prevScore ≈ 32000` (mate value), SF11's `delta = 21 + 32000/256 ≈ 146` would put the aspiration window at `[mate-146, mate+146]` and any non-mating alternative for PV[2]/PV[3] would by definition be >32000 cp worse, so every aspiration attempt for the secondary PVs would fail low and rewidens. SF was theorized to survive this because the depth reduction kicks the per-attempt cost down each iteration. This turned out to be largely irrelevant once the real bug (SEE-prune missing outer gate) was found, since the prev_score for PV[1] in the repro was actually ~6900 cp (a non-mate score), not 32000.

### Outlier-position breakdown (d14, post-Lever-1)

Most of the d14 overshoot lives in three positions. From the user's last 45-pos d14 run:
- FEN 40 (`8/8/3P3k/8/1p6/8/1P6/1K3n2 b - - 0 1`, K+P+N vs K+P): ~12.4 M nodes
- FEN 20 (`8/6pk/1p6/8/PP3p1p/5P2/4KP1q/3Q4 w - - 0 1`, K+Q+2p vs K+Q+3p endgame): ~170 M at d20
- FEN 26 (`5k2/7R/...`): now ~226 k cold at d13 but ~150 M at d20

These are all **horizon-stretching endgames** with long forced sequences that include checks. The Lever-1 win on FEN 26 at d13 was that universal LMP slices off responding quiets in the check chain; at d20 the chain is just long enough that even with universal LMP, the residual node count is hundreds of millions. They're qualitatively the same shape as the prior FEN-26 cliff but stretched out over more depth.

### Levers tested

**Lazy SMP multi-threading (LANDED 2026-05-14).** Engine grows `Vec<WorkerState>` (per-thread history / counter-moves / cont-history / capture-history / pawn-cache); main thread runs the canonical iterative-deepening loop and returns the result; `threads - 1` helper threads run the same loop with their own state and contribute only via the shared TT. Stop coordination via `Arc<AtomicBool>` set when main thread finishes. CLI `bench <tt> <threads> <depth>` and `play --threads N` expose it. Retrospective + hint panel also use `available_parallelism()` by default; `--deterministic` collapses to single-thread for bit-identical narration. Aggregate scaling on this 24-core machine:

| | d=14 bench | d=20 bench |
|---|---|---|
| 1 thread | 6.5 s | 116.8 s |
| 4 threads | 3.7 s (1.77×) | 71.1 s (1.64×) |
| 8 threads | 3.1 s (2.11×) | **43.0 s (2.72×)** |
| Multi-PV=3 d=14 FEN 20 (retrospective workload) | 880 ms | 226 ms (8T) |

Per-position variance is high under Lazy SMP (a single FEN at 8T can swing 1.7s–26s between runs because TT-race ordering varies); the aggregate is stable because variance averages across the 45-pos set. Determinism contract: `threads=1` is bit-deterministic across runs (verified at FEN 26 d=13 = 135,061 nodes every run); all analytical paths (REPL `analyze` / `search`, retrospective, hint panel) default to `threads=1` unless the caller explicitly sets `threads > 1` via SearchParams or the CLI. Sub-linear 2-4-thread speedup is the known cost of "same-depth helpers all run the same iterative-deepening sequence"; SF11's `skipSize` / `skipPhase` de-syncing would lift this but isn't ported yet.

**SF11 aspiration depth-reduction-on-fail-high (LANDED 2026-05-14).** Ported the `failed_high_cnt` mechanism from SF11 search.cpp:450, 453, 485, 492. Consecutive fail-highs accumulate the counter; each re-search runs at `max(1, rootDepth - failed_high_cnt)`. The reduction resets to 0 on every fail-low. The result is that fail-high chains are progressively cheaper — a 6-attempt fail-high cycle at d=20 (previously all at d=20) now runs at d=20, 19, 18, 17, 16, 16 — converging on a shallower-but-still-useful PV instead of paying 6× full depth.

| | nodes / time before | nodes / time after | Δ |
|---|---|---|---|
| FEN 20 d=20 / 128 MB cold | 36.8 M / 13.3 s | **10.0 M / 3.7 s** | **−73%, 3.7× faster** |
| Full d=20 bench (45-pos / 128 MB) | ~146 s (pre-aspiration) | **115.9 s** | **−21%** |
| 45-pos d=14 / 16 MB | 14.2 M / 6.4 s | **12.0 M / 5.2 s** | −15% nodes, −19% time |
| 45-pos d=14 / 128 MB | 14.4 M / 7.3 s | **13.1 M / 6.5 s** | −9% nodes, −11% time |
| FEN 26 d=13 / 16 MB | 138 k | 135 k | unchanged |

SF11's full aspiration tuning (initial delta `21 + |prev|/256`, growth `delta + delta/4 + 5`) was also tested but regressed FEN 26 d=13 by ~3× (138 k → 447 k) — the wider initial costs us more in alpha-beta inefficiency than it saves in re-searches. Kept our existing `delta=17` + `2×` growth; depth-reduction is the only piece that paid off in our codebase. The trade-off: when an aspiration chain converges via depth-reduction, the returned PV is from a shallower search than `depth`. We're reporting `depth=20` even when the converged search ran at `depth=16`. SF11 has this exact same behaviour; it's a deliberate accuracy-vs-time trade.

**SF11 qsearch depth tracking + recapture-only (LANDED 2026-05-14).** Ported SF11 search.cpp:1350 (qsearch signature takes `Depth`) + 1522 (recurse with `depth - 1`) + 1459 (recapture_square = parent move's to-sq). Picker at `depth <= QS_RECAPTURES (-5)` filters to captures landing on recapture_square. Previously our qsearch ignored depth, so capture chains in K+R-vs-K+R-with-passers and K+2R-vs-K+Q+p endgames ran all the way to `MAX_PLY = 64`. The deep-ply explosion we'd attributed to negamax extensions was actually qsearch's fault.

| | nodes / time before | nodes / time after | Δ |
|---|---|---|---|
| FEN 19 d=20 / 128 MB cold | 391 M / 60 s | **7.8 M / 2.4 s** | **−98%, 25× faster** |
| FEN 41 d=14 / 16 MB cold | 44.1 M / 7.9 s | **1.45 M / 0.5 s** | **−97%, 16× faster** |
| FEN 41 d=14 / 128 MB cold | 8.3 M / 2.2 s | **1.46 M / 0.6 s** | −82%, 3.8× faster |
| 45-pos bench d=13 / 16 MB | 10.5 M / 4.1 s | **8.4 M / 3.8 s** | −20% nodes, −8% time |
| 45-pos bench d=14 / 16 MB | 20.5 M / 7.2 s | **14.2 M / 6.4 s** | −31% nodes, −11% time |
| 45-pos bench d=14 / 128 MB | 22.1 M / 9.5 s | **14.4 M / 7.3 s** | −35% nodes, −23% time |
| Italian Game d=18 (quadrant) | 8.1 M / 4.7 s | 7.9 M / 4.5 s | within noise |

Cost: ~30 LOC. The MovePicker already had the `QS_RECAPTURES → filter to recapture_square` logic — it just hadn't been fed real depths. Two attempts at the SF11 qsearch delta/futility prune (search.cpp:1471-1492) on top of this regressed middlegame +60–70% due to per-move do/undo overhead from missing pre-do `gives_check`; reverted, kept depth tracking alone. All 787 tests pass. NPS dropped 2.6→2.2 Mnps (slightly more work per qsearch frame), but wall-clock is faster everywhere because vastly fewer frames are visited.

**Lever 1: universal `moveCountPruning` (LANDED).** ~10 LOC change. FEN 26 cold d13 went 484 M → 226 k (2,140×). 45-pos cold d13 bench went 101 M → 17.5 M (5.8×). Cost: FEN 43 mate puzzle moved from "mate at d5 / 3.3 k" to "mate at d8 / 9.9 k" — same family as SF's ~2 Elo check-extension estimate. Acceptable.

**Lever 2: quadratic SEE quiet pruning (TESTED 2026-05-14, REVERTED — regression).** Ported SF11 search.cpp:1027 verbatim — `see_ge(move, -(32 - min(lmrDepth, 18)) * lmrDepth²)` for quiets, layered on top of Lever 2b under the same Step 13 outer gate. Hoped it would catch the SEE-negative deep quiets that survived Lever 2b's history-sum gate (FEN 19 residual).

Result: catastrophic regression on the same K+R-vs-K+R-with-passers family Lever 2b couldn't fully fix:
- 45-pos d=14 128 MB cold: 22.1 M → 1.28 B (58× worse)
- FEN 41 d=14 128 MB cold: 8.3 M → 1.26 B (150× worse) — almost all of the aggregate regression
- FEN 19 d=17-19: 11×/5.7×/3.4× regressions (only d=20 slightly better)

Hypothesis: in long forced rook-vs-rook sequences, the quiets that SF11's quadratic threshold prunes are actually correct king/rook maneuvers whose SEE-negative-by-a-few-cp signal misrepresents their forcing value. Pruning them causes the search to fail-low repeatedly, generating massive re-search overhead. Same shape as Lever 2b's history-sum gate let through these moves on purpose; Lever 2 takes them out, which is the wrong call in this position class.

Reverted. Build green, 633 tests pass. Source: cleanly removed the 30-line block at search.rs:1378 — re-add from this HANDOFF entry if a future attempt has reason to believe the failure mode is different.

**Lever 2b: SF11 quiet-futility lmrDepth + history form (LANDED 2026-05-14).** Replaced our raw-`depth <= 7` gate with SF11 search.cpp:1016-1024 verbatim: `lmrDepth < 6 && static_eval + 235 + 172*lmrDepth <= alpha && (mainH + ch0 + ch1 + ch3) < 25000`, gated by `pos.non_pawn_material(us) > 0` (SF11 Step 13 outer gate). The previous "predicted small" was wrong — instrumented diagnosis showed that with chained extensions keeping raw depth high at deep ply, our raw-depth gate disabled futility precisely where it was needed. Aggregate impact (cold TT, `--new-game-between-positions`):

| | nodes / time before | nodes / time after | Δ |
|---|---|---|---|
| 45-pos bench d=13 | 17.5 M / 5.4 s | **10.5 M / 4.1 s** | −40% nodes, −24% time |
| 45-pos bench d=14 | 104.2 M / 21.1 s | **20.5 M / 7.2 s** | **−80% nodes, −66% time** |
| FEN 40 d=14 (worst outlier) | 22.0 M | **466 k** | **−98%, 47× faster** |
| FEN 20 d=14 | (untimed earlier; 170 M at d=20) | 1.02 M | tail collapsed to ~80 nodes/ply past ply 30 |
| FEN 41 d=14 | (didn't finish at d=20) | 7.45 M | residual deep tail; smaller |
| Italian d=18 (quadrant member) | 7.6 M / 4.5 s | 8.1 M / 4.7 s | +6%, small middlegame regression |
| FEN 43 mate puzzle | mate at d=8 / 9.9 k | mate at d=8 / 7.8 k | unchanged correctness |

NPS dropped (3.1 → 2.6 Mnps at d=13) — the futility check now does cont-history reads per quiet, paid back many times over by the node savings. The middlegame regressions are single-digit %; the endgame wins are 10-50×.

Code: search.rs:1298 `do_futility_prune` block. Removed `futility_prune` helper and `SHALLOW_PRUNE_MAX_DEPTH` const (now dead code). All 787 tests pass.

**Singular extensions + multi-cut (THIRD ATTEMPT, 2026-05-14, REGRESSED).** Now that universal `moveCountPruning` was in tree, we re-attempted the SF11 step-14 logic: `excluded_move` on the stack, half-depth verification at `tt_value - 2*depth`, TT key XOR'd by `excludedMove << 16`, NMP/TT-save gated on `!excluded_move`, `singular_lmr → r -= 2` in LMR. Full plumbing landed cleanly (build green, 787 tests pass), but on the quadrant:
- FEN 26 d13 cold: 226 k → 157 M (~700× regression vs Lever-1 baseline)
- Italian d18 cold: 7.6 M → 14.3 M / 4.5 s → 8.5 s (~90% slower)
- FEN 20 of the 45-pos bench stalled for multiple minutes; aborted

Both regressions are in *the same kind of position* the previous attempts regressed on, despite Lever 1 being in place. Hypothesised root cause: in horizon-stretching forced sequences (which FEN 26 and FEN 20 both have), every TT move's only legal response is singular — so the gate fires on most nodes in the chain, each adds a half-depth verification *plus* `+1 ply` to the TT move, and the chain stretches further than it did pre-SE. Multi-cut doesn't fire enough to amortise the verification cost. **Reverted.** Worth re-attempting on top of Lever 2b now that extension chains are tamer. Plumbing recoverable from git reflog.

### Outlier profiling — 2026-05-14 (post-Lever-1)

Per-ply node-histogram + selDepth instrumentation landed in tree (see "Temporary perf-investigation infrastructure" below). Profiling FENs 1, 20, 26, 40, 41 at d10/12/14 with cold TT (`--new-game-between-positions`) found:

- **All four outliers (20, 26, 40, 41) hit `MAX_PLY = 64` at d=10.** Extension chains are running past the recursion cap. FEN 1 (start position) reaches only seldepth 18 at d=10 — normal.
- **FENs 20 and 26 have a *small repeating* deep tail** (≤100 nodes per ply past ply 25), consistent with a short perpetual-check loop in qsearch. Combined tail-vs-bell is <5% of total nodes at d=14. These are nuisance, not catastrophe.
- **FENs 40 and 41 have an *exponentially-growing* deep tail.** FEN 40 at d=14 is 22 M nodes total, of which ~17 M (77%) live in plies 50–63, peaking at 4.3 M nodes in the ply-63 (MAX_PLY-clamped) bucket. FEN 41 d=12 is the same shape, peaking at 1.1 M at ply 63.

A/B-disabling extensions one at a time on FEN 40 d=14 (`--new-game-between-positions`):

| Configuration | FEN 40 nodes | Speedup |
|---|---|---|
| All four extensions on (baseline) | 21.96 M | 1× |
| Last-captures off | 9.62 M | 2.3× |
| Passed-pawn off | 779 k | 28× |
| Both off | 206 k | 107× |

**The passed-pawn extension is the dominant chain-stacking culprit** in FENs 40 / 20 / 26. The trigger (`is_first_killer && is_advanced_pawn_push && pawn_passed`) matches SF11 verbatim, but in both-sides-passers endgames (K+P+N vs K+P, K+Q vs K+Q pawn race, K+R vs K+R pawn race) the killer at deep plies is the passer push, so the +1 ply fires on most plies and the chain stretches without bound.

But — and this is the snag — **disabling passed-pawn extension regresses FEN 41**:

| FEN | d=13 baseline | d=13 passed-pawn off | Ratio |
|---|---|---|---|
| 1 (startpos) | 179 k | 179 k | 1.0× |
| 2 (Kiwipete) | 305 k | 305 k | 1.0× |
| 8 (middlegame) | 461 k | 461 k | 1.0× |
| 14 (middlegame) | 178 k | 178 k | 1.0× |
| 20 (K+Q endgame) | 928 k | 313 k | **3.0× faster** |
| 26 (K+R endgame) | 226 k | 80 k | **2.8× faster** |
| 40 (K+P+N vs K+P) | 4.96 M | 551 k | **9.0× faster** |
| 41 (K+2R vs K+Q+p) | 5.40 M | 10.56 M | **0.5× — regression** |

FEN 41 *needs* the extension to find tactics (q-vs-2R with both-sides-passers has real resolutions); FEN 40 doesn't (the pawn race is maneuvering, not tactical). Middlegame positions are unaffected — the extension only fires when killer happens to be an advanced passer push, which is rare in middlegame.

### Why SF11 doesn't run away — the depth-metric gap

Initially I proposed tightening our passed-pawn extension (NPM gate, ply gate, stack-once rule). **All of those were my own inventions, not SF11 features.** Re-reading SF11's search.cpp lines 996-1031 (Step 13. "Pruning at shallow depth") more carefully:

- SF11 has **no raw-depth gate** on its quiet pruning. Instead, every rule is gated on `lmrDepth = max(newDepth - reduction(improving, depth, moveCount), 0)`.
- **Futility (line 1017):** `lmrDepth < 6 && static_eval + 235 + 172*lmrDepth ≤ alpha && hist_sum < 25000`
- **Countermove (line 1011):** `lmrDepth < 4 + adj && cont[0] + cont[1] < threshold`
- **SEE quiet pruning (line 1027):** threshold `-(32 - min(lmrDepth, 18)) * lmrDepth²` — gates implicitly via the lmrDepth² term

**Our shallow pruning is gated on raw `depth <= SHALLOW_PRUNE_MAX_DEPTH (= 7)`** (search.rs:1304). When extensions stack and keep raw `depth` high at deep ply, our pruning *never fires* on the responding quiets. SF11's `lmrDepth` gate stays small because LMR has reduced the move — so SF11 prunes the same quiet we don't, and the chain breaks implicitly.

This is the actual SF11 mechanism: **chained extensions don't run away because the quiets they generate are aggressively pruned via `lmrDepth`, not raw depth.** The extension triggers and `advanced_pawn_push` thresholds match SF11 verbatim, but our pruning gate is the wrong shape and lets the chain stretch unbounded.

Also relevant: **SF11's `MAX_PLY = 246`; ours is 64.** Even with the right pruning, the deeper natural search horizon would help.

### What remains gated off in tree

`endgame.rs` was split into a directory module ([`core/engine/src/endgame/`](core/engine/src/endgame/)) with one file per evaluator. `probe()` returns `ProbeResult::{Override, Scale, ScaleBoth, None}`. Twelve scaling functions ported with unit tests: `KRPKR`, `KRPKB`, `KRPPKRP`, `KBPKB`, `KBPPKB`, `KBPKN`, `KNPK`, `KNPKB`, `KBPsK`, `KQKRPs`, `KPsK`, `KPKP`. Dispatch chain wrapped in `if SCALING_ENABLED { ... }` (currently `false`); four `dispatcher_routes_to_*` tests are `#[ignore]`d. Was originally framed as a fix for the "endgame bombers" — that framing was largely a misread; Lever 1 collapsed most of the bench-cost gap without scaling. Re-enabling is still potentially worthwhile for *teaching-accurate* endgame evals (e.g. recognising fortress draws), but is no longer load-bearing for raw bench performance.

## Engine perf reference numbers (2026-05-14, post-Lazy-SMP)

**Single-thread bench (SF11 default 45 positions, `--new-game-between-positions`):**
- d13 / 16 MB: 8.4 M nodes / 3.8 s / 2.2 Mnps
- d14 / 16 MB: 12.0 M nodes / 5.2 s / 2.3 Mnps
- d14 / 128 MB: 13.1 M nodes / 6.5 s / 2.0 Mnps
- d20 / 128 MB: 226 M nodes / 116 s / 2.0 Mnps

**Multi-thread bench (this machine, 24 logical cores, 128 MB shared TT cold-per-pos):**
- d=14: 6.5 s (1T) → 5.1 s (2T) → 3.7 s (4T) → 3.1 s (8T)
- d=20: 117 s (1T) → 71 s (4T) → **43 s (8T)**

Single-thread NPS is ~2.0–2.2 Mnps (vs SF11's 3.1 Mnps). The qsearch depth-tracking added some per-frame work for the depth bookkeeping but the node savings dominate; further NPS recovery would come from the deferred `pos.occupied()` incremental field.

**Per-position snapshot** (single-thread, `--new-game-between-positions`):

| Position | Depth | TT | Nodes | Time |
|---|---|---|---|---|
| FEN 1 (startpos) | 13 | 16 MB | 356 k | 191 ms |
| FEN 1 (startpos) | 20 | 128 MB | 28.9 M | 17.5 s |
| FEN 26 (K+R endgame) | 13 | 16 MB | 138 k | 52 ms |
| FEN 2 (Kiwipete) | 13 | 16 MB | 228 k | 120 ms |
| FEN 19 (K+R race) | 20 | 128 MB | 7.8 M | 2.4 s |
| FEN 20 (K+Q endgame) | 20 | 128 MB | 10.0 M | 3.7 s |
| FEN 41 (K+2R vs K+Q+p) | 14 | 16 MB | 1.45 M | 518 ms |
| FEN 41 (K+2R vs K+Q+p) | 20 | 128 MB | 23.8 M | 9.7 s |
| Italian Game | 18 | 16 MB | 7.9 M | 4.5 s |

**SF11 reference (128 MB TT, our machine, 46 FENs incl. 1 Chess960 we skip):**
- d7: 182 k nodes / 0.1 s / 1.7 Mnps
- d14: 6.93 M / 2.2 s / 3.1 Mnps
- d20: 68.17 M / 22.1 s / 3.1 Mnps

Post all the 2026-05-14 changes, every position finishes at d=20 in a few seconds (worst is FEN 1 startpos at 28.9 M / 17.5 s single-threaded; FEN 41 at 23.8 M / 9.7 s; everything else under 11 M / 6.5 s). The aggregate single-thread gap to SF11 d=14 is ~2× nodes / ~3× time; per-position the gap is uniform rather than concentrated in outliers. NPS gap (~2.0 Mnps vs ~3.1 Mnps) is the main remaining single-thread headroom, but is diffuse across positions and would need micro-optimisation work to close (incremental `pos.occupied()` is the highest-likelihood standalone win). With Lazy SMP at 8 threads the wall-clock gap effectively closes — we run the full d=20 bench in 43 s vs SF11 single-thread's 22.1 s, and the user has multi-core throughout the target deployment surfaces (desktop + iOS/Android).

## Engine perf, deferred

The current production search has, in tree: PGO, reverse-futility pruning, statScore-LMR, cutNode plumbing, full SF11-gated CMP, ProbCut with `2 + 2 * cutNode` budget, lazy eval (gated on `trace.is_none()`), sticky `tt_pv` save, PEXT slider attacks under BMI2. Each was measured and documented in commit messages and inline `//!` docs at landing time.

**Search features still to port (would reduce nodes-per-depth):**
- **NMP zugzwang-verification at high depth** (SF11 lines 838-886) — `nmpMinPly` / `nmpColor` mechanism. Tried 2026-05-14, net-neutral aggregate but regresses FEN 19 d=19 8.7×. It's a correctness feature, not a speed feature; re-attempt later weighing correctness vs perf.
- **SF11 qsearch delta/futility prune** (SF11 lines 1471-1492). Tried 2026-05-14 on top of qs-depth: regressed Kiwipete +72%, FEN 3 +62% due to per-move do/undo overhead from missing pre-do `gives_check`. Worth re-attempting once `pos.gives_check(m)` is implemented (needs check-squares cache).
- **Quadratic SEE quiet pruning** (SF11 line 1027) — replace our `Value::ZERO` quiet-SEE threshold with `-(32 - min(lmrDepth, 18)) * lmrDepth²`. Reverted 2026-05-14 (Lever 2 catastrophic). Failure mode may differ now that qsearch chains are bounded; re-attempt with caution.
- **`ttPv → r -= 2` LMR consumer** — sticky save is in tree; consumer measured at +30-80% wall-clock regression in isolation. Re-attempt if a future investigation reveals it's needed for balance with a relaxer.
- **Internal Iterative Deepening** (SF11 step 11, ~1 Elo). When `depth >= 7` and no TT move, run `depth - 7` to seed TT. Tiny gain alone.
- **Razoring** (SF11 step 7, ~1 Elo). Trivial code change.

**Per-node speedups still to try (NPS gain at fixed search shape):**
- **Incremental `pos.occupied()`** as a `by_all: Bitboard` field, toggled in `remove_piece` / `put_piece`. Likely actually-real gain (removes work, no cache trade-off). Highest-likelihood standalone NPS win; closes much of the ~2.0 → 3.1 Mnps gap to SF11.

**Threading refinements (deferred):**
- **Singular extensions + multi-cut, fourth attempt** — three previous attempts regressed on horizon-stretching forced sequences. Hypothesised root cause was the extension chain stacking deeper; with qsearch chains now bounded by qs-depth, the SE failure shape may be different.
- **NPM gate for retrospective threading** — Lazy SMP wastes cycles on positions that converge in <50 ms anyway. A guard could check static-eval / non-pawn material at the root and fall back to single-thread for "easy" positions.
- **Better thread scheduling (skipped depths)** — current Lazy SMP has all helpers running the same iterative-deepening sequence. SF11's `skipSize` / `skipPhase` pattern de-syncs them by depth so different threads explore different cones simultaneously. Would likely lift the 2-4 thread regime (currently sub-linear speedup) closer to 4+ thread linear scaling.

**Failed experiments worth not retrying** (full detail in git log around 2026-05-11..2026-05-12):
- Material hash table — cache hit rate was high but wall-clock-neutral; `endgame::probe` dominates the uncached path.
- Pawn cache resize 16K→64K — colder L3 offset fewer misses.
- Shelter (king-safety) hash table — middlegame hit rate was good, NPS unchanged; function I thought was hot wasn't.
- TT `atomic_load` inlining — was already auto-inlined by LLVM.

**Important meta-point on profiling tools.** VTune's bottom-up Hotspots view is **not reliable** on our LTO release binary — five distinct hotspot phantoms led to five wasted optimizations. Don't pick perf targets from VTune Hotspots alone; corroborate via dhat (allocations), A/B isolation (hypotheses), or VTune Microarchitecture Exploration (bottleneck *kind*: frontend / backend / memory / branch-mispredict, addressed by *category* of fix rather than function attribution).

**Cross-position TT bench behaviour.** Shared TT at 16 MB makes the endgame positions ~17–17,000× faster than cold because earlier middlegame entries happen to be useful. At 128 MB the shared TT becomes net-harmful (old entries crowd out the deep entries the endgames want). The underlying issue is the per-position cost itself, not a TT bug. Post-Lever-1 the magnitude is much smaller (cold/shared ratios are now 2–6× rather than 17–17,000×), so this is mostly de-mooted.

**`ENGINE_TURN_NODE_CAP` review** — currently a flat 5 M at [`core/cli/src/play.rs:35`](core/cli/src/play.rs) and same in [`desktop/src/main.rs`](desktop/src/main.rs). Engine play hits the cap consistently at depth 20. Historically necessary because some closed positions ran 30+ minutes uncapped. With the 2026-05-14 perf landings in tree the worst single-thread cases are now seconds rather than minutes (FEN 1 startpos d=20 = 17.5 s is the new worst at 1 thread, ~6 s at 4 threads). Worth re-running a few d20 positions uncapped to pick a number in the 15–50 M range or making the cap depth-aware. Lower priority now that Lazy SMP also shortens wall-clock.

**Temporary perf-investigation infrastructure currently in tree** (clean up when no longer needed): pawn-cache `hits` / `misses` counters + `Engine::pawn_cache_stats()` accessor + CLI `pawn$:` line in `search` output; dhat-heap feature in CLI Cargo.toml + global allocator hook in `main.rs`; `Search::nodes_per_ply` histogram + `seldepth` counter + `Engine::last_nodes_per_ply()` / `last_seldepth()` accessors; `chess-tutor bench --verbose` (prints per-position selDepth + compact ply histogram) and `--positions 20,26,40-41` (1-based whitelist).

## Engine strength, deferred

- **Time management** (`core/engine/src/timeman.rs` — file doesn't exist). Today `max_time` is a simple deadline. Proper allocation needs game time + increment + moves-to-TC.
- **Baked-in magic attack tables.** Magic numbers searched at process start (LazyLock + xorshift); harvest from one local run, paste as `const`. Saves tens of ms per process start. Do when integrating the first platform app.
- **Endgame scaling factors (12 functions in tree, gated off).** See "What remains gated off in tree" above. Defer until the check-extension chain investigation clarifies whether scaling-induced eval shifts can actually be absorbed by our search, or whether scaling is the wrong abstraction for our 2000-ELO target.
- **Singular extensions** — three attempts (2026-04-30 ~2× regression; 2026-05-12 catastrophic +346% Italian; 2026-05-14 catastrophic on horizon-stretching endgames after Lever 1 landed). The 2026-05-14 attempt was the cleanest port to date: `excluded_move` on stack, half-depth verification at `tt_value - 2*depth`, TT key XOR'd by `excludedMove << 16`, NMP + TT-save gated on `!excluded_move`, `singular_lmr → r -= 2` in LMR. Build green, 787 tests pass. Bench impact: FEN 26 cold d13 226 k → 157 M, Italian d18 cold 7.6 M → 14.3 M, FEN 20 of the 45-pos bench stalled for minutes. The new hypothesis is the failure mode: in long forced check/queen-checks sequences, every TT move's response is singular, so the gate fires on most nodes in the chain — each adding a half-depth verification *plus* `+1 ply` to the TT move. Multi-cut doesn't fire often enough to amortise. Defer until either (a) the horizon-stretching outliers (FEN 20, FEN 26 at d20, FEN 40) are tamed by another mechanism so the SE failure-shape isn't masking the SE gain, or (b) we figure out which surrounding SF feature (some specific LMR relaxer? a tighter singular gate? an explicit cap on chained singular extensions?) makes the verification cheap enough.
