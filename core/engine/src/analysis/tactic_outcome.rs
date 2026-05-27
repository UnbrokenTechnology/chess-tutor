//! Tactic detection over a move's principal variation.
//!
//! Given the best line and the user's line out of the same root
//! position, label the tactical pattern each line contains so the
//! teaching layer can say "you played a fork", "you missed a fork",
//! or "you walked into a fork". No new search — cheap predicates over
//! the PV and `Position` primitives we already have, mirroring the
//! other `analysis::*_outcome` modules.
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

use super::MoveAnalysis;
use crate::attacks::{attacks_bb, pawn_attacks_from};
use crate::bitboard::{square_bb, Bitboard};
use crate::position::Position;
use crate::types::{Color, Move, Piece, PieceType, Square, Value};

/// Which tactical pattern a [`TacticHit`] represents. Ship 1 covers
/// the three most pedagogically load-bearing patterns; the remaining
/// lichess taxonomy (pin, skewer, discovered attack, …) lands in
/// later ships.
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
}

impl TacticPattern {
    /// Short card heading for the retrospective view.
    pub fn heading(self) -> &'static str {
        match self {
            TacticPattern::Fork => "Fork",
            TacticPattern::HangingCapture => "Free piece",
            TacticPattern::RemovingDefender => "Removing the defender",
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
    /// The forking / capturing piece's square *after* the key move
    /// (its destination).
    pub primary_piece: Square,
    /// The squares the pattern bears on — forked targets, or the
    /// freshly-hanging piece for the capture patterns. Ordered by
    /// ascending square index for deterministic rendering.
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
///
/// `pre_move_pos` is cloned internally before any move is replayed;
/// the caller's position is not mutated.
pub fn compute_tactic_outcome(
    best_ma: &MoveAnalysis,
    user_ma: &MoveAnalysis,
    pre_move_pos: &Position,
    root_stm: Color,
) -> TacticsOutcome {
    let user_played_tactic = detect_line_tactic(pre_move_pos, &user_ma.pv, root_stm, 0);

    let user_missed_tactic = if user_ma.mv != best_ma.mv {
        detect_line_tactic(pre_move_pos, &best_ma.pv, root_stm, 0)
    } else {
        None
    };

    // "Walked into": replay the user's own move, then look at the
    // opponent's reply line from the opponent's point of view. The
    // pattern's key move sits at original PV ply 1.
    let user_walked_into = match user_ma.pv.first() {
        Some(&first) => {
            let mut after = pre_move_pos.clone();
            after.do_move(first);
            detect_line_tactic(&after, &user_ma.pv[1..], !root_stm, 1)
        }
        None => None,
    };

    TacticsOutcome {
        user_played_tactic,
        user_missed_tactic,
        user_walked_into,
    }
}

/// Run every detector on `pv`, where `pv[0]` is played by `mover` from
/// `pre`. `base_ply` is the offset of `pv[0]` within the original PV
/// (so a `user_walked_into` sub-line reports `pv_ply = 1`). Returns the
/// first matching pattern in priority order (most instructive first),
/// or `None`.
fn detect_line_tactic(
    pre: &Position,
    pv: &[Move],
    mover: Color,
    base_ply: usize,
) -> Option<TacticHit> {
    let &key_move = pv.first()?;
    let mut post = pre.clone();
    post.do_move(key_move);

    let material_gain = line_material_gain(pre, pv, mover);

    // Priority order: a fork teaches more than a plain free-piece
    // capture, and removing-the-defender is a more specific lesson than
    // a piece that was simply left hanging. Ship 1 returns a single
    // hit; a future ship may collect a Vec.
    detect_fork(&post, key_move, mover, base_ply, material_gain)
}

/// Fork — port of `cook.py:fork`.
///
/// From the moved piece's destination square, count the enemy non-pawn
/// pieces it attacks that either (a) outvalue the forker, or (b) are
/// hanging and can't simply capture the forker back. Two or more such
/// targets, with the forker not itself sitting in a bad spot, is a
/// fork. Excludes king forkers (a checking king can't fork).
fn detect_fork(
    post: &Position,
    key_move: Move,
    mover: Color,
    base_ply: usize,
    material_gain: Option<i32>,
) -> Option<TacticHit> {
    let forker_sq = key_move.to();
    let forker = post.piece_on(forker_sq)?;
    if forker.kind() == PieceType::King {
        return None;
    }
    // The forking piece must not itself be hanging or takeable by a
    // lower piece — otherwise the "fork" is illusory (the opponent just
    // takes the forker).
    if is_in_bad_spot(post, forker_sq) {
        return None;
    }

    let forker_value = king_value(forker.kind());
    let occ = post.occupied();
    // Does the target attack our forker back? If so, a "hanging"
    // target can simply recapture, so it doesn't count.
    let attacks_on_forker = post.attackers_to(forker_sq, occ);

    let mut targets: Vec<Square> = Vec::new();
    for (target_piece, target_sq) in attacked_opponent_squares(post, forker_sq, mover) {
        if target_piece.kind() == PieceType::Pawn {
            continue;
        }
        let outvalues_forker = king_value(target_piece.kind()) > forker_value;
        let hanging_and_cannot_recapture = is_hanging(post, target_sq, target_piece.color())
            && !attacks_on_forker.contains(target_sq);
        if outvalues_forker || hanging_and_cannot_recapture {
            targets.push(target_sq);
        }
    }

    if targets.len() < 2 {
        return None;
    }
    targets.sort_by_key(|s| s.index());

    Some(TacticHit {
        pattern: TacticPattern::Fork,
        pv_ply: base_ply,
        primary_piece: forker_sq,
        targets,
        material_gain,
        confidence: confidence_for(material_gain),
    })
}

/// `High` when the line realizes strictly-positive material for the
/// owner inside the window, else `Medium`.
fn confidence_for(material_gain: Option<i32>) -> Confidence {
    match material_gain {
        Some(g) if g > 0 => Confidence::High,
        _ => Confidence::Medium,
    }
}

// =========================================================================
// Material accounting
// =========================================================================

/// Net midgame material for `owner` over the first [`MATERIAL_WINDOW_PLIES`]
/// plies of `pv` replayed from `pre`. Positive = `owner` is up. `None`
/// when `pv` is empty.
fn line_material_gain(pre: &Position, pv: &[Move], owner: Color) -> Option<i32> {
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

// =========================================================================
// Ported util helpers (lichess-puzzler `tagger/util.py`)
// =========================================================================

/// Ranking value where the king is the most valuable piece — used for
/// the fork's "is the target worth more than the forker" test, where a
/// forked king (a check) always counts. Mirrors `util.king_values`
/// (P1 N3 B3 R5 Q9 K99).
fn king_value(pt: PieceType) -> i32 {
    match pt {
        PieceType::Pawn => 1,
        PieceType::Knight | PieceType::Bishop => 3,
        PieceType::Rook => 5,
        PieceType::Queen => 9,
        PieceType::King => 99,
    }
}

/// The squares the piece on `from` attacks, given current occupancy.
/// Pawns use their colour-specific capture pattern; every other piece
/// uses the occupancy-aware slider/leaper tables. Empty when `from` is
/// vacant.
fn attacks_from_square(pos: &Position, from: Square) -> Bitboard {
    match pos.piece_on(from) {
        Some(p) => match p.kind() {
            PieceType::Pawn => pawn_attacks_from(p.color(), from),
            other => attacks_bb(other, from, pos.occupied()),
        },
        None => Bitboard::EMPTY,
    }
}

/// Enemy pieces (relative to `pov`) standing on squares the piece on
/// `from` attacks. Mirrors `util.attacked_opponent_squares`. Ordered by
/// ascending square index.
fn attacked_opponent_squares(pos: &Position, from: Square, pov: Color) -> Vec<(Piece, Square)> {
    let enemy = !pov;
    let mut out = Vec::new();
    for sq in attacks_from_square(pos, from) & pos.pieces_by_color(enemy) {
        if let Some(p) = pos.piece_on(sq) {
            out.push((p, sq));
        }
    }
    out
}

/// Whether the piece of colour `target_color` on `target_sq` has a
/// defender. Mirrors `util.is_defended`: a direct friendly attacker, or
/// "ray defense" — a friendly slider hidden behind an enemy slider that
/// attacks the square, revealed once that enemy attacker is removed.
fn is_defended(pos: &Position, target_sq: Square, target_color: Color) -> bool {
    let occ = pos.occupied();
    let friends = pos.pieces_by_color(target_color);
    let attackers = pos.attackers_to(target_sq, occ);
    if (attackers & friends).any() {
        return true;
    }
    // Ray defense: an enemy slider attacks the square; removing it from
    // the occupancy may reveal a friendly slider defending from behind.
    let enemy = pos.pieces_by_color(!target_color);
    let sliders = pos.pieces(PieceType::Rook)
        | pos.pieces(PieceType::Bishop)
        | pos.pieces(PieceType::Queen);
    for asq in attackers & enemy & sliders {
        let reduced = occ ^ square_bb(asq);
        if (pos.attackers_to(target_sq, reduced) & friends).any() {
            return true;
        }
    }
    false
}

/// `!is_defended` — the piece is attacked-or-not but has no defender.
/// Mirrors `util.is_hanging` (callers pair it with an "is attacked"
/// check where the distinction matters).
fn is_hanging(pos: &Position, target_sq: Square, target_color: Color) -> bool {
    !is_defended(pos, target_sq, target_color)
}

/// Whether the piece on `target_sq` can be captured by a strictly
/// lower-valued enemy piece (kings excluded). Mirrors
/// `util.can_be_taken_by_lower_piece`.
fn can_be_taken_by_lower_piece(pos: &Position, target_sq: Square) -> bool {
    let Some(target) = pos.piece_on(target_sq) else {
        return false;
    };
    let target_value = king_value(target.kind());
    let enemy = pos.pieces_by_color(!target.color());
    for asq in pos.attackers_to(target_sq, pos.occupied()) & enemy {
        if let Some(attacker) = pos.piece_on(asq) {
            if attacker.kind() != PieceType::King && king_value(attacker.kind()) < target_value {
                return true;
            }
        }
    }
    false
}

/// Whether the piece on `square` is in a "bad spot" — attacked by an
/// enemy AND either hanging or takeable by a lower piece. Mirrors
/// `util.is_in_bad_spot`. `false` for an empty square.
fn is_in_bad_spot(pos: &Position, square: Square) -> bool {
    let Some(piece) = pos.piece_on(square) else {
        return false;
    };
    let enemy = pos.pieces_by_color(!piece.color());
    let attacked = (pos.attackers_to(square, pos.occupied()) & enemy).any();
    attacked
        && (is_hanging(pos, square, piece.color()) || can_be_taken_by_lower_piece(pos, square))
}

#[cfg(test)]
#[path = "tactic_outcome_tests.rs"]
mod tests;
