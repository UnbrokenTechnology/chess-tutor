//! Opponent-strength solver: target Elo ⇄ bot dials, for the "opponent
//! Elo" GUI slider (+ advanced dropdown), bidirectional and live.
//!
//! Two directions, two models — chosen so the *default* is as accurate as
//! we can be while the *advanced* tweaks stay believable and monotone:
//!
//! - **Elo → dials** ([`config_for_elo`]): interpolate the LOCKED LADDER —
//!   the feel-validated 1-D schedule (RMSE ~46 vs target). Perception is a
//!   closed-form ramp; depth / qsearch / endgame step by band; avg_move_rank
//!   interpolates the ladder's tuned curve. This is what the slider returns.
//! - **dials → Elo** ([`elo_for_dials`]): the advanced-tab live readout. A
//!   **5-D multilinear interpolation over the measured grid** itself (the
//!   baked [`LOOKUP`]), **ladder-anchored**: at the slider's default it
//!   returns the target *exactly*, and each advanced tweak moves it by the
//!   model's delta. So you get the tight ladder default AND sensible deltas.
//!   The model is monotone in every dial (perception/depth/qsearch/eg ↑ ⇒
//!   stronger; avg_move_rank ↑ and a mask ⇒ weaker) so a slider never moves
//!   Elo the "wrong" way.
//!
//! [`solve_rank`] inverts the forward model for avg_move_rank (1-D
//! bisection; Elo is monotone-decreasing in rank) for a future "pin a dial,
//! hold the Elo" control.
//!
//! The ladder rungs come from `calibration/run_ladder.py`; the forward
//! model's [`LOOKUP`] is the measured grid baked by `calibration/gen_lookup.py`
//! (≈2600 configs vs the Maia ladder). The lookup is preferred over any fitted
//! equation because every parametric form compressed the 3+-dial interactions
//! (perception gates depth AND qsearch; perception×rank compounds) — the grid
//! *is* the data. King-safety is the one term not in the grid (masks were
//! pulled out) and rides as an additive depth-fading handicap. See that
//! directory for provenance.

/// The dials the slider produces and the advanced tab edits. Maps onto the
/// play config: `depth` → `SearchParams.max_depth`, `qsearch` →
/// `OpponentProfile::qsearch_max_plies`, `perception` →
/// `OpponentProfile::perception`, `avg_move_rank` → `NoiseProfile`,
/// `endgame_skill` → `OpponentProfile::endgame_skill`, the masks →
/// `OpponentProfile::eval_mask`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BotDials {
    pub depth: u32,
    /// Quiescence cap; `None` = full tactical vision.
    pub qsearch: Option<u32>,
    pub perception: f32,
    pub avg_move_rank: f32,
    /// Endgame-book tier 0/1/2; `None` = Full.
    pub endgame_skill: Option<u32>,
    pub mask_safety: bool,
    pub mask_positional: bool,
}

// ---- the locked ladder (calibration/run_ladder.py RUNGS) ----------------
struct Rung {
    elo: f64,
    depth: u32,
    qsearch: Option<u32>,
    rank: f32,
    eg: Option<u32>,
}

const LADDER: &[Rung] = &[
    Rung { elo: 500.0,  depth: 1, qsearch: Some(1), rank: 2.8, eg: Some(1) },
    Rung { elo: 600.0,  depth: 1, qsearch: Some(1), rank: 3.1, eg: Some(1) },
    Rung { elo: 700.0,  depth: 1, qsearch: Some(1), rank: 3.2, eg: Some(1) },
    Rung { elo: 800.0,  depth: 1, qsearch: Some(2), rank: 3.0, eg: Some(1) },
    Rung { elo: 900.0,  depth: 1, qsearch: Some(2), rank: 2.8, eg: Some(1) },
    Rung { elo: 1000.0, depth: 1, qsearch: Some(2), rank: 2.7, eg: Some(2) },
    Rung { elo: 1100.0, depth: 1, qsearch: Some(2), rank: 2.4, eg: Some(2) },
    Rung { elo: 1200.0, depth: 1, qsearch: Some(2), rank: 2.1, eg: Some(2) },
    Rung { elo: 1300.0, depth: 2, qsearch: Some(2), rank: 2.1, eg: Some(2) },
    Rung { elo: 1400.0, depth: 2, qsearch: Some(2), rank: 1.9, eg: Some(2) },
    Rung { elo: 1500.0, depth: 2, qsearch: Some(2), rank: 1.7, eg: Some(2) },
    Rung { elo: 1600.0, depth: 2, qsearch: None,    rank: 1.6, eg: Some(2) },
    Rung { elo: 1700.0, depth: 2, qsearch: None,    rank: 1.4, eg: Some(2) },
    Rung { elo: 1800.0, depth: 2, qsearch: None,    rank: 1.3, eg: Some(2) },
    Rung { elo: 1900.0, depth: 4, qsearch: None,    rank: 1.4, eg: Some(2) },
    Rung { elo: 2000.0, depth: 4, qsearch: None,    rank: 1.0, eg: None },
    Rung { elo: 2100.0, depth: 5, qsearch: None,    rank: 1.2, eg: None },
    Rung { elo: 2200.0, depth: 5, qsearch: None,    rank: 1.0, eg: None },
    Rung { elo: 2300.0, depth: 6, qsearch: None,    rank: 1.3, eg: None },
    Rung { elo: 2400.0, depth: 6, qsearch: None,    rank: 1.0, eg: None },
    Rung { elo: 2500.0, depth: 7, qsearch: None,    rank: 1.0, eg: None },
];

/// Slider bounds. The feel-validated ladder bottoms at 500; below that
/// [`config_for_elo`] holds the floor band and solves avg_move_rank against
/// the lookup down to ~0 (best-effort, off-ladder) so the slider can still
/// produce ~100-Elo bots. Negative targets clamp to 0.
pub const ELO_MIN: f64 = 0.0;
pub const ELO_MAX: f64 = 2500.0;

// ---- forward model: 5-D interpolation over the measured grid ------------
// `model_elo` interpolates the baked lookup (DEPTH/QSEARCH/PERCEPTION/RANK/EG
// knots + a flat row-major LOOKUP), generated by calibration/gen_lookup.py
// from the grid CSV. Monotone-clamped per axis (so sliders never move Elo the
// wrong way); the grid brackets the full GUI range so we never extrapolate.
include!("calibration_lookup.rs");

// King-safety mask is NOT in the grid (masks were pulled out of the
// Cartesian); it rides as an additive depth-fading handicap (~ -27 Elo at d1
// fading to ~ -1 at d6, from runs/grid_3840_masks/). Positional mask ≈ 0 Elo.
const F_SAFETY_BY_DEPTH: &[(f32, f64)] =
    &[(1.0, -27.5), (2.0, -27.5), (4.0, -24.7), (6.0, -1.0)];

// Full-vision qsearch / Full endgame encoded on the interp axes to match
// gen_lookup.py (QSEARCH_KNOTS ends at 10.0; EG_KNOTS ends at 3.0).
const QINF_CODE: f32 = 10.0;
const EGF_CODE: f32 = 3.0;

/// Linear interpolation over a knot table; end segments extrapolate so the
/// model is defined past the sampled range (e.g. rank > 5, depth > 6).
fn piecewise(table: &[(f32, f64)], x: f32) -> f64 {
    let interp = |x0: f32, y0: f64, x1: f32, y1: f64, at: f32| -> f64 {
        y0 + (y1 - y0) / (x1 - x0) as f64 * (at - x0) as f64
    };
    // First segment whose right knot is >= x handles it; x below the first
    // knot falls into segment 0 (extrapolating left).
    for pair in table.windows(2) {
        let (x0, y0) = pair[0];
        let (x1, y1) = pair[1];
        if x <= x1 {
            return interp(x0, y0, x1, y1, x);
        }
    }
    // Beyond the last knot: extrapolate the final segment.
    let n = table.len();
    let (x0, y0) = table[n - 2];
    let (x1, y1) = table[n - 1];
    interp(x0, y0, x1, y1, x)
}

fn qsearch_code(q: Option<u32>) -> f32 {
    q.map_or(QINF_CODE, |v| v as f32)
}
fn eg_code(eg: Option<u32>) -> f32 {
    eg.map_or(EGF_CODE, |v| v as f32)
}

/// Bracket `x` in a sorted knot table: returns `(lo, hi, t)` such that the
/// interpolant is `knots[lo] * (1 - t) + knots[hi] * t`. Clamps at both ends
/// (`t = 0`, `lo == hi`) — the grid brackets the GUI range, so we never
/// extrapolate.
fn axis_bracket(knots: &[f32], x: f32) -> (usize, usize, f64) {
    let n = knots.len();
    if x <= knots[0] {
        return (0, 0, 0.0);
    }
    if x >= knots[n - 1] {
        return (n - 1, n - 1, 0.0);
    }
    for (i, w) in knots.windows(2).enumerate() {
        if x <= w[1] {
            let t = (x - w[0]) as f64 / (w[1] - w[0]) as f64;
            return (i, i + 1, t);
        }
    }
    (n - 1, n - 1, 0.0)
}

/// 5-D multilinear interpolation over [`LOOKUP`] (row-major in
/// depth, qsearch, perception, rank, eg). Sums the 2^5 bracketing corners,
/// each weighted by the product of its per-axis fractions.
fn interp5(depth: f32, qsearch: f32, perception: f32, rank: f32, eg: f32) -> f64 {
    let (d0, d1, dt) = axis_bracket(DEPTH_KNOTS, depth);
    let (q0, q1, qt) = axis_bracket(QSEARCH_KNOTS, qsearch);
    let (p0, p1, pt) = axis_bracket(PERCEPTION_KNOTS, perception);
    let (r0, r1, rt) = axis_bracket(RANK_KNOTS, rank);
    let (e0, e1, et) = axis_bracket(EG_KNOTS, eg);

    let (nq, np, nr, ne) = (
        QSEARCH_KNOTS.len(),
        PERCEPTION_KNOTS.len(),
        RANK_KNOTS.len(),
        EG_KNOTS.len(),
    );
    let at = |di: usize, qi: usize, pi: usize, ri: usize, ei: usize| -> f64 {
        LOOKUP[(((di * nq + qi) * np + pi) * nr + ri) * ne + ei] as f64
    };

    let mut acc = 0.0;
    for (di, wd) in [(d0, 1.0 - dt), (d1, dt)] {
        for (qi, wq) in [(q0, 1.0 - qt), (q1, qt)] {
            for (pi, wp) in [(p0, 1.0 - pt), (p1, pt)] {
                for (ri, wr) in [(r0, 1.0 - rt), (r1, rt)] {
                    for (ei, we) in [(e0, 1.0 - et), (e1, et)] {
                        acc += wd * wq * wp * wr * we * at(di, qi, pi, ri, ei);
                    }
                }
            }
        }
    }
    acc
}

/// Forward-model Elo for a config: 5-D interpolation over the measured grid
/// plus the king-safety mask's additive depth handicap. Monotone in every
/// dial (the table is monotone-clamped; the mask only subtracts). Absolute,
/// so used for *deltas* in [`elo_for_dials`] and the seed in [`estimate_elo`].
fn model_elo(d: &BotDials) -> f64 {
    let base = interp5(
        d.depth as f32,
        qsearch_code(d.qsearch),
        d.perception,
        d.avg_move_rank,
        eg_code(d.endgame_skill),
    );
    let mask = if d.mask_safety {
        piecewise(F_SAFETY_BY_DEPTH, d.depth as f32)
    } else {
        0.0
    };
    base + mask
    // positional mask ≈ 0 Elo (style, not strength) — omitted.
}

fn round_to(x: f32, step: f32) -> f32 {
    (x / step).round() * step
}

/// The perception ramp, a closed form of the target Elo (the ladder's
/// faster ramp `clamp((elo-300)/900, 0, 1)`), on the 0.05 product grid.
pub fn perception_for(elo: f64) -> f32 {
    let p = (((elo - 300.0) / 900.0) as f32).clamp(0.0, 1.0);
    round_to(p, 0.05)
}

/// Solve avg_move_rank in `[1, 8]` (on the 0.1 grid) so the *absolute*
/// forward-model Elo matches `target` for otherwise-fixed `dials`. `model_elo`
/// is monotone-decreasing in rank, so plain bisection converges. Used by the
/// sub-ladder basement (below t500), where there's no rung to interpolate —
/// distinct from [`solve_rank`], which anchors via [`elo_for_dials`] (and
/// would recurse here). Returns 8 if the target is below the config's reach.
fn rank_for_model_elo(target: f64, dials: &BotDials) -> f32 {
    let eval = |r: f32| {
        let mut d = *dials;
        d.avg_move_rank = r;
        model_elo(&d)
    };
    let (mut lo, mut hi) = (1.0f32, 8.0f32);
    if eval(lo) <= target {
        return 1.0;
    }
    if eval(hi) >= target {
        return 8.0;
    }
    for _ in 0..30 {
        let mid = (lo + hi) / 2.0;
        if eval(mid) > target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    round_to((lo + hi) / 2.0, 0.1)
}

/// Default dials for a target Elo: interpolate the locked ladder. Discrete
/// dials come from the band the target sits in; perception from the closed
/// form; avg_move_rank interpolated between the bracketing rungs. Below the
/// ladder's floor (t500) it holds the floor band and solves rank for the
/// target against the lookup (off-ladder, best-effort, down to ~0 Elo).
pub fn config_for_elo(elo: f64) -> BotDials {
    let elo = elo.clamp(ELO_MIN, ELO_MAX);
    let floor = &LADDER[0];

    // Below the feel-validated ladder: hold the floor band's discrete dials
    // (depth/qsearch/eg), take perception from the closed-form ramp, and
    // solve avg_move_rank against the lookup so the bot weakens monotonically
    // toward ~0 Elo. The grid brackets this region (rank to 8, perception 0).
    if elo < floor.elo {
        let mut d = BotDials {
            depth: floor.depth,
            qsearch: floor.qsearch,
            perception: perception_for(elo),
            avg_move_rank: 1.0,
            endgame_skill: floor.eg,
            mask_safety: false,
            mask_positional: false,
        };
        // Anchor to the ladder floor so strength is continuous across the
        // t500 boundary and on the ladder's real-Elo scale: the lookup's
        // absolute scale over-reads the floor (the t500 config models ~750),
        // so solve the rank whose model Elo equals target + that floor offset.
        let floor_offset = model_elo(&config_for_elo(floor.elo)) - floor.elo;
        d.avg_move_rank = rank_for_model_elo(elo + floor_offset, &d);
        return d;
    }

    let mut lo = floor;
    let mut hi = &LADDER[LADDER.len() - 1];
    for r in LADDER {
        if r.elo <= elo {
            lo = r;
        }
    }
    for r in LADDER.iter().rev() {
        if r.elo >= elo {
            hi = r;
        }
    }
    let rank = if (hi.elo - lo.elo).abs() < 1e-9 {
        lo.rank
    } else {
        let f = ((elo - lo.elo) / (hi.elo - lo.elo)) as f32;
        lo.rank + f * (hi.rank - lo.rank)
    };
    BotDials {
        depth: lo.depth,
        qsearch: lo.qsearch,
        perception: perception_for(elo),
        avg_move_rank: round_to(rank, 0.1),
        endgame_skill: lo.eg,
        mask_safety: false,
        mask_positional: false,
    }
}

/// The Elo to *display* for `dials`, given the slider's `target` (the
/// anchor). Ladder-anchored: at the unmodified default it returns `target`
/// exactly; each advanced tweak moves it by the forward model's delta.
pub fn elo_for_dials(dials: &BotDials, target: f64) -> f64 {
    let default = config_for_elo(target);
    target + (model_elo(dials) - model_elo(&default))
}

/// Slider target to *seed* when a dialog opens onto an existing config (the
/// ladder-anchored [`elo_for_dials`] is the precise display thereafter).
///
/// Inverts [`config_for_elo`]: finds the target `T` whose default config has
/// the same model Elo as `dials`. This keeps the seed on the slider's (ladder)
/// scale — the raw lookup is *absolute* and over-reads the floor (a 500-Elo
/// config models ~750), so seeding from it directly would jump the slider.
/// `model_elo(config_for_elo(T))` rises with `T`, so bisect.
pub fn estimate_elo(dials: &BotDials) -> f64 {
    let target = model_elo(dials);
    let (mut lo, mut hi) = (ELO_MIN, ELO_MAX);
    if model_elo(&config_for_elo(lo)) >= target {
        return lo;
    }
    if model_elo(&config_for_elo(hi)) <= target {
        return hi;
    }
    for _ in 0..40 {
        let mid = (lo + hi) / 2.0;
        if model_elo(&config_for_elo(mid)) < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) / 2.0
}

/// avg_move_rank in `[1, 8]` that makes [`elo_for_dials`] return `target`
/// for the otherwise-fixed `dials`. Elo is monotone-decreasing in rank, so
/// plain bisection converges. For a "pin a dial, keep the Elo" control.
pub fn solve_rank(target: f64, dials: &BotDials) -> f32 {
    let eval = |r: f32| {
        let mut d = *dials;
        d.avg_move_rank = r;
        elo_for_dials(&d, target)
    };
    let (mut lo, mut hi) = (1.0f32, 8.0f32);
    if eval(lo) <= target {
        return 1.0;
    }
    if eval(hi) >= target {
        return 8.0;
    }
    for _ in 0..30 {
        let mid = (lo + hi) / 2.0;
        if eval(mid) > target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    round_to((lo + hi) / 2.0, 0.1)
}

#[cfg(test)]
#[path = "calibration_tests.rs"]
mod tests;
