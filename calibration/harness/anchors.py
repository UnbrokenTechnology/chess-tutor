"""The Maia anchor ladder + the measured human ratings used to pin the
rating pool to a human scale.

Only maia1 (net 1100), maia5 (net 1500), maia9 (net 1900) run as public
Lichess bots and therefore have *measured* human ratings. The other six
nets have only their training-target label and are placed on the scale
by connectivity (Ordo transitivity), not anchored. See ../README.md
"Anchor findings" for the full reasoning.

The measured values below are **Lichess rapid** snapshots (2026-06).
Caveats baked into the design:
  * They drift and are time-control-dependent — re-check before a real run.
  * The label->measured gap is non-uniform (+465 at 1100, ~-45 at 1900),
    so do NOT anchor on labels; anchor on these measured points.
  * Lichess-rapid != chess.com scale (our user's frame). A final constant
    shift to chess.com can be applied after fitting (a TODO, not a blocker
    — it's a pure offset on top of a correctly-shaped curve).
"""

from __future__ import annotations

# net label -> measured Lichess rapid rating (None = no measured anchor).
MEASURED_RAPID: dict[int, int | None] = {
    1100: 1565,   # maia1
    1200: None,
    1300: None,
    1400: None,
    1500: 1680,   # maia5
    1600: None,
    1700: None,
    1800: None,
    1900: 1855,   # maia9
}

# The single hard anchor: the middle measured point (least extrapolated).
# Ordo fixes the pool offset here; the other two measured points are passed
# as *loose* anchors (Ordo balances them) and serve as spacing cross-checks.
PRIMARY_ANCHOR_LABEL = 1500
PRIMARY_ANCHOR_NAME = "maia-1500"
PRIMARY_ANCHOR_RATING = 1680

# Loose anchors (name -> rating) — the other two measured points.
LOOSE_ANCHORS: dict[str, int] = {
    "maia-1100": 1565,
    "maia-1900": 1855,
}

ALL_NET_LABELS = list(MEASURED_RAPID.keys())
