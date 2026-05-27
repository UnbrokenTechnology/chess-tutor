//! Tactic detection over a move's principal variation.
//!
//! Given the best line and the user's line out of the same root
//! position, label the tactical pattern each line contains so the
//! teaching layer can say "you played a fork", "you missed a fork",
//! or "you walked into a fork". No new search — cheap predicates over
//! the PV and `Position` primitives we already have, mirroring the
//! other `analysis::*_outcome` modules.
//!
//! ## Module layout
//!
//! - this `mod.rs` — the public types ([`TacticPattern`],
//!   [`Confidence`], [`TacticHit`], [`TacticsOutcome`], [`PriorMove`]),
//!   the [`compute_tactic_outcome`] entry point that assembles the
//!   three outcome slots, and the shared material-accounting /
//!   confidence helpers the detectors lean on.
//! - [`detectors`] — [`detect_line_tactic`] (the per-line priority
//!   chain) plus one `detect_*` function per [`TacticPattern`]. That's
//!   where new patterns land.
//!
//! Predicate primitives ("hanging", "bad spot", "trapped", …) live in
//! [`super::tactic_util`], shared with the trapped-piece overlay.
//!
//! ## Predicate provenance
//!
//! The per-pattern predicates are hand-transliterated from
//! lichess-puzzler's `tagger/cook.py` (`reference/lichess-puzzler/`,
//! AGPL-3.0 — never shipped, never modified). The taxonomy and the
//! shape of each test (which squares to check, the value comparisons,
//! the "bad spot" / "hanging" sub-predicates) are validated against
//! lichess's millions of tagged puzzles; mirroring them gives parity
//! with the strongest open-source benchmark. Per the idea/expression
//! dichotomy (see `CLAUDE.md`), the algorithms and heuristics are not
//! copyrightable; this is independently authored Rust, not copied
//! source. lichess's puzzle model walks a `mainline` where `pov`'s
//! moves are at the odd indices; we walk a `MoveAnalysis.pv` where
//! `pv[0]` is played by `root_stm` from `pre_move_pos`, so each
//! predicate is adapted to that framing.
//!
//! ## Three surfaces, one library
//!
//! [`compute_tactic_outcome`] returns a [`TacticsOutcome`] with three
//! independent slots, all populated from the same detector set:
//!
//! - `user_played_tactic` — a pattern fires on the user's own line.
//! - `user_missed_tactic` — a pattern fires on the engine's best line
//!   and the user chose a different move.
//! - `user_walked_into` — a pattern fires for the *opponent* on their
//!   best reply to the user's move.

mod detectors;
pub(crate) use detectors::detect_line_tactic;

use super::MoveAnalysis;
use crate::analysis::win_chances::win_chances;
use crate::position::Position;
use crate::types::{Color, Move, PieceType, Value};

/// Which tactical pattern a [`TacticHit`] represents.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TacticPattern {
    /// One piece attacks two or more enemy pieces that can't all be
    /// saved. Port of `cook.py:fork`.
    Fork,
    /// A capture of an enemy piece that was attacked and undefended.
    /// Port of `cook.py:hanging`.
    HangingCapture,
    /// A capture of the only piece defending another enemy piece,
    /// leaving that piece hanging. Port of `cook.py:capturing_defender`.
    RemovingDefender,
    /// An enemy piece with no safe square and no favourable trade out —
    /// the mover is poised to win it. Port of `cook.py:trapped_piece` /
    /// `util.is_trapped`, adapted to our single-move framing.
    TrappedPiece,
    /// An enemy piece pinned against its king, which the move exploits —
    /// either the pin stops the piece defending/attacking, or it can't
    /// flee an attack. Port of `cook.py:pin_prevents_{attack,escape}`.
    Pin,
    /// A ray piece attacks two enemy pieces in a line; the more valuable
    /// front one must move, exposing the one behind. Port of
    /// `cook.py:skewer`.
    Skewer,
    /// Moving one piece unmasks an attack from a friendly piece behind
    /// it onto an enemy target. Port of `cook.py:discovered_attack`.
    DiscoveredAttack,
    /// A check delivered by a piece other than the one that moved (the
    /// move unmasks it). Port of `cook.py:discovered_check`.
    DiscoveredCheck,
    /// The move gives check from two pieces at once — the king must
    /// move. Port of `cook.py:double_check`.
    DoubleCheck,
    /// The line deliberately gives up material (down ≥ 2 points by the
    /// mover's second move) and is sound anyway. Port of
    /// `cook.py:sacrifice`. Used only as a *standalone* hit — when a line
    /// is a winning sacrifice but no geometric pattern fires. When a
    /// geometric pattern *does* fire, the sacrifice is recorded on its
    /// [`TacticHit::sacrifice`] flag instead, so the richer lesson leads.
    Sacrifice,
    /// An in-between move: instead of an expected immediate recapture, the
    /// mover inserts a forcing move elsewhere, then takes the offered
    /// piece a move later. Port of `cook.py:intermezzo` (zwischenzug).
    Intermezzo,
    /// The mover's move lures an enemy defender *off* a duty square (or
    /// forces a capture/check that pulls it away), leaving what it guarded
    /// undefended. Port of `cook.py:deflection`.
    Deflection,
    /// The mover offers a piece that an enemy K/Q/R captures, drawing it
    /// onto a square where the mover then checks or wins it. Port of
    /// `cook.py:attraction`.
    Attraction,
    /// A defender's line to a piece is blocked by interposing a piece on
    /// the ray — by the mover (player interference) or by the opponent's
    /// own piece (self-interference) — after which the now-undefended
    /// piece falls. Port of `cook.py:interference` / `self_interference`.
    Interference,
    /// The mover vacates a square (without capturing) to clear a friendly
    /// ray piece's line, enabling the tactic. Port of `cook.py:clearance`.
    Clearance,
    /// A battery: the mover captures on a square defended through it by a
    /// friendly ray piece directly behind, which recaptures. Port of
    /// `cook.py:x_ray`.
    XRay,
}

impl TacticPattern {
    /// Short card heading for the retrospective view.
    pub fn heading(self) -> &'static str {
        match self {
            TacticPattern::Fork => "Fork",
            TacticPattern::HangingCapture => "Free piece",
            TacticPattern::RemovingDefender => "Removing the defender",
            TacticPattern::TrappedPiece => "Trapped piece",
            TacticPattern::Pin => "Pin",
            TacticPattern::Skewer => "Skewer",
            TacticPattern::DiscoveredAttack => "Discovered attack",
            TacticPattern::DiscoveredCheck => "Discovered check",
            TacticPattern::DoubleCheck => "Double check",
            TacticPattern::Sacrifice => "Sacrifice",
            TacticPattern::Intermezzo => "In-between move",
            TacticPattern::Deflection => "Deflection",
            TacticPattern::Attraction => "Attraction",
            TacticPattern::Interference => "Interference",
            TacticPattern::Clearance => "Clearance",
            TacticPattern::XRay => "X-ray",
        }
    }
}

/// How sure we are the pattern wins material — gates which surfaces
/// the hit appears on. The coaching surface (a later ship) shows
/// `High` only; `Medium` stays in the retrospective where the student
/// can study the line at leisure.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Confidence {
    /// The pattern fires AND the line realizes positive material for
    /// the tactic's owner within the first four plies.
    High,
    /// The pattern fires but material is delayed beyond four plies (a
    /// positional fork, a long combination), or no material is won at
    /// all in the window. Surfaced in the retrospective only.
    Medium,
}

/// One detected tactic: the pattern, where in the PV it fires, the
/// piece that delivers it, the targets, and how confident we are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TacticHit {
    pub pattern: TacticPattern,
    /// Ply in the analysed PV the pattern's key move occupies. `0` for
    /// the user's / best line's own move; `1` for the opponent's reply
    /// in a `user_walked_into` hit.
    pub pv_ply: usize,
    /// The forking / capturing / pinning piece's square *after* the key
    /// move (its destination). For a discovered attack or check this is
    /// the piece that *moved* (the one that unmasked the attack), which
    /// may not be the attacking piece itself.
    pub primary_piece: Square,
    /// The squares the pattern bears on — forked targets, the
    /// freshly-hanging piece for the capture patterns, the pinned/
    /// skewered enemy piece, or the checked king. Ordered by ascending
    /// square index for deterministic rendering.
    pub targets: Vec<Square>,
    /// Net material for the tactic's owner over the first four plies of
    /// the line, in engine-cp midgame. `None` when the line is too
    /// short to assess.
    pub material_gain: Option<i32>,
    pub confidence: Confidence,
    /// Whether the line is also a *sacrifice* — the owner is down ≥ 2
    /// points of material by their second move yet the combination is
    /// sound (port of `cook.py:sacrifice`). A geometric pattern (fork,
    /// pin, …) can co-occur with a sacrifice; this flag records that
    /// while the `pattern` keeps naming the richer lesson. Always `true`
    /// when `pattern == TacticPattern::Sacrifice` (the standalone case).
    pub sacrifice: bool,
}

/// The tactic story for one analysed move. Each slot is independent;
/// any combination may be present.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TacticsOutcome {
    /// A tactic the user's chosen move plays.
    pub user_played_tactic: Option<TacticHit>,
    /// A tactic on the engine's best line that the user passed up (only
    /// populated when the user's move differs from best).
    pub user_missed_tactic: Option<TacticHit>,
    /// A tactic the *opponent* gets to play on their best reply — i.e.
    /// the user walked into it.
    pub user_walked_into: Option<TacticHit>,
}

use crate::types::Square;

/// The opponent's move that produced `pre_move_pos`, paired with the
/// piece (if any) it captured. Lets the hanging-capture detector tell a
/// genuine free piece from a plain recapture: if the opponent's last
/// move just took a piece of equal-or-greater value on the same square
/// the user now captures, the user isn't winning material, they're
/// completing an exchange. This is lichess's `op_capture` guard
/// (`cook.py:hanging`), which reads the move *into* the puzzle position.
///
/// `None` at the start of a game, or when an ad-hoc caller (analysing a
/// bare FEN) has no move history — the guard is simply skipped.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PriorMove {
    /// The move the opponent played to reach `pre_move_pos`.
    pub mv: Move,
    /// The piece that move captured, or `None` if it was quiet.
    pub captured: Option<PieceType>,
}

impl PriorMove {
    /// Build from the opponent's move and the position it was played in
    /// (the position *before* `pre_move_pos`), resolving what it
    /// captured. The natural call for a retrospective worker that holds
    /// the prior board in its game history.
    pub fn new(pos_before_move: &Position, mv: Move) -> PriorMove {
        PriorMove {
            mv,
            captured: captured_kind(pos_before_move, mv),
        }
    }
}

/// Material window (plies) over which a [`Confidence::High`] hit must
/// realize its gain. Ply 0 is the key move; ply 3 is the second move
/// for the tactic's owner — enough to collect a fork's second target.
const MATERIAL_WINDOW_PLIES: usize = 4;

/// Minimum win-probability (from the mover's POV, via [`win_chances`])
/// for a material-losing line to count as a *sound* sacrifice rather
/// than a plain blunder. At or above this, the sacrifice "worked" — the
/// mover has at least full compensation — so we (a) may surface it as a
/// played `Sacrifice` tactic and (b) suppress any spurious "you walked
/// into …" claim arising from the opponent simply accepting the
/// material. Below it, the material loss is just a loss.
///
/// `0.0` = "at least equal." This is the lever for the long-standing
/// one-ply-guarantee misfire (a move that loses material at ply 0 but is
/// winning by ply 4 is a played tactic, not a missed/walked-into one).
/// It's a deliberately conservative tuning surface — the `win_chances`
/// constant was fitted by lila on NNUE evals, not our classical eval, so
/// revisit this threshold if/when the sigmoid is refit.
const SOUND_SACRIFICE_WC: f64 = 0.0;

/// Minimum win-probability the best move must gain over the user's move
/// before we call the user's move a *missed* tactic. Below this gap the
/// two moves are close enough that "you missed THE move" isn't honest.
/// Ports lichess's generator uniqueness gate *in spirit* — it uses 0.7
/// for "clearly the only move" in puzzle generation; teaching nags at a
/// lower bar but still wants a real gap. Tuning surface.
const MISS_MIN_WC_GAP: f64 = 0.15;

/// At or above this win-probability *after the user's own move*, a missed
/// improvement is noise: the student is already comfortably winning, so we
/// don't nag about a stronger line. Ports the generator's "already winning
/// / already up material" suppression. Tuning surface.
const ALREADY_WINNING_WC: f64 = 0.80;

/// "Don't nag" gate for the missed-tactic slot. Only flag a missed tactic
/// when the best move was meaningfully better than what the user played
/// (a real win-probability gap) AND the user isn't already comfortably
/// winning. Ports lichess's generator suppression/uniqueness gates in
/// spirit — they gate which positions become puzzles; we gate which
/// retrospective claims a 1200 student is worth interrupting for.
fn missed_tactic_worth_flagging(best_ma: &MoveAnalysis, user_ma: &MoveAnalysis) -> bool {
    let user_wc = win_chances(user_ma.score);
    win_chances(best_ma.score) - user_wc >= MISS_MIN_WC_GAP && user_wc < ALREADY_WINNING_WC
}

/// Compute the [`TacticsOutcome`] for a single analysed move.
///
/// - `best_ma` — the engine's top line from `pre_move_pos`.
/// - `user_ma` — the line for the move the user actually played.
/// - `pre_move_pos` — the position the user moved from (`root_stm` to
///   move).
/// - `root_stm` — the side that moved (the user's colour).
/// - `prior_move` — the opponent's move into `pre_move_pos`, if known
///   (see [`PriorMove`]). Used only to suppress recapture false
///   positives in the hanging-capture detector; pass `None` when there
///   is no move history.
///
/// `pre_move_pos` is cloned internally before any move is replayed;
/// the caller's position is not mutated.
pub fn compute_tactic_outcome(
    best_ma: &MoveAnalysis,
    user_ma: &MoveAnalysis,
    pre_move_pos: &Position,
    root_stm: Color,
    prior_move: Option<PriorMove>,
) -> TacticsOutcome {
    // The user's move is a *sound sacrifice* when the line gives material
    // away (`is_sacrifice`) yet the eval says the user is at least equal.
    // This gates two things below: synthesizing a standalone `Sacrifice`
    // played-hit, and suppressing a misfiring "you walked into …" claim.
    let user_sacrifice_sound = is_sacrifice(pre_move_pos, &user_ma.pv, root_stm)
        && win_chances(user_ma.score) >= SOUND_SACRIFICE_WC;

    let user_played_tactic = detect_line_tactic(pre_move_pos, &user_ma.pv, root_stm, 0, prior_move)
        // No geometric pattern, but a sound sacrifice — surface the
        // sacrifice itself so a winning material-down combination reads as
        // *played*, not as a blunder.
        .or_else(|| {
            user_sacrifice_sound.then(|| synthesize_sacrifice_hit(pre_move_pos, &user_ma.pv, root_stm))
        });

    let user_missed_tactic = if user_ma.mv != best_ma.mv
        && missed_tactic_worth_flagging(best_ma, user_ma)
    {
        detect_line_tactic(pre_move_pos, &best_ma.pv, root_stm, 0, prior_move)
    } else {
        None
    };

    // "Walked into": replay the user's own move, then look at the
    // opponent's reply line from the opponent's point of view. The
    // pattern's key move sits at original PV ply 1. The move *into* that
    // sub-line's start position is the user's own move, so that — not
    // `prior_move` — is the relevant recapture context here.
    //
    // Suppressed when the user played a sound sacrifice: the opponent
    // "winning" the offered material is the point of the sacrifice, not a
    // tactic the user blundered into. This is the one-ply-guarantee
    // misfire fix — without it, the opponent accepting a sound sac reads
    // as "you walked into a free-piece capture."
    let user_walked_into = match user_ma.pv.first() {
        Some(&first) if !user_sacrifice_sound => {
            let mut after = pre_move_pos.clone();
            let sub_prior = PriorMove::new(pre_move_pos, first);
            after.do_move(first);
            detect_line_tactic(&after, &user_ma.pv[1..], !root_stm, 1, Some(sub_prior))
        }
        _ => None,
    };

    TacticsOutcome {
        user_played_tactic,
        user_missed_tactic,
        user_walked_into,
    }
}

/// Build a standalone [`TacticPattern::Sacrifice`] hit for a line with no
/// geometric pattern. `primary_piece` is where the combination opens (the
/// mover's first destination); there is no single geometric target, so
/// `targets` is empty. `material_gain` is the (negative) net over the
/// window — honest about the material given. Caller must already have
/// confirmed [`is_sacrifice`] and soundness.
fn synthesize_sacrifice_hit(pre: &Position, pv: &[Move], mover: Color) -> TacticHit {
    let material_gain = line_material_gain(pre, pv, mover);
    TacticHit {
        pattern: TacticPattern::Sacrifice,
        pv_ply: 0,
        primary_piece: pv[0].to(),
        targets: Vec::new(),
        material_gain,
        confidence: confidence_for(material_gain),
        sacrifice: true,
    }
}

// =========================================================================
// Shared helpers (used by the detectors)
// =========================================================================

/// `High` when the line realizes strictly-positive material for the
/// owner inside the window, else `Medium`.
pub(super) fn confidence_for(material_gain: Option<i32>) -> Confidence {
    match material_gain {
        Some(g) if g > 0 => Confidence::High,
        _ => Confidence::Medium,
    }
}

/// Net midgame material for `owner` over the first [`MATERIAL_WINDOW_PLIES`]
/// plies of `pv` replayed from `pre`. Positive = `owner` is up. `None`
/// when `pv` is empty.
pub(super) fn line_material_gain(pre: &Position, pv: &[Move], owner: Color) -> Option<i32> {
    if pv.is_empty() {
        return None;
    }
    let mut scratch = pre.clone();
    let mut net = 0;
    for &mv in pv.iter().take(MATERIAL_WINDOW_PLIES) {
        if let Some((captor, captured_value)) = capture_value(&scratch, mv) {
            net += if captor == owner {
                captured_value
            } else {
                -captured_value
            };
        }
        scratch.do_move(mv);
    }
    Some(net)
}

/// Material balance for `color` in lichess point units (P1 N3 B3 R5 Q9;
/// kings are excluded — they always cancel). Positive = `color` is ahead.
/// Mirrors `util.material_diff`.
fn material_diff_points(pos: &Position, color: Color) -> i32 {
    const VALUES: [(PieceType, i32); 5] = [
        (PieceType::Pawn, 1),
        (PieceType::Knight, 3),
        (PieceType::Bishop, 3),
        (PieceType::Rook, 5),
        (PieceType::Queen, 9),
    ];
    VALUES
        .iter()
        .map(|&(pt, v)| (pos.count(color, pt) as i32 - pos.count(!color, pt) as i32) * v)
        .sum()
}

/// Whether `pv` (played from `pre`, with `pv[0]` by `mover`) is a
/// *sacrifice* for `mover` — port of `cook.py:sacrifice`.
///
/// True when, by the mover's **second** move or later, `mover` is down
/// ≥ 2 points of material relative to the pre-move balance, and no
/// *opponent* reply in the line is a promotion. (A promoting opponent
/// reply means the material deficit came from the opponent queening, not
/// from the mover giving material — lichess excludes that case.)
///
/// Framing map: lichess walks a `mainline` whose `pov` moves sit at odd
/// indices, baselined on the position after the opponent's setup move.
/// Our `pv` has the mover's own moves at even indices, baselined on the
/// true root `pre`. So "mover's second move onward" is even indices
/// `≥ 2`, and the opponent's replies (the promotion guard's subject) are
/// the odd indices.
///
/// This is purely material/structural — it does not check that the
/// sacrifice is *sound*. The caller gates soundness with
/// [`super::win_chances`] on the line's eval before surfacing it as a
/// played tactic.
pub(super) fn is_sacrifice(pre: &Position, pv: &[Move], mover: Color) -> bool {
    use crate::types::MoveKind;
    let initial = material_diff_points(pre, mover);
    let mut scratch = pre.clone();
    let mut went_down = false;
    for (i, &mv) in pv.iter().enumerate() {
        // Odd index = opponent reply. A promoting one disqualifies the line.
        if i % 2 == 1 && mv.kind() == MoveKind::Promotion {
            return false;
        }
        scratch.do_move(mv);
        // After the mover's second move onward (even index ≥ 2): is the
        // mover down ≥ 2 points versus the pre-move balance?
        if i >= 2 && i % 2 == 0 && material_diff_points(&scratch, mover) - initial <= -2 {
            went_down = true;
        }
    }
    went_down
}

/// `(captor colour, captured midgame value)` for a capturing move,
/// resolved against the pre-move position. `None` for non-captures.
/// En passant captures a pawn; castling is never a capture.
fn capture_value(pos: &Position, mv: Move) -> Option<(Color, i32)> {
    use crate::types::MoveKind;
    let captor = pos.piece_on(mv.from())?.color();
    match mv.kind() {
        MoveKind::Castling => None,
        MoveKind::EnPassant => Some((captor, Value::mg_of_piece(PieceType::Pawn).0)),
        MoveKind::Normal | MoveKind::Promotion => {
            let captured = pos.piece_on(mv.to())?;
            Some((captor, Value::mg_of_piece(captured.kind()).0))
        }
    }
}

/// The kind of piece `mv` captures, resolved against the position it's
/// played in. `None` for a quiet move or castling. En passant always
/// takes a pawn.
fn captured_kind(pos: &Position, mv: Move) -> Option<PieceType> {
    use crate::types::MoveKind;
    match mv.kind() {
        MoveKind::Castling => None,
        MoveKind::EnPassant => Some(PieceType::Pawn),
        MoveKind::Normal | MoveKind::Promotion => pos.piece_on(mv.to()).map(|p| p.kind()),
    }
}

#[cfg(test)]
mod tests;
