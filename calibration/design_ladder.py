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
  2. Miss + blunder are modest, human-weighted garnish, ramped down as Elo
     rises. Their Elo cost (miss ~-1.6/%, blunder ~-2.0/%, pooled) is
     SUBTRACTED from the rank job so the totals still land on target. (Post
     the 2-ply miss-gating, miss is the WEAKER lever per % — we still apply
     more of it because it reads more human, its Elo cost is just smaller.)
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
    # Re-measured POST promotion-easing (build_ladder, 2026-06-06, commit
    # 4437e73). Changes vs the 06-05 run are all inside the ±90 bars — the
    # promotion fix is a rare-event change, as expected. Floor reaches r7 ≈ 57.
    "d1-q0": [(1.0, 890), (1.5, 865), (2.0, 757), (3.0, 529),
              (4.0, 375), (5.0, 162), (6.0, 95), (7.0, 57)],
    "d1-q1": [(1.0, 1621), (1.5, 1472), (2.0, 1261), (3.0, 930)],
    # d4 rank still unmeasured — r1 anchor MEASURED (1939, drifted -85 vs the
    # 06-05 run via Ordo re-anchoring); slope still a guess (-700/unit). A
    # measured d4 rank sweep remains the top-end model gap.
    "d4": [(1.0, 1939), (2.0, 1239)],  # slope -700/unit (guess)
}

#: base name -> (BotConfig dial kwargs at rank 1, full-strength Elo).
BASES: dict[str, tuple[dict, int]] = {
    "d1-q0": (dict(depth=1, qsearch_depth=0), 890),
    "d1-q1": (dict(depth=1, qsearch_depth=1), 1621),
    "d4": (dict(depth=4), 1939),
}
#: Order to try bases (weakest first); pick the first whose full-strength
#: Elo clears the target by `BASE_MARGIN` (room for rank + noise to weaken).
BASE_ORDER = ["d1-q0", "d1-q1", "d4"]
BASE_MARGIN = 120

# Elo lost per percentage-point of each noise lever. POOLED across both
# post-miss-gating build_ladder runs (06-05 + 06-06): each single-lever delta
# rides on a noisy d1-q1 baseline (which drifted 1653->1621 between runs via
# Ordo re-anchoring, comparable to the deltas themselves), so any one run
# over/under-states the slope. Pooling the per-run deltas: MISS ~-1.6/%,
# BLUNDER ~-2.0/%. Both are weak, noisy levers used only as small garnish; the
# predicted-vs-measured bias absorbs the residual. (m10/b10 land below the
# noise floor — even came out positive in 06-06 — so the slope is fit off the
# wide m30/m50 + b30 spans.) Miss stays the WEAKER lever per % post-gating; we
# still apply more of it because it reads more human.
MISS_SLOPE = 1.6
BLUNDER_SLOPE = 2.0

# Targets to design a rung for (every 100). Below ~200 the basement isn't
# precisely placeable; above d4(~1996) we use fixed d5/d6 ceiling rungs.
TARGETS = list(range(300, 1801, 100))

# Fixed rungs outside the design range: the measured basement floor and the
# top ceiling ladder (no base rank-tunes above d4 in our data).
FIXED: list[BotConfig] = [
    BotConfig("floor-d1q0r4m40", depth=1, qsearch_depth=0, avg_move_rank=4.0,
              miss_chance=0.4, endgame_skill=0),  # ~-200, no endgame books
    BotConfig("ceil-d4", depth=4),   # ~1996
    BotConfig("ceil-d5", depth=5),   # ~2227
    BotConfig("ceil-d6", depth=6),   # ~2426 (likely all-win -> excluded; that's its job)
]

TARGET_SPACING = 250
REDUNDANT = 100
EST_GAMES_PER_SEC = 60


def _clamp01(x: float) -> float:
    return max(0.0, min(1.0, x))


#: Miss% on the weakest designed rung (t300), ramping linearly to 0 by
#: MISS_ZERO_AT. Bumped from 0.35 -> 0.80 once the 2-ply miss-gating made
#: miss BELIEVABLE (it declines only combinations, never a free piece), so a
#: near-beginner bot can lean on it heavily for human "saw it, didn't play
#: it" texture instead of leaning on rank-noise. CAVEAT: build_ladder only
#: measured miss to 50%, and the cost curve is concave, so MISS_SLOPE * miss
#: OVER-estimates the Elo cost above ~50% — high-miss rungs are predicted a
#: touch strong; the measured-vs-predicted bias corrects it (add an m70
#: sweep point to build_ladder next round to measure the cost directly).
MISS_AT_FLOOR = 0.80
MISS_ZERO_AT = 1500


def miss_for(target: int) -> float:
    """Believable human miss%: MISS_AT_FLOOR on the weakest rung (t300),
    linearly down to 0 by MISS_ZERO_AT."""
    lo = TARGETS[0]
    if target >= MISS_ZERO_AT:
        return 0.0
    frac = (MISS_ZERO_AT - target) / (MISS_ZERO_AT - lo)
    return round(MISS_AT_FLOOR * _clamp01(frac), 2)


def blunder_for(target: int) -> float:
    """Gentler blunder% (miss-weighted per round 2), ~0 by ~1200, cap 0.20."""
    return round(0.20 * _clamp01(1 - target / 1200), 2)


def tier_for(target: int) -> int | None:
    """Endgame-book skill tier by Elo band — a weak rung shouldn't play
    flawless KBNK. 0=no books, 1=Basic (trivial mates), 2=Intermediate
    (opposition/piece technique), None=Full. Thresholds LOWERED 2026-06-06:
    even weak humans convert basic mates, and (chess.com validation) a bot
    that draws KQvK by the 50-move rule reads as broken, not weak. Basic@600,
    Intermediate@1200, Full@1600. Modestly lifts the measured Elo of
    endgame-reaching rungs (they convert wins they used to draw)."""
    if target < 600:
        return 0
    if target < 1200:
        return 1
    if target < 1600:
        return 2
    return None


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
    eg = tier_for(target)
    cfg = BotConfig(
        name=f"t{target}",
        avg_move_rank=rank,
        miss_chance=miss,
        blunder_chance=blunder,
        endgame_skill=eg,
        **kwargs,
    )
    info = dict(base=base_name, rank=rank, miss=miss, blunder=blunder,
                noise_cost=round(noise_cost), full=full, eg=eg)
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
    print(f"{'target':>7}  {'base':<6}{'rank':>6}{'miss':>6}{'blndr':>6}{'eg':>5}  config dials")
    for cfg, info in designed:
        dials = " ".join(cfg.uci_args()[1:-2])  # drop 'uci' and trailing --seed N
        eg = "Full" if info["eg"] is None else str(info["eg"])
        print(f"{cfg.name:>7}  {info['base']:<6}{info['rank']:>6.2f}"
              f"{info['miss']:>6.0%}{info['blunder']:>6.0%}{eg:>5}  {dials}")
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
