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
    /// `>= 1` (blunder never picks #1) — either a qualifying line
    /// that cleared the severity gate, or `lines.len() - 1` as the
    /// worst-available fallback when no line qualified.
    Blunder(usize),
    /// Wild branch fired: play this legal move directly, bypassing
    /// the engine ranking entirely.
    Wild(Move),
}

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
            );
            // pool can only be empty if lines.len() == 1, which the
            // outer guard already excluded. Defensive fallback to
            // worst-available so we never panic if the invariant
            // changes.
            let idx = if pool.is_empty() {
                lines.len() - 1
            } else {
                pool[(rng as usize) % pool.len()]
            };
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
/// The two-sided fallback is the load-bearing property: if the
/// upper band were a hard ceiling, the picker would have nothing
/// to do in positions where every non-best move is a piece sacrifice.
/// By admitting the *least* sacrificial of those moves into the
/// pool, the bot can still register a "blunder roll" while never
/// throwing away a piece if a less-bad option exists.
fn blunder_pool(
    lines: &[SearchLine],
    top_score: Value,
    min_loss: i32,
    max_loss: i32,
) -> Vec<usize> {
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
        return in_band;
    }
    // Empty band — gather the tie-classes closest to the band on each
    // side. Lines further from the band on either side are excluded.
    let mut pool: Vec<usize> = Vec::new();
    for (i, line) in lines.iter().enumerate().skip(1) {
        let loss = score_delta_cp(top_score, line.score);
        if Some(loss) == best_below_loss || Some(loss) == best_above_loss {
            pool.push(i);
        }
    }
    pool
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
mod tests {
    use super::*;
    use crate::engine::SearchLine;
    use crate::types::{Move, Square, Value};

    /// Stub line with the given score and an empty PV — `pick` only
    /// reads `score`, so the rest is filler.
    fn line(score_cp: i32) -> SearchLine {
        SearchLine {
            pv: Vec::<Move>::new(),
            score: Value(score_cp),
            depth: 1,
            ply_traces: Vec::new(),
            settled_ply: None,
        }
    }

    /// Distinct stub moves keyed by an index — used by the wild-branch
    /// tests where we need to tell apart "which legal move came back".
    fn stub_move(seed: u8) -> Move {
        // Any two squares will do; the picker treats Move as an opaque
        // value. Mapping `seed` to a unique from-square gives us a
        // stable identity for assertion comparisons.
        let from = Square::from_index(seed % 64);
        let to = Square::from_index(seed.wrapping_add(8) % 64);
        Move::normal(from, to)
    }

    #[test]
    fn off_profile_always_picks_first() {
        let noise = NoiseProfile::default();
        let lines = vec![line(50), line(40), line(30), line(20)];
        for ply in 0..20 {
            assert_eq!(pick(&noise, 0xCAFE, ply, &lines, &[]), NoisePick::Line(0));
        }
    }

    #[test]
    fn single_line_always_picks_zero() {
        let noise = NoiseProfile {
            candidate_pool: 4,
            temperature_cp: 200,
            blunder_chance: 1.0, // even a guaranteed blunder has nothing to pick
            ..Default::default()
        };
        let lines = vec![line(10)];
        // Wild is off → only one line and no qualifying alternative.
        assert_eq!(pick(&noise, 0xCAFE, 1, &lines, &[]), NoisePick::Line(0));
    }

    #[test]
    fn empty_lines_picks_zero() {
        // Defensive — caller checks emptiness, but pick shouldn't panic.
        let noise = NoiseProfile::default();
        let lines: Vec<SearchLine> = Vec::new();
        assert_eq!(pick(&noise, 0, 0, &lines, &[]), NoisePick::Line(0));
    }

    #[test]
    fn pool_one_skips_softmax_even_with_temperature() {
        // candidate_pool=1 is the "softmax off" signal regardless of
        // temperature. The user must opt into pool > 1 to get noise.
        let noise = NoiseProfile {
            candidate_pool: 1,
            temperature_cp: 1_000,
            ..Default::default()
        };
        let lines = vec![line(0), line(-10), line(-20)];
        for ply in 0..10 {
            assert_eq!(pick(&noise, 0xBEEF, ply, &lines, &[]), NoisePick::Line(0));
        }
    }

    #[test]
    fn zero_temperature_with_pool_picks_first() {
        // Without temperature, softmax collapses to "always #1" even at
        // wide pool. This is the "give me variety only when scores are
        // close" knob if the user later sets temperature.
        let noise = NoiseProfile {
            candidate_pool: 4,
            temperature_cp: 0,
            ..Default::default()
        };
        let lines = vec![line(100), line(99), line(98), line(97)];
        for ply in 0..10 {
            assert_eq!(pick(&noise, 0xFEED, ply, &lines, &[]), NoisePick::Line(0));
        }
    }

    #[test]
    fn softmax_picks_within_pool_only() {
        // High temperature + 3-deep pool: the picker must never return
        // 3 (which sits outside the pool), even though we provided 4
        // lines.
        let noise = NoiseProfile {
            candidate_pool: 3,
            temperature_cp: 500, // very flat — all three weighted similarly
            ..Default::default()
        };
        let lines = vec![line(20), line(15), line(10), line(-200)];
        for ply in 0..200 {
            let pick = pick(&noise, 0xABCD, ply, &lines, &[]);
            match pick {
                NoisePick::Line(idx) => assert!(idx < 3, "softmax leaked outside pool: {idx}"),
                other => panic!("non-softmax pick at ply {ply}: {other:?}"),
            }
        }
    }

    #[test]
    fn softmax_actually_varies_across_plies() {
        let noise = NoiseProfile {
            candidate_pool: 3,
            temperature_cp: 50,
            ..Default::default()
        };
        let lines = vec![line(0), line(-10), line(-20)];
        let mut seen = [0usize; 3];
        for ply in 0..200 {
            match pick(&noise, 0xDEAD, ply, &lines, &[]) {
                NoisePick::Line(idx) => seen[idx] += 1,
                other => panic!("non-softmax pick at ply {ply}: {other:?}"),
            }
        }
        let distinct = seen.iter().filter(|&&c| c > 0).count();
        assert!(distinct >= 2, "softmax never varied: {seen:?}");
        assert!(seen[0] >= seen[1] && seen[0] >= seen[2], "modal pick wasn't #1: {seen:?}");
    }

    #[test]
    fn blunder_with_no_in_band_lines_picks_closest_below() {
        // No line falls in the band [100, INF]. The fallback pool is
        // the line(s) with the largest loss strictly below the band's
        // lower edge — here that's idx 3 (loss=90). The bot picks
        // there rather than playing #1, preserving the "gradual
        // decline" property in quiet positions where no real blunder
        // is available.
        let noise = NoiseProfile {
            candidate_pool: 1,
            blunder_chance: 1.0,
            blunder_min_loss_cp: 100,
            blunder_max_loss_cp: i32::MAX,
            ..Default::default()
        };
        let lines = vec![line(0), line(-10), line(-50), line(-90)];
        for ply in 0..20 {
            assert_eq!(
                pick(&noise, 0xABCD, ply, &lines, &[]),
                NoisePick::Blunder(3),
                "fallback should pick the largest sub-band loss (idx 3, -90)",
            );
        }
    }

    #[test]
    fn blunder_picks_only_in_band_lines_when_some_qualify() {
        // Band [100, INF] with losses 50, 99, 100, 300: in-band set
        // is {idx 3 (loss=100), idx 4 (loss=300)}. The picker must
        // never pick #1 or the sub-band lines (50, 99).
        let noise = NoiseProfile {
            candidate_pool: 1,
            blunder_chance: 1.0,
            blunder_min_loss_cp: 100,
            blunder_max_loss_cp: i32::MAX,
            guaranteed_mate_in: 0,
            ..Default::default()
        };
        let lines = vec![line(0), line(-50), line(-99), line(-100), line(-300)];
        for ply in 0..50 {
            match pick(&noise, 0x1234, ply, &lines, &[]) {
                NoisePick::Blunder(idx) => assert!(
                    idx == 3 || idx == 4,
                    "blunder picked outside in-band set: {idx}",
                ),
                NoisePick::Line(idx) => panic!(
                    "blunder branch should fire (chance=1.0), got Line({idx})",
                ),
                NoisePick::Wild(_) => panic!("wild fired without wild_chance > 0"),
            }
        }
    }

    #[test]
    fn blunder_band_excludes_too_catastrophic() {
        // The whole point of the upper band: with max=400, an alt
        // line at loss=1000 (queen-hang territory) must never be
        // picked when a 200-cp option exists. Band = [100, 400];
        // in-band set = {idx 2 (loss=200)}; the loss=1000 line is
        // excluded.
        let noise = NoiseProfile {
            candidate_pool: 1,
            blunder_chance: 1.0,
            blunder_min_loss_cp: 100,
            blunder_max_loss_cp: 400,
            guaranteed_mate_in: 0,
            ..Default::default()
        };
        let lines = vec![line(0), line(-50), line(-200), line(-1000)];
        for ply in 0..50 {
            assert_eq!(
                pick(&noise, 0xCAFE, ply, &lines, &[]),
                NoisePick::Blunder(2),
                "should only pick the in-band move (idx 2, -200)",
            );
        }
    }

    #[test]
    fn blunder_band_fallback_pools_closest_on_each_side() {
        // Band [50, 100] with losses 10, 30, 110, 240: in-band is
        // empty. Closest-below (largest loss < 50) is idx 2 (loss=30).
        // Closest-above (smallest loss > 100) is idx 3 (loss=110).
        // The 240-cp line is excluded because 110 is closer to the
        // band from above. Pool = {idx 2, idx 3}; pick must be one
        // of those.
        let noise = NoiseProfile {
            candidate_pool: 1,
            blunder_chance: 1.0,
            blunder_min_loss_cp: 50,
            blunder_max_loss_cp: 100,
            guaranteed_mate_in: 0,
            ..Default::default()
        };
        let lines = vec![line(0), line(-10), line(-30), line(-110), line(-240)];
        let mut seen_below = 0;
        let mut seen_above = 0;
        for ply in 0..200 {
            match pick(&noise, 0xBEEF, ply, &lines, &[]) {
                NoisePick::Blunder(2) => seen_below += 1,
                NoisePick::Blunder(3) => seen_above += 1,
                NoisePick::Blunder(idx) => panic!(
                    "fallback picked outside the closest-on-each-side pool: {idx}",
                ),
                other => panic!("non-blunder pick: {other:?}"),
            }
        }
        assert!(seen_below > 0, "closest-below tier never picked");
        assert!(seen_above > 0, "closest-above tier never picked");
    }

    #[test]
    fn blunder_band_fallback_with_only_above_band_lines() {
        // No in-band, no below-band — every line is catastrophic
        // (e.g. forced position where any deviation loses heavily).
        // The pool collapses to the smallest above-band loss; the bot
        // takes the least-bad of the bad options.
        let noise = NoiseProfile {
            candidate_pool: 1,
            blunder_chance: 1.0,
            blunder_min_loss_cp: 100,
            blunder_max_loss_cp: 300,
            guaranteed_mate_in: 0,
            ..Default::default()
        };
        // Losses: 500, 800, 1200 — all > max=300.
        let lines = vec![line(0), line(-500), line(-800), line(-1200)];
        for ply in 0..30 {
            assert_eq!(
                pick(&noise, 0xFACE, ply, &lines, &[]),
                NoisePick::Blunder(1),
                "should pick the least-catastrophic above-band line (idx 1, -500)",
            );
        }
    }

    #[test]
    fn blunder_band_fallback_includes_tied_losses() {
        // Two lines at the same closest-below loss should both be
        // in the fallback pool — ties are kept rather than the picker
        // arbitrarily favouring one.
        let noise = NoiseProfile {
            candidate_pool: 1,
            blunder_chance: 1.0,
            blunder_min_loss_cp: 200,
            blunder_max_loss_cp: 400,
            guaranteed_mate_in: 0,
            ..Default::default()
        };
        // Losses: 50, 100, 100 — in-band empty; closest-below = 100
        // (tied at idx 2 and idx 3). Pool = {2, 3}.
        let lines = vec![line(0), line(-50), line(-100), line(-100)];
        let mut seen = [0usize; 4];
        for ply in 0..200 {
            match pick(&noise, 0xDEAD, ply, &lines, &[]) {
                NoisePick::Blunder(idx) => {
                    assert!(idx == 2 || idx == 3, "out-of-pool pick: {idx}");
                    seen[idx] += 1;
                }
                other => panic!("non-blunder pick: {other:?}"),
            }
        }
        assert!(seen[2] > 0 && seen[3] > 0, "tied losses must both be reachable: {seen:?}");
    }

    #[test]
    fn blunder_suppressed_when_mate_guarded() {
        let noise = NoiseProfile {
            candidate_pool: 1,
            blunder_chance: 1.0,
            blunder_min_loss_cp: 100,
            blunder_max_loss_cp: i32::MAX,
            guaranteed_mate_in: 3,
            ..Default::default()
        };
        let mate_in_2 = Value::MATE.0 - 3;
        let lines = vec![line(mate_in_2), line(0), line(-100)];
        for ply in 0..20 {
            assert_eq!(pick(&noise, 0xFACE, ply, &lines, &[]), NoisePick::Line(0));
        }
    }

    #[test]
    fn blunder_allowed_for_mate_beyond_guarantee() {
        let noise = NoiseProfile {
            candidate_pool: 1,
            blunder_chance: 1.0,
            blunder_min_loss_cp: 100,
            blunder_max_loss_cp: i32::MAX,
            guaranteed_mate_in: 3,
            ..Default::default()
        };
        let mate_in_5 = Value::MATE.0 - 9;
        let lines = vec![line(mate_in_5), line(0), line(-100)];
        let mut saw_blunder = false;
        for ply in 0..20 {
            if matches!(pick(&noise, 0xFACE, ply, &lines, &[]), NoisePick::Blunder(_)) {
                saw_blunder = true;
                break;
            }
        }
        assert!(saw_blunder, "blunder branch never fired against mate-in-5");
    }

    #[test]
    fn guaranteed_mate_zero_disables_protection() {
        let noise = NoiseProfile {
            candidate_pool: 1,
            blunder_chance: 1.0,
            blunder_min_loss_cp: 100,
            blunder_max_loss_cp: i32::MAX,
            guaranteed_mate_in: 0,
            ..Default::default()
        };
        let mate_in_1 = Value::MATE.0 - 1;
        let lines = vec![line(mate_in_1), line(0), line(-100)];
        let mut saw_blunder = false;
        for ply in 0..20 {
            if matches!(pick(&noise, 0xFACE, ply, &lines, &[]), NoisePick::Blunder(_)) {
                saw_blunder = true;
                break;
            }
        }
        assert!(saw_blunder, "guaranteed_mate_in=0 should not protect mate-in-1");
    }

    #[test]
    fn mate_guard_does_not_protect_being_mated() {
        let noise = NoiseProfile {
            candidate_pool: 1,
            blunder_chance: 1.0,
            blunder_min_loss_cp: 100,
            blunder_max_loss_cp: i32::MAX,
            guaranteed_mate_in: 5,
            ..Default::default()
        };
        let getting_mated_in_2 = -(Value::MATE.0 - 3);
        let lines = vec![line(getting_mated_in_2), line(-200), line(-1000)];
        for ply in 0..20 {
            // No assertion on exact pick — just that the function
            // doesn't panic and returns a valid line index. Either
            // Line(idx) or Blunder(idx) is fine here.
            match pick(&noise, 0xBABE, ply, &lines, &[]) {
                NoisePick::Line(idx) | NoisePick::Blunder(idx) => {
                    assert!(idx < lines.len())
                }
                NoisePick::Wild(_) => panic!("wild fired without wild_chance > 0"),
            }
        }
    }

    #[test]
    fn pick_is_deterministic_for_same_inputs() {
        let noise = NoiseProfile {
            candidate_pool: 4,
            temperature_cp: 200,
            blunder_chance: 0.3,
            blunder_min_loss_cp: 80,
            blunder_max_loss_cp: i32::MAX,
            guaranteed_mate_in: 1,
            wild_chance: 0.1,
        };
        let lines = vec![line(0), line(-20), line(-50), line(-150), line(-400)];
        let legal = vec![stub_move(0), stub_move(1), stub_move(2), stub_move(3)];
        for ply in 0..20 {
            let a = pick(&noise, 0xABCD, ply, &lines, &legal);
            let b = pick(&noise, 0xABCD, ply, &lines, &legal);
            assert_eq!(a, b, "same inputs gave different picks at ply {ply}");
        }
    }

    #[test]
    fn pick_varies_with_seed() {
        let noise = NoiseProfile {
            candidate_pool: 4,
            temperature_cp: 200,
            ..Default::default()
        };
        let lines = vec![line(0), line(-20), line(-40), line(-80)];
        let seq_a: Vec<_> = (0..50).map(|p| pick(&noise, 0x1111_2222, p, &lines, &[])).collect();
        let seq_b: Vec<_> = (0..50).map(|p| pick(&noise, 0xAAAA_BBBB, p, &lines, &[])).collect();
        assert_ne!(seq_a, seq_b, "seed didn't affect the pick sequence");
    }

    // ---- wild branch -------------------------------------------------

    #[test]
    fn wild_fires_only_when_chance_set() {
        // Default profile → no wild even with legal moves provided.
        let noise = NoiseProfile::default();
        let legal = vec![stub_move(0), stub_move(1)];
        for ply in 0..20 {
            assert_eq!(pick(&noise, 0x9999, ply, &[], &legal), NoisePick::Line(0));
        }
    }

    #[test]
    fn wild_with_no_legal_moves_falls_through() {
        // Wild can't fire without a legal-move list; should fall back
        // to the engine-result branches (which also have nothing here).
        let noise = NoiseProfile {
            wild_chance: 1.0,
            ..Default::default()
        };
        assert_eq!(pick(&noise, 0x9999, 0, &[line(0)], &[]), NoisePick::Line(0));
    }

    #[test]
    fn wild_picks_from_full_legal_list_not_just_top_k() {
        // 8 legal moves, only 3 "search lines". With wild_chance=1.0
        // every pick should be a Wild that comes from the legal list —
        // including moves the search never surfaced.
        let noise = NoiseProfile {
            wild_chance: 1.0,
            guaranteed_mate_in: 0, // disable mate-guard
            ..Default::default()
        };
        let lines = vec![line(0), line(-10), line(-20)];
        let legal: Vec<Move> = (0..8).map(stub_move).collect();
        let mut seen_indices = [false; 8];
        for ply in 0..200 {
            match pick(&noise, 0xC0DE, ply, &lines, &legal) {
                NoisePick::Wild(mv) => {
                    let idx = legal.iter().position(|m| *m == mv).expect("wild move not in legal list");
                    seen_indices[idx] = true;
                }
                other => panic!("wild_chance=1.0 must always pick Wild; got {other:?}"),
            }
        }
        let distinct = seen_indices.iter().filter(|&&b| b).count();
        assert!(distinct >= 4, "wild barely varied — saw only {distinct}/8 legal moves");
    }

    #[test]
    fn wild_suppressed_when_mate_guarded() {
        // Bot has mate-in-1, guaranteed_mate_in=1 — wild must not fire
        // (would throw away the forced mate).
        let noise = NoiseProfile {
            wild_chance: 1.0,
            guaranteed_mate_in: 1,
            ..Default::default()
        };
        let mate_in_1 = Value::MATE.0 - 1;
        let lines = vec![line(mate_in_1)];
        let legal: Vec<Move> = (0..4).map(stub_move).collect();
        for ply in 0..20 {
            assert_eq!(pick(&noise, 0xFACE, ply, &lines, &legal), NoisePick::Line(0));
        }
    }

    #[test]
    fn blunder_takes_precedence_over_wild_and_softmax() {
        // With blunder_chance=1.0 the blunder branch should always
        // win, regardless of how the other knobs are set. Pins the
        // branch ordering documented at the module level: blunder is
        // the calibrated mistake signal and gets first crack, then
        // wild, then softmax.
        let noise = NoiseProfile {
            candidate_pool: 4,
            temperature_cp: 200,
            blunder_chance: 1.0,
            wild_chance: 1.0,
            guaranteed_mate_in: 0,
            ..Default::default()
        };
        let lines = vec![line(0), line(-50), line(-200), line(-400)];
        let legal: Vec<Move> = (0..6).map(stub_move).collect();
        for ply in 0..30 {
            match pick(&noise, 0xBEEF, ply, &lines, &legal) {
                NoisePick::Blunder(idx) => assert!(
                    idx >= 1,
                    "blunder must never pick #1 (got Blunder({idx}))",
                ),
                other => panic!(
                    "non-blunder pick at ply {ply}: {other:?} (blunder rolls first \
                     at chance=1.0 — must always win)",
                ),
            }
        }
    }
}
