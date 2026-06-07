# Perception-era ELO ladder (FINAL candidate, 2026-06-07)

Lever set: depth · qsearch-depth · **perception** · avg-rank · endgame
tier. (miss%/blunder% removed — both emerge organically from
perception.) Rank on the 0.1 grid, perception on the 0.05 grid.
ELOs are **lichess-anchored** (measured Maia rapid anchors); chess.com
runs ~200–350 lower (e.g. test t500 against chess.com's ~250 bot).

Sources: basement = dense d1q0 rank curve (100 games/pair,
`runs/extremes/basement_results.csv`); middle = ladder passes 1+2
(`runs/ladder/ladder_pass{1,2}.pgn`); ceiling = dense ceiling run
(`runs/extremes/ceiling_results.csv`). Reproduce: `python run_ladder.py`.

| rung | depth | qsearch | perception | rank | endgame | expected ELO | notes |
|------|-------|---------|------------|------|---------|--------------|-------|
| t100 | 1 | 0 | 0.00 | 5.0 | none | ~95 | dense curve, ±65 |
| t200 | 1 | 0 | 0.00 | 4.0 | none | ~195 | dense curve |
| t300 | 1 | 0 | 0.00 | 3.5 | none | ~300 | dense curve (interp) |
| t400 | 1 | 0 | 0.00 | 3.0 | none | ~405 | dense curve (interp) |
| t500 | 1 | 0 | 0.00 | 2.6 | none | ~485 | dense curve (interp) |
| t600 | 1 | 0 | 0.00 | 2.0 | none | ~600 | dense curve |
| t700 | 1 | 0 | 0.00 | 1.6 | basic | ~705 | extrapolated from p1/p2 |
| t800 | 1 | 0 | 0.10 | 1.0 | basic | ~810 | extrapolated from p1/p2 |
| t900 | 1 | 1 | 0.00 | 1.3 | basic | ~930 | p2: 914 |
| t1000 | 2 | 2 | 0.00 | 1.0 | basic | ~1020 | p1/p2: 1025/1011 |
| t1100 | 1 | 1 | 0.10 | 1.3 | basic | ~1115 | p2: 1117 |
| t1200 | 2 | 2 | 0.10 | 1.2 | inter | ~1190 | p2: 1164 |
| t1300 | 2 | 2 | 0.15 | 1.0 | inter | ~1310 | p1/p2: 1320/1298 |
| t1400 | 2 | 2 | 0.20 | 1.0 | inter | ~1410 | p1/p2: 1414/1404 |
| t1500 | 2 | 2 | 0.25 | 1.0 | inter | ~1490 | p2: 1490 |
| t1600 | 2 | 2 | 0.35 | 1.0 | full | ~1623 | p1/p2: 1622/1623 |
| t1700 | 2 | 2 | 0.45 | 1.0 | full | ~1715 | p1: 1733 |
| t1800 | 2 | 2 | 0.60 | 1.0 | full | ~1800 | p1/p2: 1815/1787 |
| t1900 | 4 | full | 0.50 | 1.0 | full | ~1880 | p2: 1872 |
| t2000 | 4 | full | 1.00 | 1.0 | full | ~2005 | p1/p2: 2043/1969 |
| t2100 | 5 | full | 0.60 | 1.0 | full | ~2100 | p1/p2: 2107/2089 |
| t2200 | 5 | full | 1.00 | 1.0 | full | ~2150 | depth-quantized (−50) |
| t2300 | 6 | full | 0.60 | 1.0 | full | ~2315 | dense ceiling |
| t2400 | 6 | full | 1.00 | 1.0 | full | ~2360 | depth-quantized (−40) |
| t2500 | 7 | full | 1.00 | 1.0 | full | ~2555 | depth-quantized (+55) |

## Known structure (carries into the grid design)

- **Perception × qsearch is sub-additive**: on d1q0 the dial spans only
  ~195 Elo (a blind bot is already blind); on d4 it spans ~960. The
  lever's power scales with base strength.
- **Perception saturates above its knee**, and the knee climbs with
  depth (d1q1/d2q2 ≈ 0.6, d4 ≈ 0.6–0.8, d6+ ≈ inert by 0.6 with a
  razor shoulder at 0.5–0.6). Useful dial range is below the knee.
- **The top quantizes to depth**: d5 ≈ 2150, d6 ≈ 2350, d7 ≈ 2555,
  d8 ≈ 2770. Smooth 100-point rungs above 2100 would need a finer
  strength lever up there (e.g. node caps) — deferred.
- **Extreme-band measurements float** (±100 run-to-run) unless played
  densely: rungs below ~800 and above ~2200 only score against each
  other, so use `run_extremes.py`-style dense mini-pools for them.
- **Rank slopes shrink under perception** (~−120/unit on d1q0-p0 vs
  ~−150 bare; ~−380..−450/unit on d2q2 mid-perception vs ~550 bare).
