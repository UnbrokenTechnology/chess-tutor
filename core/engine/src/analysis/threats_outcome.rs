//! [`ThreatsOutcome`] — hanging, SEE-losing, and Stockfish-pattern
//! pressure, for both sides, at the position immediately after the
//! user's move vs the pre-move baseline.
//!
//! Three threat categories:
//!
//! - **Hanging** — attacked by ≥ 1 enemy piece AND undefended. The
//!   simplest 400–1200 player pattern: "opponent takes for free."
//! - **SEE-losing** — attacked AND defended, but the
//!   static-exchange evaluator says the opponent still wins
//!   strictly-positive material if they initiate the exchange.
//!   Classic 1000–1400 case: our piece is defended once but
//!   attacked by two lower-value enemies (fork the defender with an
//!   overload).
//! - **Pressured** — neither hanging nor SEE-losing, but facing a
//!   Stockfish-evaluator threat pattern (minor-on-major,
//!   rook-on-queen, safe-pawn-threat) that forces the piece to
//!   move or concede positional ground.

use super::{post_user_move, MoveAnalysis};
use crate::bitboard::Bitboard;
use crate::position::Position;
use crate::types::{Color, Move, PieceType, Square, Value};

/// One square + the piece on it. Colour is implicit from the
/// containing context (which list/field the location appears in).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PieceLocation {
    pub square: Square,
    pub piece: PieceType,
}

/// A hanging piece plus every enemy piece attacking it. The hanging
/// piece's colour is implicit from which list on [`ThreatsOutcome`]
/// contains this entry (`ours_hanging` vs `theirs_hanging`); the
/// attackers are always the opposite colour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HangingPiece {
    pub location: PieceLocation,
    /// Enemy pieces attacking `location.square`. Non-empty by
    /// construction — a piece with zero enemy attackers wouldn't be
    /// on the hanging list. Ordered by ascending square index so
    /// renderers produce deterministic output.
    pub attackers: Vec<PieceLocation>,
}

/// A piece under "pressure": attacked in a way that would force it
/// to move or concede material, but *not* already on the hanging
/// or SEE-losing lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PressuredPiece {
    pub location: PieceLocation,
    /// Enemy pieces applying this specific pressure kind. Ordered
    /// by ascending square index for deterministic output.
    pub attackers: Vec<PieceLocation>,
    pub kind: PressureKind,
}

/// Which Stockfish-evaluator threat pattern this pressure entry
/// represents.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PressureKind {
    /// A knight or bishop attacks an enemy rook or queen.
    MinorOnMajor,
    /// A rook attacks the enemy queen.
    RookOnQueen,
    /// A pawn on a safe square attacks an enemy non-pawn piece.
    SafePawnThreat,
}

/// Structured snapshot of threatened pieces in the position
/// immediately after the user's move, compared against the pre-move
/// baseline.
///
/// `*_delta` counts compare against the same measure at the
/// pre-move position, so callers can answer "did this move
/// *create* a threat on our side, or *resolve* one?"
///
/// POV convention: `ours_*` fields refer to the user's side
/// (`root_stm`); `theirs_*` fields refer to the opponent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreatsOutcome {
    /// Our pieces after the user's move that are attacked and
    /// undefended.
    pub ours_hanging: Vec<HangingPiece>,
    /// Their pieces after the user's move that are attacked and
    /// undefended. **Raw / static snapshot** — does not check
    /// whether the opponent's next move can refute the threat. Use
    /// [`theirs_hanging_guaranteed`](Self::theirs_hanging_guaranteed)
    /// for teaching surfaces.
    pub theirs_hanging: Vec<HangingPiece>,
    /// Our pieces after the user's move that are defended but still
    /// lose material in an SEE-assessed exchange initiated by the
    /// enemy.
    pub ours_see_losing: Vec<HangingPiece>,
    /// Their pieces where the same SEE assessment favours us. Raw /
    /// static, like `theirs_hanging`. Use
    /// [`theirs_see_losing_guaranteed`](Self::theirs_see_losing_guaranteed)
    /// for teaching surfaces.
    pub theirs_see_losing: Vec<HangingPiece>,
    /// Subset of [`theirs_hanging`](Self::theirs_hanging) that
    /// survives *every* legal opponent response — the target piece
    /// stays on its square AND our cheapest attacker remains
    /// SEE-positive after every reply. This is the honest "you can
    /// win material" surface: phrasing the static list as a winnable
    /// claim mis-teaches when the opponent's reply (defend, move the
    /// target, capture an attacker, or pose a bigger counter-threat)
    /// refutes the win.
    pub theirs_hanging_guaranteed: Vec<HangingPiece>,
    /// Subset of [`theirs_see_losing`](Self::theirs_see_losing) that
    /// survives every legal opponent response, by the same logic as
    /// `theirs_hanging_guaranteed`.
    pub theirs_see_losing_guaranteed: Vec<HangingPiece>,
    /// Our pieces under Stockfish-style positional pressure.
    pub ours_pressured: Vec<PressuredPiece>,
    /// Their pieces under the same form of positional pressure from
    /// our side.
    pub theirs_pressured: Vec<PressuredPiece>,
    /// `ours_hanging.len() − (count at pre-move)`. Positive means
    /// this move *created* a hanging piece on our side.
    pub ours_hanging_delta: i32,
    pub theirs_hanging_delta: i32,
    pub ours_see_losing_delta: i32,
    pub theirs_see_losing_delta: i32,
    pub ours_pressured_delta: i32,
    pub theirs_pressured_delta: i32,
}

/// Compute hanging-piece + SEE-losing + Stockfish-pressure
/// comparisons against `pre_move_pos`, measured at the position
/// immediately after the user's move.
///
/// Pieces are deemed hanging if `attackers_to(sq, occupied) & enemy
/// != empty` AND `attackers_to(sq, occupied) & ours == empty`.
/// Kings excluded — "hanging king" isn't a meaningful teaching
/// concept.
pub fn compute_threats_outcome(
    ma: &MoveAnalysis,
    pre_move_pos: &Position,
    root_stm: Color,
) -> ThreatsOutcome {
    // Pre-move baseline: each category's count at the position
    // before the user moved.
    let pre_ours_hang = list_hanging(pre_move_pos, root_stm).len();
    let pre_theirs_hang = list_hanging(pre_move_pos, !root_stm).len();
    let pre_ours_see = list_see_losing(pre_move_pos, root_stm).len();
    let pre_theirs_see = list_see_losing(pre_move_pos, !root_stm).len();
    let pre_ours_pressured = list_pressured(pre_move_pos, root_stm).len();
    let pre_theirs_pressured = list_pressured(pre_move_pos, !root_stm).len();

    let scratch = post_user_move(pre_move_pos, ma);

    let ours_hanging = list_hanging(&scratch, root_stm);
    let theirs_hanging = list_hanging(&scratch, !root_stm);
    let ours_see_losing = list_see_losing(&scratch, root_stm);
    let theirs_see_losing = list_see_losing(&scratch, !root_stm);
    let ours_pressured = list_pressured(&scratch, root_stm);
    let theirs_pressured = list_pressured(&scratch, !root_stm);

    let theirs_hanging_guaranteed =
        filter_guaranteed_targets(&scratch, &theirs_hanging, root_stm);
    let theirs_see_losing_guaranteed =
        filter_guaranteed_targets(&scratch, &theirs_see_losing, root_stm);

    let ours_hanging_delta = ours_hanging.len() as i32 - pre_ours_hang as i32;
    let theirs_hanging_delta = theirs_hanging.len() as i32 - pre_theirs_hang as i32;
    let ours_see_losing_delta = ours_see_losing.len() as i32 - pre_ours_see as i32;
    let theirs_see_losing_delta = theirs_see_losing.len() as i32 - pre_theirs_see as i32;
    let ours_pressured_delta = ours_pressured.len() as i32 - pre_ours_pressured as i32;
    let theirs_pressured_delta = theirs_pressured.len() as i32 - pre_theirs_pressured as i32;

    ThreatsOutcome {
        ours_hanging,
        theirs_hanging,
        ours_see_losing,
        theirs_see_losing,
        theirs_hanging_guaranteed,
        theirs_see_losing_guaranteed,
        ours_pressured,
        theirs_pressured,
        ours_hanging_delta,
        theirs_hanging_delta,
        ours_see_losing_delta,
        theirs_see_losing_delta,
        ours_pressured_delta,
        theirs_pressured_delta,
    }
}

/// Return the subset of `targets` that are *guaranteed* winnable on
/// `our_color`'s next move — i.e. for every legal opponent response,
/// the target piece is still on its square AND our cheapest attacker
/// still has a SEE-positive capture there.
///
/// `pos_after_user_move` is the position immediately after the user
/// moved, with the opponent to move. Each `HangingPiece` in `targets`
/// must already describe an opponent piece in that position
/// (typically [`ThreatsOutcome::theirs_hanging`] or
/// [`ThreatsOutcome::theirs_see_losing`]).
///
/// Why this matters: the raw lists are a static after-our-move
/// snapshot. A piece can look "hanging" right after we attack it but
/// the opponent's response (defend, move the piece, capture an
/// attacker, or force us to deal with a bigger threat) refutes the
/// win. Surfacing "you can win material" before this filter produces
/// false positives that mis-teach the student.
///
/// Edge cases:
/// - Opponent has no legal moves (stalemate/mate): every target is
///   trivially "guaranteed". Acceptable; the game is over and the
///   material claim is moot.
/// - Opponent captures one of our attackers: re-running
///   `attackers_to`/SEE on the post-response position handles this
///   automatically.
/// - Opponent moves the target to a still-attacked square: counts as
///   refuted. We don't chase the target to its new square; teaching
///   value is "the original threat persists no matter what."
///
/// Known false-positive (not yet handled): when the "hanging" target
/// is actually a sacrifice setup — opponent left it there to bait
/// our capture into a prepared tactic (fork, pin, mating net). Every
/// passive opponent response leaves the piece capturable, so we
/// pass the static check; but the right move for us is to *not*
/// take, because the next move after our capture lands us in the
/// tactic. Detecting this needs a one-ply search of *our* response
/// (take vs. refuse) and an SEE/eval check on the position after
/// the opponent's follow-up. See memory note on the sacrifice-tactic
/// misfire for future work.
pub fn filter_guaranteed_targets(
    pos_after_user_move: &Position,
    targets: &[HangingPiece],
    our_color: Color,
) -> Vec<HangingPiece> {
    targets
        .iter()
        .filter(|h| is_target_guaranteed(pos_after_user_move, h, our_color))
        .cloned()
        .collect()
}

fn is_target_guaranteed(
    pos_after_user_move: &Position,
    hanging: &HangingPiece,
    our_color: Color,
) -> bool {
    let target_sq = hanging.location.square;
    let mut scratch_for_movegen = pos_after_user_move.clone();
    let legal = crate::movegen::legal_moves_vec(&mut scratch_for_movegen);
    if legal.is_empty() {
        // Stalemate / mate — no responses to refute the claim. Edge
        // case; game is over so the "win material" framing is moot.
        return true;
    }
    for mv in legal {
        let mut scratch = pos_after_user_move.clone();
        scratch.do_move(mv);
        if !target_still_winnable(&scratch, target_sq, our_color) {
            return false;
        }
    }
    true
}

fn target_still_winnable(pos: &Position, target_sq: Square, our_color: Color) -> bool {
    let Some(piece_on_target) = pos.piece_on(target_sq) else {
        // Target moved off its square.
        return false;
    };
    if piece_on_target.color() == our_color {
        // Shouldn't happen — opponent just moved — but guard anyway.
        return false;
    }
    let occupied = pos.occupied();
    let our_attackers = pos.attackers_to(target_sq, occupied) & pos.pieces_by_color(our_color);
    if our_attackers == Bitboard::EMPTY {
        return false;
    }
    // SEE the cheapest-attacker capture. Threshold = 1 cp (any
    // strictly-positive material gain). Matches list_see_losing's
    // convention.
    let mut cheapest_from: Option<Square> = None;
    let mut cheapest_value = i32::MAX;
    for from in our_attackers {
        if let Some(p) = pos.piece_on(from) {
            let v = Value::mg_of_piece(p.kind()).0;
            if v < cheapest_value {
                cheapest_value = v;
                cheapest_from = Some(from);
            }
        }
    }
    let Some(from) = cheapest_from else {
        return false;
    };
    pos.see_ge(Move::normal(from, target_sq), Value(1))
}

/// Count pieces of both colours that are attacked and undefended.
#[cfg(test)]
fn count_hanging(pos: &Position, root_stm: Color) -> (usize, usize) {
    (
        list_hanging(pos, root_stm).len(),
        list_hanging(pos, !root_stm).len(),
    )
}

/// Return every non-king piece of `side` that's under attack by the
/// enemy and has no friendly defenders, annotated with the specific
/// enemy pieces doing the attacking.
fn list_hanging(pos: &Position, side: Color) -> Vec<HangingPiece> {
    let mut out = Vec::new();
    let occupied = pos.occupied();
    let enemy = !side;
    let our_bb = pos.pieces_by_color(side);
    let non_king = our_bb & !pos.pieces(PieceType::King);
    for sq in non_king {
        let attackers_bb = pos.attackers_to(sq, occupied);
        let enemy_attackers_bb = attackers_bb & pos.pieces_by_color(enemy);
        if enemy_attackers_bb == Bitboard::EMPTY {
            continue;
        }
        let defenders_bb = attackers_bb & our_bb;
        if defenders_bb != Bitboard::EMPTY {
            continue;
        }
        let Some(piece) = pos.piece_on(sq) else {
            continue;
        };
        let attackers: Vec<PieceLocation> = enemy_attackers_bb
            .into_iter()
            .filter_map(|asq| {
                pos.piece_on(asq).map(|ap| PieceLocation {
                    square: asq,
                    piece: ap.kind(),
                })
            })
            .collect();
        out.push(HangingPiece {
            location: PieceLocation {
                square: sq,
                piece: piece.kind(),
            },
            attackers,
        });
    }
    out
}

/// Return every non-king piece of `side` that is **defended** yet
/// still loses material in an exchange according to SEE — i.e. a
/// piece with both enemy attackers and friendly defenders where the
/// enemy can still win strictly-positive material by initiating the
/// exchange.
///
/// Rationale for using the cheapest enemy attacker as the opening
/// move of the exchange: in standard SEE, the attacker who
/// captures first is the lowest-value one (sacrificing less).
/// [`Position::see_ge`] resolves the remainder of the exchange
/// optimally from there, so passing the cheapest attacker as the
/// initial capture gives an accurate verdict for whether the enemy
/// can profit.
///
/// Edge cases (both acceptable false negatives):
/// - Pinned cheapest attackers: [`Position::see_ge`] handles pin
///   geometry internally for subsequent captures. The initial
///   cheapest-attacker move we pass *might* itself be pinned,
///   silently producing a false negative.
/// - Promotion-rank captures: the constructed `Move::normal`
///   doesn't represent the promotion. `see_ge` short-circuits
///   non-`Normal` moves to `Value::ZERO >= threshold`, so a
///   threshold of 1 returns false.
fn list_see_losing(pos: &Position, side: Color) -> Vec<HangingPiece> {
    let mut out = Vec::new();
    let occupied = pos.occupied();
    let enemy = !side;
    let our_bb = pos.pieces_by_color(side);
    let non_king = our_bb & !pos.pieces(PieceType::King);
    for sq in non_king {
        let attackers_bb = pos.attackers_to(sq, occupied);
        let enemy_attackers_bb = attackers_bb & pos.pieces_by_color(enemy);
        if enemy_attackers_bb == Bitboard::EMPTY {
            continue;
        }
        let defenders_bb = attackers_bb & our_bb;
        if defenders_bb == Bitboard::EMPTY {
            // Already covered by list_hanging — don't double-report.
            continue;
        }

        // Cheapest enemy attacker = lowest midgame piece-value.
        let mut cheapest_from: Option<Square> = None;
        let mut cheapest_value = i32::MAX;
        for from in enemy_attackers_bb {
            if let Some(p) = pos.piece_on(from) {
                let v = Value::mg_of_piece(p.kind()).0;
                if v < cheapest_value {
                    cheapest_value = v;
                    cheapest_from = Some(from);
                }
            }
        }
        let Some(from) = cheapest_from else {
            continue;
        };

        let capture = Move::normal(from, sq);
        if !pos.see_ge(capture, Value(1)) {
            continue;
        }

        let Some(piece) = pos.piece_on(sq) else {
            continue;
        };
        let attackers: Vec<PieceLocation> = enemy_attackers_bb
            .into_iter()
            .filter_map(|asq| {
                pos.piece_on(asq).map(|ap| PieceLocation {
                    square: asq,
                    piece: ap.kind(),
                })
            })
            .collect();
        out.push(HangingPiece {
            location: PieceLocation {
                square: sq,
                piece: piece.kind(),
            },
            attackers,
        });
    }
    out
}

/// Return every non-king piece of `side` that faces a
/// Stockfish-evaluator threat pattern — minor-on-major,
/// rook-on-queen, or safe-pawn-threat. Each returned
/// [`PressuredPiece`] is one `(target, kind)` pair; a single target
/// attacked by attackers of multiple pattern kinds produces one
/// entry per matched pattern.
///
/// **No de-dup with hanging / SEE-losing lists.** These pressure
/// patterns frequently overlap with SEE-losing, so filtering at the
/// engine layer would suppress nearly every entry. The CLI is
/// responsible for not double-narrating a target that already
/// appears on a more urgent list.
///
/// "Safe pawn" here = a pawn whose own square is not attacked by
/// any piece of `side`. Stockfish's evaluator uses a richer
/// "strongly safe" definition, but for teaching narration the
/// simpler version — "the pawn isn't being threatened back" —
/// lines up with how a 1000–1400 player actually reads the
/// position.
fn list_pressured(pos: &Position, side: Color) -> Vec<PressuredPiece> {
    let mut out = Vec::new();
    let occupied = pos.occupied();
    let enemy = !side;
    let our_bb = pos.pieces_by_color(side);
    let enemy_bb = pos.pieces_by_color(enemy);

    let knight_bb = pos.pieces(PieceType::Knight);
    let bishop_bb = pos.pieces(PieceType::Bishop);
    let rook_bb = pos.pieces(PieceType::Rook);
    let pawn_bb = pos.pieces(PieceType::Pawn);

    // --- MinorOnMajor: our rook or queen attacked by an enemy
    //     knight or bishop.
    let our_majors = pos.pieces_of(side, PieceType::Rook) | pos.pieces_of(side, PieceType::Queen);
    for sq in our_majors {
        let enemy_attackers = pos.attackers_to(sq, occupied) & enemy_bb;
        let minors = enemy_attackers & (knight_bb | bishop_bb);
        if minors == Bitboard::EMPTY {
            continue;
        }
        let Some(target) = pos.piece_on(sq) else {
            continue;
        };
        let attackers: Vec<PieceLocation> = minors
            .into_iter()
            .filter_map(|asq| {
                pos.piece_on(asq).map(|ap| PieceLocation {
                    square: asq,
                    piece: ap.kind(),
                })
            })
            .collect();
        out.push(PressuredPiece {
            location: PieceLocation {
                square: sq,
                piece: target.kind(),
            },
            attackers,
            kind: PressureKind::MinorOnMajor,
        });
    }

    // --- RookOnQueen: our queen attacked by an enemy rook.
    for sq in pos.pieces_of(side, PieceType::Queen) {
        let enemy_attackers = pos.attackers_to(sq, occupied) & enemy_bb;
        let rooks = enemy_attackers & rook_bb;
        if rooks == Bitboard::EMPTY {
            continue;
        }
        let attackers: Vec<PieceLocation> = rooks
            .into_iter()
            .filter_map(|asq| {
                pos.piece_on(asq).map(|ap| PieceLocation {
                    square: asq,
                    piece: ap.kind(),
                })
            })
            .collect();
        out.push(PressuredPiece {
            location: PieceLocation {
                square: sq,
                piece: PieceType::Queen,
            },
            attackers,
            kind: PressureKind::RookOnQueen,
        });
    }

    // --- SafePawnThreat: our non-pawn, non-king piece attacked by
    //     an enemy pawn whose own square isn't attacked by us.
    let our_non_pawn_non_king = our_bb & !pawn_bb & !pos.pieces(PieceType::King);
    for sq in our_non_pawn_non_king {
        let enemy_attackers = pos.attackers_to(sq, occupied) & enemy_bb;
        let pawn_attackers = enemy_attackers & pawn_bb;
        if pawn_attackers == Bitboard::EMPTY {
            continue;
        }
        let safe_pawns: Vec<PieceLocation> = pawn_attackers
            .into_iter()
            .filter_map(|asq| {
                let back_attackers = pos.attackers_to(asq, occupied) & our_bb;
                if back_attackers != Bitboard::EMPTY {
                    return None;
                }
                pos.piece_on(asq).map(|ap| PieceLocation {
                    square: asq,
                    piece: ap.kind(),
                })
            })
            .collect();
        if safe_pawns.is_empty() {
            continue;
        }
        let Some(target) = pos.piece_on(sq) else {
            continue;
        };
        out.push(PressuredPiece {
            location: PieceLocation {
                square: sq,
                piece: target.kind(),
            },
            attackers: safe_pawns,
            kind: PressureKind::SafePawnThreat,
        });
    }

    out
}

#[cfg(test)]
mod tests {
    use super::super::test_support::ma_with_pv;
    use super::*;

    #[test]
    fn threats_outcome_empty_when_no_hangs_pre_or_post() {
        let pos = Position::startpos();
        let e4 = Move::normal(Square::E2, Square::E4);
        let ma = ma_with_pv(vec![e4], Some(0));
        let outcome = compute_threats_outcome(&ma, &pos, Color::White);
        assert!(outcome.ours_hanging.is_empty());
        assert!(outcome.theirs_hanging.is_empty());
        assert_eq!(outcome.ours_hanging_delta, 0);
        assert_eq!(outcome.theirs_hanging_delta, 0);
    }

    #[test]
    fn threats_outcome_detects_move_that_creates_our_hang() {
        let fen = "4k3/8/8/8/8/4p3/8/1N4K1 w - - 0 1";
        let pos = Position::from_fen(fen).unwrap();
        let (pre_ours, pre_theirs) = count_hanging(&pos, Color::White);
        assert_eq!(pre_ours, 0);
        assert_eq!(pre_theirs, 0);

        let nd2 = Move::normal(Square::B1, Square::D2);
        let ma = ma_with_pv(vec![nd2], Some(0));
        let outcome = compute_threats_outcome(&ma, &pos, Color::White);
        let hanging = outcome
            .ours_hanging
            .iter()
            .find(|p| p.location.square == Square::D2 && p.location.piece == PieceType::Knight)
            .unwrap_or_else(|| {
                panic!(
                    "expected our knight on d2 to be hanging, got {:?}",
                    outcome.ours_hanging
                )
            });
        assert_eq!(outcome.ours_hanging_delta, 1);
        assert_eq!(hanging.attackers.len(), 1);
        assert_eq!(hanging.attackers[0].square, Square::E3);
        assert_eq!(hanging.attackers[0].piece, PieceType::Pawn);
    }

    #[test]
    fn threats_outcome_no_hangs_when_defender_present() {
        let fen = "4k3/8/8/8/8/4p3/4K3/1N6 w - - 0 1";
        let pos = Position::from_fen(fen).unwrap();
        let nd2 = Move::normal(Square::B1, Square::D2);
        let ma = ma_with_pv(vec![nd2], Some(0));
        let outcome = compute_threats_outcome(&ma, &pos, Color::White);
        assert_eq!(outcome.ours_hanging_delta, 0);
        assert_eq!(outcome.theirs_hanging_delta, 0);
    }

    #[test]
    fn threats_outcome_sign_flipped_for_white_pov() {
        let fen = "1n4k1/8/4P3/8/8/8/8/4K3 b - - 0 1";
        let pos = Position::from_fen(fen).unwrap();
        let nd7 = Move::normal(Square::B8, Square::D7);
        let ma = ma_with_pv(vec![nd7], Some(0));
        let outcome = compute_threats_outcome(&ma, &pos, Color::White);
        let hanging = outcome
            .theirs_hanging
            .iter()
            .find(|p| p.location.square == Square::D7 && p.location.piece == PieceType::Knight)
            .unwrap_or_else(|| {
                panic!(
                    "expected opponent's knight on d7 to be hanging from white POV, got {:?}",
                    outcome.theirs_hanging
                )
            });
        assert_eq!(outcome.theirs_hanging_delta, 1);
        assert_eq!(hanging.attackers.len(), 1);
        assert_eq!(hanging.attackers[0].square, Square::E6);
        assert_eq!(hanging.attackers[0].piece, PieceType::Pawn);
    }

    #[test]
    fn threats_outcome_empty_pv_uses_pre_move_position() {
        let pos = Position::startpos();
        let ma = ma_with_pv(Vec::new(), None);
        let outcome = compute_threats_outcome(&ma, &pos, Color::White);
        assert!(outcome.ours_hanging.is_empty());
        assert!(outcome.theirs_hanging.is_empty());
    }

    #[test]
    fn threats_outcome_records_multiple_attackers() {
        // Black knight on d5 attacked by d1 rook + e4 pawn; no
        // black defenders.
        let fen = "4k3/8/8/3n4/4P3/8/8/3R2K1 b - - 0 1";
        let pos = Position::from_fen(fen).unwrap();
        let hanging = list_hanging(&pos, Color::Black);
        let knight = hanging
            .iter()
            .find(|p| p.location.square == Square::D5)
            .expect("knight on d5 should be hanging");
        assert_eq!(knight.attackers.len(), 2);
        // Attackers ordered by ascending square index — d1 (3)
        // before e4 (28).
        assert_eq!(knight.attackers[0].square, Square::D1);
        assert_eq!(knight.attackers[0].piece, PieceType::Rook);
        assert_eq!(knight.attackers[1].square, Square::E4);
        assert_eq!(knight.attackers[1].piece, PieceType::Pawn);
    }

    // ---- list_see_losing ---------------------------------------------

    #[test]
    fn see_losing_flags_defended_piece_overloaded_by_cheap_attackers() {
        let fen = "4k3/8/3p4/4N3/6n1/8/8/4R1K1 b - - 0 1";
        let pos = Position::from_fen(fen).unwrap();
        let see_losing = list_see_losing(&pos, Color::White);
        let entry = see_losing
            .iter()
            .find(|p| p.location.square == Square::E5)
            .expect("e5 knight should be SEE-losing");
        assert_eq!(entry.location.piece, PieceType::Knight);
        assert_eq!(entry.attackers.len(), 2);
    }

    #[test]
    fn see_losing_does_not_flag_equal_defended_trade() {
        let fen = "k3r3/8/8/4R3/8/8/8/K3R3 b - - 0 1";
        let pos = Position::from_fen(fen).unwrap();
        let see_losing = list_see_losing(&pos, Color::White);
        assert!(
            see_losing.iter().all(|p| p.location.square != Square::E5),
            "even-trade rook should not be flagged, got {:?}",
            see_losing
        );
    }

    #[test]
    fn see_losing_skips_strictly_hanging_piece() {
        let fen = "4k3/8/8/8/8/4p3/8/1N4K1 w - - 0 1";
        let pos = Position::from_fen(fen).unwrap();
        let mut scratch = pos.clone();
        scratch.do_move(Move::normal(Square::B1, Square::D2));
        let see_losing = list_see_losing(&scratch, Color::White);
        assert!(
            see_losing.is_empty(),
            "hanging-only pieces belong on the hanging list, got {:?}",
            see_losing
        );
    }

    #[test]
    fn compute_threats_outcome_populates_see_losing_delta() {
        let pre_fen = "4k3/3p4/8/4N3/6n1/8/8/4R1K1 b - - 0 1";
        let pre = Position::from_fen(pre_fen).unwrap();
        let push = Move::normal(Square::D7, Square::D6);
        let ma = ma_with_pv(vec![push], Some(0));
        let outcome = compute_threats_outcome(&ma, &pre, Color::White);
        assert_eq!(
            outcome.ours_see_losing_delta, 1,
            "d7-d6 should create one SEE-losing piece on our side"
        );
        assert_eq!(outcome.theirs_see_losing_delta, 0);
    }

    // ---- list_pressured ---------------------------------------------

    #[test]
    fn list_pressured_safe_pawn_threat_fires_against_minor() {
        let fen = "4k3/8/5n2/4P3/8/8/8/4K3 w - - 0 1";
        let pos = Position::from_fen(fen).unwrap();
        let pressured = list_pressured(&pos, Color::Black);
        let entry = pressured
            .iter()
            .find(|p| p.location.square == Square::F6)
            .unwrap_or_else(|| panic!("expected f6 knight in pressured list, got {pressured:?}"));
        assert_eq!(entry.kind, PressureKind::SafePawnThreat);
        assert_eq!(entry.location.piece, PieceType::Knight);
        assert_eq!(entry.attackers.len(), 1);
        assert_eq!(entry.attackers[0].square, Square::E5);
        assert_eq!(entry.attackers[0].piece, PieceType::Pawn);
    }

    #[test]
    fn list_pressured_unsafe_pawn_threat_does_not_fire() {
        let fen = "4k3/8/3p1n2/4P3/8/8/8/4K3 w - - 0 1";
        let pos = Position::from_fen(fen).unwrap();
        let pressured = list_pressured(&pos, Color::Black);
        assert!(
            pressured.iter().all(|p| p.location.square != Square::F6
                || p.kind != PressureKind::SafePawnThreat),
            "f6 knight should not appear under SafePawnThreat when attacker pawn is itself attacked, got {pressured:?}",
        );
    }

    #[test]
    fn list_pressured_minor_on_major_fires() {
        let fen = "4k3/r7/2N5/8/8/8/8/4K3 w - - 0 1";
        let pos = Position::from_fen(fen).unwrap();
        let pressured = list_pressured(&pos, Color::Black);
        let entry = pressured
            .iter()
            .find(|p| p.location.square == Square::A7 && p.kind == PressureKind::MinorOnMajor)
            .unwrap_or_else(|| panic!("expected a7 rook MinorOnMajor entry, got {pressured:?}"));
        assert_eq!(entry.location.piece, PieceType::Rook);
        assert_eq!(entry.attackers.len(), 1);
        assert_eq!(entry.attackers[0].square, Square::C6);
        assert_eq!(entry.attackers[0].piece, PieceType::Knight);
    }

    #[test]
    fn list_pressured_rook_on_queen_fires() {
        let fen = "3q1k2/8/8/8/8/8/8/3R2K1 w - - 0 1";
        let pos = Position::from_fen(fen).unwrap();
        let pressured = list_pressured(&pos, Color::Black);
        let entry = pressured
            .iter()
            .find(|p| p.location.square == Square::D8 && p.kind == PressureKind::RookOnQueen)
            .unwrap_or_else(|| panic!("expected d8 queen RookOnQueen entry, got {pressured:?}"));
        assert_eq!(entry.location.piece, PieceType::Queen);
        assert_eq!(entry.attackers.len(), 1);
        assert_eq!(entry.attackers[0].square, Square::D1);
        assert_eq!(entry.attackers[0].piece, PieceType::Rook);
    }

    #[test]
    fn list_pressured_no_dedup_with_hanging() {
        let fen = "4k3/r7/2N5/8/8/8/8/4K3 w - - 0 1";
        let pos = Position::from_fen(fen).unwrap();
        let hanging = list_hanging(&pos, Color::Black);
        let pressured = list_pressured(&pos, Color::Black);
        assert!(
            hanging.iter().any(|h| h.location.square == Square::A7),
            "a7 rook should be hanging in this position",
        );
        assert!(
            pressured.iter().any(|p| p.location.square == Square::A7),
            "list_pressured should NOT filter out the hanging rook — found {pressured:?}",
        );
    }

    #[test]
    fn compute_threats_outcome_populates_pressured_delta() {
        let pre_fen = "1N2k3/r7/8/8/8/8/8/6K1 w - - 0 1";
        let pre = Position::from_fen(pre_fen).unwrap();
        let pre_pressured = list_pressured(&pre, Color::Black);
        assert!(
            pre_pressured
                .iter()
                .all(|p| p.location.square != Square::A7),
            "pre-move should have no pressure on a7, got {pre_pressured:?}",
        );

        let nc6 = Move::normal(Square::B8, Square::C6);
        let ma = ma_with_pv(vec![nc6], Some(0));
        let outcome = compute_threats_outcome(&ma, &pre, Color::White);
        assert_eq!(
            outcome.theirs_pressured_delta, 1,
            "Nc6 should create one new pressure on the opponent's a7 rook"
        );
        let entry = outcome
            .theirs_pressured
            .iter()
            .find(|p| p.location.square == Square::A7)
            .unwrap_or_else(|| {
                panic!(
                    "expected a7 in theirs_pressured, got {:?}",
                    outcome.theirs_pressured
                )
            });
        assert_eq!(entry.kind, PressureKind::MinorOnMajor);
    }

    #[test]
    fn list_pressured_minor_on_minor_does_not_fire() {
        let fen = "4k3/5n2/8/8/2B5/8/8/4K3 w - - 0 1";
        let pos = Position::from_fen(fen).unwrap();
        let pressured = list_pressured(&pos, Color::Black);
        assert!(
            pressured
                .iter()
                .all(|p| p.kind != PressureKind::MinorOnMajor),
            "MinorOnMajor must require the target to be rook or queen, got {pressured:?}",
        );
    }

    #[test]
    fn threats_outcome_ignores_kings() {
        let fen = "4k3/8/8/8/8/8/4Q3/4K3 b - - 0 1";
        let pos = Position::from_fen(fen).unwrap();
        let (ours, theirs) = count_hanging(&pos, Color::Black);
        assert_eq!(ours, 0, "king in check must not count as hanging");
        assert_eq!(theirs, 0, "white king should not count either");
    }

    // ---- filter_guaranteed_targets ----------------------------------

    #[test]
    fn guarantee_filter_drops_e5_after_nf3_because_nc6_defends() {
        // After 1.e4 e5 2.Nf3 — Black to move. e5 looks hanging
        // statically (attacked by Nf3, no defender), but 2...Nc6
        // defends. The guarantee filter must drop e5 so the
        // retrospective doesn't tell the student they can win the
        // pawn — Nxe5? would lose a knight after ...Nxe5.
        let fen = "rnbqkbnr/pppp1ppp/8/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R b KQkq - 1 2";
        let pos = Position::from_fen(fen).unwrap();
        let hanging = list_hanging(&pos, Color::Black);
        assert!(
            hanging.iter().any(|h| h.location.square == Square::E5),
            "static check should still flag e5 as hanging (precondition for test)"
        );
        let guaranteed = filter_guaranteed_targets(&pos, &hanging, Color::White);
        assert!(
            guaranteed.iter().all(|h| h.location.square != Square::E5),
            "Nc6 defends e5 — guarantee filter must drop the entry, got {guaranteed:?}"
        );
    }

    #[test]
    fn guarantee_filter_keeps_target_when_opponent_is_in_stalemate() {
        // Edge-case branch: when the opponent has no legal moves at
        // all, every static target is trivially "guaranteed"
        // (there's no response that could refute). Game-over
        // territory; teaching value moot. But the branch should
        // exercise so we know it doesn't panic.
        // Stalemate position: black king on h8, white queen on g6,
        // white king on f7. Black to move, no legal moves.
        let fen = "7k/5K2/6Q1/8/8/8/8/8 b - - 0 1";
        let pos = Position::from_fen(fen).unwrap();
        // Construct a synthetic HangingPiece (the test doesn't care
        // whether it's actually hanging — the filter's stalemate
        // short-circuit returns true unconditionally).
        let synthetic = HangingPiece {
            location: PieceLocation {
                square: Square::H8,
                piece: PieceType::King,
            },
            attackers: Vec::new(),
        };
        let kept = filter_guaranteed_targets(&pos, &[synthetic.clone()], Color::White);
        assert_eq!(kept.len(), 1, "stalemate branch must keep all targets");
    }
}
