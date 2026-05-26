//! The three static threat-list scanners: hanging, SEE-losing, and
//! Stockfish-style pressure. Each takes a [`Position`] and the side
//! to scan, returning structured entries.

use super::types::{HangingPiece, PieceLocation, PressureKind, PressuredPiece};
use crate::bitboard::Bitboard;
use crate::position::Position;
use crate::types::{Color, Move, PieceType, Square, Value};

/// Return every non-king piece of `side` that's under attack by the
/// enemy and has no friendly defenders, annotated with the specific
/// enemy pieces doing the attacking.
/// List pieces of `side` that are attacked by at least one enemy
/// piece *and* undefended by any friendly piece in `pos`. Public for
/// the coaching surface (live, pre-user-move) — the analytical paths
/// reach this via [`ThreatsOutcome::ours_hanging`] /
/// [`ThreatsOutcome::theirs_hanging`].
///
/// [`ThreatsOutcome`]: super::ThreatsOutcome
pub fn list_hanging(pos: &Position, side: Color) -> Vec<HangingPiece> {
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
/// List pieces of `side` that are attacked, defended, but still lose
/// material in an SEE-assessed exchange initiated by the enemy. Public
/// for the same reason as [`list_hanging`].
pub fn list_see_losing(pos: &Position, side: Color) -> Vec<HangingPiece> {
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

        // Cheapest enemy attacker = lowest midgame piece-value. Kings
        // are excluded from the SEE-initiator pool: a king can't
        // legally capture a defended piece (it would move into
        // check), and Value::mg_of_piece(King) == 0 would otherwise
        // make the king look like a free-of-cost first captor and
        // produce a phantom "you lose this trade" verdict (e.g. king
        // takes queen, defender recaptures the king — physically
        // impossible).
        let non_king_attackers = enemy_attackers_bb & !pos.pieces(PieceType::King);
        if non_king_attackers == Bitboard::EMPTY {
            continue;
        }
        let mut cheapest_from: Option<Square> = None;
        let mut cheapest_value = i32::MAX;
        for from in non_king_attackers {
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
pub(super) fn list_pressured(pos: &Position, side: Color) -> Vec<PressuredPiece> {
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
