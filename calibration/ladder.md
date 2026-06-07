# Perception-era ELO ladder — LOCKED candidate (2026-06-07)

**Monotone-perception schedule** (user redesign): perception rises
0.0 → 1.0 by t1400 and never retreats; above that, avg-rank is the
judgment lever ("sees all moves, picks imperfectly"). The believable
human arc — *can't see → vision fills in → sees all, misjudges* — maps
to the dial schedule directly.

Lever set: depth · qsearch-depth · perception · avg-rank · endgame
tier. (miss%/blunder% removed — both emerge from perception.) Rank on
the 0.1 grid, perception on the 0.05 grid.

ELOs are **lichess-anchored** (measured Maia rapid anchors). chess.com
runs ~200–350 lower — test e.g. t500 vs chess.com's ~250 bot.

Confirming pass: bias −25, **RMSE 39** (at/below the ±50–60 per-config
noise floor of 40-game pairings). The −25 is uniform loose-anchor drift
(weak-heavy pool compresses the Maia anchors), absorbed at lock.

| rung | depth | qsearch | perception | rank | endgame | ELO | source |
|------|-------|---------|------------|------|---------|-----|--------|
| t100 | 1 | 0 | 0.00 | 5.0 | none | ~100 | dense |
| t200 | 1 | 0 | 0.00 | 4.0 | none | ~200 | dense |
| t300 | 1 | 0 | 0.00 | 3.5 | none | ~300 | dense interp |
| t400 | 1 | 0 | 0.00 | 3.0 | none | ~400 | dense interp |
| t500 | 1 | 0 | 0.00 | 2.6 | none | ~490 | dense interp |
| t600 | 1 | 0 | 0.00 | 2.0 | none | ~600 | dense |
| t700 | 1 | 0 | 0.40 | 1.6 | basic | ~700 | cell |
| t800 | 1 | 1 | 0.60 | 3.0 | basic | ~800 | cell |
| t900 | 1 | 1 | 0.60 | 2.6 | basic | ~900 | cell interp |
| t1000 | 1 | 1 | 0.70 | 2.3 | basic | ~1010 | cell |
| t1100 | 2 | 2 | 0.80 | 2.5 | inter | ~1080 | cell |
| t1200 | 2 | 2 | 0.80 | 2.2 | inter | ~1180 | cell interp |
| t1300 | 2 | 2 | 0.90 | 2.0 | inter | ~1320 | cell |
| t1400 | 2 | 2 | **1.00** | 1.7 | inter | ~1400 | curve |
| t1500 | 2 | 2 | **1.00** | 1.6 | full | ~1470 | measured |
| t1600 | 2 | 2 | **1.00** | 1.5 | full | ~1585 | curve |
| t1700 | 2 | 2 | **1.00** | 1.3 | full | ~1730 | curve |
| t1800 | 2 | 2 | **1.00** | 1.2 | full | ~1790 | measured |
| t1900 | 4 | full | 1.00 | 1.6 | full | ~1905 | measured |
| t2000 | 4 | full | 1.00 | 1.0 | full | ~2000 | measured |
| t2100 | 5 | full | 1.00 | 1.2 | full | ~2110 | measured |
| t2200 | 5 | full | 1.00 | 1.0 | full | ~2205 | measured |
| t2300 | 6 | full | 0.60 | 1.0 | full | ~2300 | measured |
| t2400 | 6 | full | 1.00 | 1.0 | full | ~2365 | depth-quantized |
| t2500 | 7 | full | 1.00 | 1.0 | full | ~2475 | depth-quantized |

(Perception column blank in the engine flags = 1.00; "full" qsearch =
flag omitted. ELOs are the pooled best estimate: basement from the
dense `run_extremes` curve, the rest from the monotone confirming pass.)

## Structure (carries into the grid design)

- **Perception is monotone in ELO** and saturates at 1.0 by t1400 — by
  construction, so the solver never sees a dial flip.
- **avg-rank is U-shaped**: high in the basement (no other lever there),
  ~1.0 through the perception-driven middle, rising again from t1400 as
  the judgment lever. Only perception must be monotone.
- **Perception × qsearch is sub-additive**: spans ~195 Elo on d1q0 vs
  ~960 on d4 — a blind base has little left for perception to hide. (So
  perception is NOT depth-compensated in the engine; the calibration
  cells price the interaction empirically.)
- **Perception's knee climbs with depth** (d1q1/d2q2 ≈ 0.6, d4 ≈ 0.6–0.8,
  d6+ ≈ inert by 0.6). Useful range is below the knee.
- **The top (>2100) quantizes to depth**: d5 ≈ 2150, d6 ≈ 2350,
  d7 ≈ 2475–2555, d8 ≈ 2750. Smooth 100-pt rungs up there would need a
  finer strength lever (node caps) — deferred; above the product core.
- **Extreme bands float ±100** unless measured densely (rungs only score
  against each other) — use the `run_extremes.py` protocol.

## Reproduce

`python run_ladder.py` (monotone schedule + confirming pass);
`python run_extremes.py --band basement|ceiling` (dense extremes);
`python run_schedule_cells.py` (the transition/judgment cells).
