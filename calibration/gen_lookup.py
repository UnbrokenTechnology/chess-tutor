"""Codegen: grid_results.csv -> the baked Rust 5-D lookup table for
core/engine/src/calibration.rs.

The product does multivariate (5-linear) interpolation over the measured
grid (depth x qsearch x perception x rank x eg), so we bake the measured
Elo at every knot. This:
  1. reads the no-mask grid configs,
  2. lays them on the 5-D knot lattice (NaN where a config was excluded —
     all-win/all-loss extremes Ordo couldn't rate),
  3. fills the gaps (nearest-along-axis, iterated),
  4. monotone-clamps per axis (depth/qsearch/perception/eg up, rank down)
     so the sliders always move Elo the right way and grid noise can't make
     a fiber non-monotone,
  5. emits the Rust knot arrays + a flat `const LOOKUP: [f32; N]` (row-major
     in depth,qsearch,perception,rank,eg order).

qsearch full-vision (CSV "inf") is encoded as 10.0 on the interp axis so a
finite GUI cap (q3..q9) interpolates between q2 and full. eg Full ("F") = 3.

Run:  python gen_lookup.py [path/to/grid_results.csv]   # prints the Rust block
"""

from __future__ import annotations

import sys
import csv
from pathlib import Path

QINF = 10.0  # full-vision position on the qsearch interp axis
EGF = 3.0    # Full endgame tier

AXES = ["depth", "qsearch", "perception", "avg_move_rank", "eg"]


def encode(row):
    q = QINF if str(row["qsearch_depth"]).strip() == "inf" else float(row["qsearch_depth"])
    eg = EGF if str(row["endgame_skill"]).strip() == "F" else float(row["endgame_skill"])
    return (float(row["depth"]), q, float(row["perception"]),
            float(row["avg_move_rank"]), eg)


def load(path):
    rows = [r for r in csv.DictReader(open(path)) if r["kind"] == "grid"
            and r["mask_safety"] == "0" and r["mask_positional"] == "0"]
    pts = {}
    for r in rows:
        try:
            elo = float(r["elo"])
        except ValueError:
            continue
        pts[encode(r)] = elo
    # distinct sorted knots per axis
    knots = []
    for i in range(5):
        knots.append(sorted({k[i] for k in pts}))
    return pts, knots


def build_table(pts, knots):
    import itertools
    shape = [len(k) for k in knots]
    tbl = {}
    miss = 0
    for idx in itertools.product(*(range(s) for s in shape)):
        key = tuple(knots[a][idx[a]] for a in range(5))
        if key in pts:
            tbl[idx] = pts[key]
        else:
            tbl[idx] = None
            miss += 1
    return tbl, shape, miss


def fill(tbl, shape):
    """Fill None by nearest present along each axis, iterated to fixpoint."""
    import itertools
    changed = True
    while changed:
        changed = False
        for idx in itertools.product(*(range(s) for s in shape)):
            if tbl[idx] is not None:
                continue
            vals = []
            for a in range(5):
                for step in (1, -1):
                    j = list(idx)
                    j[a] += step
                    if 0 <= j[a] < shape[a] and tbl[tuple(j)] is not None:
                        vals.append(tbl[tuple(j)])
            if vals:
                tbl[idx] = sum(vals) / len(vals)
                changed = True
    return tbl


# axis monotone direction: +1 stronger as the knot rises, -1 weaker
MONO = {"depth": +1, "qsearch": +1, "perception": +1, "avg_move_rank": -1, "eg": +1}


def monotone_clamp(tbl, shape):
    """Iterated per-axis pool: enforce each 1-D fiber is monotone in the
    axis's direction (cumulative max/min)."""
    import itertools
    for _ in range(6):
        for a in range(5):
            d = MONO[AXES[a]]
            others = [range(shape[b]) for b in range(5) if b != a]
            from itertools import product as prod
            for combo in prod(*others):
                # reconstruct the fiber along axis a
                def at(k):
                    idx = list(combo[:a]) + [k] + list(combo[a:])
                    return tuple(idx)
                fiber = [tbl[at(k)] for k in range(shape[a])]
                if d > 0:
                    for k in range(1, shape[a]):
                        if fiber[k] < fiber[k - 1]:
                            fiber[k] = fiber[k - 1]
                else:
                    for k in range(1, shape[a]):
                        if fiber[k] > fiber[k - 1]:
                            fiber[k] = fiber[k - 1]
                for k in range(shape[a]):
                    tbl[at(k)] = fiber[k]
    return tbl


def emit_rust(knots, shape, tbl):
    import itertools
    names = ["DEPTH", "QSEARCH", "PERCEPTION", "RANK", "EG"]
    out = []
    for nm, ks in zip(names, knots):
        arr = ", ".join(f"{k:g}" for k in ks)
        out.append(f"const {nm}_KNOTS: &[f32] = &[{arr}];")
    n = 1
    for s in shape:
        n *= s
    flat = []
    for idx in itertools.product(*(range(s) for s in shape)):
        flat.append(round(tbl[idx]))
    out.append(f"/// Measured Elo at each ({'x'.join(names)}) knot, row-major.")
    out.append(f"/// {shape} = {n} entries. From the grid via gen_lookup.py.")
    body = ", ".join(f"{v}.0" for v in flat)
    out.append(f"const LOOKUP: [f32; {n}] = [{body}];")
    return "\n".join(out)


def main():
    path = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("runs/grid/grid_results.csv")
    pts, knots = load(path)
    tbl, shape, miss = build_table(pts, knots)
    print(f"// {len(pts)} measured no-mask configs; lattice {shape} = "
          f"{1 if not shape else __import__('math').prod(shape)}; {miss} filled",
          file=sys.stderr)
    print(f"// knots: depth {knots[0]} qsearch {knots[1]} perception {knots[2]} "
          f"rank {knots[3]} eg {knots[4]}", file=sys.stderr)
    fill(tbl, shape)
    monotone_clamp(tbl, shape)
    print(emit_rust(knots, shape, tbl))


if __name__ == "__main__":
    main()
