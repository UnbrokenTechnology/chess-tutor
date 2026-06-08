"""Full-factorial config grid over the strength + style dials.

Cartesian product of the dial value-lists in :class:`GridSpec`, so dial
*interactions* are captured. Axes (perception-era, 2026-06-07 — miss/blunder
dials REMOVED from the engine, perception added):

- **depth x qsearch-depth** — the primary tactical axis. qsearch caps how
  many plies of captures quiescence resolves (``None`` = full vision). q0
  is dropped (off-product: a q0 bot can't see the immediate recapture and
  parks its queen — sub-human, never used by the slider).
- **perception** — the move-visibility dial (``1.0`` = sees every move =
  byte-identical bypass). The keystone weak-bot lever; sampled densely
  below the ~0.6 knee, sparse above.
- **avg_move_rank** — variety / tilt (plays the Nth-best move on average).
- **endgame_skill** — book tier ``{0,1,2,None=Full}``; the conversion
  lever. eg x avg_move_rank interacts: at eg0 the endgame eval is flat, so
  a high-rank bot wanders a won endgame instead of converting (the
  t500-vs-Martin lesson) — so eg must vary crossed with rank.
- **masks** — eval-mask combos (``safety`` / ``positional``). They
  sign-flip with tactical level (the low-band experiment: masking HELPS a
  blind bot, HURTS a sighted one), so they must vary *crossed with* depth x
  qsearch to be fittable. See ``pools.GRID_MASK_COMBOS``.

``guaranteed_mate_in`` is NOT a grid axis — minor lever, fixed at 1.

Config names encode every dial so they are unique and readable in the
CSV, e.g. ``d4-q2-p40-r2-eg2-safety`` = depth 4, qsearch 2, perception
0.40, rank 2, endgame-skill 2, safety mask on.
"""

from __future__ import annotations

import itertools
from dataclasses import dataclass, field

from .engines import BotConfig

# A mask combo: (label, disabled-eval-slugs). ("", ()) = no mask.
MaskCombo = tuple[str, tuple[str, ...]]


@dataclass
class GridSpec:
    """Per-dial value lists; the grid is their Cartesian product."""

    depth: list[int] = field(default_factory=lambda: [4])
    #: Quiescence horizon caps. None = full tactical vision.
    qsearch_depth: list[int | None] = field(default_factory=lambda: [None])
    #: Move-visibility dial. 1.0 = sees every move (bypass).
    perception: list[float] = field(default_factory=lambda: [1.0])
    avg_move_rank: list[float] = field(default_factory=lambda: [1.0])
    #: Endgame-book tier: 0=none, 1=basic, 2=inter, None=Full.
    endgame_skill: list[int | None] = field(default_factory=lambda: [None])
    #: Eval-mask combos as a grid axis. [("", ())] = no masking.
    masks: list[MaskCombo] = field(default_factory=lambda: [("", ())])
    #: Fixed scalar (NOT a grid axis); mate-vision is its own 1-D sweep.
    guaranteed_mate_in: int = 1

    def count(self) -> int:
        return (
            len(self.depth)
            * len(self.qsearch_depth)
            * len(self.perception)
            * len(self.avg_move_rank)
            * len(self.endgame_skill)
            * len(self.masks)
        )


def _pct(x: float) -> int:
    return int(round(x * 100))


def _q_label(q: int | None) -> str:
    return "qinf" if q is None else f"q{q}"


def _eg_label(eg: int | None) -> str:
    return "egF" if eg is None else f"eg{eg}"


def config_name(depth, q, p, rank, eg, mask_label) -> str:
    name = f"d{depth}-{_q_label(q)}-p{_pct(p)}-r{rank:g}-{_eg_label(eg)}"
    if mask_label:
        name += f"-{mask_label}"
    return name


def build_grid(spec: GridSpec) -> list[BotConfig]:
    configs: list[BotConfig] = []
    for depth, q, p, rank, eg, (mask_label, mask_slugs) in itertools.product(
        spec.depth,
        spec.qsearch_depth,
        spec.perception,
        spec.avg_move_rank,
        spec.endgame_skill,
        spec.masks,
    ):
        configs.append(
            BotConfig(
                name=config_name(depth, q, p, rank, eg, mask_label),
                depth=depth,
                qsearch_depth=q,
                perception=p,
                avg_move_rank=rank,
                endgame_skill=eg,
                guaranteed_mate_in=spec.guaranteed_mate_in,
                disable_eval=mask_slugs,
            )
        )
    return configs
