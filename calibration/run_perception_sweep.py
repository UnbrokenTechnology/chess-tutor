"""Perception-dial sweep: one base (depth/qsearch) x perception 0.0..1.0
in 0.1 steps, round-robin with the Maia ladder, rated -> the dial->Elo
curve shape (linear? saturating? stepped?).

The perception lever is discrete under the hood (step thresholds, finite
move sets, a margin-curve corner), so the dial->Elo relation is an
empirical question; sampling over hundreds of games per config smooths
the per-move discreteness. Round-robin (configs play each other AND the
Maia ladder) keeps weak rungs ratable: a p=0.0 config that loses every
Maia game still connects through its neighbours.

Run:  python run_perception_sweep.py --base d2q2   # then --base d1q1
      python run_perception_sweep.py --base d2q2 --design-only
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

# Base (depth, qsearch) pairs the sweep can run over. Everything else is
# the engine's no-op profile: rank 1.0, no miss/blunder, full endgame
# books — perception is the ONLY moving dial.
BASES: dict[str, dict] = {
    "d2q2": dict(depth=2, qsearch_depth=2),
    "d1q1": dict(depth=1, qsearch_depth=1),
}

EST_GAMES_PER_SEC = 49


def configs_for(base: str) -> list[tuple[float, BotConfig]]:
    out = []
    for i in range(11):
        p = i / 10.0
        name = f"{base}p{i:02d}"  # d2q2p00 .. d2q2p10
        out.append((p, BotConfig(name, perception=p, **BASES[base])))
    return out


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
    ap = argparse.ArgumentParser(description="Perception-dial Elo sweep")
    ap.add_argument("--base", choices=sorted(BASES), required=True)
    ap.add_argument("--design-only", action="store_true")
    ap.add_argument("--games-per-pair", type=int, default=40)
    ap.add_argument("--concurrency", type=int, default=16)
    ap.add_argument("--sims", type=int, default=400)
    args = ap.parse_args()

    sweep = configs_for(args.base)
    print(f"=== perception sweep on {args.base} ===")
    for p, cfg in sweep:
        dials = " ".join(cfg.uci_args()[1:-2])
        print(f"  {cfg.name:>10}  p={p:.1f}  {dials}")
    if args.design_only:
        return

    players: list[Player] = [c for _, c in sweep] + list(maia_ladder())
    n = len(players)
    gpp = max(2, _even(args.games_per_pair))
    pairs = n * (n - 1) // 2
    total = pairs * gpp
    print(
        f"\nround-robin C({n},2)={pairs} x {gpp} = ~{total:,} games "
        f"(~{total / EST_GAMES_PER_SEC / 60:.0f} min)"
    )

    out_dir = paths.runs_dir() / "perception_sweep"
    out_dir.mkdir(parents=True, exist_ok=True)
    pgn = out_dir / f"{args.base}.pgn"
    spec = TournamentSpec(
        players=players,
        games_per_pair=gpp,
        concurrency=args.concurrency,
        tournament="roundrobin",
    )
    if _count_results(pgn) >= total:
        print(f"[sweep] {total} games present — skip play, re-rate")
    else:
        run_gauntlet(spec, pgn_path=pgn)

    measured = {f"maia-{lab}": r for lab, r in anchors.MEASURED_RAPID.items() if r}
    ratings = rate(
        pgn, loose_anchors=measured, sims=args.sims, out_name=f"perception_{args.base}"
    )

    # ---- the curve ----
    print(f"\n=== {args.base}: perception -> Elo ===")
    print(f"{'p':>5}{'elo':>8}{'+/-':>6}{'step':>8}")
    prev = None
    rows = []
    for p, cfg in sweep:
        r = ratings.get(cfg.name)
        if r is None:
            print(f"{p:>5.1f}{'excluded':>8}")
            rows.append((p, None, None))
            continue
        err = "----" if r.error is None else f"{r.error:.0f}"
        step = "" if prev is None else f"{r.rating - prev:>+8.0f}"
        print(f"{p:>5.1f}{r.rating:>8.0f}{err:>6}{step}")
        rows.append((p, r.rating, r.played))
        prev = r.rating

    valid = [(p, e) for p, e, _ in rows if e is not None]
    if len(valid) >= 3:
        lo, hi = valid[0][1], valid[-1][1]
        print(f"\nspan: {lo:.0f} -> {hi:.0f}  ({hi - lo:+.0f} Elo across the dial)")

    # ---- maia sanity ----
    print("\n=== maia anchors (sanity) ===")
    for m in maia_ladder():
        r = ratings.get(m.name)
        if r:
            print(f"  {m.name:<10}{r.rating:>7.0f}")

    csv_path = out_dir / f"{args.base}_results.csv"
    with open(csv_path, "w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["base", "perception", "elo", "games"])
        for p, e, g in rows:
            w.writerow([args.base, f"{p:.1f}", "" if e is None else f"{e:.1f}", g or ""])
    print(f"\nwrote {csv_path}")


if __name__ == "__main__":
    main()
