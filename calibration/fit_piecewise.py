"""Portable PIECEWISE-LINEAR forward model: config -> Elo, for solver.rs.

The LASSO closed form is non-monotone in perception (its p - p^2 parabola
peaks at ~0.84 then falls), which breaks the product's bidirectional
slider ("lower perception -> show lower Elo"). So instead we build an
**additive piecewise-linear** model with **enforced monotone slopes**:

  Elo(config) = BASE
              + f_perception(p)      # steep below the ~0.55 knee, gentle above
              + f_rank(rank)         # decreasing
              + f_depth(depth)       # increasing, saturating
              + f_qsearch(q)         # increasing, saturating
              + f_eg(eg)             # increasing, small
              + safety(depth)*mask_safety      # depth-fading penalty (kept interaction)
              + POSITIONAL*mask_positional     # ~0 (style)

Each f is sourced from a monotonicity-CONSTRAINED GBT's partial-dependence
curve, fit to linear segments at the grid knots, then clamped to a minimum
slope so no segment is ever perfectly flat (a user nudging any slider must
see Elo move the expected way — perception/depth/qsearch/eg up => stronger,
rank up => weaker, safety mask => weaker).

Emits the breakpoints as a ready-to-paste table (the Rust constants) and
validates additive-piecewise RMSE vs the grid + per-dial monotonicity.

Run:  python fit_piecewise.py
"""

from __future__ import annotations

import json
from pathlib import Path

import numpy as np
from sklearn.ensemble import HistGradientBoostingRegressor

import fit  # load_grid, sample_weights, FEATURES, QINF_CODE, EGF_CODE

# Knots (the grid sample points, + 0.8 for perception knee detail).
KNOTS = {
    "perception": [0.0, 0.2, 0.4, 0.6, 0.8, 1.0],
    "avg_move_rank": [1.0, 2.0, 3.5, 5.0],
    "depth": [1, 2, 4, 6],
    "qsearch": [1, 2, fit.QINF_CODE],
    "eg": [0, 1, 2, fit.EGF_CODE],
}
# Minimum |slope| per unit (the "never flat" fudge), in the monotone
# direction. Perception above the knee is gently positive, not flat.
MIN_SLOPE = {
    "perception": +120.0,   # ~1.2 Elo / 0.01 — gentle residual above the knee
    "avg_move_rank": -25.0,  # always weakens
    "depth": +6.0,           # always helps, even where it saturates
    "qsearch": +4.0,
    "eg": +4.0,
}
# Monotone direction per feature (+1 stronger as it rises, -1 weaker).
MONO = {"depth": +1, "qsearch": +1, "perception": +1,
        "avg_move_rank": -1, "eg": +1, "mask_safety": -1, "mask_positional": -1}


def fit_monotone_gbt(df, w):
    X = df[fit.FEATURES].to_numpy(float)
    y = df["elo"].to_numpy(float)
    cst = [MONO[f] for f in fit.FEATURES]
    m = HistGradientBoostingRegressor(
        max_iter=500, learning_rate=0.05, max_leaf_nodes=31,
        l2_regularization=1.0, random_state=0, monotonic_cst=cst,
    )
    m.fit(X, y, sample_weight=w)
    return m


def _ref_row():
    """A reference config (mid-ish) the partial-dependence curves vary one
    dial around; its prediction anchors BASE."""
    return {"depth": 2, "qsearch": fit.QINF_CODE, "perception": 0.6,
            "avg_move_rank": 2.0, "eg": fit.EGF_CODE,
            "mask_safety": 0, "mask_positional": 0}


def pd_curve(model, df, feat, knots):
    """Partial dependence: mean prediction over the dataset as `feat` is
    swept across `knots` (the honest marginal effect, averaging interactions)."""
    base = df[fit.FEATURES].to_numpy(float)
    fi = fit.FEATURES.index(feat)
    out = []
    for k in knots:
        X = base.copy()
        X[:, fi] = k
        out.append(float(model.predict(X).mean()))
    return out


def enforce_min_slope(knots, vals, feat):
    """Clamp consecutive segments to at least MIN_SLOPE in the monotone
    direction, so no segment is flat or wrong-signed."""
    s = MIN_SLOPE[feat]
    out = list(vals)
    for i in range(1, len(out)):
        dx = knots[i] - knots[i - 1]
        need = out[i - 1] + s * dx          # minimum allowed endpoint
        if s > 0:
            out[i] = max(out[i], need)
        else:
            out[i] = min(out[i], need)
    return out


def piecewise_eval(knots, vals, x):
    if x <= knots[0]:
        # extrapolate left with the first segment's slope
        m = (vals[1] - vals[0]) / (knots[1] - knots[0])
        return vals[0] + m * (x - knots[0])
    if x >= knots[-1]:
        m = (vals[-1] - vals[-2]) / (knots[-1] - knots[-2])
        return vals[-1] + m * (x - knots[-1])
    for i in range(1, len(knots)):
        if x <= knots[i]:
            frac = (x - knots[i - 1]) / (knots[i] - knots[i - 1])
            return vals[i - 1] + frac * (vals[i] - vals[i - 1])
    return vals[-1]


def safety_by_depth(model, df):
    """Depth-dependent safety-mask penalty: mean[pred(safety=1) - pred(safety=0)]
    over configs at each depth. The one interaction we keep."""
    base = df[fit.FEATURES].to_numpy(float)
    di, si = fit.FEATURES.index("depth"), fit.FEATURES.index("mask_safety")
    out = {}
    for d in KNOTS["depth"]:
        X = base.copy(); X[:, di] = d
        X0 = X.copy(); X0[:, si] = 0
        X1 = X.copy(); X1[:, si] = 1
        out[d] = float((model.predict(X1) - model.predict(X0)).mean())
    return out


PR_KNOTS = [0.0, 0.5, 1.0, 2.0, 3.0, 4.0, 6.0, 8.0]  # perception*rank product


def cross_curve(model, df, funcs):
    """f_pr(p*r): the perception x rank interaction as a curve over the
    product. = the 2D partial-dependence of (p,r) MINUS the two 1-D main
    effects (mean-zero deviation, so it adds cleanly on top of f_p/f_r),
    binned by p*r."""
    P, R = KNOTS["perception"], KNOTS["avg_move_rank"]
    base = df[fit.FEATURES].to_numpy(float)
    pi, ri = fit.FEATURES.index("perception"), fit.FEATURES.index("avg_move_rank")
    M = {}
    for p in P:
        for r in R:
            X = base.copy(); X[:, pi] = p; X[:, ri] = r
            M[(p, r)] = float(model.predict(X).mean())
    grand = sum(M.values()) / len(M)
    PDp = {p: sum(M[(p, r)] for r in R) / len(R) for p in P}
    PDr = {r: sum(M[(p, r)] for p in P) / len(P) for r in R}
    pts = [(p * r, M[(p, r)] - PDp[p] - PDr[r] + grand) for p in P for r in R]
    vals = []
    for i, k in enumerate(PR_KNOTS):
        lo = (PR_KNOTS[i - 1] + k) / 2 if i > 0 else -1
        hi = (PR_KNOTS[i + 1] + k) / 2 if i + 1 < len(PR_KNOTS) else 99
        near = [v for (x, v) in pts if lo < x <= hi]
        vals.append(sum(near) / len(near) if near else (vals[-1] if vals else 0.0))
    return PR_KNOTS, vals


def _sweep_row(p, r):
    return {"depth": 2, "qsearch": fit.QINF_CODE, "perception": p,
            "avg_move_rank": r, "eg": fit.EGF_CODE, "mask_safety": 0, "mask_positional": 0}


def is_monotone(model_fn) -> bool:
    """ELO up as perception rises (every rank), down as rank rises (every p)."""
    ps = [i / 20 for i in range(21)]
    rs = [1 + i * 0.25 for i in range(29)]
    for r in (1, 2, 4, 8):
        prev = None
        for p in ps:
            e = model_fn(_sweep_row(p, r))
            if prev is not None and e < prev - 1e-6:
                return False
            prev = e
    for p in (0.0, 0.4, 0.8, 1.0):
        prev = None
        for r in rs:
            e = model_fn(_sweep_row(p, r))
            if prev is not None and e > prev + 1e-6:
                return False
            prev = e
    return True


def main():
    df = fit.load_grid("runs/grid/grid_results.csv")
    w = fit.sample_weights(df)
    gbt = fit_monotone_gbt(df, w)

    ref = _ref_row()
    funcs = {}
    for feat, knots in KNOTS.items():
        raw = pd_curve(gbt, df, feat, knots)
        vals = enforce_min_slope(knots, raw, feat)
        ref_val = piecewise_eval(knots, vals, ref[feat])
        funcs[feat] = (knots, [v - ref_val for v in vals])

    safety = safety_by_depth(gbt, df)
    positional = float(
        (gbt.predict(_set(df, "mask_positional", 1)) -
         gbt.predict(_set(df, "mask_positional", 0))).mean())
    BASE = float(gbt.predict(np.array([[ref[f] for f in fit.FEATURES]], float))[0])
    pr_knots, pr_raw = cross_curve(gbt, df, funcs)

    def make_model(cross_scale):
        def m(row):
            e = BASE
            for feat, (k, v) in funcs.items():
                e += piecewise_eval(k, v, row[feat])
            e += cross_scale * piecewise_eval(pr_knots, pr_raw,
                                              row["perception"] * row["avg_move_rank"])
            e += safety[_nearest(row["depth"], KNOTS["depth"])] * row["mask_safety"]
            e += positional * row["mask_positional"]
            return e
        return m

    # Largest cross-term scale in [0,1] that keeps the model monotone in
    # perception (up) and rank (down) — the "fudge to preserve direction".
    scale = 1.0
    if not is_monotone(make_model(1.0)):
        lo, hi = 0.0, 1.0
        for _ in range(24):
            mid = (lo + hi) / 2
            if is_monotone(make_model(mid)):
                lo = mid
            else:
                hi = mid
        scale = round(lo, 3)
    model = make_model(scale)
    pr_vals = [round(scale * v, 1) for v in pr_raw]

    rows = df[fit.FEATURES].to_dict("records")
    y = df["elo"].to_numpy(float)
    rmse = float(np.sqrt((w * (np.array([model(r) for r in rows]) - y) ** 2).sum() / w.sum()))
    rmse_noX = float(np.sqrt((w * (np.array([make_model(0.0)(r) for r in rows]) - y) ** 2).sum() / w.sum()))

    print(f"piecewise RMSE vs grid: additive {rmse_noX:.0f} -> +cross {rmse:.0f} "
          f"(LASSO 118 / GBT 147 / floor ~130)")
    print(f"cross-term scale kept for monotonicity: {scale:.2f}; "
          f"model monotone={is_monotone(model)}\n")
    print(f"BASE = {BASE:.0f}")
    for feat, (k, v) in funcs.items():
        pts = ", ".join(f"({kk:g}:{vv:+.0f})" for kk, vv in zip(k, v))
        print(f"f_{feat:<13} {pts}")
    print(f"f_perc_x_rank  " + ", ".join(f"({k:g}:{v:+.0f})" for k, v in zip(pr_knots, pr_vals)))
    print(f"safety(depth)  " + ", ".join(f"(d{d}:{e:+.0f})" for d, e in safety.items()))
    print(f"positional     {positional:+.0f}")

    # Iterative inverse demo: solve avg_move_rank for a target, given the
    # other dials (Newton-ish bisection; monotone in rank -> converges).
    def solve_rank(target, depth, qsearch, p, eg, ms=0, mp=0):
        lo, hi = 1.0, 8.0
        f = lambda r: model({"depth": depth, "qsearch": qsearch, "perception": p,
                             "avg_move_rank": r, "eg": eg, "mask_safety": ms, "mask_positional": mp})
        if f(lo) <= target: return 1.0
        if f(hi) >= target: return 8.0
        for _ in range(30):
            m = (lo + hi) / 2
            lo, hi = (m, hi) if f(m) > target else (lo, m)
        return round((lo + hi) / 2, 1)
    print("\ninverse demo (solve rank for target, d2 qinf egF, perception=ramp):")
    for t in (900, 1200, 1500):
        p = min(1.0, max(0.0, (t - 300) / 900))
        r = solve_rank(t, 2, fit.QINF_CODE, p, fit.EGF_CODE)
        back = model({"depth": 2, "qsearch": fit.QINF_CODE, "perception": p,
                      "avg_move_rank": r, "eg": fit.EGF_CODE, "mask_safety": 0, "mask_positional": 0})
        print(f"  target {t} (p={p:.2f}) -> rank {r} -> model says {back:.0f}")

    out = {"base": BASE, "funcs": {f: {"knots": k, "vals": v} for f, (k, v) in funcs.items()},
           "perc_x_rank": {"knots": pr_knots, "vals": pr_vals, "scale": scale},
           "safety_by_depth": safety, "positional": positional, "rmse": rmse}
    Path("runs/grid/piecewise_model.json").write_text(json.dumps(out, indent=2))
    print("\nwrote runs/grid/piecewise_model.json (the Rust constants)")


def _set(df, feat, val):
    X = df[fit.FEATURES].to_numpy(float)
    X[:, fit.FEATURES.index(feat)] = val
    return X


def _nearest(x, opts):
    return min(opts, key=lambda o: abs(o - x))


if __name__ == "__main__":
    main()
