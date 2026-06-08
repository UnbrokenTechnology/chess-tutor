"""Fit `config -> Elo` models from the grid, for the opponent-Elo slider.

Reads ``runs/grid/grid_results.csv`` (from run_grid.py) and fits THREE
models on the grid rows, cross-validated so we compare honest out-of-sample
error rather than overfit:

1. **HistGradientBoosting** — the forward model the solver inverts. Learns
   all interactions / caps / sign-flips automatically. Saved to
   ``runs/grid/forward_model.joblib`` + a tiny ``forward_meta.json`` so a
   later solver can load and invert it.
2. **Engineered-linear + LASSO** — hand-built candidate features
   (log depth, perception^2, mask*depth, perception*depth, ...) with
   LASSO auto-selecting; prints readable coefficients = the interpretable
   backbone. The mask sign-flip needs an interaction term (`mask*depth`),
   so it's in the candidate set.
3. **Symbolic regression** (gplearn) — an interpretable *equation*. OPTIONAL
   (skipped with a note if gplearn isn't installed).

Accuracy is capped by per-config Elo noise (~+/-50 at ~400 games), so
samples are weighted by 1/error^2 and we report CV RMSE against that floor.

Diagnostics written to ``runs/grid/plots/``: pred-vs-actual, the
**perception knee** (1-D partial dependence), and the **mask x depth
sign-flip** (the safety/positional mask Elo delta at each depth).

Run:  python fit.py                      # fit + diagnostics
      python fit.py --suggest 1400       # demo: invert for a target Elo
      python fit.py --csv path/to.csv
"""

from __future__ import annotations

import argparse
import itertools
import json
from pathlib import Path

import numpy as np
import pandas as pd
from sklearn.ensemble import HistGradientBoostingRegressor
from sklearn.linear_model import LassoCV
from sklearn.model_selection import KFold
from sklearn.pipeline import make_pipeline
from sklearn.preprocessing import StandardScaler

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402

import joblib  # noqa: E402

# qsearch "inf" (full vision) encoded as a large ordinal; the response
# saturates by ~q6 so the exact value past 2 barely matters to the trees.
QINF_CODE = 8
# endgame "F" (Full) encoded as one tier above eg2.
EGF_CODE = 3

# Raw model features fed to the GBT forward model (the solver's inputs).
FEATURES = [
    "depth",
    "qsearch",
    "perception",
    "avg_move_rank",
    "eg",
    "mask_safety",
    "mask_positional",
]


def load_grid(csv: Path) -> pd.DataFrame:
    df = pd.read_csv(csv)
    df = df[df["kind"] == "grid"].copy()
    df["qsearch"] = df["qsearch_depth"].map(
        lambda v: QINF_CODE if str(v).strip() == "inf" else int(float(v))
    )
    df["eg"] = df["endgame_skill"].map(
        lambda v: EGF_CODE if str(v).strip() == "F" else int(float(v))
    )
    for c in ("elo", "elo_error", "games", "perception", "avg_move_rank",
              "depth", "mask_safety", "mask_positional"):
        df[c] = pd.to_numeric(df[c], errors="coerce")
    df = df.dropna(subset=["elo"])
    df = df[df["games"] > 0]
    return df.reset_index(drop=True)


def sample_weights(df: pd.DataFrame) -> np.ndarray:
    """1/error^2 — precise configs (small Ordo +/-) count more."""
    err = df["elo_error"].fillna(df["elo_error"].median()).clip(lower=1.0)
    return (1.0 / err**2).to_numpy()


def _rmse(y, yhat, w=None) -> float:
    e2 = (np.asarray(y) - np.asarray(yhat)) ** 2
    if w is None:
        return float(np.sqrt(e2.mean()))
    w = np.asarray(w)
    return float(np.sqrt((w * e2).sum() / w.sum()))


def _cv_oof(make_model, X, y, w, weight_kw: str) -> np.ndarray:
    """Manual 5-fold out-of-fold predictions with sample weights — version-
    robust (avoids cross_val_predict's metadata-routing differences)."""
    oof = np.zeros_like(y, dtype=float)
    for tr, te in KFold(n_splits=5, shuffle=True, random_state=0).split(X):
        m = make_model()
        m.fit(X[tr], y[tr], **{weight_kw: w[tr]})
        oof[te] = m.predict(X[te])
    return oof


# --------------------------------------------------------------------------
# Model 1 — GBT forward model
# --------------------------------------------------------------------------
def fit_gbt(df: pd.DataFrame, w: np.ndarray):
    X = df[FEATURES].to_numpy(dtype=float)
    y = df["elo"].to_numpy(dtype=float)

    def make():
        return HistGradientBoostingRegressor(
            max_iter=500, learning_rate=0.05, max_leaf_nodes=31,
            l2_regularization=1.0, random_state=0,
        )

    oof = _cv_oof(make, X, y, w, "sample_weight")
    cv_rmse = _rmse(y, oof, w)
    model = make()
    model.fit(X, y, sample_weight=w)
    return model, cv_rmse, oof


# --------------------------------------------------------------------------
# Model 2 — engineered-linear + LASSO (interpretable)
# --------------------------------------------------------------------------
def engineered_features(df: pd.DataFrame) -> pd.DataFrame:
    d = df["depth"].astype(float)
    p = df["perception"].astype(float)
    r = df["avg_move_rank"].astype(float)
    q = df["qsearch"].astype(float)
    eg = df["eg"].astype(float)
    ms = df["mask_safety"].astype(float)
    mp = df["mask_positional"].astype(float)
    feats = {
        "depth": d,
        "log2_depth": np.log2(d),
        "min_depth6": np.minimum(d, 6),
        "perception": p,
        "perception_sq": p**2,
        "avg_move_rank": r,
        "q1": (q == 1).astype(float),
        "q2": (q == 2).astype(float),   # qinf = baseline
        "eg0": (eg == 0).astype(float),
        "eg1": (eg == 1).astype(float),
        "eg2": (eg == 2).astype(float),  # egF = baseline
        "mask_safety": ms,
        "mask_positional": mp,
        # interactions the structure demands:
        "perc_x_depth": p * d,            # perception knee scales with depth
        "perc_x_qsearch": p * q,          # perception x qsearch sub-additivity
        "perc_x_rank": p * r,
        "safety_x_depth": ms * d,         # the mask sign-flip
        "positional_x_depth": mp * d,     # the mask sign-flip
    }
    return pd.DataFrame(feats)


def fit_lasso(df: pd.DataFrame, w: np.ndarray):
    Xdf = engineered_features(df)
    X = Xdf.to_numpy(dtype=float)
    y = df["elo"].to_numpy(dtype=float)
    def make():
        return make_pipeline(
            StandardScaler(),
            LassoCV(cv=5, max_iter=20000, random_state=0),
        )

    oof = _cv_oof(make, X, y, w, "lassocv__sample_weight")
    cv_rmse = _rmse(y, oof, w)
    pipe = make()
    pipe.fit(X, y, lassocv__sample_weight=w)
    lasso = pipe.named_steps["lassocv"]
    scaler = pipe.named_steps["standardscaler"]
    # Coefficients are on standardized features; report per-std-unit effect.
    coefs = sorted(
        zip(Xdf.columns, lasso.coef_),
        key=lambda kv: -abs(kv[1]),
    )
    # De-standardize to RAW-unit coefficients: ŷ = b + Σ cⱼ·(xⱼ−μⱼ)/σⱼ
    # = [b − Σ cⱼμⱼ/σⱼ] + Σ (cⱼ/σⱼ)·xⱼ. Gives a readable closed form in
    # the actual dial units (a polynomial in perception/rank/depth/...).
    raw = lasso.coef_ / scaler.scale_
    raw_intercept = float(lasso.intercept_ - (lasso.coef_ * scaler.mean_ / scaler.scale_).sum())
    raw_terms = sorted(
        ((n, float(raw[i])) for i, n in enumerate(Xdf.columns) if abs(raw[i]) > 1e-9),
        key=lambda kv: -abs(kv[1]),
    )
    return pipe, cv_rmse, coefs, lasso.intercept_, (raw_intercept, raw_terms)


# --------------------------------------------------------------------------
# Model 3 — symbolic regression (optional)
# --------------------------------------------------------------------------
def fit_symbolic(df: pd.DataFrame, w: np.ndarray):
    try:
        from gplearn.genetic import SymbolicRegressor
    except Exception:
        return None, None
    X = df[FEATURES].to_numpy(dtype=float)
    y = df["elo"].to_numpy(dtype=float)
    sr = SymbolicRegressor(
        population_size=2000, generations=20,
        function_set=("add", "sub", "mul", "div", "log", "min", "max"),
        parsimony_coefficient=0.001, random_state=0, n_jobs=1,
        feature_names=FEATURES,  # equation prints depth/perception/... not X0..X6
    )
    sr.fit(X, y, sample_weight=w)
    return sr, str(sr._program)


# --------------------------------------------------------------------------
# Diagnostics
# --------------------------------------------------------------------------
def plots(df, model, oof_gbt, out_dir: Path):
    out_dir.mkdir(parents=True, exist_ok=True)
    y = df["elo"].to_numpy(dtype=float)

    # pred vs actual (out-of-fold)
    plt.figure(figsize=(5, 5))
    plt.scatter(y, oof_gbt, s=8, alpha=0.4)
    lo, hi = min(y.min(), oof_gbt.min()), max(y.max(), oof_gbt.max())
    plt.plot([lo, hi], [lo, hi], "r--", lw=1)
    plt.xlabel("measured Elo"); plt.ylabel("GBT out-of-fold prediction")
    plt.title("GBT pred vs actual (CV)")
    plt.tight_layout(); plt.savefig(out_dir / "pred_vs_actual.png", dpi=110)
    plt.close()

    # perception knee: predicted Elo vs perception at a few depths,
    # holding others at typical values (qinf, rank 1, egF, no mask).
    base = {"depth": 2, "qsearch": QINF_CODE, "perception": 1.0,
            "avg_move_rank": 1.0, "eg": EGF_CODE,
            "mask_safety": 0, "mask_positional": 0}
    ps = np.linspace(0, 1, 21)
    plt.figure(figsize=(6, 4))
    for dep in (1, 2, 4, 6):
        rows = []
        for pv in ps:
            c = dict(base, depth=dep, perception=pv)
            rows.append([c[f] for f in FEATURES])
        plt.plot(ps, model.predict(np.array(rows, float)), label=f"d{dep}")
    plt.xlabel("perception"); plt.ylabel("predicted Elo")
    plt.title("Perception knee (by depth)"); plt.legend()
    plt.tight_layout(); plt.savefig(out_dir / "perception_knee.png", dpi=110)
    plt.close()

    # mask x depth sign-flip: Elo delta of each mask vs none, per depth,
    # at a sighted base (qinf) and a blind-ish base (q1) to expose the flip.
    fig, axes = plt.subplots(1, 2, figsize=(10, 4), sharey=True)
    for ax, (qlab, qv) in zip(axes, [("qinf", QINF_CODE), ("q1", 1)]):
        depths = [1, 2, 4, 6]
        for mlab, ms, mp in [("safety", 1, 0), ("positional", 0, 1)]:
            deltas = []
            for dep in depths:
                base_c = {"depth": dep, "qsearch": qv, "perception": 1.0,
                          "avg_move_rank": 1.0, "eg": EGF_CODE,
                          "mask_safety": 0, "mask_positional": 0}
                masked = dict(base_c, mask_safety=ms, mask_positional=mp)
                d0 = model.predict(np.array([[base_c[f] for f in FEATURES]], float))[0]
                d1 = model.predict(np.array([[masked[f] for f in FEATURES]], float))[0]
                deltas.append(d1 - d0)
            ax.plot(depths, deltas, marker="o", label=mlab)
        ax.axhline(0, color="k", lw=0.6)
        ax.set_xlabel("depth"); ax.set_title(f"mask Elo delta ({qlab})")
        ax.legend()
    axes[0].set_ylabel("Elo delta vs no mask")
    plt.tight_layout(); plt.savefig(out_dir / "mask_x_depth.png", dpi=110)
    plt.close()


# --------------------------------------------------------------------------
# Starter inverter (the real constrained solver is a separate step)
# --------------------------------------------------------------------------
def suggest_config(model, target: float, fixed: dict | None = None) -> dict:
    """Nearest config (over a fine dial grid) whose predicted Elo matches
    `target`. A naive starting point — the production solver will add
    believability constraints (monotone schedule, eg/mask policy)."""
    fixed = fixed or {}
    space = {
        "depth": [1, 2, 4, 6],
        "qsearch": [1, 2, QINF_CODE],
        "perception": [round(x, 2) for x in np.arange(0, 1.0001, 0.05)],
        "avg_move_rank": [round(x, 1) for x in np.arange(1.0, 6.01, 0.1)],
        "eg": [0, 1, 2, EGF_CODE],
        "mask_safety": [0],
        "mask_positional": [0],
    }
    for k, v in fixed.items():
        space[k] = [v]
    keys = FEATURES
    combos = np.array(list(itertools.product(*(space[k] for k in keys))), dtype=float)
    preds = model.predict(combos)  # one batched predict over the whole space
    i = int(np.argmin(np.abs(preds - target)))
    return dict(zip(keys, combos[i].tolist())) | {"pred_elo": round(float(preds[i]), 1)}


def main() -> None:
    ap = argparse.ArgumentParser(description="Fit config->Elo models from the grid")
    ap.add_argument("--csv", default="runs/grid/grid_results.csv")
    ap.add_argument("--suggest", type=float, default=None,
                    help="demo: invert the GBT for this target Elo")
    args = ap.parse_args()

    csv = Path(args.csv)
    if not csv.exists():
        raise SystemExit(f"no grid CSV at {csv} — run run_grid.py first")
    df = load_grid(csv)
    w = sample_weights(df)
    print(f"loaded {len(df)} grid configs from {csv}")
    print(f"measured Elo range: {df['elo'].min():.0f} .. {df['elo'].max():.0f}")
    print(f"median per-config Ordo error: {df['elo_error'].median():.0f} "
          f"(this is the CV-RMSE floor)\n")

    # 1. GBT forward model
    gbt, gbt_rmse, oof = fit_gbt(df, w)
    print(f"[GBT]   5-fold CV weighted RMSE = {gbt_rmse:.1f} Elo")

    out_dir = csv.parent
    joblib.dump({"model": gbt, "features": FEATURES,
                 "qinf_code": QINF_CODE, "egf_code": EGF_CODE},
                out_dir / "forward_model.joblib")
    (out_dir / "forward_meta.json").write_text(json.dumps({
        "features": FEATURES, "qinf_code": QINF_CODE, "egf_code": EGF_CODE,
        "cv_rmse": round(gbt_rmse, 1), "n_configs": len(df),
        "elo_min": round(float(df["elo"].min())),
        "elo_max": round(float(df["elo"].max())),
    }, indent=2))
    print(f"        saved forward_model.joblib (+ meta) to {out_dir}")

    # 2. LASSO interpretable
    _, lasso_rmse, coefs, intercept, raw_eq = fit_lasso(df, w)
    print(f"\n[LASSO] 5-fold CV weighted RMSE = {lasso_rmse:.1f} Elo")
    print(f"        intercept {intercept:.0f}; selected terms "
          "(per-std-unit Elo effect):")
    for name, c in coefs:
        if abs(c) > 0.5:
            print(f"          {name:<20}{c:+8.1f}")
    raw_intercept, raw_terms = raw_eq
    print("\n        readable closed form (RAW dial units):")
    print(f"          Elo = {raw_intercept:+.0f}")
    for name, c in raw_terms:
        print(f"                {c:+9.3f} * {name}")

    # 3. symbolic (optional)
    sym, expr = fit_symbolic(df, w)
    if sym is None:
        print("\n[SYM]   gplearn not installed — skipped "
              "(pip install gplearn to enable the symbolic equation)")
    else:
        print(f"\n[SYM]   equation: {expr}")

    # diagnostics
    plots(df, gbt, oof, out_dir / "plots")
    print(f"\nwrote plots to {out_dir / 'plots'} "
          "(pred_vs_actual, perception_knee, mask_x_depth)")

    if args.suggest is not None:
        cfg = suggest_config(gbt, args.suggest)
        print(f"\n[suggest] config closest to Elo {args.suggest:.0f}: {cfg}")


if __name__ == "__main__":
    main()
