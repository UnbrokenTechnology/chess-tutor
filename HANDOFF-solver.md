# HANDOFF — opponent-Elo solver + the grid-lookup bake (LANDED)

Snapshot **2026-06-08 (overnight)**. Read [`CLAUDE.md`] first. Companion to
[`HANDOFF-perception.md`] (the ladder/calibration thread this grew out of)
and [`calibration/HANDOFF-calibration.md`] (harness internals).

---

## TL;DR — the bake is DONE; feel-test it

The "opponent Elo" slider is built end-to-end and the forward model is now a
**5-D multilinear interpolation over the measured grid** (was an additive
piecewise model that extrapolated the 3+-dial interactions — a d7/p0 bot read
~2023 Elo but plays ~1050 because perception gates the search). The grid was
re-run with bands that BRACKET the full GUI range, rated, baked into Rust, and
the GUI capped to the bracketed range. **All committed, all tests pass.**

**→ The one thing left is the user feel-test** (the morning goal): open the
desktop, move the Elo slider, confirm a sensible bot; drop perception to 0 in
the Advanced dropdown and watch the "Resulting strength" tank.

---

## What landed (newest-first, on `main`)

- **`d4b1698`** — bake: `model_elo` → `interp5()` over the baked
  `core/engine/src/calibration_lookup.rs` (2592 measured Elos, 0 filled =
  full coverage) + king-safety mask as an additive depth handicap (masks
  aren't in the grid); `QINF_CODE` 8→10 to match the qsearch interp axis;
  `gen_lookup.py` emits valid Rust float literals + a generated-file header;
  new tests (grid-point exactness + the d7/p0 regression); GUI capped to
  **depth 1..=8** and **tactical-vision floored at q1** (`dialog.rs`).
  `config_for_elo` (the ladder / default slider), `elo_for_dials`,
  `estimate_elo`, `solve_rank` are UNCHANGED — the ladder is still the tight
  feel-validated default; the lookup only drives the advanced-tab delta + the
  iterative inverse.
- **`a6a623a`** — `rate.py` crash fix at `-s 0` (Ordo omits the error column).
- **`18e9cf5`** (earlier) — the 2592-config interpolation-coverage grid spec
  + `ref-floor`/`ref-d10` pool + `gen_lookup.py`.

### Verification (from the bake)
- `model_elo(config_for_elo(t))` tracks the target: 1200→1303, 1500→1535,
  2000→2017.
- **Perception → 0 tanks** (the demo): `elo_for_dials` default vs perception=0
  reads 1000→455, 1500→805, 2000→1038.
- Full workspace tests pass; clippy clean on `calibration.rs`/`dialog.rs`.

---

## How the grid CSV was produced (note for any re-run)

The grid play phase finished (~1.1M games, all 23 batches in
`calibration/runs/grid/`). The Ordo rating pass with the default **`-s 400`**
error-bar simulations was impractical at this scale (2600 players × 1.1M
games ran 4h+ and was killed). It was re-rated with **`--sims 0`** (point
estimates only — the bake reads `elo`, not the error bars):

```
cd calibration && .venv/Scripts/python.exe run_grid.py --sims 0   # resumes, skips played batches, rates in minutes
```

`grid_results.csv` Elo range: **-392 .. 2744** (brackets the full GUI dial
span). The `elo_error` column is blank (no sims) — fine for the bake. If error
bars are ever wanted (marketing/analysis), re-rate with `--sims 200` (hours).

## Regenerating the lookup (if the grid is ever re-run)
```
calibration/.venv/Scripts/python.exe calibration/gen_lookup.py \
    calibration/runs/grid/grid_results.csv > core/engine/src/calibration_lookup.rs
```
Then `cargo test --release -p chess-tutor-engine calibration`.

---

## Open / follow-ups (none blocking)
- **User feel-test** (above) — the validation gate. If a band feels off, the
  fix is to re-tune the ladder (`run_ladder.py`) and/or re-measure; the lookup
  inherits the grid's ±~100 Maia-anchor noise (the ladder is the tight path).
- **King-safety mask values** (`F_SAFETY_BY_DEPTH` in `calibration.rs`) are the
  ≈−27 d1 → −1 d6 handicap carried over from the prior additive model; cross-
  check against `runs/grid_3840_masks/{blend,surface}_model.json` if revisited.
- **Push**: the overnight commits are local only — not yet pushed to `origin`.

## Gotchas (carried)
- Edit `calibration/run_*.py` via Edit/Write tools only (heredoc overwrites of
  existing run scripts silently didn't persist in one session).
- Run the venv with `calibration/.venv/Scripts/python.exe` from the repo root
  (or `.venv/...` from inside `calibration/`).
- Bench single-threaded; release builds; commit straight to `main`; don't run
  workspace-wide `cargo fmt`.
