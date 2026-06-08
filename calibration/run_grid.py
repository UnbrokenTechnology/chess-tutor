"""Full-factorial grid run over the strength + style dials — the bulk
data-collection workhorse ("the big run").

Cartesian product of the dials in ``GRID`` (interactions captured); each
config plays the fixed opponent pool via the seed-swap driver
(harness.experiment), rated in one loose-multi-anchored Ordo pass; dumps
``grid_results.csv`` of dials -> measured Elo for offline analysis.

The grid axes (2026-06-05 redesign, qsearch + masks folded in):
  * depth x qsearch-depth  — primary tactical axis (the keystone low-end
    lever; qsearch-depth 0 = tactically blind, None = full vision).
  * avg_move_rank / blunder_modes / miss_chance — human-realism dials.
  * masks — eval-mask combos (safety / positional) as a real axis, so the
    mask x tactical-level SIGN-FLIP from the low-band experiment is
    fittable. See pools.GRID_MASK_COMBOS for the two-boolean encoding.

guaranteed_mate_in is pulled OUT of the grid (minor lever) and measured
in its own 1-D sweep (run_mate_sweep.py); here it is fixed at 1.

Usage (calibration/ dir, venv python):
  python run_grid.py --dry-run        # size + estimate
  python run_grid.py --tiny           # tiny smoke grid end-to-end
  python run_grid.py                  # the full GRID (~6.5 h)
  python run_grid.py                  # again -> resumes / re-rates
"""

from __future__ import annotations

import argparse
import csv

from harness.experiment import estimate, run_and_rate
from harness.grid import GridSpec, build_grid
from harness.pools import GRID_MASK_COMBOS

# ---------------------------------------------------------------------------
# The grid (perception-era redesign, 2026-06-07 — miss/blunder REMOVED,
# perception added, eg expanded; masks kept as an axis per the depth-
# dependent pawn/king-safety effects):
#   depth x qsearch-depth = {1,2,4,6} x {1,2,None}  (q0 dropped: off-product)
#   perception            = {0,0.2,0.4,0.6,1.0}     (dense below the knee)
#   avg_move_rank         = {1,2,3.5,5}             (covers basement high rank)
#   endgame_skill         = {0,1,2,None=Full}       (conversion; eg x rank)
#   masks                 = none / safety / positional / both  (4 combos)
# => 4*3*5*4*4*4 = 3840 configs.
# ---------------------------------------------------------------------------
GRID = GridSpec(
    depth=[1, 2, 4, 6],
    qsearch_depth=[1, 2, None],
    perception=[0.0, 0.2, 0.4, 0.6, 1.0],
    avg_move_rank=[1.0, 2.0, 3.5, 5.0],
    endgame_skill=[0, 1, 2, None],
    masks=GRID_MASK_COMBOS,
)

TINY = GridSpec(
    depth=[2, 4],
    qsearch_depth=[2, None],
    perception=[0.0, 1.0],
    avg_move_rank=[1.0],
    endgame_skill=[1, None],
    masks=GRID_MASK_COMBOS[:2],   # none + safety
)

EST_GAMES_PER_SEC = 49  # measured on a depth-1/2/4/6 mix with noise


def _mask_bools(disable_eval: tuple[str, ...]) -> tuple[int, int]:
    """Recover the two grid booleans from a config's disabled-eval slugs.
    safety <- king-safety present; positional <- pawn-structure present."""
    safety = 1 if "king-safety" in disable_eval else 0
    positional = 1 if "pawn-structure" in disable_eval else 0
    return safety, positional


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
            "name", "kind", "depth", "qsearch_depth", "perception",
            "avg_move_rank", "endgame_skill", "mask_safety", "mask_positional",
            "elo", "elo_error", "games",
        ])
        for name, r in sorted(result.ratings.items(), key=lambda kv: -kv[1].rating):
            c = result.subjects_by_name.get(name)
            kind = ("maia" if name.startswith("maia-")
                    else "reference" if name.startswith("ref-") else "grid")
            err = "" if r.error is None else f"{r.error:.1f}"
            if c is not None:
                qd = "inf" if c.qsearch_depth is None else c.qsearch_depth
                eg = "F" if c.endgame_skill is None else c.endgame_skill
                m_safety, m_pos = _mask_bools(c.disable_eval)
                w.writerow([
                    name, kind, c.depth, qd, c.perception,
                    c.avg_move_rank, eg, m_safety, m_pos,
                    f"{r.rating:.1f}", err, r.played,
                ])
            else:
                w.writerow([name, kind, "", "", "", "", "", "", "",
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
