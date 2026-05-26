//! Move-noise sampling: turns a ranked list of `SearchLine`s (plus the
//! full legal-move list) into the move the bot actually plays.
//!
//! The play loop runs the search with [`NoiseProfile::effective_multi_pv`]
//! slots, then calls [`pick`] to decide what becomes the move. The
//! sampler has three independent branches, evaluated in this order:
//!
//! 1. **Blunder branch** (when [`NoiseProfile::blunder_chance`] > 0):
//!    drop a deliberately worse engine-considered line. The picker
//!    prefers lines whose score loss vs #1 falls inside the
//!    `[blunder_min_loss_cp, blunder_max_loss_cp]` band — uniform
//!    pick from those when at least one qualifies.
//!
//!    When no line falls in the band (every alternative is either
//!    "not blundery enough" or "too catastrophic"), the picker
//!    pools the line(s) with the largest loss strictly below the
//!    band's lower edge with the line(s) with the smallest loss
//!    strictly above the band's upper edge, and picks uniformly
//!    from the combined pool. **Lines further from the band on
//!    either side are excluded** — that's the load-bearing
//!    property that lets a bot do "small blunders only" without
//!    throwing away a queen when the only sub-band alternative is
//!    a piece sacrifice. See [`blunder_pool`].
//!
//!    Mate-guarded by [`NoiseProfile::guaranteed_mate_in`].
//!
//! 2. **Wild branch** (when [`NoiseProfile::wild_chance`] > 0): with
//!    that per-move probability, pick uniformly from **all legal
//!    moves**, ignoring the search ranking entirely. This is the
//!    beginner-bot path — the only branch that can pick a move the
//!    engine didn't even surface (e.g. leaving a piece in a pawn's
//!    path). Same mate-guard.
//!
//! 3. **Softmax branch** (when [`NoiseProfile::candidate_pool`] > 1 and
//!    [`NoiseProfile::temperature_cp`] > 0): pick from the top-K with
//!    weights `exp((score_i - score_top) / temperature_cp)`. The score
//!    delta is non-positive, so the top line is always the peak; higher
//!    temperatures flatten the distribution.
//!
//! When no branch fires, the picker returns [`NoisePick::Line(0)`] —
//! the engine's best move.
//!
//! **Branch ordering rationale:** blunder is the calibrated mistake
//! knob (always produces a worse-than-best move when it fires); wild
//! is the chaotic knob (might coincidentally pick the best move).
//! Putting blunder first means its configured rate is the committed
//! signal — wild fills whatever budget remains, rather than wild
//! pre-empting blunder when both knobs are set.
//!
//! Strict invariant: only the **play** engine consults this module.
//! Analytical paths (retrospective, hint, `analyze`) ignore the noise
//! profile and always play `lines[0]`. See [`crate::opponent`] for the
//! matching invariant on opening books and eval masking.
//!
//! Determinism: [`pick`] is a pure function of `(profile, seed, ply,
//! lines, legal_moves)`. The play loop derives the per-move seed by
//! mixing the game's
//! [`OpponentProfile::seed`](crate::opponent::OpponentProfile::seed)
//! with the current ply count, so replaying a game with the same seed
//! gives the same noise picks.

use crate::engine::SearchLine;
use crate::opponent::NoiseProfile;
use crate::types::{Move, Value};

/// Outcome of [`pick`]. The branch that fired is encoded in the
/// variant so the caller can render it accurately in diagnostic
/// output ("blunder #6 of 6" vs "softmax #3 of 6" vs "wild — engine
/// preferred X"). The move itself is either `lines[idx].pv[0]`
/// (line-based variants) or the wild legal move directly.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NoisePick {
    /// Engine-best or softmax pick: take `lines[idx].pv[0]`.
    /// `idx == 0` is the off-noise / no-branch-fired path; `idx > 0`
    /// means the softmax branch sampled this slot.
    Line(usize),
    /// Blunder branch fired: take `lines[idx].pv[0]`. `idx` is always
    /// `>= 1` (blunder never picks #1) — either an in-band line or
    /// one in the closest-on-each-side fallback pool.
    Blunder(usize),
    /// Blunder roll fired but the available alternatives were all
    /// catastrophically worse than the configured band — see
    /// [`BLUNDER_FALLBACK_TOLERANCE`]. The caller should play
    /// `lines[0].pv[0]` (best) and SHOULD log this so the user knows
    /// the configured rate is being slightly under-delivered.
    /// `closest_above_loss_cp` is the smallest loss that was rejected;
    /// caller composes the log around it.
    BlunderSkipped { closest_above_loss_cp: i32 },
    /// Wild branch fired: play this legal move directly, bypassing
    /// the engine ranking entirely.
    Wild(Move),
}

/// Above-band fallback tolerance multiplier. The closest-loss line
/// above `blunder_max_loss_cp` is admitted to the fallback pool only
/// when its loss is at most `max_loss × this`. Beyond that, the
/// position is deemed "no calibrated blunder available" and the
/// blunder roll is skipped (caller plays best). 2.0× means a bot
/// configured for [50, 100] cp blunders will accept up to 200 cp of
/// fallback slack; a bot configured for [100, 400] cp blunders will
/// accept up to 800 cp. The point of the cap is to prevent the bot
/// from gifting catastrophic blunders (e.g. hanging a queen for no
/// reason because the only non-#1 line happened to lose 2000 cp) in
/// positions where the engine's best is much stronger than every
/// alternative.
pub const BLUNDER_FALLBACK_TOLERANCE: f32 = 2.0;

/// Decide what move the bot actually plays. See module docs for the
/// branch order and semantics.
///
/// `lines` is the engine's ranked result (best first). `legal_moves`
/// is the full legal-move list for the current position; only consumed
/// by the wild branch. Either may be empty; the picker degrades to
/// [`NoisePick::Line(0)`] when it has nothing to choose from.
pub fn pick(
    noise: &NoiseProfile,
    seed: u64,
    ply: u64,
    lines: &[SearchLine],
    legal_moves: &[Move],
) -> NoisePick {
    if noise.is_off() {
        return NoisePick::Line(0);
    }

    let top_score = lines.first().map(|l| l.score).unwrap_or(Value::ZERO);
    let mate_guard = !lines.is_empty() && mate_guarded(top_score, noise.guaranteed_mate_in);

    let mut rng = mix(seed, ply);

    // Blunder branch: pick from engine-considered lines whose loss
    // vs #1 falls in the configured band. Skipped when there's
    // nothing to choose from (single line) or the bot has a forced
    // mate the user asked us to convert.
    if noise.blunder_chance > 0.0 && !mate_guard && lines.len() > 1 {
        let (roll, next) = roll_unit(rng);
        rng = next;
        if roll < noise.blunder_chance as f64 {
            let pool = blunder_pool(
                lines,
                top_score,
                noise.blunder_min_loss_cp,
                noise.blunder_max_loss_cp,
                BLUNDER_FALLBACK_TOLERANCE,
            );
            if pool.indices.is_empty() {
                // Pool empty after the tolerance cap. Two sub-cases:
                // - excluded_above_loss = Some: there *was* an above-
                //   tier candidate but it was rejected (catastrophic).
                //   Tell the caller so it can log "skipped because the
                //   only available blunder was -X cp."
                // - excluded_above_loss = None: no above tier at all
                //   AND no below tier — only possible if lines.len()
                //   was 1, which the outer guard already excluded.
                //   Defensive fall-through to BlunderSkipped with 0,
                //   though this branch shouldn't be reachable.
                let rejected = pool.excluded_above_loss.unwrap_or(0);
                return NoisePick::BlunderSkipped {
                    closest_above_loss_cp: rejected,
                };
            }
            let idx = pool.indices[(rng as usize) % pool.indices.len()];
            return NoisePick::Blunder(idx);
        }
    }

    // Wild branch: bypass the search ranking. Mate-guarded so we don't
    // randomly walk away from a forced win the engine has fully
    // resolved.
    if noise.wild_chance > 0.0 && !mate_guard && !legal_moves.is_empty() {
        let (roll, next) = roll_unit(rng);
        rng = next;
        if roll < noise.wild_chance as f64 {
            let idx = (rng as usize) % legal_moves.len();
            return NoisePick::Wild(legal_moves[idx]);
        }
    }

    if lines.len() <= 1 {
        return NoisePick::Line(0);
    }

    // Softmax branch over the top `candidate_pool` lines.
    let pool = noise.candidate_pool.max(1).min(lines.len());
    if pool == 1 || noise.temperature_cp <= 0 {
        return NoisePick::Line(0);
    }
    NoisePick::Line(softmax_pick(&lines[..pool], noise.temperature_cp, rng))
}

/// Mix the game seed with the current ply count through SplitMix64.
/// Pure function; same `(seed, ply)` always yields the same draw.
fn mix(seed: u64, ply: u64) -> u64 {
    let mut x = seed
        .wrapping_add(ply.wrapping_mul(0x9E37_79B9_7F4A_7C15))
        .wrapping_add(0xD1B5_4A32_D192_ED03);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    x
}

/// Step a SplitMix64 state and return a uniform `f64` in `[0, 1)`
/// alongside the next state. Two values from one input keeps the
/// caller's mental model simple (one mix per noise decision).
fn roll_unit(rng: u64) -> (f64, u64) {
    // Top 53 bits give the full f64 mantissa with no rounding bias.
    let bits = rng >> 11;
    let unit = bits as f64 / ((1u64 << 53) as f64);
    let next = mix(rng, 0xC0FF_EE15_BEEF_F00D);
    (unit, next)
}

/// Score gap (in centipawns) of `other` behind `top`, clamped at 0.
/// Mate scores are huge — non-mate alternatives to a winning mate
/// will exceed any realistic blunder loss band, so the blunder
/// branch's mate-guard runs separately.
fn score_delta_cp(top: Value, other: Value) -> i32 {
    (top.0 - other.0).max(0)
}

/// Result of [`blunder_pool`]. `indices` are the lines the picker
/// should sample from; `excluded_above_loss` is the smallest above-
/// band loss when the fallback-tolerance cap rejected the above tier.
/// The caller uses `excluded_above_loss` to render a useful "blunder
/// skipped" log when `indices` ends up empty (no plausible below side
/// either).
pub struct BlunderPool {
    pub indices: Vec<usize>,
    pub excluded_above_loss: Option<i32>,
}

/// Build the blunder candidate pool for `lines` given the
/// `[min_loss, max_loss]` preference band:
///
/// - **In-band lines** (loss in `[min_loss, max_loss]`): preferred,
///   returned as-is and the caller picks uniformly from them.
/// - **No in-band lines**: pool together the lines with the largest
///   loss strictly below `min_loss` (the most-blundery of the
///   "not-blundery-enough" group) and the lines with the smallest
///   loss strictly above `max_loss` (the least-catastrophic of the
///   "too-catastrophic" group). Both sets are kept (with ties
///   included) so the caller can pick uniformly across them.
///
/// **Above-band tolerance cap.** The above tier is admitted only if
/// its loss is `<= max_loss × fallback_tolerance`. Without that cap,
/// a position where every non-#1 line was catastrophically bad (e.g.
/// the engine sees a forcing tactic and every alternative loses
/// 20+ pawns) would have the picker happily take the 20-pawn drop —
/// blowing the configured "small blunders only" intent. When the
/// above tier is rejected, [`BlunderPool::excluded_above_loss`]
/// carries the rejected loss so the caller can log the skip.
///
/// The two-sided fallback is the load-bearing property: if the upper
/// band were a hard ceiling, the picker would have nothing to do in
/// positions where every non-best move is a piece sacrifice. By
/// admitting the *least* sacrificial of those moves (within the
/// tolerance cap), the bot can still register a "blunder roll" while
/// never throwing away a piece if a less-bad option exists.
pub fn blunder_pool(
    lines: &[SearchLine],
    top_score: Value,
    min_loss: i32,
    max_loss: i32,
    fallback_tolerance: f32,
) -> BlunderPool {
    let mut in_band: Vec<usize> = Vec::new();
    let mut best_below_loss: Option<i32> = None; // largest loss strictly < min_loss
    let mut best_above_loss: Option<i32> = None; // smallest loss strictly > max_loss
    for (i, line) in lines.iter().enumerate().skip(1) {
        let loss = score_delta_cp(top_score, line.score);
        if loss >= min_loss && loss <= max_loss {
            in_band.push(i);
        } else if loss < min_loss {
            best_below_loss = Some(match best_below_loss {
                Some(prev) => prev.max(loss),
                None => loss,
            });
        } else {
            best_above_loss = Some(match best_above_loss {
                Some(prev) => prev.min(loss),
                None => loss,
            });
        }
    }
    if !in_band.is_empty() {
        return BlunderPool {
            indices: in_band,
            excluded_above_loss: None,
        };
    }
    // Empty band — apply the above-tolerance cap.
    let above_cap = (max_loss as f32 * fallback_tolerance) as i32;
    let above_loss_admitted = best_above_loss.filter(|&loss| loss <= above_cap);
    let excluded_above_loss = match (best_above_loss, above_loss_admitted) {
        (Some(loss), None) => Some(loss), // existed but was capped out
        _ => None,
    };
    let mut pool: Vec<usize> = Vec::new();
    for (i, line) in lines.iter().enumerate().skip(1) {
        let loss = score_delta_cp(top_score, line.score);
        if Some(loss) == best_below_loss || Some(loss) == above_loss_admitted {
            pool.push(i);
        }
    }
    BlunderPool {
        indices: pool,
        excluded_above_loss,
    }
}

/// True when `top` is a mate-in-N score with `N <= guaranteed_mate_in`.
/// Guard's purpose: a 1400-ELO bot may miss positional plans, but
/// blundering forced mates the engine has fully resolved looks like a
/// bug rather than a teaching scenario.
fn mate_guarded(top: Value, guaranteed_mate_in: u32) -> bool {
    if guaranteed_mate_in == 0 {
        return false;
    }
    let mate = Value::MATE.0;
    let abs = top.0.abs();
    // Same mate-distance test the CLI score formatter uses (play.rs).
    if abs < mate - Value::MAX_PLY {
        return false;
    }
    let plies_to_mate = mate - abs;
    let full_moves = ((plies_to_mate + 1) / 2) as u32;
    // Only protect mates the bot is actually winning (top > 0).
    // Being mated isn't something a blunder can "save".
    top.0 > 0 && full_moves <= guaranteed_mate_in
}

/// Boltzmann-weighted pick over `lines` with the given temperature in
/// centipawns. Weights are `exp((score_i - score_top) / temperature)`,
/// so the top line is always the peak (delta = 0 -> weight = 1).
/// `rng` is consumed for the single uniform draw; the function returns
/// an index into `lines`.
fn softmax_pick(lines: &[SearchLine], temperature_cp: i32, rng: u64) -> usize {
    let top = lines[0].score.0 as f64;
    let temp = temperature_cp as f64;
    let weights: Vec<f64> = lines
        .iter()
        .map(|l| {
            let delta = (l.score.0 as f64) - top;
            (delta / temp).exp()
        })
        .collect();
    let total: f64 = weights.iter().sum();
    if !total.is_finite() || total <= 0.0 {
        return 0;
    }
    let (unit, _) = roll_unit(rng);
    let target = unit * total;
    let mut acc = 0.0;
    for (i, w) in weights.iter().enumerate() {
        acc += w;
        if target < acc {
            return i;
        }
    }
    // Floating-point rounding can land target == total; fall back to
    // the last bucket rather than returning out-of-range.
    lines.len() - 1
}

#[cfg(test)]
#[path = "noise_tests.rs"]
mod tests;
