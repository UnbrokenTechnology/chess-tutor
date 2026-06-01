# Context brief: fix MultiPV-around-mate distance instability

**This is a starting-point brief, not a finished plan.** It gathers everything a
fresh context needs to *design* the fix; the actual approach is to be decided in
that session (start by reading the SF11 reference + our aspiration code below).

Point a fresh Claude here: *"Read `PLAN-multipv-mate-fix.md`."* (The
`project_multipv_mate_pathology` memory also auto-loads and summarises this.)

---

## Symptom (user-visible)

The eval bar's **mate-in-N jumps around across consecutive moves** — e.g. a real
game went `M6→M5→M4→M3→M5→M2→M1`, where a move the retrospective graded **best**
made the count go *up* (M3→M5). The move is genuinely a forced mate and "best" is
correct; only the **reported mate distance** is wrong.

## Root cause

MultiPV (≥2) search around a forced mate reports the wrong (too-long) mate
distance — and it corrupts **even the top line's** score, so you can't dodge it
by reading PV1. The retrospective runs `multi_pv = 2` on every move (since the
opponent-move retrospective shipped, 2026-06-01), so this is now hit constantly
near mates.

Mechanism (from the older investigation): secondary PVs use aspiration windows
centred on the previous PV's score; when #1 is a mate (~32000), alternatives are
by definition >32000 cp away, so windows fail and re-widen pathologically. We
have SF11's depth-reduction-on-fail-high (`aspiration-depth-reduce-landed`,
commit 2dbf5c6) but **not** its full delta tuning (deferred because an early
attempt regressed FEN 26 d=13 ~3×).

## Minimal repro + acceptance test

```
chess-tutor search "rB5r/6pp/k2nQ3/pN5q/B5N1/1P6/P1P2PPP/R4RK1 w - - 1 2" \
  --depth 12 --force-include e6d6
```
- `--multi-pv 1` → `Qxd6+` reported as **#3** (the true mate).
- `--multi-pv 2` → **same** `Qxd6+` reported as **#6** (a longer mating line).

**Done when:** `--multi-pv 2` agrees with `--multi-pv 1` (#3) on this position,
without regressing the parity-audit bench. (Cross-check the older lockup repro
too: `4Rb2/p5p1/1p2Q3/2kN2q1/B1p5/8/PPPP1PPP/R1B3K1 w - - 0 24 --multi-pv 3
--depth 10` should finish under ~5s.)

## What is already correct — do NOT touch

- **Eval-bar display** (`ffdb9a2`): the `−1`-ply + render-in-moves fix in
  `core/ui/src/session/view_builders.rs` `eval_bar_fill_and_label` is correct —
  it faithfully rendered the corrupted `#6` as `M5`. The display is not the bug.
- **The caps hotfix**: `ANALYSIS_NODE_CAP = 100M` / `ANALYSIS_TIME_MS = 10s` in
  `core/ui/src/worker.rs`. Keep these even after the real fix (backstop).
- We can't short-circuit MultiPV when #1 is mate: the teaching layer wants to
  distinguish mate-in-2 from mate-in-3 ("you found a mate, but a slower one"),
  and SF keeps searching all slots through mates (`search.cpp:419`).

## Candidate fixes (priority order, least-disruptive first)

1. **Re-attempt SF11's aspiration delta tuning** now that the pruning stack has
   matured: SF11's `21 + |prev|/256` initial delta + `delta + delta/4 + 5`
   growth. (This is the deferred half of the depth-reduction work we already have.)
2. **Port `searchAgainCounter`** — the other half of SF11's depth-reduction
   adjustment.
3. **Per-`pvIdx` depth caps** — last resort.

A/B each change ALONE against the four-position bench quadrant before stacking
(`feedback_pruning_bundles` — a 2026-05-13 bundle regressed 28×). Analytical
paths stay single-threaded for determinism (`feedback_single_thread_bench`).

## Files to read

- **Ours:** `core/engine/src/search/run.rs` — `aspiration_search` + the MultiPV
  driver `run` (`pv_idx` loop, the per-slot aspiration seed). Also `state.rs`
  for `RootMove`/window state.
- **Caps + config:** `core/ui/src/worker.rs` (`RETROSPECTIVE_MULTI_PV = 2`,
  `ANALYSIS_NODE_CAP`, `ANALYSIS_TIME_MS`).
- **Reference:** `reference/Stockfish-sf_11/src/search.cpp` — the aspiration
  loop (initial delta, widening, `searchAgainCounter`) and the MultiPV-through-
  mate handling (~line 419). Port the *ideas*, hand-write the Rust (see CLAUDE.md
  licensing rules).

## Related memories

`project_multipv_mate_pathology` (the durable summary + both repros),
`project_aspiration_depth_reduce_landed`, `feedback_pruning_bundles`,
`project_parity_audit_baseline`, `feedback_single_thread_bench`,
`feedback_bench_annotation`.
