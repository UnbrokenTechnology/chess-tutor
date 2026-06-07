"""Measure the cells the MONOTONE-PERCEPTION ladder schedule needs
(user redesign 2026-06-07: perception rises monotonically with ELO and
hits 1.0 by ~1400 — "sees all moves, picks imperfectly" — with rank
carrying the judgment-weakness from there up).

New cells: the d2q2 rank curve at p=1.0 (only r1 was measured), the
high-p x rank combos for the 700-1300 band, d3 bare, and rank on deep
depth (d4/d5 r1.1-1.2 — the old data warns deep-depth rank slopes are
steep ~1640/unit; this measures whether 0.1 steps are usable there).

Run:  python run_schedule_cells.py
"""

from __future__ import annotations

import argparse
import csv

from harness import anchors, paths
from harness.engines import BotConfig, Player
from harness.gauntlet import TournamentSpec
from harness.gauntlet import run as run_gauntlet
from harness.pools import maia_ladder
from harness.rate import rate

CONFIGS: list[BotConfig] = [
    # --- d2q2 rank curve at FULL perception (t1400-t1800 band) ---
    BotConfig("d2q2r12", depth=2, qsearch_depth=2, avg_move_rank=1.2),
    BotConfig("d2q2r14", depth=2, qsearch_depth=2, avg_move_rank=1.4),
    BotConfig("d2q2r16", depth=2, qsearch_depth=2, avg_move_rank=1.6),
    BotConfig("d2q2r18", depth=2, qsearch_depth=2, avg_move_rank=1.8),
    BotConfig("d2q2r20", depth=2, qsearch_depth=2, avg_move_rank=2.0),
    BotConfig("d2q2r25", depth=2, qsearch_depth=2, avg_move_rank=2.5),
    # --- high-p x rank (t1200-t1300 band, eg2) ---
    BotConfig("d2q2p08r20", depth=2, qsearch_depth=2, perception=0.8, avg_move_rank=2.0, endgame_skill=2),
    BotConfig("d2q2p08r25", depth=2, qsearch_depth=2, perception=0.8, avg_move_rank=2.5, endgame_skill=2),
    BotConfig("d2q2p09r22", depth=2, qsearch_depth=2, perception=0.9, avg_move_rank=2.2, endgame_skill=2),
    # --- d1q1 mid-p x rank (t1000-t1100 band, eg1) ---
    BotConfig("d1q1p06r20", depth=1, qsearch_depth=1, perception=0.6, avg_move_rank=2.0, endgame_skill=1),
    BotConfig("d1q1p06r25", depth=1, qsearch_depth=1, perception=0.6, avg_move_rank=2.5, endgame_skill=1),
    BotConfig("d1q1p06r30", depth=1, qsearch_depth=1, perception=0.6, avg_move_rank=3.0, endgame_skill=1),
    BotConfig("d1q1p07r23", depth=1, qsearch_depth=1, perception=0.7, avg_move_rank=2.3, endgame_skill=1),
    # --- d1q0 rising-p basement top (t700-t900 band, eg1) ---
    BotConfig("d1q0p02r20", depth=1, qsearch_depth=0, perception=0.2, avg_move_rank=2.0, endgame_skill=1),
    BotConfig("d1q0p04r16", depth=1, qsearch_depth=0, perception=0.4, avg_move_rank=1.6, endgame_skill=1),
    BotConfig("d1q0p06r13", depth=1, qsearch_depth=0, perception=0.6, avg_move_rank=1.3, endgame_skill=1),
    # --- upper rungs at full perception ---
    BotConfig("d3", depth=3),
    BotConfig("d4r11", depth=4, avg_move_rank=1.1),
    BotConfig("d5r11", depth=5, avg_move_rank=1.1),
    BotConfig("d5r12", depth=5, avg_move_rank=1.2),
]

EST_GAMES_PER_SEC = 45


def _count_results(pgn) -> int:
    if not pgn.exists():
        return 0
    return sum(
        1
        for ln in open(pgn, "r", encoding="utf-8", errors="ignore")
        if ln.startswith("[Result ")
    )


def _even(n: int) -> int:
    return n if n % 2 == 0 else n + 1


def main() -> None:
    ap = argparse.ArgumentParser(description="Monotone-schedule cell sweep")
    ap.add_argument("--design-only", action="store_true")
    ap.add_argument("--games-per-pair", type=int, default=40)
    ap.add_argument("--concurrency", type=int, default=16)
    ap.add_argument("--sims", type=int, default=400)
    args = ap.parse_args()

    print("=== schedule cells ===")
    for cfg in CONFIGS:
        dials = " ".join(cfg.uci_args()[1:-2])
        print(f"  {cfg.name:>12}  {dials}")
    if args.design_only:
        return

    players: list[Player] = [*CONFIGS, *maia_ladder()]
    n = len(players)
    gpp = max(2, _even(args.games_per_pair))
    pairs = n * (n - 1) // 2
    total = pairs * gpp
    print(
        f"\nround-robin C({n},2)={pairs} x {gpp} = ~{total:,} games "
        f"(~{total / EST_GAMES_PER_SEC / 60:.0f} min)"
    )

    out_dir = paths.runs_dir() / "schedule_cells"
    out_dir.mkdir(parents=True, exist_ok=True)
    pgn = out_dir / "cells.pgn"
    spec = TournamentSpec(
        players=players,
        games_per_pair=gpp,
        concurrency=args.concurrency,
        tournament="roundrobin",
    )
    if _count_results(pgn) >= total:
        print(f"[cells] {total} games present — skip play, re-rate")
    else:
        run_gauntlet(spec, pgn_path=pgn)

    measured = {f"maia-{lab}": r for lab, r in anchors.MEASURED_RAPID.items() if r}
    ratings = rate(pgn, loose_anchors=measured, sims=args.sims, out_name="schedule_cells")

    print("\n=== measured ===")
    rows = []
    for cfg in CONFIGS:
        r = ratings.get(cfg.name)
        if r is None:
            print(f"  {cfg.name:>12}  excluded")
            rows.append((cfg.name, None, None))
            continue
        err = "----" if r.error is None else f"{r.error:.0f}"
        print(f"  {cfg.name:>12}{r.rating:>8.0f}  +/-{err}")
        rows.append((cfg.name, r.rating, r.played))

    print("\n=== maia anchors (sanity) ===")
    for m in maia_ladder():
        r = ratings.get(m.name)
        if r:
            print(f"  {m.name:<10}{r.rating:>7.0f}")

    csv_path = out_dir / "cells_results.csv"
    with open(csv_path, "w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["name", "elo", "games"])
        for name, e, g in rows:
            w.writerow([name, "" if e is None else f"{e:.1f}", g or ""])
    print(f"\nwrote {csv_path}")


if __name__ == "__main__":
    main()
