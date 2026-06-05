"""Throwaway throughput benchmark: measure real games/sec on a depth mix
representative of the grid (incl. slow depth-6 + MultiPV-widening noise),
so grid runtime estimates are grounded, not guessed."""

import time

from harness import paths
from harness.engines import BotConfig
from harness.gauntlet import TournamentSpec, run as run_gauntlet
from harness.pools import opponent_pool

# 4 configs per depth: clean + wild (single-PV) + miss + rank (both widen
# MultiPV to 10 -> the slow path the grid is full of).
seeds = []
for d in (1, 2, 4, 6):
    seeds.append(BotConfig(f"b-d{d}", depth=d))
    seeds.append(BotConfig(f"b-d{d}-m40", depth=d, miss_chance=0.4))
    seeds.append(BotConfig(f"b-d{d}-r6", depth=d, avg_move_rank=6.0))

opp = opponent_pool()
# Seed-swap structure (opponents are seeds): configs don't play each other,
# matching the planned grid — measures the exact game population.
spec = TournamentSpec(
    players=[*opp, *seeds],
    games_per_pair=6,
    concurrency=16,
    tournament="gauntlet",
    seeds=len(opp),
    pgn_name="bench_rate",
)
t = time.time()
pgn = run_gauntlet(spec)
el = time.time() - t
n = sum(1 for ln in open(pgn, encoding="utf-8", errors="ignore") if ln.startswith("[Result"))
print(f"\n=== {n} games in {el:.1f}s = {n/el:.0f} games/s "
      f"(depth 1/2/4/6 mix with wild/miss/rank noise, concurrency 16) ===")
