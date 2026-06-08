"""Q1-floor basement sweep (user feel-test follow-up 2026-06-07).

Feel-test verdict on t500 (d1q0-p0-r2.6): the ELO felt right but q0 is
UNHUMAN — it can't see the immediate recapture, so it parks its queen in
front of a pawn (the hang becomes the engine's own #0 move, which the
self-hang filter always keeps). Even a 400 human sees "I just hung my
queen to that pawn." Fix: make qsearch=1 the FLOOR (sees immediate
recaptures) and drive strength down with avg-rank instead. chess.com's
worst bot is ~250 (~500 for us), so t100-t400 aren't needed — t500
becomes the floor.

Maps the d1q1-p0 (eg0) rank curve so the new q1 basement rungs can be
read off. p0 (no perception) is the right floor lever: perception is
the HIGHER-elo dial in the monotone schedule, and p0 already limits
tactical vision so the bot stays weak with less rank than expected.

Run:  python run_q1floor.py
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
    # d1q1-p0 rank curve, no endgame books (a sub-1000 bot botches them).
    BotConfig("q1r1", depth=1, qsearch_depth=1, perception=0.0, avg_move_rank=1.0, endgame_skill=0),
    BotConfig("q1r2", depth=1, qsearch_depth=1, perception=0.0, avg_move_rank=2.0, endgame_skill=0),
    BotConfig("q1r3", depth=1, qsearch_depth=1, perception=0.0, avg_move_rank=3.0, endgame_skill=0),
    BotConfig("q1r4", depth=1, qsearch_depth=1, perception=0.0, avg_move_rank=4.0, endgame_skill=0),
    BotConfig("q1r5", depth=1, qsearch_depth=1, perception=0.0, avg_move_rank=5.0, endgame_skill=0),
    BotConfig("q1r6", depth=1, qsearch_depth=1, perception=0.0, avg_move_rank=6.0, endgame_skill=0),
    BotConfig("q1r7", depth=1, qsearch_depth=1, perception=0.0, avg_move_rank=7.0, endgame_skill=0),
    BotConfig("q1r8", depth=1, qsearch_depth=1, perception=0.0, avg_move_rank=8.0, endgame_skill=0),
    # A couple with basic books for the ~800-1000 upper basement.
    BotConfig("q1r25e1", depth=1, qsearch_depth=1, perception=0.0, avg_move_rank=2.5, endgame_skill=1),
    BotConfig("q1r3e1", depth=1, qsearch_depth=1, perception=0.0, avg_move_rank=3.0, endgame_skill=1),
]

EST_GAMES_PER_SEC = 60


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
    ap = argparse.ArgumentParser(description="Q1-floor basement rank sweep")
    ap.add_argument("--design-only", action="store_true")
    ap.add_argument("--games-per-pair", type=int, default=60)
    ap.add_argument("--concurrency", type=int, default=16)
    ap.add_argument("--sims", type=int, default=400)
    args = ap.parse_args()

    print("=== q1-floor sweep ===")
    for cfg in CONFIGS:
        print(f"  {cfg.name:>10}  {' '.join(cfg.uci_args()[1:-2])}")
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

    out_dir = paths.runs_dir() / "q1floor"
    out_dir.mkdir(parents=True, exist_ok=True)
    pgn = out_dir / "q1floor.pgn"
    spec = TournamentSpec(
        players=players,
        games_per_pair=gpp,
        concurrency=args.concurrency,
        tournament="roundrobin",
    )
    if _count_results(pgn) >= total:
        print(f"[q1floor] {total} games present — skip play, re-rate")
    else:
        run_gauntlet(spec, pgn_path=pgn)

    measured = {f"maia-{lab}": r for lab, r in anchors.MEASURED_RAPID.items() if r}
    ratings = rate(pgn, loose_anchors=measured, sims=args.sims, out_name="q1floor")

    print("\n=== d1q1-p0 rank curve ===")
    print(f"{'config':>10}{'elo':>8}{'+/-':>6}")
    rows = []
    for cfg in CONFIGS:
        r = ratings.get(cfg.name)
        if r is None:
            print(f"{cfg.name:>10}  excluded")
            rows.append((cfg.name, None, None))
            continue
        err = "----" if r.error is None else f"{r.error:.0f}"
        print(f"{cfg.name:>10}{r.rating:>8.0f}{err:>6}")
        rows.append((cfg.name, r.rating, r.played))

    print("\n=== maia anchors (sanity) ===")
    for m in maia_ladder():
        r = ratings.get(m.name)
        if r:
            print(f"  {m.name:<10}{r.rating:>7.0f}")

    csv_path = out_dir / "q1floor_results.csv"
    with open(csv_path, "w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["name", "elo", "games"])
        for name, e, g in rows:
            w.writerow([name, "" if e is None else f"{e:.1f}", g or ""])
    print(f"\nwrote {csv_path}")


if __name__ == "__main__":
    main()
