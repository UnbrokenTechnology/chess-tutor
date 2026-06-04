"""Full-factorial grid run — the bulk data-collection workhorse.

Builds the Cartesian product of the dial value-lists in ``GRID`` (every
combination, so dial *interactions* are captured), plays each config as a
gauntlet seed against the fixed opponent pool (Maia ladder + reference
rungs), rates everything with one Ordo pass (loose multi-anchored on the
measured Maia points), and dumps a tidy ``grid_results.csv`` of
``dials -> measured Elo`` for offline analysis from any direction.

Designed for an unattended multi-day run:
  * Seeds are batched (Windows command-line length limits one fastchess
    invocation to ~150 engines); one PGN per batch under runs/grid/.
  * Re-running SKIPS already-complete batch PGNs and ``-resume``s partial
    ones, so a killed run continues where it stopped.
  * Adjudication (two-sided resign + draw + maxmoves) keeps games short.

Usage (calibration/ dir, venv python):
  python run_grid.py --dry-run                 # print grid size + estimate
  python run_grid.py --tiny                     # tiny smoke grid end-to-end
  python run_grid.py                            # the full GRID
  python run_grid.py                            # again -> resumes / re-rates
"""

from __future__ import annotations

import argparse
import csv

from harness import anchors, paths
from harness.engines import BotConfig
from harness.gauntlet import TournamentSpec, run as run_gauntlet
from harness.grid import GridSpec, build_grid
from harness.pools import opponent_pool
from harness.rate import rate

# ---------------------------------------------------------------------------
# The grid. Depth capped at 8: the pilot showed no-noise depth is a high
# floor (d1~1750, d6~2440) — depths above ~8 only add strength past our
# 2000 teaching ceiling and cost the most time. The move-distribution dials
# carry the human range, so they get the most resolution.
# ---------------------------------------------------------------------------
GRID = GridSpec(
    depth=[1, 2, 4, 6, 8],
    avg_move_rank=[1.0, 2.0, 4.0, 6.0],
    blunder_chance=[0.0, 0.2, 0.4, 0.6],
    miss_chance=[0.0, 0.3],
    wild_chance=[0.0, 0.2, 0.4, 0.6],
)

TINY = GridSpec(depth=[2, 4], avg_move_rank=[1.0], wild_chance=[0.0, 0.4])

BATCH_SIZE = 60          # seeds per fastchess invocation (cmd-length safe)
EST_GAMES_PER_SEC = 45   # conservative; pilot hit ~60 with shallow configs


def even(n: int) -> int:
    return n if n % 2 == 0 else n + 1


def count_results(pgn) -> int:
    if not pgn.exists():
        return 0
    n = 0
    with open(pgn, "r", encoding="utf-8", errors="ignore") as f:
        for line in f:
            if line.startswith("[Result "):
                n += 1
    return n


def main() -> None:
    ap = argparse.ArgumentParser(description="Full-factorial grid run")
    ap.add_argument("--dry-run", action="store_true", help="print size + estimate, don't run")
    ap.add_argument("--tiny", action="store_true", help="use the tiny smoke grid")
    ap.add_argument("--games-per-config", type=int, default=400)
    ap.add_argument("--concurrency", type=int, default=14)
    ap.add_argument("--batch-size", type=int, default=BATCH_SIZE)
    ap.add_argument("--sims", type=int, default=400, help="Ordo error-bar simulations")
    args = ap.parse_args()

    spec = TINY if args.tiny else GRID
    if args.tiny:
        args.games_per_config = min(args.games_per_config, 64)

    seeds = build_grid(spec)
    opponents = opponent_pool()
    n_opp = len(opponents)
    games_per_pair = max(2, even(round(args.games_per_config / n_opp)))
    per_config = games_per_pair * n_opp
    total_games = len(seeds) * per_config
    n_batches = (len(seeds) + args.batch_size - 1) // args.batch_size
    est_sec = total_games / EST_GAMES_PER_SEC

    print(f"grid: {len(seeds)} configs  x  {n_opp} opponents  x  {games_per_pair} games/pair")
    print(f"      = {per_config} games/config, {total_games:,} games total")
    print(f"      {n_batches} batches of <= {args.batch_size} seeds, concurrency {args.concurrency}")
    print(f"      est. wall-clock ~{est_sec/3600:.1f} h at {EST_GAMES_PER_SEC} games/s "
          f"(slower if many high-depth configs)")
    if args.dry_run:
        return

    grid_dir = paths.runs_dir() / ("grid_tiny" if args.tiny else "grid")
    grid_dir.mkdir(parents=True, exist_ok=True)

    # --- run batches (skip complete, resume partial) ---
    batch_pgns = []
    for b in range(n_batches):
        batch_seeds = seeds[b * args.batch_size:(b + 1) * args.batch_size]
        pgn = grid_dir / f"batch_{b:03d}.pgn"
        batch_pgns.append(pgn)
        expected = len(batch_seeds) * per_config
        have = count_results(pgn)
        if have >= expected:
            print(f"[batch {b+1}/{n_batches}] complete ({have} games) — skip")
            continue
        print(f"[batch {b+1}/{n_batches}] {len(batch_seeds)} seeds, "
              f"{have}/{expected} games done — running")
        tspec = TournamentSpec(
            players=[*batch_seeds, *opponents],  # seeds FIRST for gauntlet
            games_per_pair=games_per_pair,
            concurrency=args.concurrency,
            tournament="gauntlet",
            seeds=len(batch_seeds),
        )
        run_gauntlet(tspec, pgn_path=pgn)

    # --- one Ordo pass over all batches, loose multi-anchored ---
    all_pgn = grid_dir / "all.pgn"
    with open(all_pgn, "w", encoding="utf-8", errors="ignore") as out:
        for pgn in batch_pgns:
            if pgn.exists():
                out.write(pgn.read_text(encoding="utf-8", errors="ignore"))
    measured = {f"maia-{lab}": r for lab, r in anchors.MEASURED_RAPID.items() if r}
    ratings = rate(
        all_pgn,
        loose_anchors=measured,
        sims=args.sims,
        out_name="grid_tiny" if args.tiny else "grid",
    )

    # --- dump dials -> Elo CSV ---
    by_name: dict[str, BotConfig] = {c.name: c for c in seeds}
    by_name.update({c.name: c for c in opponents if isinstance(c, BotConfig)})
    csv_path = grid_dir / "grid_results.csv"
    with open(csv_path, "w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow([
            "name", "kind", "depth", "avg_move_rank", "blunder_chance",
            "blunder_min_material", "blunder_max_material", "miss_chance",
            "wild_chance", "guaranteed_mate_in", "disable_eval",
            "elo", "elo_error", "games",
        ])
        for name, r in sorted(ratings.items(), key=lambda kv: -kv[1].rating):
            c = by_name.get(name)
            kind = "maia" if name.startswith("maia-") else (
                "reference" if name.startswith("ref-") else "grid")
            if c is not None:
                w.writerow([
                    name, kind, c.depth, c.avg_move_rank, c.blunder_chance,
                    c.blunder_min_material, c.blunder_max_material, c.miss_chance,
                    c.wild_chance, c.guaranteed_mate_in, "|".join(c.disable_eval),
                    f"{r.rating:.1f}", "" if r.error is None else f"{r.error:.1f}",
                    r.played,
                ])
            else:
                w.writerow([name, kind, "", "", "", "", "", "", "", "", "",
                            f"{r.rating:.1f}",
                            "" if r.error is None else f"{r.error:.1f}", r.played])
    print(f"\nwrote {csv_path}  ({len(ratings)} rated players)")
    print("top grid configs:")
    grid_rows = [(n, r) for n, r in ratings.items() if not n.startswith(("maia-", "ref-"))]
    for n, r in sorted(grid_rows, key=lambda kv: -kv[1].rating)[:5]:
        print(f"  {n:<28}{r.rating:7.0f}  +/-{'' if r.error is None else f'{r.error:.0f}'}")


if __name__ == "__main__":
    main()
