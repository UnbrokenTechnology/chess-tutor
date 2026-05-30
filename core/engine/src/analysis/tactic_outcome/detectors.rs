//! Per-pattern tactic detectors and the per-line priority chain.
//!
//! Each `detect_*` is a hand-transliteration of a `cook.py` predicate,
//! adapted to our single-move framing (`pv[0]` played by `mover` from
//! `pre`; `post` is the position right after it). See the parent module's
//! `//!` for provenance and the `super::tactic_util` primitives they share.
//!
//! ## Framing note that recurs below
//!
//! After the key move it is the *opponent's* turn, so in `post` the side
//! to move is `!mover`. Several predicates (trapped, pin, the checks) are
//! naturally expressed about the opponent's pieces / king, which is
//! exactly `post.side_to_move()`'s material.

use super::{confidence_for, line_material_gain, PriorMove, TacticHit, TacticPattern};
use crate::analysis::list_hanging;
use crate::analysis::tactic_util::{
    attacked_opponent_squares, attacks_from_square, is_hanging, is_in_bad_spot, is_trapped,
    king_value,
};
use crate::attacks::{attacks_bb, between_bb, line_bb};
use crate::bitboard::square_bb;
use crate::movegen::legal_moves_vec;
use crate::position::Position;
use crate::types::{Color, Move, MoveKind, PieceType, Square};

#[cfg(test)]
#[path = "detectors_tests.rs"]
mod tests;

/// Rook / bishop / queen — the pieces whose attacks travel along a ray, so
/// they can pin, skewer, and be unmasked in a discovered attack.
fn is_ray_piece(pt: PieceType) -> bool {
    matches!(pt, PieceType::Rook | PieceType::Bishop | PieceType::Queen)
}

/// Run every detector on `pv`, where `pv[0]` is played by `mover` from
/// `pre`. `base_ply` is the offset of `pv[0]` within the original PV
/// (so a `user_walked_into` sub-line reports `pv_ply = 1`). Returns the
/// first matching pattern in priority order (most instructive first),
/// or `None`.
pub(crate) fn detect_line_tactic(
    pre: &Position,
    pv: &[Move],
    mover: Color,
    base_ply: usize,
    prior: Option<PriorMove>,
) -> Option<TacticHit> {
    let &key_move = pv.first()?;
    let mut post = pre.clone();
    post.do_move(key_move);

    let material_gain = line_material_gain(pre, pv, mover);

    // Terminal-node scan: does the whole line force a checkmate the mover
    // delivers, and if so what is its named geometry? Recorded on the hit's
    // `mate_pattern` (independent of the geometric `pattern`), or — when no
    // geometric pattern fires — synthesized as a standalone `Checkmate` hit.
    let mate = super::mate::detect_mate_pattern(pre, pv, mover);

    // Multi-ply patterns (wave 4) read several plies of the line, so build
    // the board sequence once and share it: `boards[0] == pre`, `boards[i]`
    // = after `pv[0..i]`. Capped to the early window so a named tactic stays
    // attributable to the user's move.
    let boards = line_boards(pre, pv, WAVE4_MAX_PLIES);

    // Priority order: a fork teaches more than a plain free-piece capture,
    // and removing-the-defender is a more specific lesson than a piece left
    // hanging. The capture/threat patterns come first (concrete material),
    // then the geometric/forcing patterns. Within the new wave: double
    // check before discovered check (the stronger statement when both
    // hold); the king-targeting checks before the material-line patterns.
    // A single hit is returned; a future ship may collect a Vec. Ordering
    // here is a tuning surface — when two patterns fire on one line, this
    // decides which lesson the student sees.
    detect_fork(&post, key_move, mover, base_ply, material_gain)
        .or_else(|| detect_removing_defender(pre, &post, key_move, mover, base_ply, material_gain))
        .or_else(|| detect_hanging_capture(pre, key_move, mover, base_ply, material_gain, prior))
        .or_else(|| detect_trapped_piece(&post, base_ply, material_gain))
        .or_else(|| detect_double_check(&post, key_move, mover, base_ply, material_gain))
        .or_else(|| detect_discovered_check(&post, key_move, mover, base_ply, material_gain))
        .or_else(|| detect_skewer(&post, key_move, mover, base_ply, material_gain))
        .or_else(|| detect_discovered_attack(&post, key_move, mover, base_ply, material_gain))
        .or_else(|| detect_pin(&post, key_move, mover, base_ply, material_gain))
        // Wave-4 multi-ply patterns, in the agreed priority order. These fire
        // only when no single-move pattern above claims the line — the
        // immediate pattern is always the primary lesson.
        .or_else(|| detect_intermezzo(&boards, pv, mover, base_ply, material_gain, prior))
        .or_else(|| detect_deflection(&boards, pv, mover, base_ply, material_gain))
        .or_else(|| detect_attraction(&boards, pv, mover, base_ply, material_gain))
        .or_else(|| detect_interference(&boards, pv, mover, base_ply, material_gain))
        .or_else(|| detect_clearance(&boards, pv, mover, base_ply, material_gain))
        .or_else(|| detect_x_ray(&boards, pv, mover, base_ply, material_gain))
        // Low-priority motifs (wave 6): only claim the slot when nothing
        // richer fired.
        .or_else(|| detect_attacking_f2_f7(pre, &post, key_move, mover, base_ply, material_gain))
        .or_else(|| detect_under_promotion(&boards, pv, base_ply, material_gain))
        // A geometric pattern can co-occur with a sacrifice and/or end in a
        // named checkmate; record both on the hit's flags while the pattern
        // keeps naming the richer lesson. (The *standalone* sacrifice case —
        // no geometric pattern — is synthesized in `compute_tactic_outcome`,
        // which has the eval needed to confirm soundness.)
        .map(|mut hit| {
            hit.sacrifice = super::is_sacrifice(pre, pv, mover);
            hit.mate_pattern = mate.map(|m| m.pattern);
            hit
        })
        // A forced mate with no geometric pattern: surface it as a standalone
        // `Checkmate` hit carrying the named geometry.
        .or_else(|| mate.map(|m| super::mate::synthesize_checkmate_hit(pre, pv, mover, base_ply, m)))
        // Stamp the move occupying the hit's ply within this line — `pv[0]`
        // for a single-move pattern, the resolving move for a multi-ply one.
        // Escape detection and CLI move-naming read this.
        .map(|mut hit| {
            hit.key_move = hit.pv_ply.checked_sub(base_ply).and_then(|i| pv.get(i)).copied();
            hit
        })
}

// =========================================================================
// Ship 1 patterns
// =========================================================================

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
        sacrifice: false,
        mate_pattern: None,
        key_move: None,
    })
}

/// Hanging-piece capture — port of `cook.py:hanging`.
///
/// The key move captures a non-pawn enemy piece that was attacked and
/// undefended in the pre-move position — a free piece. We reuse
/// [`list_hanging`] (the same "attacked AND no friendly defender" scan
/// that backs [`crate::analysis::ThreatsOutcome`]) on the pre-move
/// position, so the definition of "hanging" is identical across the
/// teaching layer.
///
/// `prior` carries the opponent's move into the pre-move position. It
/// implements lichess's `op_capture` guard: when that move just took a
/// piece worth at least as much on the very square we now capture, this
/// is a recapture completing an exchange — not a won free piece — so we
/// don't flag it. When `prior` is `None` (no move history) the guard is
/// skipped.
fn detect_hanging_capture(
    pre: &Position,
    key_move: Move,
    mover: Color,
    base_ply: usize,
    material_gain: Option<i32>,
    prior: Option<PriorMove>,
) -> Option<TacticHit> {
    if !pre.is_capture(key_move) {
        return None;
    }
    let to = key_move.to();
    // En passant captures a pawn (and leaves `to` empty pre-move); the
    // pawn exclusion below covers it either way.
    let captured = pre.piece_on(to)?;
    if captured.kind() == PieceType::Pawn {
        return None;
    }
    // The captured piece must have been attacked-and-undefended before
    // the capture — i.e. on the enemy's hanging list.
    let hanging = list_hanging(pre, !mover);
    if !hanging.iter().any(|h| h.location.square == to) {
        return None;
    }

    // Recapture guard: if the opponent's last move took an equal-or-
    // greater piece on this same square, the "hang" is just the far side
    // of a trade we initiated, not free material.
    if let Some(prior) = prior {
        if prior.mv.to() == to {
            if let Some(prior_captured) = prior.captured {
                if king_value(prior_captured) >= king_value(captured.kind()) {
                    return None;
                }
            }
        }
    }

    Some(TacticHit {
        pattern: TacticPattern::HangingCapture,
        pv_ply: base_ply,
        primary_piece: to,
        targets: vec![to],
        material_gain,
        confidence: confidence_for(material_gain),
        sacrifice: false,
        mate_pattern: None,
        key_move: None,
    })
}

/// Removing the defender — port of `cook.py:capturing_defender`.
///
/// The key move captures an enemy piece X that was the sole defender of
/// another enemy piece Y we were already pressuring; with X gone, Y is
/// left hanging. We detect this by diffing the enemy's hanging list
/// across the capture: a piece Y that X attacked pre-move, was *not*
/// hanging before, but *is* hanging after, was being held up by X
/// alone. Excludes king capturers (matching lichess).
///
/// Checked ahead of [`detect_hanging_capture`] in the priority chain:
/// when a capture is both a free piece *and* unguards another, the
/// removing-the-defender framing is the richer lesson.
fn detect_removing_defender(
    pre: &Position,
    post: &Position,
    key_move: Move,
    mover: Color,
    base_ply: usize,
    material_gain: Option<i32>,
) -> Option<TacticHit> {
    if !pre.is_capture(key_move) {
        return None;
    }
    let from = key_move.from();
    if pre.piece_on(from)?.kind() == PieceType::King {
        return None;
    }
    let to = key_move.to();
    let enemy = !mover;

    // The squares the captured defender X (on `to` pre-move) was
    // guarding: enemy pieces it attacked.
    let guarded_by_x = attacks_from_square(pre, to) & pre.pieces_by_color(enemy);
    if guarded_by_x.is_empty() {
        return None;
    }

    // A piece is "freed" if it's hanging now but wasn't before — the
    // capture is what exposed it.
    let pre_hanging: Vec<Square> = list_hanging(pre, enemy)
        .iter()
        .map(|h| h.location.square)
        .collect();

    let mut freed: Vec<Square> = list_hanging(post, enemy)
        .into_iter()
        .map(|h| h.location.square)
        .filter(|&y| guarded_by_x.contains(y) && !pre_hanging.contains(&y))
        .collect();

    if freed.is_empty() {
        return None;
    }
    freed.sort_by_key(|s| s.index());

    Some(TacticHit {
        pattern: TacticPattern::RemovingDefender,
        pv_ply: base_ply,
        primary_piece: to,
        targets: freed,
        material_gain,
        confidence: confidence_for(material_gain),
        sacrifice: false,
        mate_pattern: None,
        key_move: None,
    })
}

/// Trapped piece — port of `cook.py:trapped_piece` / `util.is_trapped`,
/// adapted to our single-move framing.
///
/// After the key move it is the *opponent's* turn (the side that didn't
/// just move). lichess tests trapped-ness on a board where the trapped
/// piece's owner is to move; here that owner is exactly
/// `post.side_to_move()`. So we scan the opponent's pieces and report the
/// first one [`is_trapped`] reports — a piece with no safe square and no
/// favourable trade out, which the mover is poised to win. This fires for
/// all three slots: the user's move trapping an enemy piece, the best
/// line doing so, and (via the walked-into sub-line, where the opponent
/// is the mover) the user's own piece getting trapped.
///
/// `is_trapped` already filters by colour, piece kind, check, and pins,
/// so the scan can hand it every enemy square. Iteration is LSB-first, so
/// the lowest-indexed trapped square is reported — deterministic.
fn detect_trapped_piece(
    post: &Position,
    base_ply: usize,
    material_gain: Option<i32>,
) -> Option<TacticHit> {
    let victim = post.side_to_move();
    let trapped_sq = post
        .pieces_by_color(victim)
        .into_iter()
        .find(|&sq| is_trapped(post, sq))?;

    Some(TacticHit {
        pattern: TacticPattern::TrappedPiece,
        pv_ply: base_ply,
        primary_piece: trapped_sq,
        targets: vec![trapped_sq],
        material_gain,
        confidence: confidence_for(material_gain),
        sacrifice: false,
        mate_pattern: None,
        key_move: None,
    })
}

// =========================================================================
// Wave 2 patterns (Pin, Skewer, Discovered attack, Discovered/Double check)
// =========================================================================

/// Double check — port of `cook.py:double_check`.
///
/// The key move leaves the opponent's king attacked by two pieces at
/// once. A double check can only be answered by a king move, so it is
/// always forcing. `primary_piece` is the piece that moved (one of the
/// two checkers); `targets` is the checked king.
fn detect_double_check(
    post: &Position,
    key_move: Move,
    mover: Color,
    base_ply: usize,
    material_gain: Option<i32>,
) -> Option<TacticHit> {
    if post.checkers().popcount() < 2 {
        return None;
    }
    let king = post.king_square(!mover);
    Some(TacticHit {
        pattern: TacticPattern::DoubleCheck,
        pv_ply: base_ply,
        primary_piece: key_move.to(),
        targets: vec![king],
        material_gain,
        confidence: confidence_for(material_gain),
        sacrifice: false,
        mate_pattern: None,
        key_move: None,
    })
}

/// Discovered check — port of `cook.py:discovered_check`.
///
/// The opponent's king is in check after the move, but the moved piece is
/// *not* one of the checkers: the check is delivered by a piece the move
/// unmasked. (A double check is reported first by [`detect_double_check`].)
fn detect_discovered_check(
    post: &Position,
    key_move: Move,
    mover: Color,
    base_ply: usize,
    material_gain: Option<i32>,
) -> Option<TacticHit> {
    let checkers = post.checkers();
    if checkers.is_empty() || checkers.contains(key_move.to()) {
        return None;
    }
    let king = post.king_square(!mover);
    Some(TacticHit {
        pattern: TacticPattern::DiscoveredCheck,
        pv_ply: base_ply,
        // The piece that moved (and unmasked the check) is the actor; the
        // revealed checker sits in `checkers` for any caller that wants it.
        primary_piece: key_move.to(),
        targets: vec![king],
        material_gain,
        confidence: confidence_for(material_gain),
        sacrifice: false,
        mate_pattern: None,
        key_move: None,
    })
}

/// Skewer — port of `cook.py:skewer`.
///
/// The moved ray piece attacks an enemy piece F that is forced to move —
/// it is the king, or it is in a bad spot — with a *less valuable* enemy
/// piece B directly behind it on the same line. When F steps aside, B
/// falls. `targets` is `[F, B]`; `primary_piece` is the skewering piece.
///
/// "Behind" is found by x-ray: remove F from the occupancy and see what
/// the ray piece newly reaches on that line — the next blocker is B.
fn detect_skewer(
    post: &Position,
    key_move: Move,
    mover: Color,
    base_ply: usize,
    material_gain: Option<i32>,
) -> Option<TacticHit> {
    let r_sq = key_move.to();
    let r = post.piece_on(r_sq)?;
    if !is_ray_piece(r.kind()) {
        return None;
    }
    // A skewering piece that is itself hanging is no threat.
    if is_in_bad_spot(post, r_sq) {
        return None;
    }
    let enemy = post.pieces_by_color(!mover);
    let occ = post.occupied();
    let with = attacks_bb(r.kind(), r_sq, occ);

    for f_sq in with & enemy {
        let Some(f) = post.piece_on(f_sq) else {
            continue;
        };
        // F must be forced to move: it's the king, or it's in a bad spot
        // (attacked, and hanging or takeable by something cheaper).
        if f.kind() != PieceType::King && !is_in_bad_spot(post, f_sq) {
            continue;
        }
        // The piece directly behind F on this ray, revealed by removing F.
        let revealed = attacks_bb(r.kind(), r_sq, occ ^ square_bb(f_sq)) & !with;
        for b_sq in revealed & enemy {
            let Some(b) = post.piece_on(b_sq) else {
                continue;
            };
            if king_value(f.kind()) > king_value(b.kind()) {
                let mut targets = vec![f_sq, b_sq];
                targets.sort_by_key(|s| s.index());
                return Some(TacticHit {
                    pattern: TacticPattern::Skewer,
                    pv_ply: base_ply,
                    primary_piece: r_sq,
                    targets,
                    material_gain,
                    confidence: confidence_for(material_gain),
                    sacrifice: false,
                    mate_pattern: None,
                    key_move: None,
                });
            }
        }
    }
    None
}

/// Discovered attack — adapted from `cook.py:discovered_attack`.
///
/// Moving the key piece off a line unmasks a *different* friendly ray
/// piece's attack on an enemy target. We look for a friendly ray piece R
/// (not the one that just moved) whose line to an enemy target T passes
/// through the square the move vacated (`from`), where in the post-move
/// position R attacks T. Targets that are pawns, or the king (that's a
/// discovered *check*, reported earlier), are excluded; the revealed
/// target must outvalue R or sit in a bad spot, so the discovery actually
/// threatens material.
fn detect_discovered_attack(
    post: &Position,
    key_move: Move,
    mover: Color,
    base_ply: usize,
    material_gain: Option<i32>,
) -> Option<TacticHit> {
    let from = key_move.from();
    let to = key_move.to();
    let occ = post.occupied();
    let ours = post.pieces_by_color(mover);
    let enemy = post.pieces_by_color(!mover);
    let sliders = (post.pieces(PieceType::Rook)
        | post.pieces(PieceType::Bishop)
        | post.pieces(PieceType::Queen))
        & ours;

    for r_sq in sliders {
        if r_sq == to {
            continue; // the piece that moved isn't the unmasked attacker
        }
        let Some(r) = post.piece_on(r_sq) else {
            continue;
        };
        // A revealed attacker that is itself hanging is no real threat.
        if is_in_bad_spot(post, r_sq) {
            continue;
        }
        for t_sq in attacks_bb(r.kind(), r_sq, occ) & enemy {
            // The move must have unblocked this line: the vacated square
            // lies strictly between the slider and its target.
            if !between_bb(r_sq, t_sq).contains(from) {
                continue;
            }
            let Some(t) = post.piece_on(t_sq) else {
                continue;
            };
            if t.kind() == PieceType::Pawn || t.kind() == PieceType::King {
                continue;
            }
            if king_value(t.kind()) > king_value(r.kind()) || is_in_bad_spot(post, t_sq) {
                return Some(TacticHit {
                    pattern: TacticPattern::DiscoveredAttack,
                    pv_ply: base_ply,
                    primary_piece: to,
                    targets: vec![t_sq],
                    material_gain,
                    confidence: confidence_for(material_gain),
                    sacrifice: false,
                    mate_pattern: None,
                    key_move: None,
                });
            }
        }
    }
    None
}

/// Pin — port of `cook.py:pin_prevents_attack` / `pin_prevents_escape`.
///
/// After the move, an enemy piece P is pinned against its own king. Two
/// ways the pin is a tactic:
///
/// - **prevents escape**: P is attacked by one of our pieces worth less
///   than P. Because P can't move off the pin line, it can't flee — we
///   win material. (We relax lichess's "attacker on the pin line"
///   condition: a pinned piece attacked by anything cheaper is winnable,
///   since it can't run regardless of where the attacker stands.)
/// - **prevents attack**: P, if it could move, would attack one of our
///   pieces (off the pin line) that outvalues P or is hanging — but the
///   pin neutralises that threat.
///
/// `targets` is the pinned piece; `primary_piece` is the piece that moved.
fn detect_pin(
    post: &Position,
    key_move: Move,
    mover: Color,
    base_ply: usize,
    material_gain: Option<i32>,
) -> Option<TacticHit> {
    let enemy = !mover;
    let enemy_king = post.king_square(enemy);
    let occ = post.occupied();
    let ours = post.pieces_by_color(mover);
    let pinned = post.blockers_for_king(enemy) & post.pieces_by_color(enemy);

    for p_sq in pinned {
        let Some(p) = post.piece_on(p_sq) else {
            continue;
        };
        let p_value = king_value(p.kind());
        let pin_ray = line_bb(p_sq, enemy_king);

        // prevents escape: attacked by one of our cheaper pieces.
        let attacked_by_cheaper = (post.attackers_to(p_sq, occ) & ours)
            .into_iter()
            .any(|a_sq| {
                post.piece_on(a_sq)
                    .is_some_and(|a| king_value(a.kind()) < p_value)
            });
        if attacked_by_cheaper {
            return Some(pin_hit(key_move, p_sq, base_ply, material_gain));
        }

        // prevents attack: P would hit one of our valuable/hanging pieces
        // off the pin line.
        for t_sq in attacks_from_square(post, p_sq) & ours {
            if pin_ray.contains(t_sq) {
                continue;
            }
            let Some(t) = post.piece_on(t_sq) else {
                continue;
            };
            if king_value(t.kind()) > p_value || is_hanging(post, t_sq, mover) {
                return Some(pin_hit(key_move, p_sq, base_ply, material_gain));
            }
        }
    }
    None
}

fn pin_hit(
    key_move: Move,
    pinned_sq: Square,
    base_ply: usize,
    material_gain: Option<i32>,
) -> TacticHit {
    TacticHit {
        pattern: TacticPattern::Pin,
        pv_ply: base_ply,
        primary_piece: key_move.to(),
        targets: vec![pinned_sq],
        material_gain,
        confidence: confidence_for(material_gain),
        sacrifice: false,
        mate_pattern: None,
        key_move: None,
    }
}

// =========================================================================
// Wave 4 patterns (multi-ply: read several plies of the line)
//
// Unlike the single-key-move patterns above, these describe a *sequence*:
// the mover's move sets something up that resolves a move or two later.
// lichess walks its `mainline` with `.parent` / `.parent.parent`
// navigation; we mirror that by replaying the `pv` into a `boards` vector
// (`boards[0] == pre`, `boards[i]` = after `pv[0..i]`) and indexing it.
// The scan is bounded to the early window so a named tactic stays
// attributable to the user's move.
// =========================================================================

/// Plies of the line wave-4 detectors look at. Bounds cost and keeps the
/// lesson near the user's move. Covers a resolution at the mover's 2nd
/// move (`pv[2]`) or 3rd move (`pv[4]`), plus attraction's follow-up.
const WAVE4_MAX_PLIES: usize = 5;

/// The `pv` indices a multi-ply pattern can resolve on — the mover's 2nd
/// move (`pv[2]`) or 3rd move (`pv[4]`). lichess scans every solver move;
/// we bound to the first two past the opener.
const RESOLVE_PLIES: [usize; 2] = [2, 4];

/// Replay `pv` from `pre`: `boards[0] == pre`, `boards[i]` = position after
/// `pv[0..i]`, capped at `max_plies` moves.
fn line_boards(pre: &Position, pv: &[Move], max_plies: usize) -> Vec<Position> {
    let n = pv.len().min(max_plies);
    let mut boards = Vec::with_capacity(n + 1);
    let mut cur = pre.clone();
    boards.push(cur.clone());
    for &mv in pv.iter().take(n) {
        cur.do_move(mv);
        boards.push(cur.clone());
    }
    boards
}

fn wave4_hit(
    pattern: TacticPattern,
    pv_ply: usize,
    primary_piece: Square,
    targets: Vec<Square>,
    material_gain: Option<i32>,
) -> TacticHit {
    TacticHit {
        pattern,
        pv_ply,
        primary_piece,
        targets,
        material_gain,
        confidence: confidence_for(material_gain),
        sacrifice: false,
        mate_pattern: None,
        key_move: None,
    }
}

/// Intermezzo (zwischenzug) — port of `cook.py:intermezzo`.
///
/// The opponent's move into `pre` (`prior`) captured on a square. Instead
/// of recapturing at once, the mover inserts a forcing move (`pv[0]`), the
/// opponent replies (`pv[1]`), and only then takes the offered piece
/// (`pv[2]`). The recapture was legal immediately, the in-between moves
/// left the square uncontested, and the original capture was real. Needs
/// the prior move; without history it can't fire.
fn detect_intermezzo(
    boards: &[Position],
    pv: &[Move],
    mover: Color,
    base_ply: usize,
    material_gain: Option<i32>,
    prior: Option<PriorMove>,
) -> Option<TacticHit> {
    let prior = prior?;
    if prior.captured.is_none() || pv.len() < 3 || boards.len() < 3 {
        return None;
    }
    let capture_square = pv[2].to();
    // The delayed move is a capture, on the square the opponent took on.
    if !boards[2].is_capture(pv[2]) || prior.mv.to() != capture_square {
        return None;
    }
    // The in-between move wasn't itself the recapture.
    if pv[0].to() == capture_square {
        return None;
    }
    // The opponent's reply didn't contest the square (its piece wasn't
    // already attacking it).
    let occ = boards[1].occupied();
    let op_attackers =
        boards[1].attackers_to(capture_square, occ) & boards[1].pieces_by_color(!mover);
    if op_attackers.contains(pv[1].from()) {
        return None;
    }
    // The recapture was available immediately (legal in `pre`).
    let mut scratch = boards[0].clone();
    let recapture_available = legal_moves_vec(&mut scratch)
        .iter()
        .any(|m| m.from() == pv[2].from() && m.to() == capture_square);
    if !recapture_available {
        return None;
    }
    Some(wave4_hit(
        TacticPattern::Intermezzo,
        base_ply + 2,
        capture_square,
        vec![capture_square],
        material_gain,
    ))
}

/// Deflection — port of `cook.py:deflection`.
///
/// The resolving move lands on a square an enemy piece *used to guard*, but
/// which a forcing earlier move (a check, or a recapture the opponent had
/// to make) pulled that guard away from. The guard attacked the square
/// before moving and no longer does after.
fn detect_deflection(
    boards: &[Position],
    pv: &[Move],
    _mover: Color,
    base_ply: usize,
    material_gain: Option<i32>,
) -> Option<TacticHit> {
    for &k in &RESOLVE_PLIES {
        if pv.len() <= k || boards.len() <= k + 1 {
            continue;
        }
        let node = pv[k];
        let square = node.to();
        let is_promo = node.kind() == MoveKind::Promotion;
        let captured = boards[k].piece_on(square);
        if captured.is_none() && !is_promo {
            continue;
        }
        let Some(capturing) = boards[k + 1].piece_on(square).map(|p| p.kind()) else {
            continue;
        };
        // Capturing a strictly more valuable piece is just winning material.
        if let Some(c) = captured {
            if king_value(c.kind()) > king_value(capturing) {
                continue;
            }
        }
        let op_move = pv[k - 1];
        let player_move = pv[k - 2];
        // (a) the mover's earlier move wasn't a clearly-winning capture.
        //     (lichess compares a piece value to a piece-type ordinal here —
        //     a likely bug; we read the sensible "didn't win material".)
        let prev_capture = boards[k - 2].piece_on(player_move.to());
        let Some(moved1) = boards[k - 1].piece_on(player_move.to()).map(|p| p.kind()) else {
            continue;
        };
        let move1_not_winning =
            prev_capture.is_none_or(|c| king_value(c.kind()) < king_value(moved1));
        if !move1_not_winning {
            continue;
        }
        // (b,c) the deflection square isn't where the previous two moves landed.
        if square == op_move.to() || square == player_move.to() {
            continue;
        }
        // (d) the opponent's move was forced: it recaptured on the mover's
        //     move-1 square, or that move gave check.
        if !(op_move.to() == player_move.to() || boards[k - 1].checkers().any()) {
            continue;
        }
        // (e) the deflected piece (at its origin) attacked the square — or the
        //     promotion variant (same file, the piece guarded the push square).
        let guarded_before = attacks_from_square(&boards[k - 1], op_move.from()).contains(square);
        let promo_variant = is_promo
            && square.file() == op_move.from().file()
            && attacks_from_square(&boards[k - 1], op_move.from()).contains(node.from());
        if !(guarded_before || promo_variant) {
            continue;
        }
        // (f) after moving, the deflected piece no longer attacks the square.
        if attacks_from_square(&boards[k], op_move.to()).contains(square) {
            continue;
        }
        return Some(wave4_hit(
            TacticPattern::Deflection,
            base_ply + k,
            square,
            vec![square],
            material_gain,
        ));
    }
    None
}

/// Attraction — port of `cook.py:attraction`.
///
/// The mover puts a piece on a square (`pv[0]`); an enemy K/Q/R captures it
/// (`pv[1]`, drawn onto that square); the mover then attacks the square
/// (`pv[2]`). If the attracted piece is the king the attack is a check
/// (done); otherwise the mover captures the square a move later (`pv[4]`).
fn detect_attraction(
    boards: &[Position],
    pv: &[Move],
    mover: Color,
    base_ply: usize,
    material_gain: Option<i32>,
) -> Option<TacticHit> {
    if pv.len() < 3 || boards.len() < 4 {
        return None;
    }
    let square = pv[0].to();
    // The opponent captures on the mover's square.
    if pv[1].to() != square {
        return None;
    }
    // The attracted piece (now on `square`) is an enemy K/Q/R.
    let attracted = boards[2].piece_on(square)?;
    if attracted.color() != !mover
        || !matches!(
            attracted.kind(),
            PieceType::King | PieceType::Queen | PieceType::Rook
        )
    {
        return None;
    }
    // The mover then attacks `square` with the piece it just moved (`pv[2]`).
    let occ = boards[3].occupied();
    let attackers = boards[3].attackers_to(square, occ) & boards[3].pieces_by_color(mover);
    if !attackers.contains(pv[2].to()) {
        return None;
    }
    let fire = || {
        Some(wave4_hit(
            TacticPattern::Attraction,
            base_ply,
            square,
            vec![square],
            material_gain,
        ))
    };
    if attracted.kind() == PieceType::King {
        return fire(); // attacking the king's square is a check
    }
    // Otherwise the mover must later capture on the square.
    if pv.len() >= 5 && pv[4].to() == square {
        return fire();
    }
    None
}

/// Interference — port of `cook.py:interference` (player) and
/// `self_interference` (opponent). The mover captures a piece that hangs
/// only because a defender's ray to it was blocked by an interposed piece:
/// the mover's own (player interference) or the opponent's own
/// (self-interference).
fn detect_interference(
    boards: &[Position],
    pv: &[Move],
    mover: Color,
    base_ply: usize,
    material_gain: Option<i32>,
) -> Option<TacticHit> {
    let enemy = !mover;
    for &k in &RESOLVE_PLIES {
        if pv.len() <= k || boards.len() <= k + 1 {
            continue;
        }
        let square = pv[k].to();
        let before = &boards[k];
        let Some(captured) = before.piece_on(square) else {
            continue;
        };
        if captured.color() != enemy || !is_hanging(before, square, enemy) {
            continue;
        }
        // Self-interference: the defender stood (and attacked the square) in
        // the board after the mover's previous move; the opponent's reply
        // interposed on its ray.
        if ray_defender_blocked(&boards[k - 1], square, enemy, pv[k - 1].to()) {
            return Some(interference_hit(square, base_ply + k, material_gain));
        }
        // Player interference: the defender stood in the line's earlier board;
        // the mover's own previous move interposed on its ray. (lichess also
        // requires the capture square differ from the opponent's last move.)
        if square != pv[k - 1].to()
            && ray_defender_blocked(&boards[k - 2], square, enemy, pv[k - 2].to())
        {
            return Some(interference_hit(square, base_ply + k, material_gain));
        }
    }
    None
}

/// Whether some ray piece of `defender_color` attacks `square` in `board`
/// and `interpose_sq` lies strictly between them (so interposing there cuts
/// the defense).
fn ray_defender_blocked(
    board: &Position,
    square: Square,
    defender_color: Color,
    interpose_sq: Square,
) -> bool {
    let occ = board.occupied();
    for d_sq in board.attackers_to(square, occ) & board.pieces_by_color(defender_color) {
        if board
            .piece_on(d_sq)
            .is_some_and(|p| is_ray_piece(p.kind()))
            && between_bb(square, d_sq).contains(interpose_sq)
        {
            return true;
        }
    }
    false
}

fn interference_hit(square: Square, pv_ply: usize, material_gain: Option<i32>) -> TacticHit {
    wave4_hit(
        TacticPattern::Interference,
        pv_ply,
        square,
        vec![square],
        material_gain,
    )
}

/// Clearance — port of `cook.py:clearance`. The mover moves a ray piece to
/// a square (no capture); a forcing earlier move had vacated a square on
/// that ray (or its destination), clearing the line that makes it work.
fn detect_clearance(
    boards: &[Position],
    pv: &[Move],
    _mover: Color,
    base_ply: usize,
    material_gain: Option<i32>,
) -> Option<TacticHit> {
    for &k in &RESOLVE_PLIES {
        if pv.len() <= k || boards.len() <= k + 1 {
            continue;
        }
        let node = pv[k];
        // A non-capturing move of a ray piece.
        if boards[k].piece_on(node.to()).is_some() {
            continue;
        }
        let Some(piece) = boards[k + 1].piece_on(node.to()) else {
            continue;
        };
        if !is_ray_piece(piece.kind()) {
            continue;
        }
        let prev = pv[k - 2]; // the mover's earlier (clearing) move
        if prev.kind() == MoveKind::Promotion
            || prev.to() == node.from()
            || prev.to() == node.to()
            || boards[k].checkers().any()
        {
            continue;
        }
        // If the move gives check, the opponent's reply mustn't have been a
        // king move (else it's a king walk, not a clearance).
        if boards[k + 1].checkers().any()
            && boards[k]
                .piece_on(pv[k - 1].to())
                .is_some_and(|p| p.kind() == PieceType::King)
        {
            continue;
        }
        // The cleared-from square is the destination, or lies on the ray.
        let on_ray = prev.from() == node.to()
            || between_bb(node.from(), node.to()).contains(prev.from());
        if !on_ray {
            continue;
        }
        // The clearing move either vacated an empty-in-`pre` square (quiet
        // repositioning) or left its piece in a bad spot.
        let vacated_was_quiet = boards[k - 2].piece_on(prev.to()).is_none();
        let cleared_piece_bad = is_in_bad_spot(&boards[k - 1], prev.to());
        if !(vacated_was_quiet || cleared_piece_bad) {
            continue;
        }
        return Some(wave4_hit(
            TacticPattern::Clearance,
            base_ply + k,
            node.to(),
            vec![node.to()],
            material_gain,
        ));
    }
    None
}

/// X-ray / battery — port of `cook.py:x_ray`. A run of captures on one
/// square: the mover captures, the opponent recaptures from a square that
/// lay between the mover's next attacker and the target, and the mover
/// recaptures through that line.
fn detect_x_ray(
    boards: &[Position],
    pv: &[Move],
    _mover: Color,
    base_ply: usize,
    material_gain: Option<i32>,
) -> Option<TacticHit> {
    for &k in &RESOLVE_PLIES {
        if pv.len() <= k || boards.len() <= k + 1 {
            continue;
        }
        let node = pv[k];
        if !boards[k].is_capture(node) {
            continue;
        }
        let op = pv[k - 1];
        // The opponent's recapture landed on the same square, and wasn't a king.
        if op.to() != node.to()
            || boards[k]
                .piece_on(op.to())
                .is_some_and(|p| p.kind() == PieceType::King)
        {
            continue;
        }
        // The mover's earlier move also resolved on that square.
        if pv[k - 2].to() != op.to() {
            continue;
        }
        // The recapturer came from between the mover's final attacker and the
        // square — it was x-rayed.
        if between_bb(node.from(), node.to()).contains(op.from()) {
            return Some(wave4_hit(
                TacticPattern::XRay,
                base_ply + k,
                node.to(),
                vec![node.to()],
                material_gain,
            ));
        }
    }
    None
}

// =========================================================================
// Wave 6 motifs (low priority — claim the slot only when nothing else fires)
// =========================================================================

/// Attacking f2/f7 — port of `cook.py:attacking_f2_f7`.
///
/// The key move captures on f2 or f7 — the square beside an uncastled enemy
/// king's home — with that king still on e1/e8. The classic beginner hit on
/// the weakest square in front of the king.
fn detect_attacking_f2_f7(
    pre: &Position,
    post: &Position,
    key_move: Move,
    mover: Color,
    base_ply: usize,
    material_gain: Option<i32>,
) -> Option<TacticHit> {
    if !pre.is_capture(key_move) {
        return None;
    }
    // f7 sits next to e8 (Black's king home); f2 next to e1 (White's).
    let king_sq = match key_move.to() {
        Square::F7 => Square::E8,
        Square::F2 => Square::E1,
        _ => return None,
    };
    let king = post.piece_on(king_sq)?;
    if king.kind() != PieceType::King || king.color() != !mover {
        return None;
    }
    Some(TacticHit {
        pattern: TacticPattern::AttackingF2F7,
        pv_ply: base_ply,
        primary_piece: key_move.to(),
        targets: vec![key_move.to()],
        material_gain,
        confidence: confidence_for(material_gain),
        sacrifice: false,
        mate_pattern: None,
        key_move: None,
    })
}

/// Under-promotion — port of `cook.py:under_promotion`.
///
/// Walk the mover's moves in the early window. The first that delivers
/// checkmate makes this an under-promotion only if it was a knight promotion
/// (the classic `=N#` a queen couldn't give); a non-knight move that mates is
/// *not* one (and ends the scan). Otherwise, the first promotion to anything
/// but a queen is an under-promotion.
fn detect_under_promotion(
    boards: &[Position],
    pv: &[Move],
    base_ply: usize,
    material_gain: Option<i32>,
) -> Option<TacticHit> {
    // The mover's moves are at even offsets within this line.
    let mut i = 0;
    while i < pv.len() && i + 1 < boards.len() {
        let mv = pv[i];
        if super::is_checkmate(&boards[i + 1]) {
            return (mv.kind() == MoveKind::Promotion && mv.promoted_to() == PieceType::Knight)
                .then(|| under_promo_hit(mv, base_ply + i, material_gain));
        }
        if mv.kind() == MoveKind::Promotion && mv.promoted_to() != PieceType::Queen {
            return Some(under_promo_hit(mv, base_ply + i, material_gain));
        }
        i += 2;
    }
    None
}

fn under_promo_hit(mv: Move, pv_ply: usize, material_gain: Option<i32>) -> TacticHit {
    TacticHit {
        pattern: TacticPattern::UnderPromotion,
        pv_ply,
        primary_piece: mv.to(),
        targets: vec![mv.to()],
        material_gain,
        confidence: confidence_for(material_gain),
        sacrifice: false,
        mate_pattern: None,
        key_move: None,
    }
}
