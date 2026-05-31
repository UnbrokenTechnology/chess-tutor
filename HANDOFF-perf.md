# Handoff: chess-tutor-2 — engine perf / strength

Forward-looking engine perf and strength context. **Read this only when returning to perf or strength work.** For current UX-focused iteration see [`HANDOFF-ux.md`](HANDOFF-ux.md); for the project overview and build commands see [`HANDOFF.md`](HANDOFF.md).

> The detailed history — the full SF11 parity audit (file-by-file walk, every divergence + disposition), the lever-by-lever A/B deltas, and the 2026-05-14 perf-landing series — lives in **git history** (search around the `parity-audit-log.md` deletion commit and commits `4efaec6` / `5a2a68a` / `063266e` / `45be8a7` / `4cfd1c7` / `698a79b`). This file keeps only the *current state* and the *forward-looking* levers. Inline `//!` docs in `search/`, `tt.rs`, `eval/` carry the "why this design" rationale.

## Current state vs SF11 (post parity audit, 2026-05-26)

The engine is a **faithful SF11 classical port.** The W1 parity audit walked all 9 SF11 file-groups; only two true correctness bugs surfaced (E1 king-ring pawn color, P1 en-passant key), both fixed. The rest of the gap was missing/simplified pruning, closed by landing the SF11 pruning stack as **balanced bundles** (quiet-LMR + capture-LMR + refined-eval-for-gates + faithful NMP family) plus the B1+B3 structural rework (prune-before-`do_move`, cached check info).

**Authoritative bench numbers** (45-position SF11-mirrored set, warm-TT, single-thread — the production config):

| Config | Ours: nodes | Ours: NPS | SF11: nodes | Node ratio |
|---|---:|---:|---:|---:|
| `bench 16 1 14` | 9,739,495 | ~2.45 M/s | 6,567,129 | **1.48×** |
| `bench 128 1 20` | 138,713,681 | ~2.26 M/s | ~68,000,000 | **2.04×** |

Done-criteria (~2× of SF11 at both depths) essentially met: **d=14 1.48× / d=20 2.04× SF** (started the audit at 2.03× / 3.3×).

**The residual is a diffuse NPS gap (~0.65× SF), not pruning.** It lives in movegen/eval/TT-entry-size, spread across all positions — not concentrated in any outlier. Candidates, ranked by likelihood:
- Full-pseudo-legal generation + filter vs SF's targeted `generate<GenType>` (constant-factor movegen cost).
- Eval cost per node.
- 16 B vs 10 B TT entry → ~half the effective capacity/MB (diagnostic: capacity is a *minor, noisy* lever, not the 2× story — measured non-monotonic across 16/32/64/128 MB).

## Threading policy — single-thread default (load-bearing)

**All shipped surfaces (desktop + CLI) default to `threads = 1`.** Lazy SMP stays in tree but is opt-in (`play --threads N`, `bench <tt> <threads> <depth>`). Three reasons:

1. **Determinism (the load-bearing one).** Lazy SMP introduces per-run score variance → the same move gets different verdicts across runs / after takebacks, a teaching disconnect. Measured: p50 51 cp / p95 121 cp same-move range at 8T on user-facing positions; worse on tactical positions (missed-mate flapping, ~29000 cp swings). Single-thread = 0 cp variance. See memory [[feedback_determinism]], [[feedback_single_thread_bench]].
2. **iOS deployment.** Single-core is friendlier to the thermal/battery envelope; per-thread `WorkerState` is ~8 MB cont-history.
3. **Cost is small.** Depth-10 retrospective ~120 ms single-thread (vs ~60 ms multi); engine moves ~40 ms.

`chess-tutor noise-bench` ([`core/cli/src/noise_bench.rs`](core/cli/src/noise_bench.rs)) measures Lazy SMP variance — re-run any time search params change. Multi-thread scaling on this 24-core machine: d=20 8T ≈ 2.7× single-thread (sub-linear; SF11's `skipSize`/`skipPhase` de-syncing would lift the 2–4-thread regime but isn't ported).

**Atomics aren't the speed cost they look like in VTune** — our TT uses `Ordering::Relaxed`, identical machine code to plain loads on x86/ARM64. The "atomic_load hotspot" was non-inlined-wrapper overhead, fixed by `#[inline(always)]` (`tt.rs:88-97`). The multi-thread scaffolding has a real *memory* cost, not a per-instruction speed cost.

## The hard-won lesson: SF pruning is a balanced set

Every isolated pruning lever **regressed** when A/B'd alone against our tuned tree (MP1 +1.4%, S8 +0.57%, S10 capture-LMR +23%, Q1 qsearch-futility +7.8%). The same levers were **wins** once bundled with their SF companions — capture-LMR went +23% solo → −16.5% with the LMR relaxers present; the faithful NMP family went +4–12% solo → −5.8% bundled with its eval-balance companions (Q3/Q4 + razoring + S5/C2). **Never A/B a single SF relaxer against our tree.** See memories [[feedback_pruning_bundles]], [[feedback_lmr_base_divergence]], [[feedback_lever2_regressed]].

## Deferred levers (forward-looking)

**Search features still to port (would reduce nodes-per-depth):**
- **NMP zugzwang-verification refinement** — the depth≥13 verification w/ `nmpMinPly`/`nmpColor` landed in the NMP bundle; the `NULL_MIN_DEPTH = 3` floor is a kept divergence (SF nulls at any depth, covered by razoring `<2` + RFP `<6`). Lowering the floor is a follow-up lever.
- **Internal Iterative Deepening (S8)** — `depth≥7 && !ttMove → depth−7 re-search`. ~1 Elo; regressed +0.57% solo at d=14; bundle-revisit candidate (interacts with ordering levers).
- **Quadratic SEE quiet pruning (S10/line 1027)** — reverted catastrophically (Lever 2, 58× regression on K+R-vs-K+R-with-passers; see [[feedback_lever2_regressed]]). Failure mode may differ now that qsearch chains are bounded; re-attempt with caution only with structural reason.
- **`gives_check` is in tree** (commit `26d64cd`, oracle-tested) and unblocks a future Q1 (qsearch futility) riding a balanced bundle.

**Per-node speedups (NPS gain at fixed search shape):**
- **Incremental `pos.occupied()`** as a `by_all: Bitboard` toggled in `remove_piece`/`put_piece`. Highest-likelihood standalone NPS win (removes work, no cache trade-off).

**Singular extensions — DON'T retry yet.** Three attempts all regressed on horizon-stretching forced sequences (2026-04-30 ~2×; 2026-05-12 +346% Italian; 2026-05-14 catastrophic after Lever 1). The 2026-05-14 port was clean (excluded_move, half-depth verify, TT-key XOR, multi-cut) and still blew up: in long forced check/queen sequences every TT move's response is singular, so the gate fires on most nodes, each adding a half-depth verify + `+1 ply`. Multi-cut doesn't amortise. See [[feedback_singular_extensions_third_attempt]]. Defer until the d=20 horizon outliers are tamed by another mechanism.

**Failed experiments — don't retry** (full detail in git log ~2026-05-11..14):
- Material hash table — cache hit rate high but wall-clock-neutral; `endgame::probe` dominates the uncached path.
- Pawn-cache resize 16K→64K — colder L3 offset the saved misses.
- Shelter (king-safety) hash table — good hit rate, NPS unchanged.
- TT `atomic_load` inlining — already auto-inlined by LLVM.

**Meta-point on profiling.** VTune's bottom-up Hotspots is **not reliable** on our LTO release binary — five phantom hotspots led to five wasted optimizations. Corroborate via dhat (allocations), A/B isolation (hypotheses), or VTune Microarchitecture Exploration (bottleneck *kind*), never Hotspots alone.

## Known residual outliers (d=20, 1T)

The "pathological outlier" class is gone — the worst positions cluster in the 10–30 M range (healthy deep-search cost, not chain blowups): FEN 1 (startpos, broad PV), FEN 41 (K+2R vs K+Q+p), FEN 20 (K+Q endgame), FEN 26 (K+R endgame). The passed-pawn extension still feeds the deep tails on both-sides-passers endgames; the SF11 fix is `lmrDepth`-gated quiet pruning (landed), not raw-depth. FEN 41 *needs* the extension to find tactics; FEN 40 doesn't. See memories [[feedback_passed_pawn_ext_chain]], [[project_fen26_check_extension_investigation]].

## Engine strength, deferred

- **Endgame scaling factors (12 functions in tree, gated off).** `endgame/` directory module; `probe()` returns `Override/Scale/ScaleBoth/None`. Dispatch wrapped in `if SCALING_ENABLED { ... }` (`false`); four `dispatcher_routes_to_*` tests `#[ignore]`d. No longer load-bearing for bench perf (Lever 1 collapsed the gap), but worth re-enabling for **teaching-accurate** endgame evals — drawish bishop/rook-pawn endings are over-valued, exactly the endgames our 1200 student mishandles. See [[feedback_endgame_evaluator_gradients]].
- **Time management** (`core/engine/src/timeman.rs` — doesn't exist). Today `max_time` is a simple deadline. Proper allocation needs game time + increment + moves-to-TC. (Determinism requirement keeps us on depth-budget, not time-budget — see [[feedback_determinism]].)
- **Baked-in magic attack tables.** Magic numbers searched at process start (LazyLock + xorshift; tens of ms). Harvest from one local run, paste as `const`. Do when integrating the first platform app. Approved size trade-off (~900 KB static).
- **`ENGINE_TURN_NODE_CAP`** — flat 5 M at [`core/cli/src/play.rs`](core/cli/src/play.rs) + `desktop/src/main.rs`. Engine play hits the cap at depth 20. Worst single-thread cases are now seconds (FEN 1 d=20 ≈ 17.5 s), so a higher / depth-aware cap is worth picking. The 100 M / 10 s analytical-path caps (retrospective / desktop) stay as a backstop.

## Temporary perf-investigation infrastructure in tree

Clean up when no longer needed: pawn-cache hits/misses counters + `Engine::pawn_cache_stats()`; dhat-heap feature in CLI Cargo.toml + allocator hook; `Search::nodes_per_ply` histogram + `seldepth` counter + accessors; `bench --verbose` (per-position selDepth + ply histogram) and `--positions 20,26,40-41` whitelist.
