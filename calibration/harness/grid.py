"""Full-factorial config grid over the strength + style dials.

Cartesian product of the dial value-lists in :class:`GridSpec`, so dial
*interactions* are captured. The axes:

- **depth x qsearch-depth** — the primary tactical axis (the keystone
  low-end lever). qsearch-depth caps how many plies of captures
  quiescence resolves: ``0`` = tactically blind (hangs pieces),
  ``None`` = full vision.
- **avg_move_rank / blunder_modes / miss_chance** — the human-realism
  move-selection dials. Blunder rate and *severity* are combined into
  enumerated **blunder modes** ``(chance, min_material, max_material)``
  because severity only matters when a blunder fires (it is conditional
  on chance > 0, not an independent axis).
- **masks** — eval-mask combos as a grid dimension (NEW). The low-band
  experiment showed masks interact with tactical level (a sign-flip), so
  they must vary *crossed with* depth x qsearch to be fittable. Two
  underlying booleans (``safety``, ``positional``) => 4 combos; see
  ``pools.GRID_MASK_COMBOS``.

``guaranteed_mate_in`` is NOT a grid axis — it's a minor lever measured
in its own 1-D sweep (``run_mate_sweep.py``); the grid fixes it at 1.

Config names encode every dial so they are unique and readable in the
CSV, e.g. ``d4-q2-r2-b60q-m20-safety`` = depth 4, qsearch-depth 2, rank
2, blunder 0.60 up to a queen, miss 0.20, safety mask on.
"""

from __future__ import annotations

import itertools
from dataclasses import dataclass, field

from .engines import BotConfig

# A blunder mode: (chance, min_material_pts, max_material_pts).
BlunderMode = tuple[float, float, float]

# A mask combo: (label, disabled-eval-slugs). ("", ()) = no mask.
MaskCombo = tuple[str, tuple[str, ...]]

# Severity label from the max-material ceiling, for compact config names.
_SEVERITY = {2.0: "p", 4.0: "mi", 9.0: "q"}  # pawn / minor / queen


@dataclass
class GridSpec:
    """Per-dial value lists; the grid is their Cartesian product."""

    depth: list[int] = field(default_factory=lambda: [4])
    #: Quiescence horizon caps. None = full tactical vision.
    qsearch_depth: list[int | None] = field(default_factory=lambda: [None])
    avg_move_rank: list[float] = field(default_factory=lambda: [1.0])
    #: (chance, min_material, max_material). chance 0 => no blunder.
    blunder_modes: list[BlunderMode] = field(default_factory=lambda: [(0.0, 1.0, 4.0)])
    miss_chance: list[float] = field(default_factory=lambda: [0.0])
    #: Eval-mask combos as a grid axis. [("", ())] = no masking.
    masks: list[MaskCombo] = field(default_factory=lambda: [("", ())])
    #: Fixed scalar (NOT a grid axis); mate-vision is its own 1-D sweep.
    guaranteed_mate_in: int = 1

    def count(self) -> int:
        return (
            len(self.depth)
            * len(self.qsearch_depth)
            * len(self.avg_move_rank)
            * len(self.blunder_modes)
            * len(self.miss_chance)
            * len(self.masks)
        )


def _pct(x: float) -> int:
    return int(round(x * 100))


def _q_label(q: int | None) -> str:
    return "qinf" if q is None else f"q{q}"


def _blunder_label(mode: BlunderMode) -> str:
    chance, _lo, hi = mode
    if chance <= 0.0:
        return "b0"
    return f"b{_pct(chance)}{_SEVERITY.get(hi, str(int(hi)))}"


def config_name(depth, q, rank, mode, miss, mask_label) -> str:
    name = (
        f"d{depth}-{_q_label(q)}-r{rank:g}-{_blunder_label(mode)}-m{_pct(miss)}"
    )
    if mask_label:
        name += f"-{mask_label}"
    return name


def build_grid(spec: GridSpec) -> list[BotConfig]:
    configs: list[BotConfig] = []
    for depth, q, rank, mode, miss, (mask_label, mask_slugs) in itertools.product(
        spec.depth,
        spec.qsearch_depth,
        spec.avg_move_rank,
        spec.blunder_modes,
        spec.miss_chance,
        spec.masks,
    ):
        chance, lo, hi = mode
        configs.append(
            BotConfig(
                name=config_name(depth, q, rank, mode, miss, mask_label),
                depth=depth,
                qsearch_depth=q,
                avg_move_rank=rank,
                blunder_chance=chance,
                blunder_min_material=lo,
                blunder_max_material=hi,
                miss_chance=miss,
                guaranteed_mate_in=spec.guaranteed_mate_in,
                disable_eval=mask_slugs,
            )
        )
    return configs
