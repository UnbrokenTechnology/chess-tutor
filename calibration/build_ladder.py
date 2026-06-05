"""Phase A — bootstrap a dense, Maia-anchored *seed ladder* from a small
hand-picked set of configs, by full round-robin.

The big grid (run_grid.py) uses the **seed-swap**: 2880 configs play a
fixed seed pool but never each other, to stay O(configs x pool). That is
affordable only if the seed pool is a **dense ladder** (no gap wider than
~200-250 Elo), so every config has near-level opponents and isn't stranded
as all-win / all-loss. We only have 3 *measured* human anchors (maia-1100/
1500/1900), so the dense rungs must be OUR OWN bots — placed by the
measurement, not assumed.

This script builds that ladder. For a small sample (~15-25 players) the
n^2 round-robin the big grid avoids is trivially cheap (~C(24,2) ~ 276
pairings), so we just play **everyone against everyone + the Maia nets**
in one pass and let Ordo rate the whole connected graph, anchored (loose)
on the 3 measured Maia points. One pass beats the iterative
promote-and-repeat at this scale: it fully connects in a single shot and
is robust to our Elo guesses being wrong (a mis-ranked bot still trades
games with whatever is actually near it).

Output: each rung's measured Elo, all-win/all-loss exclusions, a spacing
report (gaps to fill / redundant pairs to cull), and a greedy ~200-Elo-
spaced seed suggestion to paste into pools.py.

Workflow: run -> read the spacing report -> edit CANDIDATES (add a rung in
a flagged gap, cull a redundant pair) -> re-run. Then hand-validate 3-4
rungs against chess.com bots; that doubles as measuring the lichess->
chess.com offset. If a hand-checked rung lands where it feels right, snap
it to a round number and add it as a `--manual-anchor` on a re-rate (that
extends the trusted set downward; see PLAN / HANDOFF-endgame-skill.md).

Run: python build_ladder.py            # full run
     python build_ladder.py --dry-run  # size + estimate only
     python build_ladder.py --manual-anchor d1-q0-r2=250   # add a hand anchor
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

# ---------------------------------------------------------------------------
# Candidate ladder rungs — EDIT THIS TABLE between runs.
#
# Guessed Elos (in comments) are from the qdepth probe + the two Martin
# playtests; the run replaces them with measured values and flags gaps.
# Lever notes that shaped these picks:
#   * depth > 6 (full qsearch, rank 1) is ~perfect for us (beats every Maia
#     net) — no need to measure higher; `d6` is the ceiling rung (its job
#     is to be the unbeatable top so the real top configs have something to
#     lose to; it will likely be all-win -> excluded, which is fine).
#   * low qsearch = *blind to tactics*, not merely "worse": it makes a bot
#     blunder in believable low-Elo ways AND flips positional signals
#     negative (space is useless if you can't back it up). The primary
#     low-end spreader, alongside rank.
#   * avg_move_rank is a STRONG lever — it doesn't just avoid the best move,
#     it makes the bot miss the *only* good move. Main knob for the basement.
#   * miss% reads more human than blunder% (humans play scared and fail to
#     see tactics) — preferred for the one noisy basement rung.
#   * NO full sweeps here (no 10/20/30/40% comparisons) — the grid measures
#     sweeps. We just need a handful of arbitrary, rankable rungs.
#
# Sample DENSELY at the bottom (sub-1100 is the real gap; Maia covers
# ~1565-1855) and sparsely up high.
# ---------------------------------------------------------------------------
CANDIDATES: list[BotConfig] = [
    # --- rank-response curve on the BLIND base (d1-q0) -------------------
    # r1..r4 + the new r1.5, so the fit can see HOW rank scales between 1
    # and 2 (the steepest, most useful stretch). If r1->r2 is one big cliff
    # the equation can only say "set r to 1 or 2"; the curve shape (linear?
    # quadratic?) is what lets us fine-tune. (round-1 Elos in comments)
    BotConfig("d1-q0-r4-m40", depth=1, qsearch_depth=0, avg_move_rank=4.0, miss_chance=0.4),  # -72 floor
    BotConfig("d1-q0-r4", depth=1, qsearch_depth=0, avg_move_rank=4.0),    # 43
    BotConfig("d1-q0-r3", depth=1, qsearch_depth=0, avg_move_rank=3.0),    # 271
    BotConfig("d1-q0-r2", depth=1, qsearch_depth=0, avg_move_rank=2.0),    # 490  "Martin-ish" (lichess)
    BotConfig("d1-q0-r1.5", depth=1, qsearch_depth=0, avg_move_rank=1.5),  # NEW — between r1 & r2
    BotConfig("d1-q0", depth=1, qsearch_depth=0),                          # 852  (r1 baseline)

    # --- single-lever study on the SEES-FIRST-CAPTURE base (d1-q1) -------
    # One point each for rank / miss / blunder / combined, all off the same
    # ~1612 baseline so the deltas are clean (rank vs miss vs blunder on
    # identical footing). miss reads more human than blunder; r1.5 here
    # checks whether the rank curve shape transfers across bases. These
    # also FILL the 850->1590 gap that rank overshot last run.
    BotConfig("d1-q1", depth=1, qsearch_depth=1),                         # 1612 (r1 baseline)
    BotConfig("d1-q1-r1.5", depth=1, qsearch_depth=1, avg_move_rank=1.5),  # NEW
    BotConfig("d1-q1-r2", depth=1, qsearch_depth=1, avg_move_rank=2.0),   # 920 (rank overshoots)
    BotConfig("d1-q1-m10", depth=1, qsearch_depth=1, miss_chance=0.1),    # NEW — gentle miss
    BotConfig("d1-q1-m30", depth=1, qsearch_depth=1, miss_chance=0.3),    # NEW
    BotConfig("d1-q1-b10", depth=1, qsearch_depth=1, blunder_chance=0.1),  # NEW — gentle blunder
    BotConfig("d1-q1-b30", depth=1, qsearch_depth=1, blunder_chance=0.3),  # NEW
    BotConfig("d1-q1-m20-b20", depth=1, qsearch_depth=1, miss_chance=0.2, blunder_chance=0.2),  # NEW — combined

    # --- clean depth-2 / blind baseline (a distinct lever from the d1 rungs) ---
    BotConfig("d2-q0", depth=2, qsearch_depth=0),                         # 1589

    # --- upper ceiling ladder (above the Maia band ~1800) ---------------
    # depth > 6 is ~perfect (beats every Maia) so we stop at d6; d5 fills
    # the 1998->2497 gap so the strongest grid configs aren't all-win.
    BotConfig("d4", depth=4),                                             # 1998
    BotConfig("d5", depth=5),                                             # NEW — ~2250
    BotConfig("d6", depth=6),                                             # 2497 ceiling
]

#: Flag adjacent rated rungs farther apart than this (Elo) — a gap a config
#: could get stranded in; add a rung near the midpoint.
TARGET_SPACING = 250
#: Flag adjacent rungs closer than this (Elo) — redundant, cull one.
REDUNDANT = 100

EST_GAMES_PER_SEC = 60  # round-robin of fast low-depth bots + Maia nodes=1


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


def _parse_manual(specs: list[str]) -> dict[str, int]:
    """``["name=elo", ...]`` -> ``{name: elo}`` hand-set loose anchors."""
    out: dict[str, int] = {}
    for s in specs:
        name, _, val = s.partition("=")
        out[name.strip()] = int(val)
    return out


def main() -> None:
    ap = argparse.ArgumentParser(description="Build a Maia-anchored seed ladder")
    ap.add_argument("--games-per-pair", type=int, default=40)
    ap.add_argument("--concurrency", type=int, default=16)
    ap.add_argument("--sims", type=int, default=400)
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument(
        "--manual-anchor",
        action="append",
        default=[],
        metavar="NAME=ELO",
        help="Add a hand-set loose anchor (e.g. d1-q0-r2=250). Repeatable. "
        "Use after hand-validating a rung you trust.",
    )
    args = ap.parse_args()

    players: list[Player] = [*CANDIDATES, *maia_ladder()]
    n = len(players)
    gpp = max(2, _even(args.games_per_pair))
    pairs = n * (n - 1) // 2
    total = pairs * gpp
    hrs = total / EST_GAMES_PER_SEC / 3600
    print(
        f"{len(CANDIDATES)} candidates + {len(maia_ladder())} Maia = {n} players; "
        f"round-robin C({n},2)={pairs} pairs x {gpp} = ~{total:,} games "
        f"(~{hrs*60:.0f} min at {EST_GAMES_PER_SEC} g/s)"
    )
    if args.dry_run:
        return

    out_dir = paths.runs_dir() / "ladder"
    out_dir.mkdir(parents=True, exist_ok=True)
    pgn = out_dir / "ladder.pgn"

    spec = TournamentSpec(
        players=players,
        games_per_pair=gpp,
        concurrency=args.concurrency,
        tournament="roundrobin",
    )
    # Round-robin of ~24 players fits one fastchess command (no batching);
    # the command-line-length cap only bites the 2880-config grid.
    have = _count_results(pgn)
    if have >= total:
        print(f"[ladder] {have} games already present — skip play, re-rate")
    else:
        print(f"[ladder] {have}/{total} games — running round-robin")
        run_gauntlet(spec, pgn_path=pgn)

    measured = {f"maia-{lab}": r for lab, r in anchors.MEASURED_RAPID.items() if r}
    manual = _parse_manual(args.manual_anchor)
    if manual:
        print(f"[ladder] manual anchors: {manual}")
    loose = {**measured, **manual}
    ratings = rate(pgn, loose_anchors=loose, sims=args.sims, out_name="ladder")

    cand_names = {c.name for c in CANDIDATES}

    # ---- the rated ladder, low -> high -----------------------------------
    rated = sorted(ratings.values(), key=lambda r: r.rating)
    print("\n=== rated ladder (low -> high) ===")
    print(f"{'rung':<16}{'Elo':>7}{'+/-':>7}{'games':>8}   gap-below")
    prev = None
    for r in rated:
        kind = "maia" if r.name.startswith("maia-") else "cfg"
        err = "----" if r.error is None else f"{r.error:.0f}"
        gap = "" if prev is None else f"{r.rating - prev:>+6.0f}"
        flag = ""
        if prev is not None:
            d = r.rating - prev
            if d > TARGET_SPACING:
                flag = "  <-- GAP: add a rung ~here"
            elif d < REDUNDANT:
                flag = "  <-- close (cull one)"
        print(f"{r.name:<16}{r.rating:>7.0f}{err:>7}{r.played:>8}   {gap:>7}{flag}  [{kind}]")
        prev = r.rating

    # ---- exclusions (all-win / all-loss -> no finite Elo) -----------------
    excluded = sorted(cand_names - set(ratings))
    if excluded:
        print(
            "\nexcluded (all-win or all-loss vs the field — off the measurable "
            f"top/bottom): {', '.join(excluded)}"
        )
        print("  (expected for the ceiling/basement extremes; their job is to give "
              "the next rung in someone to beat / lose to.)")

    # ---- greedy ~TARGET_SPACING-spaced seed suggestion -------------------
    keep: list = []
    for r in rated:
        if not keep or r.rating - keep[-1].rating >= TARGET_SPACING:
            keep.append(r)
    print(f"\n=== suggested seed pool (greedy ~{TARGET_SPACING} Elo spacing) ===")
    print("(always keep the 3 measured Maia as scale anchors regardless of this cull)")
    for r in keep:
        kind = "maia" if r.name.startswith("maia-") else "cfg"
        print(f"  {r.name:<16}{r.rating:>7.0f}  [{kind}]")

    # ---- CSV ------------------------------------------------------------
    csv_path = out_dir / "ladder_results.csv"
    with open(csv_path, "w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["name", "kind", "elo", "elo_error", "games"])
        for r in rated:
            kind = "maia" if r.name.startswith("maia-") else "cfg"
            err = "" if r.error is None else f"{r.error:.1f}"
            w.writerow([r.name, kind, f"{r.rating:.1f}", err, r.played])
    print(f"\nwrote {csv_path}")


if __name__ == "__main__":
    main()
