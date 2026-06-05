"""Full-factorial config grid over the move-quality dials.

Cartesian product of the dial value-lists in :class:`GridSpec`, so dial
*interactions* are captured. Blunder rate and blunder *severity* are
combined into enumerated **blunder modes** ``(chance, min_material,
max_material)`` because severity only matters when a blunder fires
(severity is conditional on chance > 0, not an independent axis).

Eval masks are deliberately NOT a grid dimension (2^8 = 256x blowup);
they are their own experiment (``run_masks.py``).

Config names encode every dial so they are unique and readable in the
CSV, e.g. ``d4-r2-b60q-m20-w30-g2`` = depth 4, rank 2, blunder 0.60 up to
a queen, miss 0.20, wild 0.30, guaranteed-mate-in 2.
"""

from __future__ import annotations

import itertools
from dataclasses import dataclass, field

from .engines import BotConfig

# A blunder mode: (chance, min_material_pts, max_material_pts).
BlunderMode = tuple[float, float, float]

# Severity label from the max-material ceiling, for compact config names.
_SEVERITY = {2.0: "p", 4.0: "mi", 9.0: "q"}  # pawn / minor / queen


@dataclass
class GridSpec:
    """Per-dial value lists; the grid is their Cartesian product."""

    depth: list[int] = field(default_factory=lambda: [4])
    avg_move_rank: list[float] = field(default_factory=lambda: [1.0])
    #: (chance, min_material, max_material). chance 0 => no blunder.
    blunder_modes: list[BlunderMode] = field(default_factory=lambda: [(0.0, 1.0, 4.0)])
    miss_chance: list[float] = field(default_factory=lambda: [0.0])
    wild_chance: list[float] = field(default_factory=lambda: [0.0])
    guaranteed_mate_in: list[int] = field(default_factory=lambda: [1])
    disable_eval: tuple[str, ...] = ()  # fixed across the grid

    def count(self) -> int:
        return (
            len(self.depth)
            * len(self.avg_move_rank)
            * len(self.blunder_modes)
            * len(self.miss_chance)
            * len(self.wild_chance)
            * len(self.guaranteed_mate_in)
        )


def _pct(x: float) -> int:
    return int(round(x * 100))


def _blunder_label(mode: BlunderMode) -> str:
    chance, _lo, hi = mode
    if chance <= 0.0:
        return "b0"
    return f"b{_pct(chance)}{_SEVERITY.get(hi, str(int(hi)))}"


def config_name(depth, rank, mode, miss, wild, mate) -> str:
    return (
        f"d{depth}-r{rank:g}-{_blunder_label(mode)}"
        f"-m{_pct(miss)}-w{_pct(wild)}-g{mate}"
    )


def build_grid(spec: GridSpec) -> list[BotConfig]:
    configs: list[BotConfig] = []
    for depth, rank, mode, miss, wild, mate in itertools.product(
        spec.depth,
        spec.avg_move_rank,
        spec.blunder_modes,
        spec.miss_chance,
        spec.wild_chance,
        spec.guaranteed_mate_in,
    ):
        chance, lo, hi = mode
        configs.append(
            BotConfig(
                name=config_name(depth, rank, mode, miss, wild, mate),
                depth=depth,
                avg_move_rank=rank,
                blunder_chance=chance,
                blunder_min_material=lo,
                blunder_max_material=hi,
                miss_chance=miss,
                wild_chance=wild,
                guaranteed_mate_in=mate,
                disable_eval=spec.disable_eval,
            )
        )
    return configs
