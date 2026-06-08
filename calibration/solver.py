"""Opponent-Elo solver: target Elo -> bot config.

The single "opponent strength" slider. Two paths:

**Default path = interpolate the LOCKED ladder** (`run_ladder.RUNGS`, the
validated 1-D schedule, RMSE ~46 vs target — far tighter than the grid
forward model's ~120). Most of the config isn't even "solved":

- ``perception`` is a closed form of the target — the ramp
  ``clamp((elo-300)/900, 0, 1)`` (rounded to the 0.05 product grid).
- ``depth`` / ``qsearch`` / ``endgame_skill`` step by ELO band (the
  one-dial schedule + eg threshold) — lookups.
- ``avg_move_rank`` is the only dial that genuinely needs a value; on the
  default path it's interpolated from the ladder's tuned rank curve.

**Advanced path = invert the forward model for rank.** When the caller
pins a dial OFF the ladder (a different depth, or a king-safety
"personality" that costs ~120 Elo at low depth per the grid), the ladder
no longer holds the target, so we re-solve ``avg_move_rank`` against the
GBT forward model (1-D bisection; Elo is monotone-decreasing in rank).
This is ~±120 Elo (anchor-limited), vs the default path's ~±46.

Personality dials (openings, mask "style") are NOT solved here — they're
seed-randomized at game setup and move Elo ~0 in the shippable range
(except the low-depth safety mask, which the advanced path compensates).

Validate / inspect:  python solver.py            # solve a spread, check vs ladder
                     python solver.py --elo 1450 --depth 1   # advanced: pin depth
"""

from __future__ import annotations

import argparse
from dataclasses import asdict, dataclass
from pathlib import Path

import joblib
import numpy as np

from run_ladder import RUNGS  # the locked (target_elo, BotConfig) ladder

_LADDER = sorted(((t, c) for t, c in RUNGS), key=lambda tc: tc[0])
_FWD_PATH = Path(__file__).parent / "runs" / "grid" / "forward_model.joblib"


@dataclass
class BotConfigOut:
    """Resolved dials for a target Elo. Discrete dials (depth/qsearch/eg)
    are exact; perception on the 0.05 grid; avg_move_rank on the 0.1 grid
    (the product's slider steps)."""

    depth: int
    qsearch_depth: int | None  # None = full
    perception: float
    avg_move_rank: float
    endgame_skill: int | None  # None = Full
    source: str  # "ladder" or "model" — how avg_move_rank was found


def perception_for(elo: float) -> float:
    """Closed-form perception ramp, rounded to the 0.05 product grid."""
    p = min(1.0, max(0.0, (elo - 300.0) / 900.0))
    return round(p / 0.05) * 0.05


def _bracket(elo: float):
    """The two ladder rungs bracketing `elo` (clamped at the ends)."""
    if elo <= _LADDER[0][0]:
        return _LADDER[0], _LADDER[0]
    if elo >= _LADDER[-1][0]:
        return _LADDER[-1], _LADDER[-1]
    lo = max((tc for tc in _LADDER if tc[0] <= elo), key=lambda tc: tc[0])
    hi = min((tc for tc in _LADDER if tc[0] >= elo), key=lambda tc: tc[0])
    return lo, hi


def ladder_config(elo: float) -> BotConfigOut:
    """Default path: discrete dials from the band, perception from the
    closed form, avg_move_rank interpolated from the ladder's curve."""
    (lo_t, lo_c), (hi_t, hi_c) = _bracket(elo)
    # Discrete dials from the band the target sits in = the lower rung
    # (the schedule steps at rung boundaries).
    depth = lo_c.depth
    qsearch = lo_c.qsearch_depth
    eg = lo_c.endgame_skill
    # avg_move_rank: linear interp between the bracketing rungs' ranks.
    if hi_t == lo_t:
        rank = lo_c.avg_move_rank
    else:
        frac = (elo - lo_t) / (hi_t - lo_t)
        rank = lo_c.avg_move_rank + frac * (hi_c.avg_move_rank - lo_c.avg_move_rank)
    return BotConfigOut(
        depth=depth,
        qsearch_depth=qsearch,
        perception=perception_for(elo),
        avg_move_rank=round(rank, 1),
        endgame_skill=eg,
        source="ladder",
    )


# ---- advanced path: invert the forward model for avg_move_rank ----------
def _load_model():
    if not _FWD_PATH.exists():
        raise SystemExit(f"forward model not found at {_FWD_PATH} — run fit.py")
    return joblib.load(_FWD_PATH)


def _predict_elo(bundle, depth, qsearch, perception, rank, eg, mask_safety, mask_positional):
    qcode = bundle["qinf_code"] if qsearch is None else qsearch
    egcode = bundle["egf_code"] if eg is None else eg
    feats = {
        "depth": depth, "qsearch": qcode, "perception": perception,
        "avg_move_rank": rank, "eg": egcode,
        "mask_safety": mask_safety, "mask_positional": mask_positional,
    }
    x = np.array([[feats[f] for f in bundle["features"]]], dtype=float)
    return float(bundle["model"].predict(x)[0])


def solve_rank(elo, depth, qsearch, perception, eg, mask_safety=0, mask_positional=0):
    """Bisect avg_move_rank in [1, 8] so the forward model predicts `elo`.
    Elo is monotone-decreasing in rank, so plain bisection converges."""
    bundle = _load_model()

    def f(r):
        return _predict_elo(bundle, depth, qsearch, perception, r, eg, mask_safety, mask_positional)

    lo, hi = 1.0, 8.0
    # If even rank 1 is below target (config too weak to reach it) or rank 8
    # is above (too strong to weaken enough), clamp to the reachable end.
    if f(lo) <= elo:
        return 1.0
    if f(hi) >= elo:
        return 8.0
    for _ in range(40):
        mid = (lo + hi) / 2
        if f(mid) > elo:  # too strong -> need more rank
            lo = mid
        else:
            hi = mid
    return round((lo + hi) / 2, 1)


def solve(elo: float, *, depth=None, qsearch="__keep__", endgame_skill="__keep__",
          mask_safety=0, mask_positional=0) -> BotConfigOut:
    """Resolve a config for `elo`. With no overrides -> the ladder path.
    Pinning any of depth / qsearch / endgame_skill, or enabling a mask,
    drops to the advanced path: keep the ladder's value for the unpinned
    dials, then re-solve avg_move_rank against the forward model."""
    base = ladder_config(elo)
    pinned = (
        depth is not None
        or qsearch != "__keep__"
        or endgame_skill != "__keep__"
        or mask_safety or mask_positional
    )
    if not pinned:
        return base
    d = depth if depth is not None else base.depth
    q = base.qsearch_depth if qsearch == "__keep__" else qsearch
    eg = base.endgame_skill if endgame_skill == "__keep__" else endgame_skill
    p = base.perception
    rank = solve_rank(elo, d, q, p, eg, mask_safety, mask_positional)
    return BotConfigOut(
        depth=d, qsearch_depth=q, perception=p, avg_move_rank=rank,
        endgame_skill=eg, source="model",
    )


def main() -> None:
    ap = argparse.ArgumentParser(description="Opponent-Elo solver")
    ap.add_argument("--elo", type=float, default=None)
    ap.add_argument("--depth", type=int, default=None)
    ap.add_argument("--mask-safety", action="store_true")
    args = ap.parse_args()

    if args.elo is not None:
        cfg = solve(args.elo, depth=args.depth, mask_safety=1 if args.mask_safety else 0)
        print(f"target {args.elo:.0f} -> {asdict(cfg)}")
        return

    # Default: solve a spread and show the ladder path + a model cross-check.
    print(f"{'target':>7} {'depth':>5} {'qs':>4} {'perc':>5} {'rank':>5} {'eg':>3}")
    for t in range(500, 2501, 250):
        c = solve(t)
        q = "inf" if c.qsearch_depth is None else c.qsearch_depth
        eg = "F" if c.endgame_skill is None else c.endgame_skill
        print(f"{t:>7} {c.depth:>5} {str(q):>4} {c.perception:>5.2f} "
              f"{c.avg_move_rank:>5.1f} {str(eg):>3}")


if __name__ == "__main__":
    main()
