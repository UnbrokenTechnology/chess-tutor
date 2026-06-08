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
//!   piecewise-linear forward model, **ladder-anchored**: at the slider's
//!   default it returns the target *exactly*, and each advanced tweak moves
//!   it by the model's delta. So you get the tight ladder default AND
//!   sensible deltas. The model is monotone in every dial (perception/depth/
//!   qsearch/eg ↑ ⇒ stronger; avg_move_rank ↑ and a mask ⇒ weaker) so a
//!   slider never moves Elo the "wrong" way.
//!
//! [`solve_rank`] inverts the forward model for avg_move_rank (1-D
//! bisection; Elo is monotone-decreasing in rank) for a future "pin a dial,
//! hold the Elo" control.
//!
//! Constants are baked from the offline calibration pipeline
//! (`calibration/run_ladder.py` rungs + `calibration/fit_piecewise.py`
//! coefficients). They reproduce the grid's structure: perception
//! (saturating) + avg_move_rank are the dominant levers, depth/qsearch
//! secondary, king-safety a depth-fading handicap, endgame small. See that
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

/// Lowest / highest reproducible target.
pub const ELO_MIN: f64 = 500.0;
pub const ELO_MAX: f64 = 2500.0;

// ---- piecewise forward model (calibration/fit_piecewise.py) -------------
// Each table is (knot_x, Elo_contribution); centered so the reference config
// contributes 0 and BASE carries the level. Linear between knots, end
// segments extrapolated. perc_x_rank vals are already monotonicity-scaled.
const BASE: f64 = 1308.75;
const F_PERCEPTION: &[(f32, f64)] =
    &[(0.0, -401.4), (0.2, -292.0), (0.4, -85.2), (0.6, 0.0), (0.8, 24.0), (1.0, 48.0)];
const F_RANK: &[(f32, f64)] = &[(1.0, 358.0), (2.0, 0.0), (3.5, -368.9), (5.0, -450.9)];
const F_DEPTH: &[(f32, f64)] = &[(1.0, -21.1), (2.0, 0.0), (4.0, 34.5), (6.0, 88.2)];
// qsearch encoded with full-vision = 8 (the curve saturates past q2).
const F_QSEARCH: &[(f32, f64)] = &[(1.0, -28.0), (2.0, -24.0), (8.0, 0.0)];
// endgame encoded with Full = 3.
const F_EG: &[(f32, f64)] = &[(0.0, -12.0), (1.0, -8.0), (2.0, -4.0), (3.0, 0.0)];
const F_PERC_X_RANK: &[(f32, f64)] = &[
    (0.0, -3.9), (0.5, 12.6), (1.0, 23.4), (2.0, 2.1),
    (3.0, -27.8), (4.0, -48.1), (6.0, -48.1), (8.0, -48.1),
];
const F_SAFETY_BY_DEPTH: &[(f32, f64)] =
    &[(1.0, -27.5), (2.0, -27.5), (4.0, -24.7), (6.0, -1.0)];

const QINF_CODE: f32 = 8.0;
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

/// Raw forward-model Elo for a config — the piecewise sum. ~176 Elo RMSE
/// absolute, so it's used only for *deltas* (see [`elo_for_dials`]), never
/// as the headline number. Monotone in every dial by construction.
fn model_elo(d: &BotDials) -> f64 {
    BASE + piecewise(F_PERCEPTION, d.perception)
        + piecewise(F_RANK, d.avg_move_rank)
        + piecewise(F_DEPTH, d.depth as f32)
        + piecewise(F_QSEARCH, qsearch_code(d.qsearch))
        + piecewise(F_EG, eg_code(d.endgame_skill))
        + piecewise(F_PERC_X_RANK, d.perception * d.avg_move_rank)
        + if d.mask_safety { piecewise(F_SAFETY_BY_DEPTH, d.depth as f32) } else { 0.0 }
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

/// Default dials for a target Elo: interpolate the locked ladder. Discrete
/// dials come from the band the target sits in; perception from the closed
/// form; avg_move_rank interpolated between the bracketing rungs.
pub fn config_for_elo(elo: f64) -> BotDials {
    let elo = elo.clamp(ELO_MIN, ELO_MAX);
    let mut lo = &LADDER[0];
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

/// Standalone absolute Elo estimate for a config (the raw forward model,
/// clamped to the ladder range). Use to seed the slider's target when a
/// dialog opens onto an existing (possibly off-ladder) config; the
/// ladder-anchored [`elo_for_dials`] is the precise display thereafter.
pub fn estimate_elo(dials: &BotDials) -> f64 {
    model_elo(dials).clamp(ELO_MIN, ELO_MAX)
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
