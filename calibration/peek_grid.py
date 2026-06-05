"""Peek at a still-running grid: rate the batches finished SO FAR.

The live ``run_grid.py`` only writes ``grid_results.csv`` after the final
Ordo pass over every batch. To eyeball partial results mid-run, this does
an INDEPENDENT rating pass over the existing ``runs/grid/batch_*.pgn``
files, writing to a *separate* output name (``grid_peek``) so it cannot
collide with or disturb the live run.

Safe to run while the grid is going: it only READS the batch PGNs (the
active batch may have a truncated trailing game — Ordo/our parser ignore
incomplete records). Each config appears in exactly one batch and every
batch carries the full 18-bot anchored pool, so completed batches give
valid Elos; configs in the in-progress batch just show wider error.

Run: python peek_grid.py
"""

from __future__ import annotations

import argparse
import csv

from harness import anchors, paths
from harness.grid import build_grid
from harness.rate import rate
from run_grid import GRID, _mask_bools


def main() -> None:
    ap = argparse.ArgumentParser(description="Rate finished grid batches so far")
    # sims ONLY affect the +/- error column; the Elo point estimates are
    # identical at sims=0, which is ~20x faster — right for an eyeball.
    ap.add_argument("--sims", type=int, default=0,
                    help="Ordo error-bar simulations (0 = none, fast; default 0)")
    args = ap.parse_args()

    grid_dir = paths.runs_dir() / "grid"
    batches = sorted(grid_dir.glob("batch_*.pgn"))
    if not batches:
        print(f"no batch PGNs yet in {grid_dir}")
        return

    # Concatenate at runs/ level so the file does NOT match the live run's
    # batch_*.pgn glob (it would otherwise be picked up as a phantom batch).
    peek_pgn = paths.runs_dir() / "grid_peek.pgn"
    games = 0
    with open(peek_pgn, "w", encoding="utf-8", errors="ignore") as out:
        for b in batches:
            try:
                text = b.read_text(encoding="utf-8", errors="ignore")
            except OSError as e:                      # active batch may be locked
                print(f"  (skipping {b.name}: {e})")
                continue
            games += text.count("[Result ")
            out.write(text)
    print(f"peeking at {len(batches)} batch file(s), ~{games:,} games so far")

    measured = {f"maia-{lab}": r for lab, r in anchors.MEASURED_RAPID.items() if r}
    ratings = rate(peek_pgn, loose_anchors=measured, sims=args.sims, out_name="grid_peek")

    by_name = {c.name: c for c in build_grid(GRID)}

    # Write a partial results CSV (same columns as the final grid CSV).
    csv_path = paths.runs_dir() / "grid_peek_results.csv"
    with open(csv_path, "w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow([
            "name", "kind", "depth", "qsearch_depth", "avg_move_rank",
            "blunder_chance", "miss_chance", "mask_safety", "mask_positional",
            "elo", "elo_error", "games",
        ])
        for name, r in sorted(ratings.items(), key=lambda kv: -kv[1].rating):
            c = by_name.get(name)
            kind = ("maia" if name.startswith("maia-")
                    else "reference" if name.startswith("ref-") else "grid")
            err = "" if r.error is None else f"{r.error:.1f}"
            if c is not None:
                qd = "inf" if c.qsearch_depth is None else c.qsearch_depth
                ms, mp = _mask_bools(c.disable_eval)
                w.writerow([name, kind, c.depth, qd, c.avg_move_rank,
                            c.blunder_chance, c.miss_chance, ms, mp,
                            f"{r.rating:.1f}", err, r.played])
            else:
                w.writerow([name, kind, "", "", "", "", "", "", "",
                            f"{r.rating:.1f}", err, r.played])

    grid_rows = [(n, r) for n, r in ratings.items()
                 if not n.startswith(("maia-", "ref-"))]
    grid_rated = [r for _, r in grid_rows if r.played > 0]
    print(f"\nwrote {csv_path}")
    print(f"grid configs with games so far: {len(grid_rated)} / {GRID.count()}")
    if grid_rated:
        lo = min(grid_rated, key=lambda r: r.rating)
        hi = max(grid_rated, key=lambda r: r.rating)
        print(f"grid Elo range so far: {lo.rating:.0f} ({lo.name}) .. "
              f"{hi.rating:.0f} ({hi.name})")

    # Anchors + reference rungs: the sanity check that the pool looks right.
    print("\nanchors / reference rungs (Elo):")
    for name, r in sorted(ratings.items(), key=lambda kv: -kv[1].rating):
        if name.startswith(("maia-", "ref-")):
            err = "----" if r.error is None else f"{r.error:.0f}"
            print(f"  {name:<14} {r.rating:>6.0f}  +/-{err:>4}  ({r.played} games)")


if __name__ == "__main__":
    main()
