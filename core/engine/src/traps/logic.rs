//! Trap scanning, validation, invariant checks, and trigger matching,
//! split out of the traps module. Data types live in the parent module.

use crate::attacks::{attacks_bb, between_bb};
use crate::bitboard::Bitboard;
use crate::movegen::legal_moves_vec;
use crate::position::Position;
use crate::san;
use crate::types::{Color, Move, MoveKind, Piece, PieceType, Square, Value};

use super::*;


// =========================================================================
// Public scanning API
// =========================================================================

/// Scan every legal move for the side-to-move in `pos`. Return the
/// ones that would trigger a library trap — i.e. "if you play this,
/// your opponent gets a known refutation against you".
///
/// Intended for pre-move UI hints. Expensive for wide positions
/// (quadratic in candidate count × library size, with invariant /
/// SEE / main-line checks per candidate), but legal-move counts are
/// bounded and the library is small so this stays cheap enough to
/// run on every position change.
pub fn scan_threats(pos: &Position) -> Vec<TrapThreatened> {
    let side_to_move = pos.side_to_move();
    let mut out = Vec::new();

    for trap in LIBRARY {
        if trap.trigger.mover != side_to_move {
            continue;
        }
        let mut scratch = pos.clone();
        let legal = legal_moves_vec(&mut scratch);
        for mv in legal {
            if !trap.trigger.matches(side_to_move, pos, mv) {
                continue;
            }
            let mut after = pos.clone();
            let candidate_san = san::format(&after, mv);
            let _ = after.do_move(mv);
            if let Some(hit) = validate_and_build_hit(&after, trap) {
                out.push(TrapThreatened {
                    candidate_uci: uci_of(mv),
                    candidate_san,
                    hit,
                });
            }
        }
    }

    out
}

/// Scan the library for traps triggered by the just-played move.
/// `pos` is the position **after** the move landed (so the side-to-
/// move is the punisher's side). Returns one entry per firing trap.
pub fn scan_after_move(
    pos: &Position,
    last_move_mover: Color,
    last_move_piece_type: PieceType,
    last_move_from: Square,
    last_move_to: Square,
) -> Vec<(&'static TrapEntry, TrapHit)> {
    let mut hits = Vec::new();
    for trap in LIBRARY {
        if !trap.trigger.matches_parts(
            last_move_mover,
            last_move_piece_type,
            last_move_from,
            last_move_to,
        ) {
            continue;
        }
        if let Some(hit) = validate_and_build_hit(pos, trap) {
            hits.push((trap, hit));
        }
    }
    hits
}

/// Advance a pending trap by a played move. `pre_pos` is the
/// position **before** the move was played (needed to resolve the
/// scripted SAN into a concrete [`Move`]). The `pending` cursor is
/// mutated in place; callers check [`TrapEvent::is_terminal`] on
/// the returned event to decide whether to drop their `Option<
/// PendingTrap>`.
pub fn advance_pending(pending: &mut PendingTrap, pre_pos: &Position, played: Move) -> TrapEvent {
    match pending.expectation {
        TrapExpectation::PunisherNext(node) => {
            if !scripted_matches_played(pre_pos, node.san, played) {
                return TrapEvent::PunisherMissed {
                    trap: pending.entry,
                    expected_san: node.san,
                };
            }
            // Punisher executed the scripted move.
            if node.defender_options.is_empty() {
                // Terminal punisher node — tree has played out.
                return TrapEvent::TreeComplete {
                    trap: pending.entry,
                    gain_cp: node.terminal_gain_cp,
                };
            }
            pending.expectation = TrapExpectation::DefenderNext(node);
            TrapEvent::PunisherExecuted {
                trap: pending.entry,
                move_san: node.san,
            }
        }
        TrapExpectation::DefenderNext(parent) => {
            for option in parent.defender_options {
                if !scripted_matches_played(pre_pos, option.san, played) {
                    continue;
                }
                // Defender picked a scripted option. Advance the
                // cursor only if there's a follow-up; a `None`
                // follow-up leaves the event terminal and the caller
                // drops the pending trap.
                if let Some(next) = option.punisher_follow_up {
                    pending.expectation = TrapExpectation::PunisherNext(next);
                }
                return TrapEvent::DefenderInTree {
                    trap: pending.entry,
                    option,
                };
            }
            TrapEvent::DefenderEscaped {
                trap: pending.entry,
            }
        }
    }
}

/// True when the scripted SAN, parsed in `pos`, resolves to the same
/// [`Move`] as `played`. Used to match scripted tree nodes against
/// actual played moves.
fn scripted_matches_played(pos: &Position, scripted_san: &str, played: Move) -> bool {
    let mut scratch = pos.clone();
    match san::parse(&mut scratch, scripted_san) {
        Ok(mv) => mv == played,
        Err(_) => false,
    }
}

/// Evaluate a single invariant against a position. Made public so
/// UIs can render the full "why this works" list by calling each
/// invariant individually and showing which passed.
pub fn check_invariant(pos: &Position, kind: &InvariantKind) -> bool {
    match *kind {
        InvariantKind::PieceOn { square, piece } => pos.piece_on(square) == Some(piece),

        InvariantKind::SquareEmpty { square } => pos.piece_on(square).is_none(),

        InvariantKind::AllEmpty { mask } => (pos.occupied() & mask).is_empty(),

        InvariantKind::AnyPieceOfColor { color, square } => {
            pos.pieces_by_color(color).contains(square)
        }

        InvariantKind::PieceCount {
            color,
            piece_type,
            count,
        } => pos.count(color, piece_type) == count,

        InvariantKind::NoPieceInMask {
            color,
            piece_type,
            mask,
        } => (pos.pieces_of(color, piece_type) & mask).is_empty(),

        InvariantKind::AttackerCountByColor {
            color,
            square,
            count,
        } => {
            let attackers = pos.attackers_to(square, pos.occupied()) & pos.pieces_by_color(color);
            attackers.popcount() == count
        }

        InvariantKind::NotAttackedBy { color, square } => {
            let attackers = pos.attackers_to(square, pos.occupied()) & pos.pieces_by_color(color);
            attackers.is_empty()
        }

        InvariantKind::AttackersSubsetOf {
            color,
            square,
            allowed,
        } => {
            let attackers = pos.attackers_to(square, pos.occupied()) & pos.pieces_by_color(color);
            (attackers & !allowed).is_empty()
        }

        InvariantKind::AttackersEqual {
            color,
            square,
            mask,
        } => {
            let attackers = pos.attackers_to(square, pos.occupied()) & pos.pieces_by_color(color);
            attackers == mask
        }

        InvariantKind::RayClear { from, to } => {
            // `attacks_bb(QUEEN, from, occ)` returns the squares a
            // queen on `from` would attack through current
            // occupancy; membership of `to` means the two squares
            // are both aligned and separated only by empty squares.
            attacks_bb(PieceType::Queen, from, pos.occupied()).contains(to)
                && between_bb(from, to) & pos.occupied() == Bitboard::EMPTY
        }
    }
}

// =========================================================================
// Validator pipeline (invariants → SEE → main-line verify)
// =========================================================================

/// Run the three validation gates and, if they all pass, build the
/// [`TrapHit`] that describes the scripted refutation. `pos` must be
/// the post-trigger position — i.e. after the trigger move has been
/// played and it's the punisher's turn.
fn validate_and_build_hit(pos: &Position, trap: &TrapEntry) -> Option<TrapHit> {
    // Gate 2: invariants.
    for inv in trap.invariants {
        if !check_invariant(pos, &inv.kind) {
            return None;
        }
    }
    // Gates 3 and 4 happen together inside main-line verification:
    // at every defender branch we SEE-check the unscripted
    // alternatives and also verify the scripted move is legal.
    let (main_line_san, main_line_gain_cp) = walk_main_line(pos, trap.punisher, trap.root)?;
    Some(TrapHit {
        name: trap.name.to_string(),
        description: trap.description.to_string(),
        main_line_san,
        main_line_gain_cp,
        punisher: trap.punisher,
    })
}

/// Walk the main line from `start`, following the first
/// `is_main_defense` branch at each defender node. Returns
/// `(san sequence, material gain from `pos` to the terminal)` or
/// `None` if any scripted move fails to parse / is illegal, or the
/// SEE backstop detects a better unscripted defender move.
fn walk_main_line(
    pos: &Position,
    punisher: Color,
    start: &'static PunisherMove,
) -> Option<(Vec<String>, i32)> {
    let mut scratch = pos.clone();
    let material_before = material_delta_for(&scratch, punisher);
    let mut line = Vec::new();
    let mut node = start;

    loop {
        let mv = san::parse(&mut scratch, node.san).ok()?;
        let _ = scratch.do_move(mv);
        line.push(node.san.to_string());
        if node.defender_options.is_empty() {
            break;
        }

        // SEE backstop on the position where the defender is to move.
        if defender_has_better_unscripted_move(&scratch, node.defender_options) {
            return None;
        }

        let option = node.defender_options.iter().find(|o| o.is_main_defense)?;
        let reply = san::parse(&mut scratch, option.san).ok()?;
        let _ = scratch.do_move(reply);
        line.push(option.san.to_string());

        match option.punisher_follow_up {
            Some(next) => node = next,
            None => break,
        }
    }

    let material_after = material_delta_for(&scratch, punisher);
    Some((line, material_after - material_before))
}

/// True when the defender — in the position handed in — has a legal
/// move that isn't in the library's option list AND outscores the
/// scripted main-defense by more than 50 cp on static exchange
/// evaluation. Fires when the author missed a defender resource.
///
/// The 50 cp tolerance absorbs minor SEE wobble; at a pawn-and-a-
/// half difference we're confident a real player would pick the
/// unscripted move instead and the library's premise has broken.
fn defender_has_better_unscripted_move(pos: &Position, options: &[DefenderOption]) -> bool {
    const TOLERANCE: i32 = 50;

    let mut scratch = pos.clone();
    let legal = legal_moves_vec(&mut scratch);

    // Collect the moves that correspond to scripted options.
    let mut scripted: Vec<Move> = Vec::with_capacity(options.len());
    for opt in options {
        let mut parse_scratch = pos.clone();
        if let Ok(mv) = san::parse(&mut parse_scratch, opt.san) {
            scripted.push(mv);
        }
    }

    // SEE of the scripted main-defense — our baseline.
    let main_defense_san = match options.iter().find(|o| o.is_main_defense) {
        Some(o) => o.san,
        None => return false, // nothing to compare against
    };
    let mut md_scratch = pos.clone();
    let main_defense_mv = match san::parse(&mut md_scratch, main_defense_san) {
        Ok(mv) => mv,
        Err(_) => return false,
    };
    let main_defense_see = see_score(pos, main_defense_mv);

    for mv in &legal {
        if scripted.contains(mv) {
            continue;
        }
        let unscripted_see = see_score(pos, *mv);
        if unscripted_see > main_defense_see + TOLERANCE {
            return true;
        }
    }
    false
}

/// Rough SEE "score" for a move: 0 if the move is a quiet (non-
/// capture), otherwise the threshold bisection pinned to pawn-
/// valued increments. Built from our [`Position::see_ge`] which is
/// a boolean "≥ threshold" test.
///
/// Returns a centipawn value in `[-2000, +2000]` (queen-ish bounds).
/// Exact to the nearest pawn; that resolution is fine at the 50 cp
/// tolerance we apply upstream.
fn see_score(pos: &Position, mv: Move) -> i32 {
    if pos.piece_on(mv.to()).is_none() {
        return 0;
    }
    // Bisect on pawn-valued thresholds. We don't need precision —
    // the caller just wants "is this materially better than the
    // scripted alternative by more than half a pawn".
    let candidates = [-2000, -1000, -500, -200, -100, 0, 100, 200, 500, 1000, 2000];
    let mut best = -2000;
    for &t in &candidates {
        if pos.see_ge(mv, Value(t)) {
            best = t;
        }
    }
    best
}

/// Material delta for `color` in **conventional centipawns**
/// (pawn = 100, knight = 300, bishop = 325, rook = 500, queen = 900)
/// — the units a teaching UI and hand-written `terminal_gain_cp`
/// fixtures speak in. The engine's internal piece values (pawn EG
/// = 213, etc.) are calibrated for the classical evaluator and
/// aren't what a student means when they say "you lost a rook".
fn material_delta_for(pos: &Position, color: Color) -> i32 {
    const VALUES: [(PieceType, i32); 5] = [
        (PieceType::Pawn, 100),
        (PieceType::Knight, 300),
        (PieceType::Bishop, 325),
        (PieceType::Rook, 500),
        (PieceType::Queen, 900),
    ];
    let mut total = 0i32;
    for (pt, value) in VALUES {
        let ours = pos.count(color, pt) as i32;
        let theirs = pos.count(!color, pt) as i32;
        total += (ours - theirs) * value;
    }
    total
}

fn uci_of(mv: Move) -> String {
    let mut s = String::with_capacity(5);
    s.push_str(&mv.from().to_algebraic());
    s.push_str(&mv.to().to_algebraic());
    if mv.kind() == MoveKind::Promotion {
        s.push(match mv.promoted_to() {
            PieceType::Knight => 'n',
            PieceType::Bishop => 'b',
            PieceType::Rook => 'r',
            PieceType::Queen => 'q',
            _ => '?',
        });
    }
    s
}

// =========================================================================
// Trigger matching
// =========================================================================

impl TriggerPattern {
    /// True if `mv`, played in `pos` by `mover`, matches this trigger.
    /// `pos` is used to resolve the moving piece's type.
    fn matches(&self, mover: Color, pos: &Position, mv: Move) -> bool {
        if mover != self.mover || mv.to() != self.to {
            return false;
        }
        if let Some(required) = self.from {
            if mv.from() != required {
                return false;
            }
        }
        match pos.piece_on(mv.from()) {
            Some(p) => piece_type_of(p) == self.piece_type,
            None => false,
        }
    }

    /// Pattern-only match when the caller already has the piece type
    /// in hand (e.g. replaying a move log).
    fn matches_parts(&self, mover: Color, piece_type: PieceType, from: Square, to: Square) -> bool {
        mover == self.mover
            && piece_type == self.piece_type
            && to == self.to
            && self.from.is_none_or(|required| required == from)
    }
}

fn piece_type_of(piece: Piece) -> PieceType {
    match piece {
        Piece::WhitePawn | Piece::BlackPawn => PieceType::Pawn,
        Piece::WhiteKnight | Piece::BlackKnight => PieceType::Knight,
        Piece::WhiteBishop | Piece::BlackBishop => PieceType::Bishop,
        Piece::WhiteRook | Piece::BlackRook => PieceType::Rook,
        Piece::WhiteQueen | Piece::BlackQueen => PieceType::Queen,
        Piece::WhiteKing | Piece::BlackKing => PieceType::King,
    }
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
