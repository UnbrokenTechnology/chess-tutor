"""Dense, boundary-anchored measurement of the FASTER LINEAR-PERCEPTION
ramp basement (t500-t1000), self-hang-redesign binary (2026-06-07).

The full smooth ladder floats the basement (weak rungs lose ~100% to
the mid band -> Ordo can't anchor them; t500 collapsed to -56). Fix:
the run_extremes pattern — a dense, internally-connected basement pool
+ stable boundary anchors, here the q1floor run's RELIABLE d1q1-p0
points (NEW-binary: r2=930, r3=668, r4=478, r5=260; same base, directly
comparable). The ramp rungs rate against those.

Bases d1q1, eg0 (endgame is a separate threshold layered in at ladder
design — kept out here for a clean perception x rank measurement, and so
the rungs rate cleanly against the eg0 anchors). Perception per the
FASTER formula clamp((elo-300)/900,0,1) (user 2026-06-07: ramp perception
sooner — early Elo gains are about seeing more moves), rounded to the
0.05 product grid. Each rung gets WIDE rank brackets: the faster ramp
loads more perception onto the low rungs, and perception sets a strength
floor, so the open question is whether rank can still claw a high-p rung
down to target — the brackets either straddle the target or visibly
overshoot.

Run:  python run_rampbase.py
"""

from __future__ import annotations

import argparse
import csv

from harness import paths
from harness.engines import BotConfig, Player
from harness.gauntlet import TournamentSpec
from harness.gauntlet import run as run_gauntlet
from harness.pools import maia_ladder
from harness.rate import rate


def _c(name, p, r, eg):
    return BotConfig(name, depth=1, qsearch_depth=1, perception=p, avg_move_rank=r, endgame_skill=eg)


# (rung, FASTER-formula perception clamp((elo-300)/900) rounded to 0.05,
#  eg, [wide bracket ranks]). eg0 throughout — see module docstring.
PLAN = [
    (500,  0.20, 0, [5.0, 6.5, 8.0]),
    (600,  0.35, 0, [4.5, 6.0, 7.5]),
    (700,  0.45, 0, [3.5, 5.0, 6.5]),
    (800,  0.55, 0, [3.0, 4.5, 6.0]),
    (900,  0.65, 0, [2.5, 3.5, 5.0]),
    (1000, 0.80, 0, [2.0, 3.0, 4.0]),
]

CONFIGS: list[BotConfig] = []
for tgt, p, eg, ranks in PLAN:
    for r in ranks:
        rn = str(r).replace(".", "")
        CONFIGS.append(_c(f"t{tgt}p{int(p*100):02d}r{rn}", p, r, eg))

# Boundary anchors: the q1floor run's reliable NEW-binary d1q1-p0 points
# (run_selfhang, 2026-06-07). r2..r5 bracket the whole basement rung range
# (260-930) so no rung floats below the anchor floor.
ANCHORS = [
    _c("anch_r2", 0.0, 2.0, 0),
    _c("anch_r3", 0.0, 3.0, 0),
    _c("anch_r4", 0.0, 4.0, 0),
    _c("anch_r5", 0.0, 5.0, 0),
]
ANCHOR_ELO = {"anch_r2": 929.6, "anch_r3": 667.5, "anch_r4": 477.9, "anch_r5": 259.7}

EST_GAMES_PER_SEC = 70


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
    ap = argparse.ArgumentParser(description="Anchored ramp-basement sweep")
    ap.add_argument("--design-only", action="store_true")
    ap.add_argument("--games-per-pair", type=int, default=80)
    ap.add_argument("--concurrency", type=int, default=16)
    ap.add_argument("--sims", type=int, default=400)
    args = ap.parse_args()

    players: list[Player] = [*CONFIGS, *ANCHORS, *maia_ladder()]
    print("=== ramp-basement (anchored) ===")
    for cfg in [*CONFIGS, *ANCHORS]:
        print(f"  {cfg.name:>12}  {' '.join(cfg.uci_args()[1:-2])}")
    if args.design_only:
        return

    n = len(players)
    gpp = max(2, _even(args.games_per_pair))
    pairs = n * (n - 1) // 2
    total = pairs * gpp
    print(
        f"\nround-robin C({n},2)={pairs} x {gpp} = ~{total:,} games "
        f"(~{total / EST_GAMES_PER_SEC / 60:.0f} min)"
    )

    out_dir = paths.runs_dir() / "rampbase"
    out_dir.mkdir(parents=True, exist_ok=True)
    pgn = out_dir / "rampbase.pgn"
    spec = TournamentSpec(
        players=players,
        games_per_pair=gpp,
        concurrency=args.concurrency,
        tournament="roundrobin",
    )
    if _count_results(pgn) >= total:
        print(f"[rampbase] {total} games present — skip play, re-rate")
    else:
        run_gauntlet(spec, pgn_path=pgn)

    ratings = rate(pgn, loose_anchors=ANCHOR_ELO, sims=args.sims, out_name="rampbase")

    print("\n=== ramp-basement (target | perception | rank -> elo) ===")
    rows = []
    for tgt, p, eg, ranks in PLAN:
        print(f"  t{tgt}  p={p:.2f} eg{eg}:")
        for r in ranks:
            rn = str(r).replace(".", "")
            name = f"t{tgt}p{int(p*100):02d}r{rn}"
            rr = ratings.get(name)
            if rr is None:
                print(f"      r{r}: excluded")
                rows.append((name, None))
                continue
            err = "----" if rr.error is None else f"{rr.error:.0f}"
            print(f"      r{r}: {rr.rating:>6.0f}  +/-{err}")
            rows.append((name, rr.rating))

    print("\n=== anchors (should reproduce their pinned values) ===")
    for a in ANCHORS:
        rr = ratings.get(a.name)
        if rr:
            print(f"  {a.name:<10}{rr.rating:>7.0f}  (pinned {ANCHOR_ELO[a.name]:.0f})")

    csv_path = out_dir / "rampbase_results.csv"
    with open(csv_path, "w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["name", "elo"])
        for name, e in rows:
            w.writerow([name, "" if e is None else f"{e:.1f}"])
    print(f"\nwrote {csv_path}")


if __name__ == "__main__":
    main()
