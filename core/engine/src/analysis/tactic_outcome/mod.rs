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
    let user_played_tactic = detect_line_tactic(pre_move_pos, &user_ma.pv, root_stm, 0, prior_move);

    let user_missed_tactic = if user_ma.mv != best_ma.mv {
        detect_line_tactic(pre_move_pos, &best_ma.pv, root_stm, 0, prior_move)
    } else {
        None
    };

    // "Walked into": replay the user's own move, then look at the
    // opponent's reply line from the opponent's point of view. The
    // pattern's key move sits at original PV ply 1. The move *into* that
    // sub-line's start position is the user's own move, so that — not
    // `prior_move` — is the relevant recapture context here.
    let user_walked_into = match user_ma.pv.first() {
        Some(&first) => {
            let mut after = pre_move_pos.clone();
            let sub_prior = PriorMove::new(pre_move_pos, first);
            after.do_move(first);
            detect_line_tactic(&after, &user_ma.pv[1..], !root_stm, 1, Some(sub_prior))
        }
        None => None,
    };

    TacticsOutcome {
        user_played_tactic,
        user_missed_tactic,
        user_walked_into,
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
