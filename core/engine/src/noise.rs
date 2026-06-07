//! Move-noise sampling: turns a ranked list of `SearchLine`s (plus the
//! full legal-move list) into the move the bot actually plays.
//!
//! The play loop runs the search with [`NoiseProfile::effective_multi_pv`]
//! slots, then calls [`pick`] to decide what becomes the move. One
//! branch remains — there is no "pick a random legal move" branch, by
//! design: humans don't play randomly. The natural weaknesses are
//! modelled upstream, inside the search itself: hanging pieces by the
//! tactical-vision dial ([`crate::opponent::OpponentProfile::
//! qsearch_max_plies`]) and missed tactics / walked-into refutations by
//! the perception lever ([`crate::visibility`]).
//!
//! (Historical note: explicit `miss_chance` / `blunder_chance` branches
//! lived here until 2026-06-07. They were removed once the perception
//! lever proved it generates both organically — a miss is a winning
//! move the bot never saw; a blunder is an opponent refutation it never
//! saw. Coin-flip mistakes read as engine randomness; geometry-shaped
//! mistakes read as human.)
//!
//! **Variety branch** (when [`NoiseProfile::avg_move_rank`] > 1.0):
//! first *filter out* lines that throw away **saveable, perceived**
//! material — a line whose own PV settles with the bot down material
//! that a better line would have kept (value- and rank-scaled: a queen
//! is saved through ~rank 4, a pawn almost never; #0 always kept as a
//! possible sacrifice), then sample which surviving line *rank* to play
//! from a normal distribution centred on `avg_move_rank` (spread scales
//! with the dial). "Perceived" is the load-bearing word: the gate reads
//! each line's settled material delta straight off the
//! perception-filtered search PV, so a loss the bot never *saw* — the
//! punishing capture was pruned by [`crate::visibility`], so it isn't in
//! the PV — is **not** filtered, and the bot commits the realistic,
//! geometry-shaped blunder. A loss it *did* see (a fresh recapture at
//! the attention locus, sitting in the PV) is saved. This also catches
//! pieces the move *abandons*, not just the moved piece — any material
//! the line gives up by its settled ply. So the bot plays the Nth-best
//! *materially-sane* move: weak and varied, but it won't knowingly hand
//! over a queen (a qsearch-0 bot, blind to the recapture, has no such
//! protection — by design; its PV never shows the loss).
//! At the `1.0` floor the spread is zero, so it returns the best line.
//! Two material easings ride on top: an immediate winning capture
//! or pawn promotion at #0 is snapped back to (you don't leave a free
//! piece / decline a queening). See [`sample_rank`],
//! [`self_hang_drop_prob`].
//! Mate-guarded: a forced mate the engine has resolved within
//! [`NoiseProfile::guaranteed_mate_in`] moves is never demoted — see
//! [`mate_guarded`].
//!
//! When the branch doesn't fire, the picker returns
//! [`NoisePick::Line(0)`] — the engine's best move.
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
use crate::position::Position;
use crate::types::{Color, MoveKind, PieceType, Value};

/// Outcome of [`pick`]. The branch that fired is encoded in the
/// variant so the caller can render it accurately in diagnostic
/// output ("blunder #6 of 10" vs "variety #3 of 10"). The move itself
/// is always `lines[idx].pv[0]`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NoisePick {
    /// Engine-best or variety pick: take `lines[idx].pv[0]`.
    /// `idx == 0` is the off-noise / no-branch-fired path; `idx > 0`
    /// means the variety branch sampled this slot.
    Line(usize),
}

/// One full pawn in material-centipawns (pawn = 100, standard values) —
/// the unit the material classifiers and the capture-rescue easing
/// reason in.
pub const WIN_MATERIAL_CP: i32 = 100;

/// Decide what move the bot actually plays. See module docs for the
/// branch semantics.
///
/// `root` is the position the bot is moving from — needed to classify
/// each line's material outcome for the capture-rescue easing.
/// `lines` is the engine's ranked result (best first); it may be empty,
/// in which case the picker degrades to [`NoisePick::Line(0)`].
pub fn pick(
    noise: &NoiseProfile,
    seed: u64,
    ply: u64,
    root: &Position,
    lines: &[SearchLine],
) -> NoisePick {
    if noise.is_off() {
        return NoisePick::Line(0);
    }

    let top_score = lines.first().map(|l| l.score).unwrap_or(Value::ZERO);
    let mate_guard = !lines.is_empty() && mate_guarded(top_score, noise.guaranteed_mate_in);

    // Per-line settled material outcome (material-cp, side-to-move POV)
    // for the variety branch's capture-rescue, which needs the per-line
    // deltas to size the material a demotion would throw away.
    let deltas: Vec<i32> = if noise.avg_move_rank > 1.0 && !lines.is_empty() {
        let root_stm = root.side_to_move();
        lines
            .iter()
            .map(|l| line_material_delta_cp(root, l, root_stm))
            .collect()
    } else {
        Vec::new()
    };

    let rng = mix(seed, ply);

    // A guaranteed mate within the bot's vision is protected from the
    // variety branch. Without this, an
    // `avg_move_rank > 1` bot samples a rank > 0 and demotes itself off a
    // mate-in-N (N <= guaranteed_mate_in) that the engine has fully
    // resolved. (Observed: `guaranteed_mate_in = 1` still played the
    // 2nd-best move over a mate-in-1.) The protection is the whole point
    // of the dial — "weak in general, but always finds mate-in-N."
    if mate_guard {
        return NoisePick::Line(0);
    }

    if lines.len() <= 1 {
        return NoisePick::Line(0);
    }

    // Variety branch with a self-hang FILTER. Drop the lines that throw away
    // *saveable, perceived* material — a line whose own (perception-filtered)
    // PV settles with the bot down material that a better line keeps. Reading
    // the loss off the PV is what makes this perception-aware: a punishing
    // capture the bot never saw was pruned from its search, so it isn't in the
    // PV and the line reads as safe — the bot commits the realistic blunder; a
    // loss it *did* see (a fresh recapture at the attention locus) is in the PV
    // and gets filtered. This also catches abandoned pieces — any material the
    // line gives up, not just the moved piece (the bug the old SEE check, which
    // only looked at the moved piece's landing square, missed). The drop
    // probability is value- and rank-scaled (see `self_hang_drop_prob`): a
    // queen is saved through ~rank 4, smaller material hangs more, weaker
    // (higher-rank) bots save less. #0 is always kept — if the engine's best
    // move loses material, it's a sacrifice it chose, or the only fallback.
    // Sampling the rank AMONG THE SURVIVORS keeps variety instead of snapping
    // to the single best line.
    let best_material = deltas.iter().copied().max().unwrap_or(0);
    let playable: Vec<usize> = (0..lines.len())
        .filter(|&i| {
            if i == 0 {
                return true; // keep the engine's best (sacrifice / fallback)
            }
            // Only a line that is (a) down material and (b) avoidably so — a
            // better line keeps more — is a self-hang. A line that merely wins
            // *less* than #0 is a miss, not a hang, and stays in the pool.
            if deltas[i] < 0 && best_material > deltas[i] {
                let lost_pawns = (best_material - deltas[i]) as f64 / WIN_MATERIAL_CP as f64;
                let p_drop = self_hang_drop_prob(lost_pawns, noise.avg_move_rank);
                let (roll, _) = roll_unit(mix(rng, SELF_HANG_SALT.wrapping_add(i as u64)));
                roll >= p_drop // keep unless the roll lands in the drop mass
            } else {
                true // safe, or a mere miss → keep
            }
        })
        .collect();

    // Sample which rank to play, centred on `avg_move_rank`, over the SAFE
    // pool. At the 1.0 floor the spread is zero, so this returns the best safe
    // move unchanged.
    let kp = sample_rank(noise.avg_move_rank, playable.len(), rng);
    let k = playable[kp];

    // Material easing: a rank demotion off an *obvious* material gain — an
    // immediate winning capture OR a pawn promotion — is a believability bug.
    // Even a weak human doesn't leave a free queen sitting (capture; validation
    // showed bots sidestepping a check instead of taking the checker, or
    // stopping a rook shy of a capture), and *every* human queens a pawn one
    // step from promoting (validation showed a bot shuffling its king for ten
    // moves before pushing g2-g1=Q). #0 is the engine's best move; when its
    // first move grabs material the demoted move doesn't (`swing > 0`), keep #0
    // with a probability set by the *kind* of gain:
    //
    //   * promotion → ALWAYS (P = 1). Queening is the most obvious move in
    //     chess — there's no "didn't notice it." We only reach here when the
    //     engine already ranked the promotion #1, so this can't force a
    //     stalemating/under-valued promotion the search rejected.
    //   * capture → P(grab) = min(1, V / (C·(rank − 1))), rising with the
    //     material V (queen > rook > minor) and falling with `avg_move_rank` (a
    //     weaker bot misses more). C = [`CAPTURE_RESCUE_C`] = 6: a queen (V≈9)
    //     at rank 2 is always grabbed, a minor (V≈3) at rank 2 ~50%; rank 1
    //     always grabs. Weaker bots still miss high-value pieces sometimes.
    //
    // Only *immediate* gains are rescued; a subtle quiet best-move (a
    // defensive-only-move, a deep tactic) stays demotable, keeping the
    // "looks-like-zugzwang misjudgment" feel. So hanging material comes only
    // from tactical blindness (qsearch) or the deliberate blunder lever, never
    // incidentally from rank — and a passed pawn always queens.
    if k > 0 && !deltas.is_empty() {
        let swing = deltas[0] - deltas[k]; // material #0 secures over the demoted move
        let is_promo = matches!(
            lines[0].pv.first().map(|m| m.kind()),
            Some(MoveKind::Promotion)
        );
        if swing > 0 && (is_promo || first_move_is_capture(root, &lines[0])) {
            let p_grab = if is_promo {
                1.0
            } else {
                // Deltas are standard material-cp (pawn = WIN_MATERIAL_CP =
                // 100), so dividing gives the swing in pawns (queen ≈ 9).
                let v = swing as f64 / WIN_MATERIAL_CP as f64;
                let r = noise.avg_move_rank as f64;
                if r <= 1.0 {
                    1.0
                } else {
                    (v / (CAPTURE_RESCUE_C * (r - 1.0))).min(1.0)
                }
            };
            let (roll, _) = roll_unit(mix(rng, CAPTURE_RESCUE_SALT));
            if roll < p_grab {
                return NoisePick::Line(0);
            }
        }
    }

    NoisePick::Line(k)
}

/// Distinct SplitMix64 salt for the self-hang roll, independent of the rank
/// sample and the capture-rescue roll.
const SELF_HANG_SALT: u64 = 0x5A1E_6005_E1F1_2AA9;

/// Self-hang curve constant (pawns of saveable material per rank-unit),
/// mirroring [`CAPTURE_RESCUE_C`] for the give-away direction. `3` anchors the
/// queen at always-saved through rank 4 (`9 / (3·(4−1)) = 1.0`): it takes a
/// genuinely weak bot (rank > 4) to hang a queen. Smaller material hangs at
/// lower ranks; weaker (higher-rank) bots save less. Lower = save more.
const SELF_HANG_C: f64 = 3.0;

/// Distinct SplitMix64 salt for the capture-rescue roll so it's independent
/// of the rank sample that precedes it (same `rng`, different stream).
const CAPTURE_RESCUE_SALT: u64 = 0x5EED_CA97_0DE5_A1A5;

/// Material-easing curve constant (pawns of rank-cost per rank-unit). 6
/// sets the two anchors: a queen (≈9 pawns) at rank 2 is always grabbed
/// (9/6 caps at 1), a minor (≈3) at rank 2 is grabbed ~50%. Lower = grab
/// more; raise if weak bots feel too sharp about material.
const CAPTURE_RESCUE_C: f64 = 6.0;

/// True iff the line's first move is a capture (including en passant). The
/// variety branch's material easing keys on this — an *immediate* capture
/// ("grab the piece in front of you") is rescued from rank demotion, as
/// distinct from a multi-move tactic, which stays demotable.
fn first_move_is_capture(root: &Position, line: &SearchLine) -> bool {
    match line.pv.first() {
        None => false,
        Some(mv) => match mv.kind() {
            MoveKind::EnPassant => true,
            MoveKind::Castling => false,
            _ => root.piece_on(mv.to()).is_some(),
        },
    }
}

/// Probability of dropping (saving the bot from) a variety line that
/// needlessly throws away `lost_pawns` of saveable material. Rank-dependent
/// like the capture rescue: weaker bots (higher `avg_move_rank`) save less and
/// hang more. The queen anchor ([`SELF_HANG_C`] = 3) keeps a 9-pawn loss fully
/// saved through rank 4 (`9 / (3·(4−1)) = 1.0`). At the `1.0` rank floor only
/// #0 is played, so the value there is moot — the guard just avoids dividing
/// by zero.
fn self_hang_drop_prob(lost_pawns: f64, avg_move_rank: f32) -> f64 {
    let r = avg_move_rank as f64;
    if r <= 1.0 {
        return 1.0;
    }
    (lost_pawns / (SELF_HANG_C * (r - 1.0))).min(1.0)
}

/// Standard "point value" of a piece in material-centipawns (pawn =
/// 100), the intuitive chart a student reasons with. Used to score the
/// material swing of a line, independent of the engine's positional
/// piece values.
fn standard_piece_value_cp(pt: PieceType) -> i32 {
    match pt {
        PieceType::Pawn => 100,
        PieceType::Knight => 300,
        PieceType::Bishop => 300,
        PieceType::Rook => 500,
        PieceType::Queen => 900,
        PieceType::King => 0,
    }
}

/// Net material the side-to-move gains (positive) or loses (negative)
/// at the settled end of `line`, in material-centipawns (pawn = 100,
/// standard values). Walks the PV through `settled_ply` — the
/// *material-settled* ply (last forcing event before the first quiet
/// run; see `search::settled`) — or the PV end when `None`. The cap is
/// what keeps the count honest in both directions: it includes the
/// full forced exchange (never a mid-exchange snapshot) and excludes
/// the speculative deep-tail trades a long PV wanders into (which
/// previously classified quiet opening moves as material wins/losses).
fn line_material_delta_cp(root: &Position, line: &SearchLine, root_stm: Color) -> i32 {
    if line.pv.is_empty() {
        return 0;
    }
    let last_ply = match line.settled_ply {
        Some(idx) if idx < line.pv.len() => idx,
        _ => line.pv.len().saturating_sub(1),
    };
    material_delta_through_ply(root, line, root_stm, last_ply)
}

/// Net material (root-stm POV, material-cp) after the first `last_ply + 1`
/// plies of `line` are played from `root`, summing captured-piece values
/// with a sign for who captured. Consumed by the settled-outcome
/// classifier ([`line_material_delta_cp`], cap = settled ply).
fn material_delta_through_ply(
    root: &Position,
    line: &SearchLine,
    root_stm: Color,
    last_ply: usize,
) -> i32 {
    let mut scratch = root.clone();
    let mut net = 0i32;
    for (ply, &mv) in line.pv.iter().enumerate() {
        // Sign material by who is moving (read before the move is applied).
        let mover = scratch
            .piece_on(mv.from())
            .map(|p| p.color())
            .unwrap_or(root_stm);
        let sign = if mover == root_stm { 1 } else { -1 };
        // Capture: value of the piece on the destination (en passant takes a
        // pawn off an empty-looking square; castling never captures).
        let captured: Option<PieceType> = match mv.kind() {
            MoveKind::Castling => None,
            MoveKind::EnPassant => Some(PieceType::Pawn),
            _ => scratch.piece_on(mv.to()).map(|p| p.kind()),
        };
        if let Some(pt) = captured {
            net += sign * standard_piece_value_cp(pt);
        }
        // Promotion upgrades the pawn — count the gain (promoted piece minus
        // the pawn it replaces) so a queening line reads as the material win
        // it is. Without this the classifier saw a promotion as neutral, and
        // the easing below couldn't tell a free queening from a quiet move.
        if mv.kind() == MoveKind::Promotion {
            net += sign
                * (standard_piece_value_cp(mv.promoted_to())
                    - standard_piece_value_cp(PieceType::Pawn));
        }
        scratch.do_move(mv);
        if ply >= last_ply {
            break;
        }
    }
    net
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

/// Sample which line rank to play from a normal distribution centred on
/// `avg_move_rank` (1-based) with spread `σ = (avg_move_rank − 1.0) ×
/// [`RANK_SPREAD`]`. Rounds to the nearest rank and clamps into
/// `[1, n_lines]`; returns a 0-based index. At the `1.0` floor `σ = 0`,
/// so it deterministically returns `0` (the engine's best move).
fn sample_rank(avg_move_rank: f32, n_lines: usize, rng: u64) -> usize {
    if n_lines <= 1 {
        return 0;
    }
    let sigma = (avg_move_rank - 1.0) * RANK_SPREAD;
    if sigma <= 0.0 {
        return 0;
    }
    let z = gaussian(rng);
    let rank = (avg_move_rank + sigma * z).round();
    // Clamp to [1, n_lines], convert to 0-based.
    let clamped = rank.clamp(1.0, n_lines as f32) as usize;
    clamped - 1
}

/// Spread of the variety distribution per unit of `avg_move_rank` above
/// the `1.0` floor. `0.5` keeps ~95% of the mass within ±2σ =
/// ±(avg_move_rank − 1) ranks of the centre.
const RANK_SPREAD: f32 = 0.5;

/// One standard-normal sample via Box–Muller, deterministic in `rng`.
fn gaussian(rng: u64) -> f32 {
    let (u1, next) = roll_unit(rng);
    let (u2, _) = roll_unit(next);
    // Guard the log against u1 == 0.
    let u1 = u1.max(1e-12);
    let r = (-2.0 * u1.ln()).sqrt();
    let theta = 2.0 * std::f64::consts::PI * u2;
    (r * theta.cos()) as f32
}

#[cfg(test)]
#[path = "noise_tests.rs"]
mod tests;
