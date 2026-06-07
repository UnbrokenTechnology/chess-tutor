"""Prediction-pass sweeps for the perception-era rung rebuild
(PLAN-perception.md → new ladder, 100..2500 by 100).

Measures the curves the rung DESIGNER needs, in one round-robin with the
Maia ladder:

- **d1q0 × perception** — the basement curve (perception on a
  tactically-blind base; rungs 100–800 live here, stacked with rank).
- **d4 × perception** — the upper-mid curve (rungs ~1600–2000).
- **d6 / d8 bare** — the ceiling rungs (~2400–2550?).
- **rank × perception interaction probes** — six (base, rank, p) cells
  so the designer can estimate how the (still-valid, p=1.0) bare rank
  curves shift when perception is active. Pre-perception rank curves
  measured rank slopes scaling with base strength; the open question is
  whether a perception-weakened base behaves like a natively-weak one.

Existing data this complements (runs/perception_sweep/): d1q1 and d2q2
perception curves at 0.1 resolution.

Run:  python run_rung_sweeps.py
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

CONFIGS: list[BotConfig] = [
    # --- d1q0 × perception (basement curve) ---
    BotConfig("d1q0p00", depth=1, qsearch_depth=0, perception=0.0),
    BotConfig("d1q0p02", depth=1, qsearch_depth=0, perception=0.2),
    BotConfig("d1q0p04", depth=1, qsearch_depth=0, perception=0.4),
    BotConfig("d1q0p06", depth=1, qsearch_depth=0, perception=0.6),
    BotConfig("d1q0p08", depth=1, qsearch_depth=0, perception=0.8),
    BotConfig("d1q0p10", depth=1, qsearch_depth=0),
    # --- d4 × perception (upper-mid curve) ---
    BotConfig("d4p00", depth=4, perception=0.0),
    BotConfig("d4p02", depth=4, perception=0.2),
    BotConfig("d4p04", depth=4, perception=0.4),
    BotConfig("d4p06", depth=4, perception=0.6),
    BotConfig("d4p08", depth=4, perception=0.8),
    BotConfig("d4p10", depth=4),
    # --- ceiling rungs ---
    BotConfig("d6", depth=6),
    BotConfig("d8", depth=8),
    # --- rank × perception interaction probes ---
    BotConfig("d2q2r2p02", depth=2, qsearch_depth=2, avg_move_rank=2.0, perception=0.2),
    BotConfig("d2q2r2p06", depth=2, qsearch_depth=2, avg_move_rank=2.0, perception=0.6),
    BotConfig("d2q2r3p04", depth=2, qsearch_depth=2, avg_move_rank=3.0, perception=0.4),
    BotConfig("d1q0r2p00", depth=1, qsearch_depth=0, avg_move_rank=2.0, perception=0.0),
    BotConfig("d1q0r2p04", depth=1, qsearch_depth=0, avg_move_rank=2.0, perception=0.4),
    BotConfig("d1q0r3p02", depth=1, qsearch_depth=0, avg_move_rank=3.0, perception=0.2),
]

EST_GAMES_PER_SEC = 49


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
    ap = argparse.ArgumentParser(description="Rung-design prediction sweeps")
    ap.add_argument("--design-only", action="store_true")
    ap.add_argument("--games-per-pair", type=int, default=40)
    ap.add_argument("--concurrency", type=int, default=16)
    ap.add_argument("--sims", type=int, default=400)
    args = ap.parse_args()

    print("=== rung-design sweeps ===")
    for cfg in CONFIGS:
        dials = " ".join(cfg.uci_args()[1:-2])
        print(f"  {cfg.name:>12}  {dials}")
    if args.design_only:
        return

    players: list[Player] = [*CONFIGS, *maia_ladder()]
    n = len(players)
    gpp = max(2, _even(args.games_per_pair))
    pairs = n * (n - 1) // 2
    total = pairs * gpp
    print(
        f"\nround-robin C({n},2)={pairs} x {gpp} = ~{total:,} games "
        f"(~{total / EST_GAMES_PER_SEC / 60:.0f} min)"
    )

    out_dir = paths.runs_dir() / "rung_sweeps"
    out_dir.mkdir(parents=True, exist_ok=True)
    pgn = out_dir / "rung_sweeps.pgn"
    spec = TournamentSpec(
        players=players,
        games_per_pair=gpp,
        concurrency=args.concurrency,
        tournament="roundrobin",
    )
    if _count_results(pgn) >= total:
        print(f"[sweeps] {total} games present — skip play, re-rate")
    else:
        run_gauntlet(spec, pgn_path=pgn)

    measured = {f"maia-{lab}": r for lab, r in anchors.MEASURED_RAPID.items() if r}
    ratings = rate(pgn, loose_anchors=measured, sims=args.sims, out_name="rung_sweeps")

    print("\n=== measured (config order) ===")
    print(f"{'config':>12}{'elo':>8}{'+/-':>6}{'games':>7}")
    rows = []
    for cfg in CONFIGS:
        r = ratings.get(cfg.name)
        if r is None:
            print(f"{cfg.name:>12}{'excluded':>8}")
            rows.append((cfg.name, None, None))
            continue
        err = "----" if r.error is None else f"{r.error:.0f}"
        print(f"{cfg.name:>12}{r.rating:>8.0f}{err:>6}{r.played:>7}")
        rows.append((cfg.name, r.rating, r.played))

    print("\n=== maia anchors (sanity) ===")
    for m in maia_ladder():
        r = ratings.get(m.name)
        if r:
            print(f"  {m.name:<10}{r.rating:>7.0f}")

    csv_path = out_dir / "rung_sweeps_results.csv"
    with open(csv_path, "w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["name", "elo", "games"])
        for name, e, g in rows:
            w.writerow([name, "" if e is None else f"{e:.1f}", g or ""])
    print(f"\nwrote {csv_path}")


if __name__ == "__main__":
    main()
