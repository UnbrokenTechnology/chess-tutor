"""Shared driver: rate a set of subject configs against the fixed
opponent pool and return their Elos.

Structure (the "seed-swap"): the opponent pool are the fastchess gauntlet
*seeds* and the subjects are *non-seeds*, so every subject plays the whole
pool but subjects do NOT play each other — they connect through the shared
pool. This is what keeps games ~= subjects x pool_size (not x subjects^2),
making big grids affordable.

Subjects are batched (Windows command-line length limits one fastchess
invocation to ~150 engines). Re-running skips already-complete batch PGNs
and re-rates — so an interrupted multi-hour run resumes cleanly.
"""

from __future__ import annotations

from dataclasses import dataclass

from . import anchors, paths
from .engines import BotConfig, Player
from .gauntlet import TournamentSpec, run as run_gauntlet
from .pools import opponent_pool
from .rate import Rating, rate


def _even(n: int) -> int:
    return n if n % 2 == 0 else n + 1


def _count_results(pgn) -> int:
    if not pgn.exists():
        return 0
    n = 0
    with open(pgn, "r", encoding="utf-8", errors="ignore") as f:
        for line in f:
            if line.startswith("[Result "):
                n += 1
    return n


@dataclass
class ExperimentResult:
    ratings: dict[str, Rating]
    subjects_by_name: dict[str, BotConfig]
    games_per_pair: int
    games_per_config: int
    out_dir: object  # Path


def run_and_rate(
    subjects: list[BotConfig],
    *,
    out_subdir: str,
    games_per_config: int = 400,
    concurrency: int = 16,
    batch_size: int = 120,
    sims: int = 400,
) -> ExperimentResult:
    opponents: list[Player] = opponent_pool()
    n_opp = len(opponents)
    gpp = max(2, _even(round(games_per_config / n_opp)))
    per_config = gpp * n_opp

    out_dir = paths.runs_dir() / out_subdir
    out_dir.mkdir(parents=True, exist_ok=True)
    n_batches = (len(subjects) + batch_size - 1) // batch_size
    print(f"[{out_subdir}] {len(subjects)} configs x {n_opp} opponents x {gpp} games/pair "
          f"= {per_config} games/config; {n_batches} batches")

    batch_pgns = []
    opp_pairs = n_opp * (n_opp - 1) // 2
    for b in range(n_batches):
        batch = subjects[b * batch_size:(b + 1) * batch_size]
        pgn = out_dir / f"batch_{b:03d}.pgn"
        batch_pgns.append(pgn)
        # Games in this batch's PGN: every pairing that involves an opponent
        # (subjects don't play each other) = opp-opp pairs + opp x subjects.
        expected = (opp_pairs + n_opp * len(batch)) * gpp
        have = _count_results(pgn)
        if have >= expected:
            print(f"[{out_subdir}] batch {b+1}/{n_batches} complete ({have}) — skip")
            continue
        print(f"[{out_subdir}] batch {b+1}/{n_batches}: {len(batch)} configs, "
              f"{have}/{expected} done — running")
        spec = TournamentSpec(
            players=[*opponents, *batch],   # opponents FIRST = gauntlet seeds
            games_per_pair=gpp,
            concurrency=concurrency,
            tournament="gauntlet",
            seeds=n_opp,
        )
        run_gauntlet(spec, pgn_path=pgn)

    all_pgn = out_dir / "all.pgn"
    with open(all_pgn, "w", encoding="utf-8", errors="ignore") as out:
        for pgn in batch_pgns:
            if pgn.exists():
                out.write(pgn.read_text(encoding="utf-8", errors="ignore"))

    measured = {f"maia-{lab}": r for lab, r in anchors.MEASURED_RAPID.items() if r}
    ratings = rate(all_pgn, loose_anchors=measured, sims=sims, out_name=out_subdir)

    by_name = {c.name: c for c in subjects}
    by_name.update({c.name: c for c in opponents if isinstance(c, BotConfig)})
    return ExperimentResult(ratings, by_name, gpp, per_config, out_dir)


def estimate(n_configs: int, games_per_config: int, games_per_sec: float) -> str:
    n_opp = len(opponent_pool())
    gpp = max(2, _even(round(games_per_config / n_opp)))
    per_config = gpp * n_opp
    # subject games + repeated opp-opp overhead (~1 batch's worth per batch)
    total = n_configs * per_config
    hrs = total / games_per_sec / 3600
    return (f"{n_configs} configs x {per_config} games/config = ~{total:,} games "
            f"(+opp overhead); ~{hrs:.1f} h at {games_per_sec:.0f} games/s")
