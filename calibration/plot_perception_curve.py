"""Visualize the perception P(see) margin curve (PLAN-perception.md).

X = move visibility V, Y = P(see), one line per perception level p.
Implements the REVISED curve (2026-06-07): perception-scaled plateau
    plateau(p) = 1 - (1 - PLATEAU_FLOOR) * (1 - p)
    m = p + V - 1
    m >= 0: P = plateau + (1 - plateau) * min(1, m / RAMP)
    m <  0: P = plateau * max(0, 1 + m / CLIFF)^2
    V == 1.0 exactly: P = 1.0 (no difficulty flags -> nothing to miss)
    p >= 1.0: bypass (P = 1.0)

Usage:  python plot_perception_curve.py [out.png]
"""

import sys

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np

PLATEAU_FLOOR = 0.8
RAMP = 0.3
CLIFF = 0.45


def p_see(v: float, p: float) -> float:
    if p >= 1.0 or v >= 1.0:
        return 1.0
    plateau = 1.0 - (1.0 - PLATEAU_FLOOR) * (1.0 - p)
    m = p + v - 1.0
    if m >= 0.0:
        return plateau + (1.0 - plateau) * min(1.0, m / RAMP)
    t = 1.0 + m / CLIFF
    return plateau * t * t if t > 0.0 else 0.0


def main() -> None:
    out = sys.argv[1] if len(sys.argv) > 1 else "runs/perception_curve.png"

    # Stop just short of 1.0 so the V == 1.0 special case (always seen)
    # is drawn as its own marker, not blended into the curve.
    vs = np.linspace(0.0, 0.999, 400)
    levels = [0.0, 0.2, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 0.95]
    cmap = plt.get_cmap("viridis")

    fig, ax = plt.subplots(figsize=(9, 6), dpi=150)
    for i, p in enumerate(levels):
        color = cmap(i / (len(levels) - 1))
        ys = [p_see(v, p) for v in vs]
        ax.plot(vs, ys, color=color, lw=2, label=f"p = {p:g}")
        # The V == 1.0 special case: always seen, even at p = 0.
        ax.plot([1.0], [1.0], marker="o", ms=5, color=color, zorder=5)

    ax.annotate(
        "V = 1.0: no difficulty flags\n→ always seen (exact case)",
        xy=(1.0, 1.0),
        xytext=(0.62, 0.55),
        fontsize=8,
        arrowprops=dict(arrowstyle="->", lw=0.8),
    )

    ax.set_xlabel("move visibility V (1.0 = no difficulty features)")
    ax.set_ylabel("P(see)")
    ax.set_title(
        "Perception margin curve — scaled plateau\n"
        f"plateau(p) = 1 − {1 - PLATEAU_FLOOR:.1f}·(1−p)   ·   "
        f"ramp width {RAMP}   ·   cliff at margin −{CLIFF}"
    )
    ax.set_xlim(0.0, 1.02)
    ax.set_ylim(-0.02, 1.05)
    ax.grid(True, alpha=0.3)
    ax.legend(title="perception", loc="upper left", fontsize=9)

    fig.tight_layout()
    fig.savefig(out)
    print(f"wrote {out}")


if __name__ == "__main__":
    main()
