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

# (target_elo, config). ONE-DIAL-AT-A-TIME schedule (user 2026-06-07), Maia-
# only anchoring. Each rung-to-rung step changes exactly ONE of {depth,
# qsearch} so there's no multi-dial base-change cliff (pass-1 had a ~380-Elo
# cliff at the old d1q1->d2q2 jump, which starved the basement's anchoring).
# qsearch rises before depth (more human: calculate captures/checks deeper
# before searching wider — d1q2 over d2q1). Schedule:
#   t500-t700  d1 q1      t800-t1200 d1 q2      t1300-t1500 d2 q2
#   t1600-t1800 d2 qinf   t1900-t2500 d4..d7 qinf
# Perception = clamp((elo-300)/900,0,1) rounded 0.05 (faster ramp: 0 at t300,
# 1.0 at t1200) — early Elo gains come from seeing more moves.
#
# Anchoring is MAIA-ONLY: the q1floor bottom anchors fought the ground-truth
# Maia (pass-2 dragged them -120 / pushed Maia +120) and are themselves
# floated, so they're dropped. The Maia MEASURED band is ~1565-1855 (NOT the
# net labels), so the rungs that overlap it (t1600-t1900) anchor directly;
# lower rungs chain down through competitive (~100-Elo) links.
#
# TUNING ORDER (user): lock t1000-t1900 against Maia FIRST
# (--from 1000 --to 1900), then tune the basement (t500-t900) and top
# (t2000-t2500) off the locked mid. eg held at 2 across the mid tuning run
# (believability eg-tiers are a final layer + confirm re-measure). Basement/
# top rungs are placeholders until their pass. Mid ranks are first estimates
# — secant-correct from the measured errors, then re-run.
RUNGS: list[tuple[int, BotConfig]] = [
    # basement (placeholders — tuned after the mid locks; d1q1, d1q2 from t800)
    # eg1 (basic books) so weak bots can CONVERT won endgames instead of
    # wandering: at eg0 the endgame eval is flat, rank-noise has nothing to push
    # against -> 70-move bounce + stalemate (t500-vs-Martin feel-test). eg only
    # fires in recognized endgames, so it buys conversion without touching
    # middlegame strength. May need a small rank bump to offset.
    (500,  BotConfig("t500",  depth=1, qsearch_depth=1, perception=0.20, avg_move_rank=2.8, endgame_skill=1)),
    (600,  BotConfig("t600",  depth=1, qsearch_depth=1, perception=0.35, avg_move_rank=3.1, endgame_skill=1)),
    (700,  BotConfig("t700",  depth=1, qsearch_depth=1, perception=0.45, avg_move_rank=3.2, endgame_skill=1)),
    (800,  BotConfig("t800",  depth=1, qsearch_depth=2, perception=0.55, avg_move_rank=3.0, endgame_skill=1)),
    (900,  BotConfig("t900",  depth=1, qsearch_depth=2, perception=0.65, avg_move_rank=2.8, endgame_skill=1)),
    # mid (tuned this pass against the Maia anchors; eg held at 2)
    (1000, BotConfig("t1000", depth=1, qsearch_depth=2, perception=0.80, avg_move_rank=2.7, endgame_skill=2)),
    (1100, BotConfig("t1100", depth=1, qsearch_depth=2, perception=0.90, avg_move_rank=2.4, endgame_skill=2)),
    (1200, BotConfig("t1200", depth=1, qsearch_depth=2, perception=1.00, avg_move_rank=2.1, endgame_skill=2)),
    (1300, BotConfig("t1300", depth=2, qsearch_depth=2, perception=1.00, avg_move_rank=2.1, endgame_skill=2)),
    (1400, BotConfig("t1400", depth=2, qsearch_depth=2, perception=1.00, avg_move_rank=1.9, endgame_skill=2)),
    (1500, BotConfig("t1500", depth=2, qsearch_depth=2, perception=1.00, avg_move_rank=1.7, endgame_skill=2)),
    (1600, BotConfig("t1600", depth=2, perception=1.00, avg_move_rank=1.6, endgame_skill=2)),
    (1700, BotConfig("t1700", depth=2, perception=1.00, avg_move_rank=1.4, endgame_skill=2)),
    (1800, BotConfig("t1800", depth=2, perception=1.00, avg_move_rank=1.3, endgame_skill=2)),
    (1900, BotConfig("t1900", depth=4, perception=1.00, avg_move_rank=1.4, endgame_skill=2)),
    # top (placeholders — tuned after the mid locks; depth-quantized, qinf)
    # depth-quantized (PROVISIONAL ±100): depth steps ~150-220 and rank slopes
    # near r1.0 are very steep, and the top floats (beats all Maia, loses only
    # to the d8 ceiling — sparse-anchor noise ±100-250, the basement problem
    # mirrored). The even-hundred targets just above a depth ceiling (t2000,
    # t2200, t2400) can't be hit precisely without a finer lever (node caps,
    # deferred). These are the pass-1 measured configs (RMSE ~75) — best
    # available; refine with a dedicated top pass + ceiling bracket later.
    (2000, BotConfig("t2000", depth=4)),                          # ~1934
    (2100, BotConfig("t2100", depth=5, avg_move_rank=1.2)),       # ~2186
    (2200, BotConfig("t2200", depth=5)),                          # ~2124
    (2300, BotConfig("t2300", depth=6, avg_move_rank=1.3)),       # ~2303
    (2400, BotConfig("t2400", depth=6)),                          # ~2282
    (2500, BotConfig("t2500", depth=7)),                          # ~2458
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
    ap.add_argument("--from", dest="lo", type=int, default=0, help="min target Elo (incl.)")
    ap.add_argument("--to", dest="hi", type=int, default=10000, help="max target Elo (incl.)")
    ap.add_argument("--games-per-pair", type=int, default=40)
    ap.add_argument("--concurrency", type=int, default=16)
    ap.add_argument("--sims", type=int, default=400)
    args = ap.parse_args()

    selected = [(t, c) for t, c in RUNGS if args.lo <= t <= args.hi]
    tag = f"{args.lo}_{args.hi}" if (args.lo > 0 or args.hi < 10000) else "full"

    print(f"=== ladder rungs ({tag}, one-dial schedule, Maia-only) ===")
    for tgt, cfg in selected:
        dials = " ".join(cfg.uci_args()[1:-2])
        print(f"{cfg.name:>6}{tgt:>8}  {dials}")
    if args.design_only:
        return

    players: list[Player] = [c for _, c in selected] + [CEILING] + list(maia_ladder())
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
    pgn = out_dir / f"ladder_{tag}.pgn"
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

    # Maia-only anchoring (the q1floor bottom anchors fought the ground truth
    # and are themselves floated — see module docstring).
    measured = {f"maia-{lab}": r for lab, r in anchors.MEASURED_RAPID.items() if r}
    ratings = rate(pgn, loose_anchors=measured, sims=args.sims, out_name=f"ladder_{tag}")

    print("\n=== target vs measured ===")
    print(f"{'rung':>6}{'target':>8}{'measured':>10}{'error':>8}")
    errs = []
    rows = []
    for tgt, cfg in selected:
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

    csv_path = out_dir / f"ladder_{tag}_results.csv"
    with open(csv_path, "w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["name", "target", "elo", "games", "dials"])
        for (name, tgt, e, g), (_, cfg) in zip(rows, selected):
            dials = " ".join(cfg.uci_args()[1:-2])
            w.writerow([name, tgt, "" if e is None else f"{e:.1f}", g or "", dials])
    print(f"\nwrote {csv_path}")


if __name__ == "__main__":
    main()
