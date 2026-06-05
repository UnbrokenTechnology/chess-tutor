"""Full-factorial grid run over the move-quality dials — the bulk
data-collection workhorse.

Cartesian product of the dials in ``GRID`` (interactions captured); each
config plays the fixed opponent pool via the seed-swap driver
(harness.experiment), rated in one loose-multi-anchored Ordo pass; dumps
``grid_results.csv`` of dials -> measured Elo for offline analysis.

Eval masks are a separate experiment (run_masks.py) — full-factorial of
8 binary masks would be a 256x blowup.

Usage (calibration/ dir, venv python):
  python run_grid.py --dry-run        # size + estimate
  python run_grid.py --tiny           # tiny smoke grid end-to-end
  python run_grid.py                  # the full GRID (~11 h)
  python run_grid.py                  # again -> resumes / re-rates
"""

from __future__ import annotations

import argparse
import csv

from harness.experiment import estimate, run_and_rate
from harness.grid import GridSpec, build_grid

# ---------------------------------------------------------------------------
# The grid (chosen 2026-06-04, "~11 h overnight"): depth capped at 6 — the
# pilot showed no-noise depth is a high floor (d1~1750) and depth >6 only
# adds strength above the 2000 teaching ceiling. The realism dials (miss,
# blunder severity) get the resolution.
#   blunder modes = none + {0.3,0.6} x {pawn(max 2), minor(max 4), queen(max 9)}
# ---------------------------------------------------------------------------
GRID = GridSpec(
    depth=[1, 2, 4, 6],
    avg_move_rank=[1.0, 2.0, 4.0, 6.0],
    blunder_modes=[
        (0.0, 1.0, 4.0),
        (0.3, 1.0, 2.0), (0.6, 1.0, 2.0),   # pawn
        (0.3, 1.0, 4.0), (0.6, 1.0, 4.0),   # minor
        (0.3, 1.0, 9.0), (0.6, 1.0, 9.0),   # queen
    ],
    miss_chance=[0.0, 0.2, 0.4, 0.6],
    wild_chance=[0.0, 0.3, 0.6],
    guaranteed_mate_in=[1, 2, 3],
)

TINY = GridSpec(depth=[2, 4], avg_move_rank=[1.0], wild_chance=[0.0, 0.4])

EST_GAMES_PER_SEC = 49  # measured on a depth-1/2/4/6 mix with noise


def main() -> None:
    ap = argparse.ArgumentParser(description="Full-factorial grid run")
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--tiny", action="store_true")
    ap.add_argument("--games-per-config", type=int, default=400)
    ap.add_argument("--concurrency", type=int, default=16)
    ap.add_argument("--batch-size", type=int, default=120)
    ap.add_argument("--sims", type=int, default=400)
    args = ap.parse_args()

    spec = TINY if args.tiny else GRID
    gpc = min(args.games_per_config, 64) if args.tiny else args.games_per_config
    configs = build_grid(spec)

    print(estimate(len(configs), gpc, EST_GAMES_PER_SEC))
    if args.dry_run:
        return

    result = run_and_rate(
        configs,
        out_subdir="grid_tiny" if args.tiny else "grid",
        games_per_config=gpc,
        concurrency=args.concurrency,
        batch_size=args.batch_size,
        sims=args.sims,
    )

    csv_path = result.out_dir / "grid_results.csv"
    with open(csv_path, "w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow([
            "name", "kind", "depth", "avg_move_rank", "blunder_chance",
            "blunder_min_material", "blunder_max_material", "miss_chance",
            "wild_chance", "guaranteed_mate_in", "disable_eval",
            "elo", "elo_error", "games",
        ])
        for name, r in sorted(result.ratings.items(), key=lambda kv: -kv[1].rating):
            c = result.subjects_by_name.get(name)
            kind = ("maia" if name.startswith("maia-")
                    else "reference" if name.startswith("ref-") else "grid")
            err = "" if r.error is None else f"{r.error:.1f}"
            if c is not None:
                w.writerow([
                    name, kind, c.depth, c.avg_move_rank, c.blunder_chance,
                    c.blunder_min_material, c.blunder_max_material, c.miss_chance,
                    c.wild_chance, c.guaranteed_mate_in, "|".join(c.disable_eval),
                    f"{r.rating:.1f}", err, r.played,
                ])
            else:
                w.writerow([name, kind, "", "", "", "", "", "", "", "", "",
                            f"{r.rating:.1f}", err, r.played])
    print(f"\nwrote {csv_path}  ({len(result.ratings)} rated players)")
    grid_rows = [(n, r) for n, r in result.ratings.items()
                 if not n.startswith(("maia-", "ref-"))]
    print(f"grid configs rated: {len(grid_rows)}")
    print("Elo range across grid: "
          f"{min(r.rating for _, r in grid_rows):.0f} .. "
          f"{max(r.rating for _, r in grid_rows):.0f}")


if __name__ == "__main__":
    main()
