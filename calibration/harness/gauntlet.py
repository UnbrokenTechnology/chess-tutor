"""Run a fastchess tournament over a set of players and return the PGN.

Defaults are tuned for an unattended run: eval-based resign/draw
adjudication (now that the shim emits ``info score``) plus a ``-maxmoves``
backstop keep games short; ``-recover`` survives an engine crash and
``-autosaveinterval`` lets a killed run resume.
"""

from __future__ import annotations

import subprocess
from dataclasses import dataclass, field

from . import paths
from .engines import Player


@dataclass
class TournamentSpec:
    players: list[Player]
    #: Total games between each pairing. Played as `-rounds N -games 2
    #: -repeat`, so colors balance and total-per-pair = rounds*2.
    games_per_pair: int = 20
    concurrency: int = 8
    tournament: str = "roundrobin"  # "roundrobin" | "gauntlet"
    seeds: int = 1  # gauntlet only: first N players face all others
    # Adjudication — conservative two-sided resign so a weak bot's eval
    # swings don't end winnable games early; both sides must agree.
    resign_score: int = 900       # conventional cp (~a queen)
    resign_movecount: int = 5
    draw_movenumber: int = 40
    draw_movecount: int = 10
    draw_score: int = 10
    maxmoves: int = 200
    book_plies: int = 8
    pgn_name: str = "games"
    extra_args: list[str] = field(default_factory=list)

    @property
    def rounds(self) -> int:
        return max(1, self.games_per_pair // 2)


def build_command(spec: TournamentSpec, pgn_path) -> list[str]:
    cmd: list[str] = [str(paths.fastchess_exe()), "-tournament", spec.tournament]
    if spec.tournament == "gauntlet":
        cmd += ["-seeds", str(spec.seeds)]
    for p in spec.players:
        cmd += p.fastchess_tokens()
    cmd += [
        "-openings",
        f"file={paths.opening_book()}",
        "format=pgn",
        "order=random",
        f"plies={spec.book_plies}",
        "-rounds", str(spec.rounds),
        "-games", "2",
        "-repeat",
        "-resign",
        f"movecount={spec.resign_movecount}",
        f"score={spec.resign_score}",
        "twosided=true",
        "-draw",
        f"movenumber={spec.draw_movenumber}",
        f"movecount={spec.draw_movecount}",
        f"score={spec.draw_score}",
        "-maxmoves", str(spec.maxmoves),
        "-concurrency", str(spec.concurrency),
        "-pgnout", f"file={pgn_path}", "notation=san",
        "-recover",
        "-autosaveinterval", "50",
    ]
    cmd += spec.extra_args
    return cmd


def run(spec: TournamentSpec):
    """Run the tournament; stream fastchess output live. Returns the PGN path."""
    pgn_path = paths.runs_dir() / f"{spec.pgn_name}.pgn"
    cmd = build_command(spec, pgn_path)
    n = len(spec.players)
    print(f"[gauntlet] {spec.tournament}: {n} players, "
          f"{spec.games_per_pair} games/pair, concurrency {spec.concurrency}")
    print(f"[gauntlet] pgn -> {pgn_path}")
    subprocess.run(cmd, check=True)
    return pgn_path
