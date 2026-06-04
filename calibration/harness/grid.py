"""Full-factorial config grid.

Given per-dial value lists, produce the Cartesian product of every
combination as :class:`BotConfig` seeds. This is the data-collection
workhorse: it varies dials *together* so the fitted model can see
interactions a one-dial-at-a-time sweep would miss.

Config names compactly encode every varied dial so they are unique
(required: fastchess engine names must not collide) and human-readable
in the output CSV: ``d4-r2-b40-m30-w20`` = depth 4, avg-move-rank 2,
blunder 0.40, miss 0.30, wild 0.20.
"""

from __future__ import annotations

import itertools
from dataclasses import dataclass, field

from .engines import BotConfig


@dataclass
class GridSpec:
    """Per-dial value lists. The grid is their Cartesian product."""

    depth: list[int] = field(default_factory=lambda: [4])
    avg_move_rank: list[float] = field(default_factory=lambda: [1.0])
    blunder_chance: list[float] = field(default_factory=lambda: [0.0])
    miss_chance: list[float] = field(default_factory=lambda: [0.0])
    wild_chance: list[float] = field(default_factory=lambda: [0.0])
    # Held fixed across the grid by default (combinatorially expensive to
    # cross; eval-mask is its own "ceiling" experiment).
    guaranteed_mate_in: int = 1
    disable_eval: tuple[str, ...] = ()

    def count(self) -> int:
        return (
            len(self.depth)
            * len(self.avg_move_rank)
            * len(self.blunder_chance)
            * len(self.miss_chance)
            * len(self.wild_chance)
        )


def _pct(x: float) -> int:
    return int(round(x * 100))


def config_name(depth, rank, blunder, miss, wild) -> str:
    # rank with :g so 2.0 -> "2" but 1.5 -> "1.5"; chances as percent ints.
    return f"d{depth}-r{rank:g}-b{_pct(blunder)}-m{_pct(miss)}-w{_pct(wild)}"


def build_grid(spec: GridSpec) -> list[BotConfig]:
    configs: list[BotConfig] = []
    for depth, rank, blunder, miss, wild in itertools.product(
        spec.depth,
        spec.avg_move_rank,
        spec.blunder_chance,
        spec.miss_chance,
        spec.wild_chance,
    ):
        name = config_name(depth, rank, blunder, miss, wild)
        configs.append(
            BotConfig(
                name=name,
                depth=depth,
                avg_move_rank=rank,
                blunder_chance=blunder,
                miss_chance=miss,
                wild_chance=wild,
                guaranteed_mate_in=spec.guaranteed_mate_in,
                disable_eval=spec.disable_eval,
            )
        )
    return configs
