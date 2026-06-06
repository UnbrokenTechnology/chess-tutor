//! Move-noise sampling: turns a ranked list of `SearchLine`s (plus the
//! full legal-move list) into the move the bot actually plays.
//!
//! The play loop runs the search with [`NoiseProfile::effective_multi_pv`]
//! slots, then calls [`pick`] to decide what becomes the move. The
//! sampler has three independent branches, evaluated in this order. All
//! three operate on the engine's ranked lines — there is no "pick a
//! random legal move" branch, by design: humans don't play randomly, and
//! the natural sub-1000 weakness (hanging pieces) is modelled by the
//! engine's tactical-vision dial ([`crate::opponent::OpponentProfile::
//! qsearch_max_plies`]), not by injected randomness.
//!
//! The blunder and miss branches classify each line by its **material
//! outcome** — the net material the side-to-move has at the resolved
//! (settled) end of the line, versus the current board. This is the
//! chess.com distinction (added 2023): a *blunder* loses your own
//! material; a *miss* fails to win material that was on offer. Both are
//! kept distinct from a merely-positional centipawn drop, which is not
//! a material mistake at all.
//!
//! 1. **Miss branch** (when [`NoiseProfile::miss_chance`] > 0): when the
//!    best line *wins material* by force **through a combination**, the bot
//!    refuses it and plays the highest-scoring line that does **not** win
//!    material, even if that line is itself losing. Models "saw a winning
//!    tactic, didn't play it." The combination-vs-obvious-grab line is drawn
//!    by *2-ply material*: a win already up a pawn-or-more after the best
//!    move + the opponent's reply is an obvious grab (a hanging piece, an
//!    even trade) and is left to the variety branch's material easing — you
//!    don't "miss" a piece sitting in front of you. A win that's still
//!    even-or-down at two plies but settles winning is a combination — a
//!    quiet first move (a fork) or a real sacrifice — and stays missable.
//!    No-op when no such win exists. Mate-guarded.
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
//! 3. **Variety branch** (when [`NoiseProfile::avg_move_rank`] > 1.0):
//!    first *filter out* lines whose move hangs material (value-scaled —
//!    a queen-hang always dropped, a pawn almost never; #0 always kept as
//!    a possible sacrifice), then sample which surviving line *rank* to
//!    play from a normal distribution centred on `avg_move_rank` (spread
//!    scales with the dial). So the bot plays the Nth-best *safe* move:
//!    weak and varied, but it won't drop its queen to a one-move capture
//!    (which a qsearch-0 bot, blind to the recapture, otherwise would).
//!    At the `1.0` floor the spread is zero, so it returns the best safe
//!    move. Two material easings ride on top: an immediate winning capture
//!    or pawn promotion at #0 is snapped back to (you don't leave a free
//!    piece / decline a queening). See [`sample_rank`], [`self_hang_pawns`].
//!    Mate-guarded.
//!
//! All three branches respect [`NoiseProfile::guaranteed_mate_in`]: a
//! forced mate the engine has resolved within that many moves is never
//! demoted, declined, or blundered away — see [`mate_guarded`].
//!
//! When no branch fires, the picker returns [`NoisePick::Line(0)`] —
//! the engine's best move.
//!
//! **Branch ordering rationale:** miss comes first because declining a
//! win is a decision about the best move itself; blunder follows as the
//! calibrated material-loss knob; variety is the always-on "which decent
//! move" dial and fills whatever budget remains.
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
/// output ("blunder #6 of 10" vs "variety #3 of 10"). The move itself
/// is always `lines[idx].pv[0]`.
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

    // Per-line settled material outcome (material-cp, side-to-move POV),
    // computed when a branch that needs it is enabled — miss/blunder, or
    // the variety branch's capture-rescue (which needs the per-line deltas
    // to size the material a demotion would throw away).
    let needs_material =
        noise.miss_chance > 0.0 || noise.blunder_chance > 0.0 || noise.avg_move_rank > 1.0;
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

    // Miss branch: when the *best* move wins material through a combination
    // you had to calculate, deliberately decline it and play the best line
    // that does not win material. The combination-vs-obvious-grab test is
    // **2-ply material** (`two_ply_material_cp`): a win already up a
    // pawn-or-more after the best move + the opponent's reply is an *obvious
    // grab* — a hanging piece (`Qxd5`), an even trade settled in hand — and
    // is exempt, left instead to the variety branch's material easing. You
    // don't "miss" a piece sitting in front of you. Everything that's still
    // even-or-down at two plies but settles winning is a *combination* and
    // stays missable: a quiet first move (a fork, 2-ply 0), an even trade
    // that wins on the follow-up (a discovered attack, 2-ply 0), or a real
    // **sacrifice** (Damiano-style, 2-ply negative — material comes back with
    // interest later). One test covers all three. Eligible only when there's
    // a real (settled) material win to pass up.
    if noise.miss_chance > 0.0
        && !mate_guard
        && !deltas.is_empty()
        && deltas[0] >= WIN_MATERIAL_CP
        && two_ply_material_cp(root, &lines[0], root.side_to_move()) < WIN_MATERIAL_CP
    {
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

    // A guaranteed mate within the bot's vision is protected from the
    // variety branch too — not only miss/blunder. Without this, an
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

    // Variety branch with a self-hang FILTER. First drop the lines whose move
    // hangs material — value-scaled, the capture rescue in reverse: a
    // queen-hang is always dropped, a rook ~56%, a minor ~33%, a pawn ~11%
    // (small material still hangs believably; the queen never drops to a
    // one-move capture). #0 is always kept — if the engine's best move hangs,
    // it's a sacrifice it chose on purpose, or the only fallback. Then sample
    // the rank AMONG THE SURVIVORS, so the bot plays the Nth-best *safe* move
    // and keeps its variety — instead of snapping to the single best
    // non-hanging move, which both kills variety and floors the weakness (at
    // high rank nearly everything hangs, so it always snapped back to #0). A
    // qsearch-0 bot is blind to the recapture (it ranked `Qg6` #3, not seeing
    // `hxg6`); this filter is its only protection.
    let playable: Vec<usize> = (0..lines.len())
        .filter(|&i| {
            if i == 0 {
                return true; // keep the engine's best (sacrifice / fallback)
            }
            match self_hang_pawns(root, &lines[i]) {
                None => true, // safe move
                Some(v) => {
                    // Drop with probability v / SELF_HANG_C; keep otherwise.
                    let (roll, _) = roll_unit(mix(rng, SELF_HANG_SALT.wrapping_add(i as u64)));
                    roll >= (v / SELF_HANG_C).min(1.0)
                }
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

/// Self-hang curve constant (pawns of hung material per certainty). 9 anchors
/// the queen at always-saved (9/9 = 1); lower values still hang in proportion
/// to size. Mirror of [`CAPTURE_RESCUE_C`] for the give-away direction.
const SELF_HANG_C: f64 = 9.0;

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
/// distinct from a multi-move tactic, which stays demotable. (The miss
/// branch draws the same obvious-vs-combination line, but with a finer
/// 2-ply material read — see [`two_ply_material_cp`] — so a capture that's
/// really a sacrifice still counts as a missable combination.)
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

/// If `line`'s first move parks the moved piece on a square where the
/// opponent can win it by an immediate capture (SEE-positive), return the
/// moved piece's standard value in pawns — the material that would hang.
/// `None` when the move is safe (no winning enemy capture on the landing
/// square). This is what the variety branch's self-hang guard keys on: a
/// tactically-blind (qsearch-0) bot is happy to drop its queen to a pawn
/// because it never sees the recapture; even a weak human doesn't. Only
/// normal moves are considered — promotions are handled by their own rescue,
/// and castling/en-passant aren't the "moved-into-a-capture" case.
fn self_hang_pawns(root: &Position, line: &SearchLine) -> Option<f64> {
    let mv = *line.pv.first()?;
    if mv.kind() != MoveKind::Normal {
        return None;
    }
    let moved = root.piece_on(mv.from())?;
    let mut scratch = root.clone();
    scratch.do_move(mv);
    let dest = mv.to();
    let opp = scratch.side_to_move();
    let opp_attackers = scratch.attackers_to(dest, scratch.occupied()) & scratch.pieces_by_color(opp);
    if opp_attackers.is_empty() {
        return None; // not attacked on its new square → safe
    }
    // Name the cheapest enemy attacker so `see_ge` resolves the optimal
    // exchange (least-valuable attacker first, our defenders included).
    let from = [
        PieceType::Pawn,
        PieceType::Knight,
        PieceType::Bishop,
        PieceType::Rook,
        PieceType::Queen,
        PieceType::King,
    ]
    .into_iter()
    .find_map(|pt| {
        let bb = opp_attackers & scratch.pieces(pt);
        if bb.any() {
            Some(bb.lsb())
        } else {
            None
        }
    })?;
    // `see_ge(.., Value(1))` ⇒ capturing our piece wins the opponent material
    // (after all recaptures), i.e. our piece is hanging. Value at risk = the
    // moved piece (the centerpiece of the lost exchange).
    if scratch.see_ge(Move::normal(from, dest), Value(1)) {
        Some(standard_piece_value_cp(moved.kind()) as f64 / WIN_MATERIAL_CP as f64)
    } else {
        None
    }
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
/// if it never settled). The settled cap keeps the count quiescent — it
/// stops once the tactics have resolved rather than counting a
/// mid-exchange snapshot.
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

/// Net material the side-to-move has (positive = up) after just the first
/// **two plies** of `line` — its own first move and the opponent's reply.
/// This is the discriminator the miss branch uses to tell an *obvious grab*
/// (already up a pawn-or-more here: a hanging piece, an even trade settled
/// in hand) from a *combination* (still even-or-down at two plies, yet the
/// settled line wins — a fork off a quiet move, or a real sacrifice that
/// pays off later). Only the obvious grab is exempt from being declined.
fn two_ply_material_cp(root: &Position, line: &SearchLine, root_stm: Color) -> i32 {
    if line.pv.is_empty() {
        return 0;
    }
    let last_ply = 1.min(line.pv.len() - 1);
    material_delta_through_ply(root, line, root_stm, last_ply)
}

/// Net material (root-stm POV, material-cp) after the first `last_ply + 1`
/// plies of `line` are played from `root`, summing captured-piece values
/// with a sign for who captured. Shared by the settled-outcome classifier
/// ([`line_material_delta_cp`], cap = settled ply) and the miss branch's
/// 2-ply sacrifice check ([`two_ply_material_cp`], cap = ply 1).
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
