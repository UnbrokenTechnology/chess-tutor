"""Rate a PGN with Ordo and parse the result table.

Anchoring: one hard anchor (``-a``/``-A``) fixes the pool offset; Ordo
gives the standard Elo scale by construction, so a single anchor suffices
for absolute numbers. Optional *loose* anchors (``-y``) let Ordo balance
the other measured Maia points without over-constraining the scale.
"""

from __future__ import annotations

import re
import subprocess
from dataclasses import dataclass

from . import paths


@dataclass
class Rating:
    name: str
    rating: float
    error: float | None  # None for the (fixed) anchor, shown as "----"
    points: float
    played: int


# Ordo table rows: "  1 name : 1500.0 ---- 6.0 10 60"
_ROW = re.compile(
    r"^\s*\d+\s+(?P<name>.+?)\s*:\s*"
    r"(?P<rating>-?\d+\.\d+)\s+"
    r"(?P<error>-+|\d+\.\d+)\s+"
    r"(?P<points>-?\d+\.\d+)\s+"
    r"(?P<played>\d+)"
)


def rate(
    pgn_path,
    *,
    anchor: tuple[str, int] | None = None,
    loose_anchors: dict[str, int] | None = None,
    loose_uncertainty: int = 50,
    sims: int = 200,
    out_name: str = "ratings",
) -> dict[str, Rating]:
    """Rate a PGN. Provide EXACTLY ONE anchoring mode:

    * ``anchor=(name, rating)`` — single hard anchor (``-a``/``-A``);
      Ordo's intrinsic Elo scale + this offset give absolute numbers.
    * ``loose_anchors={name: rating, ...}`` — soft multi-point anchoring
      (``-y``). Preferred when several measured points exist and their
      spacing may not exactly match the engine-pool scale (our Maia case):
      Ordo balances the pool toward all of them. Ordo forbids combining
      ``-a`` with ``-y``.
    """
    if (anchor is None) == (loose_anchors is None):
        raise ValueError("pass exactly one of `anchor` or `loose_anchors`")

    runs = paths.runs_dir()
    out_txt = runs / f"{out_name}.txt"
    out_csv = runs / f"{out_name}.csv"
    cmd = [
        str(paths.ordo_exe()),
        "-s", str(sims),
        "-p", str(pgn_path),
        "-o", str(out_txt),
        "-c", str(out_csv),
    ]
    if anchor is not None:
        cmd += ["-a", str(anchor[1]), "-A", anchor[0]]
    else:
        # `-y` rows are "Player",Rating,Uncertainty — the uncertainty is
        # how hard each point is pinned (smaller = stiffer).
        loose_file = runs / f"{out_name}_loose_anchors.csv"
        loose_file.write_text(
            "".join(f'"{n}",{r},{loose_uncertainty}\n' for n, r in loose_anchors.items()),
            encoding="utf-8",
        )
        cmd += ["-y", str(loose_file)]

    subprocess.run(cmd, check=True, stdout=subprocess.DEVNULL)

    ratings: dict[str, Rating] = {}
    for line in out_txt.read_text(encoding="utf-8").splitlines():
        m = _ROW.match(line)
        if not m:
            continue
        err = m.group("error")
        ratings[m.group("name")] = Rating(
            name=m.group("name"),
            rating=float(m.group("rating")),
            error=None if err.startswith("-") else float(err),
            points=float(m.group("points")),
            played=int(m.group("played")),
        )
    if not ratings:
        raise RuntimeError(f"Ordo produced no parseable ratings in {out_txt}")
    return ratings
