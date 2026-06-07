"""Dense extreme-band measurement for the perception-era ladder.

The pass-1/pass-2 ladder runs showed the MIDDLE (900-2100) locked
within ~±50, but the extremes wobble ±100+ run-to-run: rungs below
~800 win ~0% against everything above them, so the basement sub-ladder
floats as a block (and the ceiling mirrors it above the Maia band).
Tweak passes can't out-tune that noise — only denser games can.

Two dense mini-round-robins at high games-per-pair:

- **basement**: the definitive d1q0+p0(+eg0) rank curve, ranks
  2.0..7.0, loose-anchored on the pooled pass-1/pass-2 values of the
  stable boundary rungs (t800-shape ~772, t900-shape ~936,
  t1000-shape ~1018).
- **ceiling**: d5..d8 with perception steps, loose-anchored on the
  measured maia-1700/1800/1900.

Run:  python run_extremes.py --band basement
      python run_extremes.py --band ceiling
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


def _basement_players() -> tuple[list[BotConfig], dict[str, float]]:
    ranks = [2.0, 2.4, 2.8, 3.2, 3.6, 4.0, 4.5, 5.0, 5.5, 6.0, 6.5, 7.0]
    configs = [
        BotConfig(
            f"b-r{str(r).replace('.', '')}",
            depth=1,
            qsearch_depth=0,
            perception=0.0,
            avg_move_rank=r,
            endgame_skill=0,
        )
        for r in ranks
    ]
    # Boundary anchors: the stable rungs just above the basement, at
    # their POOLED pass-1/pass-2 measurements.
    anchors_cfg = [
        BotConfig("anch800", depth=1, qsearch_depth=0, perception=0.05, endgame_skill=1),
        BotConfig("anch900", depth=1, qsearch_depth=1, perception=0.0, avg_move_rank=1.3, endgame_skill=1),
        BotConfig("anch1000", depth=2, qsearch_depth=2, perception=0.0, endgame_skill=1),
    ]
    loose = {"anch800": 772.0, "anch900": 936.0, "anch1000": 1018.0}
    return configs + anchors_cfg, loose


def _ceiling_players() -> tuple[list[Player], dict[str, float]]:
    configs: list[Player] = [
        BotConfig("c-d5", depth=5),
        BotConfig("c-d5p08", depth=5, perception=0.8),
        BotConfig("c-d6", depth=6),
        BotConfig("c-d6p06", depth=6, perception=0.6),
        BotConfig("c-d6p07", depth=6, perception=0.7),
        BotConfig("c-d6p08", depth=6, perception=0.8),
        BotConfig("c-d7", depth=7),
        BotConfig("c-d7p07", depth=7, perception=0.7),
        BotConfig("c-d7p08", depth=7, perception=0.8),
        BotConfig("c-d8", depth=8),
    ]
    maia_top = [m for m in maia_ladder() if m.rating >= 1700]
    loose = {
        f"maia-{lab}": r
        for lab, r in anchors.MEASURED_RAPID.items()
        if r and lab >= 1700
    }
    return configs + maia_top, loose


def _count_results(pgn) -> int:
    if not pgn.exists():
        return 0
    return sum(
        1
        for ln in open(pgn, "r", encoding="utf-8", errors="ignore")
        if ln.startswith("[Result ")
    )


def main() -> None:
    ap = argparse.ArgumentParser(description="Dense extreme-band ladder measurement")
    ap.add_argument("--band", choices=["basement", "ceiling"], required=True)
    ap.add_argument("--games-per-pair", type=int, default=100)
    ap.add_argument("--concurrency", type=int, default=16)
    ap.add_argument("--sims", type=int, default=400)
    args = ap.parse_args()

    if args.band == "basement":
        players, loose = _basement_players()
    else:
        players, loose = _ceiling_players()

    n = len(players)
    gpp = args.games_per_pair + (args.games_per_pair % 2)
    pairs = n * (n - 1) // 2
    total = pairs * gpp
    print(f"=== {args.band}: {n} players, C({n},2)={pairs} x {gpp} = ~{total:,} games ===")
    for p in players:
        if isinstance(p, BotConfig):
            print(f"  {p.name:>10}  {' '.join(p.uci_args()[1:-2])}")

    out_dir = paths.runs_dir() / "extremes"
    out_dir.mkdir(parents=True, exist_ok=True)
    pgn = out_dir / f"{args.band}.pgn"
    spec = TournamentSpec(
        players=players,
        games_per_pair=gpp,
        concurrency=args.concurrency,
        tournament="roundrobin",
    )
    if _count_results(pgn) >= total:
        print(f"[{args.band}] {total} games present — skip play, re-rate")
    else:
        run_gauntlet(spec, pgn_path=pgn)

    ratings = rate(pgn, loose_anchors=loose, sims=args.sims, out_name=f"extremes_{args.band}")

    print(f"\n=== {args.band}: measured ===")
    rows = []
    for p in players:
        r = ratings.get(p.name)
        if r is None:
            print(f"  {p.name:>10}  excluded")
            rows.append((p.name, None, None))
            continue
        err = "----" if r.error is None else f"{r.error:.0f}"
        print(f"  {p.name:>10}{r.rating:>8.0f}  +/-{err}")
        rows.append((p.name, r.rating, r.played))

    csv_path = out_dir / f"{args.band}_results.csv"
    with open(csv_path, "w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["name", "elo", "games"])
        for name, e, g in rows:
            w.writerow([name, "" if e is None else f"{e:.1f}", g or ""])
    print(f"\nwrote {csv_path}")


if __name__ == "__main__":
    main()
