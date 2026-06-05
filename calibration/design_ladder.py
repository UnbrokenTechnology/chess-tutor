"""Phase A, round 3 — DESIGN the anchor rungs from the linear knob models
learned in round 2, then measure how close we landed.

Instead of hand-picking configs, we now *invert* what build_ladder.py
measured: rank, miss, and blunder are each ~linear levers (round 2), so
for a desired Elo we can compute a config we *predict* lands there. This
script builds one rung per target Elo, prints the predicted design, plays
the round-robin (+ Maia), and reports **predicted vs measured** — both to
get good anchor rungs AND to validate the forward model the eventual
solver will invert.

Construction per target (see design_bot):
  1. Base (depth x qsearch) sets the band — rank only weakens DOWNWARD, so
     pick the cheapest base whose full-strength Elo sits comfortably above
     the target. d1-q0 (~976) -> d1-q1 (~1637) -> d4 (~1996); d5/d6 are
     fixed ceiling rungs (nothing rank-tunes above d4 in our data).
  2. Miss + blunder are modest, human-weighted garnish (miss > blunder per
     round 2), ramped down as Elo rises. Their Elo cost (miss ~-5/%,
     blunder ~-2.5/%) is SUBTRACTED from the rank job so the totals still
     land on target.
  3. Rank closes the remaining gap, via the inverted (rank -> Elo) curves.

Because the levers are sub-additive (round 2: m20+b20 < m20 plus b20), the
designs will land slightly STRONG of target; the measured-vs-predicted
error tells us the correction. Edit the MODELS / ramps at the top and
re-run to iterate.

Run: python design_ladder.py --design-only   # just print the predicted rungs
     python design_ladder.py                 # design + play + compare
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

# ---------------------------------------------------------------------------
# Round-2 measured models (lichess/Maia scale). EDIT to iterate.
# ---------------------------------------------------------------------------

#: Measured (avg_move_rank -> Elo) curves per base, best-first. Inverted by
#: `invert_rank` to solve for the rank that hits a residual Elo. d4 has no
#: measured rank sweep — we ASSUME the sighted slope (a touch steeper for
#: full qsearch); the run tells us whether that holds.
RANK_CURVES: dict[str, list[tuple[float, int]]] = {
    "d1-q0": [(1.0, 976), (1.5, 760), (2.0, 512), (3.0, 268), (4.0, -29)],
    "d1-q1": [(1.0, 1637), (1.5, 1358), (2.0, 1089)],
    "d4": [(1.0, 1996), (2.0, 1376)],  # slope -620/unit (extrapolated guess)
}

#: base name -> (BotConfig dial kwargs at rank 1, full-strength Elo).
BASES: dict[str, tuple[dict, int]] = {
    "d1-q0": (dict(depth=1, qsearch_depth=0), 976),
    "d1-q1": (dict(depth=1, qsearch_depth=1), 1637),
    "d4": (dict(depth=4), 1996),
}
#: Order to try bases (weakest first); pick the first whose full-strength
#: Elo clears the target by `BASE_MARGIN` (room for rank + noise to weaken).
BASE_ORDER = ["d1-q0", "d1-q1", "d4"]
BASE_MARGIN = 120

# Elo lost per percentage-point of each noise lever (round 2).
MISS_SLOPE = 5.0
BLUNDER_SLOPE = 2.5

# Targets to design a rung for (every 100). Below ~200 the basement isn't
# precisely placeable; above d4(~1996) we use fixed d5/d6 ceiling rungs.
TARGETS = list(range(300, 1801, 100))

# Fixed rungs outside the design range: the measured basement floor and the
# top ceiling ladder (no base rank-tunes above d4 in our data).
FIXED: list[BotConfig] = [
    BotConfig("floor-d1q0r4m40", depth=1, qsearch_depth=0, avg_move_rank=4.0, miss_chance=0.4),  # ~-200
    BotConfig("ceil-d4", depth=4),   # ~1996
    BotConfig("ceil-d5", depth=5),   # ~2227
    BotConfig("ceil-d6", depth=6),   # ~2426 (likely all-win -> excluded; that's its job)
]

TARGET_SPACING = 250
REDUNDANT = 100
EST_GAMES_PER_SEC = 60


def _clamp01(x: float) -> float:
    return max(0.0, min(1.0, x))


def miss_for(target: int) -> float:
    """Modest, human miss% — high for weak bots, ~0 by ~1500. Capped 0.35
    so it stays inside the measured range (round 2 went to 0.30)."""
    return round(0.35 * _clamp01(1 - target / 1500), 2)


def blunder_for(target: int) -> float:
    """Gentler blunder% (miss-weighted per round 2), ~0 by ~1200, cap 0.20."""
    return round(0.20 * _clamp01(1 - target / 1200), 2)


def invert_rank(curve: list[tuple[float, int]], elo: float) -> float:
    """Solve (rank -> Elo) for the rank that yields `elo`, by piecewise-
    linear interpolation; extrapolate below the last point with its slope.
    Clamped to rank >= 1 (can't play better than the engine's best)."""
    if elo >= curve[0][1]:
        return 1.0
    for (r0, e0), (r1, e1) in zip(curve, curve[1:]):
        if e0 >= elo >= e1:
            frac = (e0 - elo) / (e0 - e1)
            return r0 + frac * (r1 - r0)
    (r0, e0), (r1, e1) = curve[-2], curve[-1]
    rank = r1 + (elo - e1) * (r1 - r0) / (e1 - e0)
    return max(1.0, rank)


def design_bot(target: int) -> tuple[BotConfig, dict]:
    """Design a config we predict lands near `target` Elo. Returns the
    config plus a dict of the design decisions (for the printout)."""
    base_name = next(
        (b for b in BASE_ORDER if BASES[b][1] >= target + BASE_MARGIN),
        BASE_ORDER[-1],
    )
    kwargs, full = BASES[base_name]
    miss = miss_for(target)
    blunder = blunder_for(target)
    noise_cost = MISS_SLOPE * miss * 100 + BLUNDER_SLOPE * blunder * 100
    # Rank must bring `full` down to (target + noise_cost); the noise then
    # carries it the rest of the way to `target`.
    residual = target + noise_cost
    # Round rank to 0.1 — the GUI slider's step. A config whose rank isn't a
    # 0.1-multiple can't be reproduced in the product, so we never measure
    # one (1.25 isn't settable; it also measured ~100 Elo high). The final
    # solver must do the same when emitting a config for a target Elo.
    rank = round(max(1.0, invert_rank(RANK_CURVES[base_name], residual)), 1)
    cfg = BotConfig(
        name=f"t{target}",
        avg_move_rank=rank,
        miss_chance=miss,
        blunder_chance=blunder,
        **kwargs,
    )
    info = dict(base=base_name, rank=rank, miss=miss, blunder=blunder,
                noise_cost=round(noise_cost), full=full)
    return cfg, info


def _count_results(pgn) -> int:
    if not pgn.exists():
        return 0
    return sum(
        1
        for line in open(pgn, "r", encoding="utf-8", errors="ignore")
        if line.startswith("[Result ")
    )


def _even(n: int) -> int:
    return n if n % 2 == 0 else n + 1


def main() -> None:
    ap = argparse.ArgumentParser(description="Design + measure anchor rungs")
    ap.add_argument("--design-only", action="store_true", help="print the design table, don't play")
    ap.add_argument("--games-per-pair", type=int, default=40)
    ap.add_argument("--concurrency", type=int, default=16)
    ap.add_argument("--sims", type=int, default=400)
    args = ap.parse_args()

    designed: list[tuple[BotConfig, dict]] = [design_bot(t) for t in TARGETS]

    # ---- the predicted design table -------------------------------------
    print("=== designed rungs (predicted) ===")
    print(f"{'target':>7}  {'base':<6}{'rank':>6}{'miss':>6}{'blndr':>6}  config dials")
    for cfg, info in designed:
        dials = " ".join(cfg.uci_args()[1:-2])  # drop 'uci' and trailing --seed N
        print(f"{cfg.name:>7}  {info['base']:<6}{info['rank']:>6.2f}"
              f"{info['miss']:>6.0%}{info['blunder']:>6.0%}  {dials}")
    print(f"\n{len(designed)} designed + {len(FIXED)} fixed + 9 Maia")
    if args.design_only:
        return

    players: list[Player] = [*(c for c, _ in designed), *FIXED, *maia_ladder()]
    n = len(players)
    gpp = max(2, _even(args.games_per_pair))
    pairs = n * (n - 1) // 2
    total = pairs * gpp
    print(f"round-robin C({n},2)={pairs} x {gpp} = ~{total:,} games "
          f"(~{total/EST_GAMES_PER_SEC/60:.0f} min)")

    out_dir = paths.runs_dir() / "design_ladder"
    out_dir.mkdir(parents=True, exist_ok=True)
    pgn = out_dir / "design_ladder.pgn"
    spec = TournamentSpec(players=players, games_per_pair=gpp,
                          concurrency=args.concurrency, tournament="roundrobin")
    if _count_results(pgn) >= total:
        print(f"[design] {total} games present — skip play, re-rate")
    else:
        run_gauntlet(spec, pgn_path=pgn)

    measured = {f"maia-{lab}": r for lab, r in anchors.MEASURED_RAPID.items() if r}
    ratings = rate(pgn, loose_anchors=measured, sims=args.sims, out_name="design_ladder")

    # ---- predicted vs measured (the model-validation payoff) ------------
    print("\n=== predicted vs measured ===")
    print(f"{'rung':>7}{'target':>8}{'measured':>10}{'error':>8}   (measured - target)")
    errs = []
    for cfg, _ in designed:
        r = ratings.get(cfg.name)
        if r is None:
            print(f"{cfg.name:>7}{int(cfg.name[1:]):>8}{'excluded':>10}")
            continue
        target = int(cfg.name[1:])
        err = r.rating - target
        errs.append(err)
        print(f"{cfg.name:>7}{target:>8}{r.rating:>10.0f}{err:>+8.0f}")
    if errs:
        rmse = (sum(e * e for e in errs) / len(errs)) ** 0.5
        bias = sum(errs) / len(errs)
        print(f"\nmodel error: bias {bias:+.0f} Elo (negative = bots land weak "
              f"of target), RMSE {rmse:.0f}")

    # ---- the full rated ladder + spacing --------------------------------
    rated = sorted(ratings.values(), key=lambda r: r.rating)
    print("\n=== rated ladder (low -> high) ===")
    prev = None
    for r in rated:
        kind = "maia" if r.name.startswith("maia-") else "cfg"
        err = "----" if r.error is None else f"{r.error:.0f}"
        gap = ""
        if prev is not None:
            d = r.rating - prev
            tag = "  <-- GAP" if d > TARGET_SPACING else ("  <-- close" if d < REDUNDANT else "")
            gap = f"{d:>+6.0f}{tag}"
        print(f"  {r.name:<16}{r.rating:>7.0f}  +/-{err:>4}{gap}  [{kind}]")
        prev = r.rating

    csv_path = out_dir / "design_ladder_results.csv"
    with open(csv_path, "w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["name", "kind", "target", "elo", "elo_error", "games"])
        for r in rated:
            kind = "maia" if r.name.startswith("maia-") else "cfg"
            target = r.name[1:] if r.name.startswith("t") and r.name[1:].isdigit() else ""
            err = "" if r.error is None else f"{r.error:.1f}"
            w.writerow([r.name, kind, target, f"{r.rating:.1f}", err, r.played])
    print(f"\nwrote {csv_path}")


if __name__ == "__main__":
    main()
