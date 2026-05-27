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
use crate::position::Position;
use crate::types::{Color, Move, PieceType, Square};

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
    }
}
