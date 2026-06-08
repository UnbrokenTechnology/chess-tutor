"""Fixed opponent pools for the grid runs.

A grid config (seed) is rated by playing a GAUNTLET against a fixed
opponent pool — never the full O(n^2) round-robin among thousands of
configs. The pool has two parts:

* **Maia ladder** (9 nets) — the human-scale anchors.
* **Reference bots** — a spread of OUR configs from ~sub-600 to ~2400,
  so every grid config (however weak or strong) has *some* near-level
  opponents. Without these, configs below maia-1100 (~1565) or above
  maia-1900 would only play saturated (≈100% one-sided) pairings and get
  poorly-estimated Elo. They are the self-play-connectivity rungs the
  design called for, and they get precisely rated themselves (every seed
  plays them).

Ratings (anchored on the measured Maia points) come from one Ordo pass
over all games; the whole graph is connected because every seed plays
every pool member.
"""

from __future__ import annotations

from .engines import BotConfig, MaiaEngine, Player

# Approx pilot-measured Elos in the comments are coarse (single-anchor)
# guides for range coverage, not ground truth.
REFERENCE_BOTS: list[BotConfig] = [
    # Connectivity spine = the LOCKED ladder rungs (calibration commit
    # 2026-06-07), spread ~every 300 Elo. With the seed-swap (configs don't
    # play each other), grid configs anchor to the Maia ground truth only
    # THROUGH this pool — so it must densely bracket the whole 500-2300 range
    # or weak/strong configs float (the hard-won ladder lesson). The 9 Maia
    # add density in their 1565-1855 measured band. Dials copied verbatim
    # from run_ladder.py RUNGS so these are our calibrated reference points.
    BotConfig("ref-t500",  depth=1, qsearch_depth=1, perception=0.20, avg_move_rank=2.8, endgame_skill=1),
    BotConfig("ref-t800",  depth=1, qsearch_depth=2, perception=0.55, avg_move_rank=3.0, endgame_skill=1),
    BotConfig("ref-t1100", depth=1, qsearch_depth=2, perception=0.90, avg_move_rank=2.4, endgame_skill=2),
    BotConfig("ref-t1400", depth=2, qsearch_depth=2, perception=1.00, avg_move_rank=1.9, endgame_skill=2),
    BotConfig("ref-t1700", depth=2, perception=1.00, avg_move_rank=1.4, endgame_skill=2),
    BotConfig("ref-t2000", depth=4),
    BotConfig("ref-t2300", depth=6, avg_move_rank=1.3),
    # Ceiling: stronger than any grid config (grid depth caps at 6) so the
    # grid's strongest configs aren't all-wins. Itself all-wins -> excluded
    # from the rating pass; its job is purely to give the top a loss.
    BotConfig("ref-d8", depth=8),
]


def maia_ladder() -> list[MaiaEngine]:
    return [MaiaEngine(r) for r in range(1100, 2000, 100)]


def opponent_pool() -> list[Player]:
    """The fixed gauntlet pool every grid config plays: Maia anchors +
    reference rungs spanning ~floor..~2600. With the seed-swap (these are
    the fastchess *seeds*; configs are non-seeds), configs play this pool
    but not each other, and connect to one another through it."""
    return [*maia_ladder(), *REFERENCE_BOTS]


# Eval-mask thematic groups (for the mask experiment). The 8 categories
# bucketed by chess concept, so a backbone rule can disable a whole theme
# ("a sub-1200 bot doesn't grasp pawn play") as one unit. Slugs match
# EvalCategory::slug() in the engine.
MASK_GROUPS: dict[str, tuple[str, ...]] = {
    "pawnspace": ("pawn-structure", "passed-pawns", "space"),
    "activity": ("pieces", "mobility"),
    "safety": ("king-safety", "threats"),
    "initiative": ("initiative",),
}

ALL_MASKS: tuple[str, ...] = (
    "pawn-structure", "passed-pawns", "space", "pieces",
    "mobility", "king-safety", "threats", "initiative",
)

# Mask combos folded into the MAIN grid as a dimension — the two effects the
# low-band experiment isolated (runs/lowband_masks):
#   * ``safety`` (king-safety + threats) is a CONSISTENT Elo handicap
#     (~-185 at both low bands, never flips sign).
#   * ``positional`` merges the two near-identical SIGN-FLIPPERS
#     (pawnspace + activity): masking them HELPS a tactically-blind bot
#     (+~240 at d1-q0) but HURTS a sighted one (-~120 at d2-q0). Their Elo
#     effects are close enough that the model doesn't need them separated;
#     the pawn-vs-piece distinction is a teaching/style choice, not an Elo
#     factor. ``initiative`` is dropped (small + noisy).
# Two underlying booleans (safety on/off, positional on/off) => 4 combos.
GRID_MASK_COMBOS: list[tuple[str, tuple[str, ...]]] = [
    ("", ()),
    ("safety", MASK_GROUPS["safety"]),
    ("positional", MASK_GROUPS["pawnspace"] + MASK_GROUPS["activity"]),
    ("both", MASK_GROUPS["safety"] + MASK_GROUPS["pawnspace"] + MASK_GROUPS["activity"]),
]
