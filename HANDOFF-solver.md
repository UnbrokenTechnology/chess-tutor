# HANDOFF — opponent-Elo solver + the grid-lookup re-bake

Cold-resume snapshot, **2026-06-08**. Read [`CLAUDE.md`] first. Companion to
[`HANDOFF-perception.md`] (the ladder/calibration thread this grew out of)
and [`calibration/HANDOFF-calibration.md`] (harness internals).

---

## TL;DR — where we are

The "opponent Elo" slider is built end-to-end (engine solver + desktop GUI,
committed). A user feel-test found the forward model was **extrapolating**
at high-depth/low-perception (a d7/p0 bot read 2023 Elo but plays ~1050,
because perception *gates* search and the additive model dropped that
interaction). Decision: **stop fitting a regression equation; do multivariate
interpolation over the measured grid instead.** A re-run of the grid with
bands that BRACKET the full GUI range (so we interpolate, never extrapolate)
is **RUNNING NOW** (~5.8 h). When it finishes, bake the lookup into Rust and
wire two GUI caps. The exact steps are below.

---

## ⏳ FIRST THING: check the grid re-run

It was launched **detached** (not harness-tracked — no completion
notification will fire). Check it:

```
cd calibration
tasklist | grep -ci python                 # 2 = still running (run_grid + waiter); 0 = done/dead
ls -la runs/grid/grid_results.csv          # exists => DONE
tail -3 runs/grid_rerun.log                # progress / "wrote ... grid_results.csv"
grep -ch '^\[Result ' runs/grid/batch_*.pgn   # games so far (target ~1.03M)
```

- **If `grid_results.csv` exists** → the run finished; go to "The bake sequence".
- **If still running** → it writes per-batch PGNs then ONE Ordo rating pass
  (silent, ~10-20 min tail) then the CSV. Wait for the CSV. ~2592 configs ×
  ~396 games ≈ 1.03M games ≈ 5.8 h play + rating tail.
- **If python is gone but no CSV** → it died/was interrupted. `run_grid.py`
  is **resumable** (skip-if-complete batches): just re-run
  `.venv/Scripts/python.exe run_grid.py` and it continues, then rates + writes
  the CSV.

The **re-run grid** (committed in `run_grid.py` GRID, masks PULLED OUT):
`depth {1,2,3,4,6,8} × qsearch {1,2,None} × perception {0,.2,.4,.6,.8,1} ×
avg_move_rank {1,2,3.5,5,6.5,8} × endgame {0,1,2,None} = 2592`. These bands
bracket the full GUI dial range so the lookup never extrapolates. Pool got a
`ref-floor` (very weak) + the ceiling raised d8→`ref-d10` (the grid now reaches
d8). The OLD 3840-config (with-masks) grid + its fitted models are preserved
at `runs/grid_3840_masks/`.

---

## The bake sequence (when the CSV exists)

### 1. Generate the Rust lookup table
```
cd calibration
.venv/Scripts/python.exe gen_lookup.py runs/grid/grid_results.csv > /tmp/lookup.rs 2>/tmp/gen.txt
cat /tmp/gen.txt   # coverage: "N measured ... K filled"; sanity-check K is small
```
`gen_lookup.py` (VALIDATED on the old slice) reads the no-mask grid, lays it
on the 5-D lattice, fills excluded extremes (all-win/all-loss Ordo couldn't
rate) by nearest-along-axis, monotone-clamps each fiber (depth/qsearch/
perception/eg up, rank down), and emits: `DEPTH_KNOTS`/`QSEARCH_KNOTS`/
`PERCEPTION_KNOTS`/`RANK_KNOTS`/`EG_KNOTS` (`&[f32]`) + flat
`const LOOKUP: [f32; 2592]` (row-major depth,qsearch,perception,rank,eg).
qsearch full-vision encoded as **10.0** on the interp axis; eg Full = **3.0**.

### 2. Rewrite `core/engine/src/calibration.rs`'s forward model
Replace the additive `model_elo` (the `F_PERCEPTION`/`F_DEPTH`/... piecewise
sum + `piecewise()`) with **5-D multilinear interpolation** over the baked
table. Keep EVERYTHING ELSE as-is:
- `config_for_elo(elo)` (the LADDER interpolation — the default slider) —
  unchanged, still the tight/feel-validated path.
- `elo_for_dials(dials, target)` = `target + (model_elo(dials) −
  model_elo(config_for_elo(target)))` — ladder-anchored; just swap its inner
  `model_elo` for the lookup.
- `estimate_elo`, `solve_rank` (iterative inverse) — unchanged; they call
  `model_elo`/`elo_for_dials`.
- **King-safety mask**: keep as an additive depth-term (the
  `safety_by_depth` value from `runs/grid_3840_masks/blend_model.json` or
  `surface_model.json`, ≈ −27 d1 → −1 d6); positional ≈ 0. Masks are NOT in
  the lookup (pulled from the grid).

New `model_elo` shape:
```rust
fn model_elo(d: &BotDials) -> f64 {
    let q = qsearch_code(d.qsearch);   // None -> 10.0 (matches gen_lookup)
    let eg = eg_code(d.endgame_skill); // None -> 3.0
    let base = interp5(d.depth as f32, q, d.perception, d.avg_move_rank, eg);
    base + if d.mask_safety { piecewise(F_SAFETY_BY_DEPTH, d.depth as f32) } else { 0.0 }
}
```
`interp5`: for each of the 5 axes find the bracketing knot pair + fraction
(clamp at ends — NO extrapolation needed now since bands bracket the GUI),
then sum the 2^5 = 32 corners weighted by the products of fractions. Index
the flat `LOOKUP` row-major. (qsearch_code/eg_code already exist; just confirm
None→10.0 / None→3.0 to match the knots.)

### 3. Update the tests (`calibration_tests.rs`)
- Keep: `config_reproduces_ladder_rungs`, `config_interpolates_rank_within_a_band`,
  `config_clamps_outside_the_ladder`, `default_config_displays_its_target_exactly`,
  `solve_rank_round_trips`.
- Replace the additive-model monotonicity tests with lookup ones:
  per-dial monotonicity (the table is monotone-clamped, so assert
  `model_elo` is monotone in each dial), and **grid-point exactness**
  (interp at a knot returns the baked value).
- **Add the regression test for the bug**: `model_elo` (or `elo_for_dials`
  from a 2500 anchor) at **depth 7, perception 0, rank 1, qinf, egF** must be
  ~1000-1100 (interpolated from the real d6/p0≈1023 + d8/p0), NOT ~2000.

### 4. Cap the GUI (`desktop/src/draw/dialog.rs`, `draw_strength_controls`)
- **Depth slider**: `1..=20` → **`1..=8`** (depth >8 is off-product /
  saturated / extrapolates).
- **Tactical-vision slider**: floor at **q1** (range `1..=QSEARCH_INF`, not
  `0..`) — q0 is the off-product "parks-queen" mode and would extrapolate.
- `avg_move_rank` (1..=8) and `perception` (0..=1) already match the bracketed
  bands — leave them.

### 5. Verify + commit
```
cargo test --release -p chess-tutor-engine calibration
cargo clippy --release -p chess-tutor-engine -p chess-tutor-desktop   # calibration.rs must be clean
cargo build --release -p chess-tutor-desktop
```
Then the user runs `cargo run --release -p chess-tutor-desktop` and re-feels
the slider + advanced tab (especially the d7/p0 corner that started this).
Commit `calibration.rs` + `calibration_tests.rs` + `dialog.rs` +
`calibration/{run_grid.py,gen_lookup.py,harness/pools.py}` once it feels right.

---

## What's already committed (don't redo)

- **`ed07183`** — grid + fit pipeline (`grid.py`, `run_grid.py`, `pools.py`,
  `fit.py`); the GBT/LASSO model fit.
- **the solver + GUI commit** — `core/engine/src/calibration.rs` (+ tests),
  `NewGameForm.elo_target`/`bot_dials()`/`apply_dials()` in
  `core/ui/src/session/types.rs`, the "Target Elo" slider + "Advanced"
  dropdown + live "Resulting strength ≈ N Elo" readout in `dialog.rs`. This
  all WORKS; only the forward MODEL inside `calibration.rs` is being swapped
  (additive → grid lookup). The GUI/types/ladder code is unchanged by the bake.

UNCOMMITTED (this session, in the working tree): the band changes to
`run_grid.py`, the `ref-floor`/`ref-d10` pool change in `harness/pools.py`,
`gen_lookup.py`, and the parametric exploration scripts
(`fit_piecewise.py`, `fit_blend.py`, `fit_surface.py`). Commit the first three
with the bake; the `fit_*` scripts are analysis artifacts (commit or leave —
the LASSO equation in `fit.py` is the kept "marketing" closed form).

---

## Why this design (so you don't relitigate it)

- **Lookup, not regression.** Every parametric form (LASSO, additive
  piecewise, 2-D perc×depth, gated blend) compressed *some* interaction,
  because the surface is interactive in 3+ dials at once (perception gates
  depth AND qsearch; perception×rank compounds; depth×rank). Proven by data:
  perception penalty (p1→p0) grows **+784 at d1 → +1340 at d6**, and at p=0
  qsearch goes flat (1040/1060/1014) while rank still bites (1014→366). The
  GBT captures all of it but is a non-portable blob. **Multilinear
  interpolation over the measured grid IS the data** — exact at knots, no
  compression, sane at every corner, ~2592 f32 (~10 KB) + 32-corner interp.
- **Ladder stays the default slider.** It's the feel-validated 1-D path
  (RMSE ~46, beats the grid's ~120 anchor noise). The lookup only drives the
  advanced-tab DELTA display (ladder-anchored) + the iterative inverse.
- **Offset to chess.com ≈ 0** (feel-validated: t500≫Martin, t1200=user's
  level, t1400 beat Mateo + chess.com rated our bot ~1300). Target Elo ≈
  chess.com Elo directly. (See HANDOFF-perception.md.)
- **Masks are personality, not strength** (≈0 Elo except king-safety, which
  is a depth-fading handicap) — hence pulled from the grid and handled as one
  additive depth-term. Openings/seed = pure personality, not solved.

## Watch-items / gotchas
- `gen_lookup.py` "filled" count: if many entries are filled (the weak corner
  all-loss), the lookup is interpolating fills there — fine (off-product), but
  sanity-check it's a handful, not hundreds.
- The grid's absolute Elos carry the ~±100-150 Maia-anchor noise; the lookup
  inherits it. The ladder (default slider) is the tight path; the lookup is
  for advanced exploration where ±100 is acceptable.
- Run perf on **release** builds. Commit straight to `main`. Don't run
  workspace-wide `cargo fmt`. (Standing repo rules.)
