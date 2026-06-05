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
    # Tactical-vision floor: with configs no longer playing each other
    # (seed-swap), the weakest configs need opponents below them to be
    # ratable. A tactically-blind bot (qsearch_depth 0) hangs pieces and is
    # weaker than any sane config — the natural, low-variance floor that
    # replaced the retired wild bots. The qdepth rung gives a spread up to
    # ~the blunder/depth refs.
    BotConfig("ref-d1-q0", depth=1, qsearch_depth=0),     # ~ floor (Martin)
    BotConfig("ref-d2-q0", depth=2, qsearch_depth=0),
    BotConfig("ref-d4-q1", depth=4, qsearch_depth=1),     # sees initial capture
    BotConfig("ref-d4-q2", depth=4, qsearch_depth=2),     # sees the recapture
    BotConfig("ref-d4-b70", depth=4, blunder_chance=0.7), # ~1245
    BotConfig("ref-d1", depth=1),                          # ~1750
    BotConfig("ref-d4", depth=4),                          # ~2100
    BotConfig("ref-d6", depth=6),                          # ~2435
    # Ceiling: stronger than any grid config (grid depth caps at 8) so the
    # grid's strongest no-noise configs aren't all-wins. It will itself be
    # all-wins and get excluded from the rating pass — that's fine, its job
    # is purely to give the grid's top a beatable-by-nobody-else opponent
    # to lose to.
    BotConfig("ref-d10", depth=10),                        # ~2600 ceiling
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
