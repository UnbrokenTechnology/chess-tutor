"""Perception-era ladder: measure the PASS-1 predicted rungs
(100..2500 by 100) round-robin with the Maia ladder, report
predicted-vs-measured, iterate.

PASS-1 predictions interpolate the measured curves (2026-06-07):

- perception sweeps: d1q1 / d2q2 (runs/perception_sweep/, 0.1 res),
  d1q0 / d4 (runs/rung_sweeps/, 0.2 res)
- rank x perception probes (runs/rung_sweeps/): d2q2 rank slope
  ~-380..-450/unit under perception; d1q0-p0 ~-120/unit at low rank
- basement extrapolation: the (still valid, p=1.0) bare d1q0 rank curve
  r1 971 .. r8 144, shifted by the measured p0 offset (~-130 at r1-r2)
- ceiling: d6 = 2411, d8 = 2795 (d8 plays as POOL CEILING, not a rung);
  d5 / d7 / d6-with-perception rungs are pure guesses to be corrected
  in pass 2.

Dial granularity: rank on the 0.1 grid (GUI step — never anchor a rung
the product can't express), perception on the 0.05 grid.

Endgame tiers by band (same convention the retired bands used):
<=500 none, 600-1100 basic, 1200-1500 intermediate, 1600+ full.

Run:  python run_ladder.py            # play + rate + compare
      python run_ladder.py --design-only
"""

from __future__ import annotations

import argparse
import csv

from harness import anchors, paths
from harness.engines import BotConfig, Player
from harness.gauntlet import TournamentSpec
from harness.gauntlet import run as run_gauntlet
from harness.pools import maia_ladder
from harness.rate import rate

# (target_elo, config). PASS-1 predictions; see module doc for sources.
RUNGS: list[tuple[int, BotConfig]] = [
    # --- FINAL (2026-06-07): basement from the dense extreme run
    # (runs/extremes/basement_results.csv, 100 gpp), middle pooled from
    # passes 1+2, ceiling from the dense ceiling run. "exp" = expected
    # measured value. Top quantizes to depth steps (perception is inert
    # above its knee at d6+): 2200/2400 land ~-50, 2500 ~+50.
    (100,  BotConfig("t100",  depth=1, qsearch_depth=0, perception=0.0, avg_move_rank=5.0, endgame_skill=0)),  # exp  96
    (200,  BotConfig("t200",  depth=1, qsearch_depth=0, perception=0.0, avg_move_rank=4.0, endgame_skill=0)),  # exp 197
    (300,  BotConfig("t300",  depth=1, qsearch_depth=0, perception=0.0, avg_move_rank=3.5, endgame_skill=0)),  # exp ~300
    (400,  BotConfig("t400",  depth=1, qsearch_depth=0, perception=0.0, avg_move_rank=3.0, endgame_skill=0)),  # exp ~403
    (500,  BotConfig("t500",  depth=1, qsearch_depth=0, perception=0.0, avg_move_rank=2.6, endgame_skill=0)),  # exp ~486
    (600,  BotConfig("t600",  depth=1, qsearch_depth=0, perception=0.0, avg_move_rank=2.0, endgame_skill=0)),  # exp 600
    (700,  BotConfig("t700",  depth=1, qsearch_depth=0, perception=0.0, avg_move_rank=1.6, endgame_skill=1)),  # exp ~705
    (800,  BotConfig("t800",  depth=1, qsearch_depth=0, perception=0.1, endgame_skill=1)),                      # exp ~810
    (900,  BotConfig("t900",  depth=1, qsearch_depth=1, perception=0.0, avg_move_rank=1.3, endgame_skill=1)),  # exp ~930
    (1000, BotConfig("t1000", depth=2, qsearch_depth=2, perception=0.0, endgame_skill=1)),                      # exp ~1018
    (1100, BotConfig("t1100", depth=1, qsearch_depth=1, perception=0.1, avg_move_rank=1.3, endgame_skill=1)),  # exp ~1117
    (1200, BotConfig("t1200", depth=2, qsearch_depth=2, perception=0.1, avg_move_rank=1.2, endgame_skill=2)),  # exp ~1190
    (1300, BotConfig("t1300", depth=2, qsearch_depth=2, perception=0.15, endgame_skill=2)),                     # exp ~1310
    (1400, BotConfig("t1400", depth=2, qsearch_depth=2, perception=0.2, endgame_skill=2)),                      # exp ~1410
    (1500, BotConfig("t1500", depth=2, qsearch_depth=2, perception=0.25, endgame_skill=2)),                     # exp ~1490
    (1600, BotConfig("t1600", depth=2, qsearch_depth=2, perception=0.35)),                                      # exp ~1623
    (1700, BotConfig("t1700", depth=2, qsearch_depth=2, perception=0.45)),                                      # exp ~1715
    (1800, BotConfig("t1800", depth=2, qsearch_depth=2, perception=0.6)),                                       # exp ~1800
    (1900, BotConfig("t1900", depth=4, perception=0.5)),                                                        # exp ~1880
    (2000, BotConfig("t2000", depth=4)),                                                                        # exp ~2005
    (2100, BotConfig("t2100", depth=5, perception=0.6)),                                                        # exp ~2100
    (2200, BotConfig("t2200", depth=5)),                                                                        # exp ~2150 (-50; depth-quantized)
    (2300, BotConfig("t2300", depth=6, perception=0.6)),                                                        # exp ~2315
    (2400, BotConfig("t2400", depth=6)),                                                                        # exp ~2360 (-40; depth-quantized)
    (2500, BotConfig("t2500", depth=7)),                                                                        # exp ~2555 (+55; depth-quantized)
]


# Pool ceiling so the top rungs aren't all-wins (rated but not a rung).
CEILING = BotConfig("ceil-d8", depth=8)

EST_GAMES_PER_SEC = 45


def _count_results(pgn) -> int:
    if not pgn.exists():
        return 0
    return sum(
        1
        for ln in open(pgn, "r", encoding="utf-8", errors="ignore")
        if ln.startswith("[Result ")
    )


def _even(n: int) -> int:
    return n if n % 2 == 0 else n + 1


def main() -> None:
    ap = argparse.ArgumentParser(description="Measure the perception-era ladder")
    ap.add_argument("--design-only", action="store_true")
    ap.add_argument("--games-per-pair", type=int, default=40)
    ap.add_argument("--concurrency", type=int, default=16)
    ap.add_argument("--sims", type=int, default=400)
    args = ap.parse_args()

    print("=== ladder rungs (FINAL) ===")
    for tgt, cfg in RUNGS:
        dials = " ".join(cfg.uci_args()[1:-2])
        print(f"{cfg.name:>6}{tgt:>8}  {dials}")
    if args.design_only:
        return

    players: list[Player] = [c for _, c in RUNGS] + [CEILING] + list(maia_ladder())
    n = len(players)
    gpp = max(2, _even(args.games_per_pair))
    pairs = n * (n - 1) // 2
    total = pairs * gpp
    print(
        f"\nround-robin C({n},2)={pairs} x {gpp} = ~{total:,} games "
        f"(~{total / EST_GAMES_PER_SEC / 60:.0f} min)"
    )

    out_dir = paths.runs_dir() / "ladder"
    out_dir.mkdir(parents=True, exist_ok=True)
    pgn = out_dir / "ladder_final.pgn"
    spec = TournamentSpec(
        players=players,
        games_per_pair=gpp,
        concurrency=args.concurrency,
        tournament="roundrobin",
    )
    if _count_results(pgn) >= total:
        print(f"[ladder] {total} games present — skip play, re-rate")
    else:
        run_gauntlet(spec, pgn_path=pgn)

    measured = {f"maia-{lab}": r for lab, r in anchors.MEASURED_RAPID.items() if r}
    ratings = rate(pgn, loose_anchors=measured, sims=args.sims, out_name="ladder")

    print("\n=== target vs measured ===")
    print(f"{'rung':>6}{'target':>8}{'measured':>10}{'error':>8}")
    errs = []
    rows = []
    for tgt, cfg in RUNGS:
        r = ratings.get(cfg.name)
        if r is None:
            print(f"{cfg.name:>6}{tgt:>8}{'excluded':>10}")
            rows.append((cfg.name, tgt, None, None))
            continue
        err = r.rating - tgt
        errs.append(err)
        rows.append((cfg.name, tgt, r.rating, r.played))
        print(f"{cfg.name:>6}{tgt:>8}{r.rating:>10.0f}{err:>+8.0f}")
    if errs:
        rmse = (sum(e * e for e in errs) / len(errs)) ** 0.5
        bias = sum(errs) / len(errs)
        print(f"\nmodel error: bias {bias:+.0f}, RMSE {rmse:.0f}")

    print("\n=== maia anchors (sanity) ===")
    for m in maia_ladder():
        r = ratings.get(m.name)
        if r:
            print(f"  {m.name:<10}{r.rating:>7.0f}")
    ceil = ratings.get(CEILING.name)
    if ceil:
        print(f"  {CEILING.name:<10}{ceil.rating:>7.0f}")

    csv_path = out_dir / "ladder_results.csv"
    with open(csv_path, "w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["name", "target", "elo", "games", "dials"])
        for (name, tgt, e, g), (_, cfg) in zip(rows, RUNGS):
            dials = " ".join(cfg.uci_args()[1:-2])
            w.writerow([name, tgt, "" if e is None else f"{e:.1f}", g or "", dials])
    print(f"\nwrote {csv_path}")


if __name__ == "__main__":
    main()
