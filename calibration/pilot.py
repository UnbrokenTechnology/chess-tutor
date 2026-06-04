"""Pilot run — de-risk the harness before committing the multi-day runs.

Goals (per PLAN-elo-calibration.md "Pilot first"):
  1. Prove the full pipeline (config -> fastchess vs Maia -> Ordo -> Elo)
     end-to-end on a real connected pool.
  2. Sanity-check the absolute numbers: do our no-noise depth configs land
     plausibly, and do the dials move Elo in the right direction / range?
  3. Cross-check anchor spacing: with maia-1500 pinned, do maia-1100 and
     maia-1900 land near their measured ratings (loose-anchored)?
  4. Produce PGNs to eyeball for human-likeness.

Run (from the calibration/ dir, in the venv):
  .venv/Scripts/python.exe pilot.py --quick          # tiny smoke (~1-2 min)
  .venv/Scripts/python.exe pilot.py                  # fuller pilot

This is NOT data collection for the model — it's a methodology check.
"""

from __future__ import annotations

import argparse

from harness import anchors
from harness.engines import BotConfig, MaiaEngine, Player
from harness.gauntlet import TournamentSpec, run as run_gauntlet
from harness.rate import rate


def build_players(quick: bool, base_depth: int) -> list[Player]:
    if quick:
        # Minimal connected pool: 3 configs spanning weak->strong + 2 anchors.
        maia = [MaiaEngine(1100), MaiaEngine(1500)]
        bots = [
            BotConfig("ct-d1", depth=1),
            BotConfig("ct-d3", depth=3),
            BotConfig(f"ct-d{base_depth}", depth=base_depth),
        ]
        return [*maia, *bots]

    # Fuller pilot: 5-net ladder (includes the 3 measured anchors) + a
    # spread of configs. The quick pilot already showed no-noise depth is
    # a high floor (d1 ~1875), so this probe focuses on how hard the
    # move-distribution dials pull DOWN toward the human range — sweeping
    # wild / blunder / rank heavily, with two no-noise depth brackets.
    maia = [MaiaEngine(r) for r in (1100, 1300, 1500, 1700, 1900)]
    bots = [
        BotConfig("ct-d1", depth=1),              # weak no-noise bracket
        BotConfig("ct-d6", depth=6),              # strong no-noise bracket
        # Wild sweep (uniform-random moves — the strongest weakener) at d4.
        BotConfig("ct-d4-wild20", depth=4, wild_chance=0.2),
        BotConfig("ct-d4-wild40", depth=4, wild_chance=0.4),
        BotConfig("ct-d4-wild60", depth=4, wild_chance=0.6),
        # Blunder sweep (force material-losing moves) at d4.
        BotConfig("ct-d4-blunder40", depth=4, blunder_chance=0.4),
        BotConfig("ct-d4-blunder70", depth=4, blunder_chance=0.7),
        # Variety (play a worse-ranked engine move) — weak dial, high centre.
        BotConfig("ct-d4-rank8", depth=4, avg_move_rank=8.0),
    ]
    return [*maia, *bots]


def print_table(ratings: dict) -> None:
    print()
    print(f"{'PLAYER':<22}{'ELO':>7}{'+/-':>7}{'GAMES':>7}")
    print("-" * 43)
    for r in sorted(ratings.values(), key=lambda x: -x.rating):
        err = "----" if r.error is None else f"{r.error:.0f}"
        print(f"{r.name:<22}{r.rating:>7.0f}{err:>7}{r.played:>7}")

    # Anchor spacing cross-check: maia-1500 is pinned; how close did the
    # loose-anchored measured points land to their real ratings?
    print()
    print("anchor cross-check (measured rapid vs Ordo placement):")
    for label, name in ((1100, "maia-1100"), (1500, "maia-1500"), (1900, "maia-1900")):
        measured = anchors.MEASURED_RAPID[label]
        got = ratings.get(name)
        if measured and got:
            print(f"  {name}: measured {measured}, placed {got.rating:.0f} "
                  f"(delta {got.rating - measured:+.0f})")


def main() -> None:
    ap = argparse.ArgumentParser(description="ELO-calibration pilot run")
    ap.add_argument("--quick", action="store_true", help="tiny smoke pool")
    ap.add_argument("--games-per-pair", type=int, default=20)
    ap.add_argument("--concurrency", type=int, default=8)
    ap.add_argument("--base-depth", type=int, default=6,
                    help="depth for the strong reference + noise-sweep configs")
    ap.add_argument("--sims", type=int, default=200, help="Ordo error-bar simulations")
    args = ap.parse_args()

    players = build_players(args.quick, args.base_depth)
    names = {p.name for p in players}
    # Pilot anchoring: ONE hard anchor on the middle measured point. This
    # leaves maia-1100 / maia-1900 free to land where the games put them —
    # a direct cross-check of whether our engine-pool scale matches the
    # measured human scale. (Production switches to loose multi-anchoring;
    # the code path exists in rate.py.)
    if anchors.PRIMARY_ANCHOR_NAME not in names:
        raise SystemExit(f"primary anchor {anchors.PRIMARY_ANCHOR_NAME} not in the pool")

    spec = TournamentSpec(
        players=players,
        games_per_pair=4 if args.quick else args.games_per_pair,
        concurrency=args.concurrency,
        tournament="roundrobin",
        pgn_name="pilot_quick" if args.quick else "pilot",
    )
    pgn = run_gauntlet(spec)

    ratings = rate(
        pgn,
        anchor=(anchors.PRIMARY_ANCHOR_NAME, anchors.PRIMARY_ANCHOR_RATING),
        sims=args.sims,
        out_name="pilot_quick" if args.quick else "pilot",
    )
    print_table(ratings)


if __name__ == "__main__":
    main()
