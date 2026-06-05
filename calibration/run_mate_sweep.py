"""Standalone 1-D sweep of guaranteed-mate-in.

Pulled OUT of the main grid (run_grid.py) because it's a minor lever — a
*floor* on mate-vision (the bot always finds mates <= N), so higher = a
bit stronger, and the effect is small relative to depth/qsearch/noise.
Measuring it on a few tactical bases is enough to band it at solve time
without paying a x3 on the whole grid.

Bases span tactical levels (a blind weak bot may gain more from mate
vision than a sighted one). Each base is swept over mate-in {1,2,3}.

Run: python run_mate_sweep.py
"""

from __future__ import annotations

import csv
from dataclasses import replace

from harness.engines import BotConfig
from harness.experiment import run_and_rate

# Tactical bases (approx probe Elos in comments).
BASES = [
    BotConfig("d1-q0", depth=1, qsearch_depth=0),    # ~879  blind/weak
    BotConfig("d2-q0", depth=2, qsearch_depth=0),    # ~1590 blind/mid
    BotConfig("d2-q2", depth=2, qsearch_depth=2),    # ~1683 sighted/mid
    BotConfig("d4-q2", depth=4, qsearch_depth=2),    # ~1957 stronger
]
MATE_INS = [1, 2, 3]


def build_subjects() -> list[BotConfig]:
    subs = []
    for base in BASES:
        for g in MATE_INS:
            # g==1 is the BotConfig default; keep it as the base name so it
            # doubles as the un-swept reference row.
            name = base.name if g == 1 else f"{base.name}__g{g}"
            subs.append(replace(base, name=name, guaranteed_mate_in=g))
    return subs


def main() -> None:
    result = run_and_rate(
        build_subjects(),
        out_subdir="mate_sweep",
        games_per_config=300,
        concurrency=16,
        sims=300,
    )
    r = result.ratings

    csv_path = result.out_dir / "mate_sweep_results.csv"
    with open(csv_path, "w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["base", "guaranteed_mate_in", "elo", "elo_error", "games"])
        for base in BASES:
            for g in MATE_INS:
                name = base.name if g == 1 else f"{base.name}__g{g}"
                rr = r.get(name)
                if rr:
                    w.writerow([base.name, g, f"{rr.rating:.0f}",
                                "" if rr.error is None else f"{rr.error:.0f}", rr.played])
    print(f"\nwrote {csv_path}\n")

    print("base (mate-in=1 Elo)   " + "".join(f"{f'g{g}':>10}" for g in MATE_INS))
    print("-" * (23 + 10 * len(MATE_INS)))
    for base in BASES:
        be = r.get(base.name)
        if not be:
            print(f"{base.name:<10} (excluded — too weak for the pool floor)")
            continue
        row = f"{base.name:<10} ({be.rating:>5.0f})    "
        for g in MATE_INS:
            name = base.name if g == 1 else f"{base.name}__g{g}"
            mk = r.get(name)
            row += f"{mk.rating - be.rating:>+10.0f}" if mk else f"{'--':>10}"
        print(row)


if __name__ == "__main__":
    main()
