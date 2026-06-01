//! Move-noise sampling: turns a ranked list of `SearchLine`s (plus the
//! full legal-move list) into the move the bot actually plays.
//!
//! The play loop runs the search with [`NoiseProfile::effective_multi_pv`]
//! slots, then calls [`pick`] to decide what becomes the move. The
//! sampler has four independent branches, evaluated in this order.
//!
//! The blunder and miss branches classify each line by its **material
//! outcome** — the net material the side-to-move has at the resolved
//! (settled) end of the line, versus the current board. This is the
//! chess.com distinction (added 2023): a *blunder* loses your own
//! material; a *miss* fails to win material that was on offer. Both are
//! kept distinct from a merely-positional centipawn drop, which is not
//! a material mistake at all.
//!
//! 1. **Miss branch** (when [`NoiseProfile::miss_chance`] > 0): when a
//!    line *wins material* by force and is the best thing to do, the
//!    bot refuses it and plays the highest-scoring line that does **not**
//!    win material — even if that line is itself losing. Models "saw a
//!    winning tactic, didn't play it." No-op when no material-winning
//!    move exists. Mate-guarded.
//!
//! 2. **Blunder branch** (when [`NoiseProfile::blunder_chance`] > 0):
//!    play a line that *loses material* by force, with the amount hung
//!    falling in the `[blunder_min_material_cp, blunder_max_material_cp]`
//!    band (uniform pick among in-band lines). **Gated on existence:**
//!    the roll is only made when such a line is actually available, so a
//!    quiet position with no in-band hang simply doesn't blunder rather
//!    than diluting the configured rate. See [`material_blunder_pool`].
//!    Mate-guarded.
//!
//! 3. **Wild branch** (when [`NoiseProfile::wild_chance`] > 0): with
//!    that per-move probability, pick uniformly from **all legal
//!    moves**, ignoring the search ranking entirely. This is the
//!    beginner-bot path — the only branch that can pick a move the
//!    engine didn't even surface (e.g. leaving a piece in a pawn's
//!    path). Same mate-guard.
//!
//! 4. **Variety branch** (when [`NoiseProfile::avg_move_rank`] > 1.0):
//!    sample which line *rank* to play from a normal distribution
//!    centred on `avg_move_rank` (spread scales with the dial), then
//!    play that rank. At the `1.0` floor the spread is zero, so it
//!    returns the engine's #1. This is the "plays the Nth-best move on
//!    average" weakness dial. See [`sample_rank`].
//!
//! When no branch fires, the picker returns [`NoisePick::Line(0)`] —
//! the engine's best move.
//!
//! **Branch ordering rationale:** miss comes first because declining a
//! win is a decision about the best move itself; blunder follows as the
//! calibrated material-loss knob; wild is the chaotic knob (might
//! coincidentally pick the best move); variety is the always-on "which
//! decent move" dial and fills whatever budget remains.
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
use crate::types::{Color, Move, MoveKind, PieceType, Value};

/// Outcome of [`pick`]. The branch that fired is encoded in the
/// variant so the caller can render it accurately in diagnostic
/// output ("blunder #6 of 10" vs "variety #3 of 10" vs "wild — engine
/// preferred X"). The move itself is either `lines[idx].pv[0]`
/// (line-based variants) or the wild legal move directly.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NoisePick {
    /// Engine-best or variety pick: take `lines[idx].pv[0]`.
    /// `idx == 0` is the off-noise / no-branch-fired path; `idx > 0`
    /// means the variety branch sampled this slot.
    Line(usize),
    /// Blunder branch fired: take `lines[idx].pv[0]`. `idx` is always
    /// `>= 1` (blunder never picks #1) — a line that loses material
    /// inside the configured band. The roll is only made when such a
    /// line exists, so there is no "rolled but nothing to do" variant.
    Blunder(usize),
    /// Miss branch fired: a material-winning move was available and the
    /// bot deliberately declined it, playing `lines[idx].pv[0]` — the
    /// best line that does not win material. `idx` may be any slot
    /// (including a losing one, when every non-winning move loses).
    Miss(usize),
    /// Wild branch fired: play this legal move directly, bypassing
    /// the engine ranking entirely.
    Wild(Move),
}

/// Material gain (in material-centipawns, pawn = 100) at the settled
/// end of a line for the side to move to count that line as "winning
/// material" — the threshold above which a [`miss`](NoisePick::Miss)
/// will decline it. One full pawn: anything less isn't a material win
/// worth deliberately passing up.
pub const WIN_MATERIAL_CP: i32 = 100;

/// Decide what move the bot actually plays. See module docs for the
/// branch order and semantics.
///
/// `root` is the position the bot is moving from — needed to classify
/// each line's material outcome for the miss / blunder branches.
/// `lines` is the engine's ranked result (best first). `legal_moves`
/// is the full legal-move list for the current position; only consumed
/// by the wild branch. Either list may be empty; the picker degrades to
/// [`NoisePick::Line(0)`] when it has nothing to choose from.
pub fn pick(
    noise: &NoiseProfile,
    seed: u64,
    ply: u64,
    root: &Position,
    lines: &[SearchLine],
    legal_moves: &[Move],
) -> NoisePick {
    if noise.is_off() {
        return NoisePick::Line(0);
    }

    let top_score = lines.first().map(|l| l.score).unwrap_or(Value::ZERO);
    let mate_guard = !lines.is_empty() && mate_guarded(top_score, noise.guaranteed_mate_in);

    // Per-line settled material outcome (material-cp, side-to-move POV),
    // computed only when a branch that needs it is enabled.
    let needs_material = noise.miss_chance > 0.0 || noise.blunder_chance > 0.0;
    let deltas: Vec<i32> = if needs_material && !lines.is_empty() {
        let root_stm = root.side_to_move();
        lines
            .iter()
            .map(|l| line_material_delta_cp(root, l, root_stm))
            .collect()
    } else {
        Vec::new()
    };

    let mut rng = mix(seed, ply);

    // Miss branch: when the *best* move wins material, deliberately
    // decline it and play the best line that does not win material.
    // Eligible only when there's a real material win to pass up.
    if noise.miss_chance > 0.0 && !mate_guard && !deltas.is_empty() && deltas[0] >= WIN_MATERIAL_CP {
        let (roll, next) = roll_unit(rng);
        rng = next;
        if roll < noise.miss_chance as f64 {
            // First (highest-scoring) line that isn't a material win.
            if let Some(idx) = (0..deltas.len()).find(|&i| deltas[i] < WIN_MATERIAL_CP) {
                return NoisePick::Miss(idx);
            }
            // Every line wins material — nothing to miss; fall through.
        }
    }

    // Blunder branch: play a line that loses material inside the band.
    // Gated on existence — the roll is only made when an in-band hang
    // actually exists, so `blunder_chance` reads as "given a punishable
    // hang is available, how often do I take it" rather than being
    // silently diluted by quiet positions. Mate-guarded.
    if noise.blunder_chance > 0.0 && !mate_guard && lines.len() > 1 {
        let in_band = material_blunder_pool(
            &deltas,
            noise.blunder_min_material_cp,
            noise.blunder_max_material_cp,
        );
        if !in_band.is_empty() {
            let (roll, next) = roll_unit(rng);
            rng = next;
            if roll < noise.blunder_chance as f64 {
                let idx = in_band[(rng as usize) % in_band.len()];
                return NoisePick::Blunder(idx);
            }
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

    // Variety branch: sample which rank to play from a normal
    // distribution centred on `avg_move_rank`. At the 1.0 floor the
    // spread is zero, so this returns #1 unchanged.
    NoisePick::Line(sample_rank(noise.avg_move_rank, lines.len(), rng))
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
/// standard values). Walks the PV through `settled_ply` (or the PV end
/// if it never settled), summing captured-piece values with a sign for
/// who captured. The settled cap keeps the count quiescent — it stops
/// once the tactics have resolved rather than counting a mid-exchange
/// snapshot.
fn line_material_delta_cp(root: &Position, line: &SearchLine, root_stm: Color) -> i32 {
    if line.pv.is_empty() {
        return 0;
    }
    let last_ply = match line.settled_ply {
        Some(idx) if idx < line.pv.len() => idx,
        _ => line.pv.len().saturating_sub(1),
    };
    let mut scratch = root.clone();
    let mut net = 0i32;
    for (ply, &mv) in line.pv.iter().enumerate() {
        // Resolve the capture before applying the move.
        let captured: Option<PieceType> = match mv.kind() {
            MoveKind::Castling => None,
            MoveKind::EnPassant => Some(PieceType::Pawn),
            _ => scratch.piece_on(mv.to()).map(|p| p.kind()),
        };
        if let Some(pt) = captured {
            let captor = scratch
                .piece_on(mv.from())
                .map(|p| p.color())
                .unwrap_or(root_stm);
            let sign = if captor == root_stm { 1 } else { -1 };
            net += sign * standard_piece_value_cp(pt);
        }
        scratch.do_move(mv);
        if ply >= last_ply {
            break;
        }
    }
    net
}

/// In-band blunder candidates: non-best lines (`i >= 1`) that *lose*
/// material in `[min_loss, max_loss]` material-cp. Best-effort: a hang
/// below the band isn't blundery enough and one above it is too
/// catastrophic — both are excluded, and an empty result means "don't
/// blunder here."
fn material_blunder_pool(deltas: &[i32], min_loss: i32, max_loss: i32) -> Vec<usize> {
    deltas
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(_, &delta)| {
            let loss = -delta;
            loss >= min_loss && loss <= max_loss
        })
        .map(|(i, _)| i)
        .collect()
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
