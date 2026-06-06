"""Engine specifications for fastchess.

Two kinds of player:

- :class:`BotConfig` — one of OUR dial-configs, exposed via
  ``chess-tutor uci`` (the shim). This is the unit the calibration model
  is fit over; every field maps to a documented dial.
- :class:`MaiaEngine` — a Maia net under lc0 at ``go nodes 1`` (pure
  policy), the human-calibrated anchor ladder.

Both render to a fastchess ``-engine`` token list. fastchess parses
``args=<string>`` as a single value (spaces included) and accepts a
per-engine search limit (``depth=N`` for us, ``nodes=1`` for Maia).
"""

from __future__ import annotations

import zlib
from dataclasses import dataclass, field

from . import paths


def _stable_seed(name: str) -> int:
    """Deterministic per-config base seed from its name, so a config's
    games are reproducible across runs without hand-assigning seeds."""
    return zlib.crc32(name.encode("utf-8"))


@dataclass(frozen=True)
class BotConfig:
    """One chess-tutor bot configuration. Field defaults mirror the
    engine's no-op profile (full strength, no noise)."""

    name: str
    depth: int
    threads: int = 1
    avg_move_rank: float = 1.0
    blunder_chance: float = 0.0
    blunder_min_material: float = 1.0
    blunder_max_material: float = 4.0
    miss_chance: float = 0.0
    guaranteed_mate_in: int = 1
    #: Quiescence horizon cap (tactical-vision dial). None = full vision;
    #: 0 = tactically blind (hangs pieces). Replaces the retired wild dial.
    qsearch_depth: int | None = None
    #: Endgame-book skill tier (0=no books .. 3=Full). None = Full (the
    #: flag is omitted). A weak rung should NOT play flawless KBNK, so the
    #: ladder sets this by band (see ``design_ladder.tier_for``).
    endgame_skill: int | None = None
    disable_eval: tuple[str, ...] = ()
    seed: int | None = None

    def uci_args(self) -> list[str]:
        """The argument list after ``chess-tutor`` — i.e. ``uci ...``.

        Only non-default dials are emitted, keeping the recorded command
        (and the stderr config line) readable. ``--seed`` is always set
        for reproducibility."""
        args = ["uci", "--depth", str(self.depth)]
        if self.threads != 1:
            args += ["--threads", str(self.threads)]
        if self.avg_move_rank != 1.0:
            args += ["--avg-move-rank", f"{self.avg_move_rank}"]
        if self.blunder_chance != 0.0:
            args += ["--blunder-chance", f"{self.blunder_chance}"]
            # Only meaningful alongside a blunder chance; emit when non-default.
            if self.blunder_min_material != 1.0:
                args += ["--blunder-min-material", f"{self.blunder_min_material}"]
            if self.blunder_max_material != 4.0:
                args += ["--blunder-max-material", f"{self.blunder_max_material}"]
        if self.miss_chance != 0.0:
            args += ["--miss-chance", f"{self.miss_chance}"]
        if self.qsearch_depth is not None:
            args += ["--qsearch-depth", str(self.qsearch_depth)]
        if self.endgame_skill is not None:
            args += ["--endgame-skill", str(self.endgame_skill)]
        if self.guaranteed_mate_in != 1:
            args += ["--guaranteed-mate-in", str(self.guaranteed_mate_in)]
        if self.disable_eval:
            args += ["--disable-eval", ",".join(self.disable_eval)]
        seed = self.seed if self.seed is not None else _stable_seed(self.name)
        args += ["--seed", str(seed)]
        return args

    def fastchess_tokens(self) -> list[str]:
        return [
            "-engine",
            f"cmd={paths.chess_tutor_exe()}",
            f"name={self.name}",
            f"args={' '.join(self.uci_args())}",
            f"depth={self.depth}",
        ]


@dataclass(frozen=True)
class MaiaEngine:
    """A Maia net run under lc0 as pure policy (``go nodes 1``)."""

    rating: int  # net label: 1100..1900 in steps of 100

    @property
    def name(self) -> str:
        return f"maia-{self.rating}"

    def fastchess_tokens(self) -> list[str]:
        return [
            "-engine",
            f"cmd={paths.lc0_exe()}",
            f"name={self.name}",
            f"args=--weights={paths.maia_net(self.rating)}",
            "nodes=1",
        ]


# Type alias for "anything that renders to a fastchess -engine block".
Player = BotConfig | MaiaEngine
