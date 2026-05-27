//! Terminal-node named-mate detectors.
//!
//! Hand-transliterations of the `cook.py` mate sub-functions
//! (`back_rank_mate`, `smothered_mate`, `anastasia_mate`, `hook_mate`,
//! `arabian_mate`, `boden_or_double_bishop_mate`, `dovetail_mate`), adapted to
//! our `pv` framing. See the parent module's `//!` for provenance / licensing.
//!
//! Unlike the per-move geometric detectors in [`super::detectors`], these run
//! on the *terminal* position of a forced mating line: [`detect_mate_pattern`]
//! replays the whole `pv`, confirms it ends in a checkmate the `mover`
//! delivered, then names the geometry. The *fact* of a forced mate is already
//! a search output (a mate score); this only adds the human-readable pattern,
//! recorded on [`TacticHit::mate_pattern`] independently of whatever geometric
//! `pattern` set the mate up — the same way lichess assigns a mate tag *and*
//! the tactic tags to one puzzle.
//!
//! ## Framing
//!
//! `mover` plays `pv[0]`, `pv[2]`, … (the even indices). A mate `mover`
//! delivers therefore lands on the final, even-indexed move — an odd-length
//! line. A mate at an *even*-length line was delivered by the opponent (the
//! `mover` got mated), which the `user_walked_into` slot detects by calling
//! with `mover` = the opponent; either way the rule "the mating move is the
//! mover's" holds, and a line where it isn't returns `None` here.

use super::{Confidence, MatePattern, TacticHit, TacticPattern};
use crate::bitboard::{king_distance, Bitboard};
use crate::movegen::legal_moves_vec;
use crate::position::Position;
use crate::types::{Color, File, Move, PieceType, Rank, Square};

#[cfg(test)]
#[path = "mate_tests.rs"]
mod tests;

/// Result of a successful mate-pattern scan: the named geometry plus the data
/// a standalone [`TacticPattern::Checkmate`] hit needs.
#[derive(Copy, Clone, Debug)]
pub(super) struct MateInfo {
    pub pattern: MatePattern,
    /// Destination of the mating move (the mating piece's square).
    pub mating_sq: Square,
    /// The mated king's square.
    pub king_sq: Square,
    /// Index of the mating move within the analysed `pv`.
    pub mating_ply: usize,
}

/// Plies a named mate may span. In our framing a mate-in-`N` is `2N-1` plies,
/// so this caps at mate-in-5 — mirroring lichess's `mateIn1..5`. Longer forced
/// mates aren't named (they fall through to the geometric / non-mate slots).
const MATE_MAX_PLIES: usize = 9;

/// Replay `pv` from `pre` and, if it ends in a checkmate delivered by `mover`,
/// name the mating geometry. Applies the `cook.py` sub-detectors in the same
/// exclusive order (smothered → back-rank → anastasia → hook → arabian →
/// boden/double-bishop → dovetail). Returns `None` for a non-mating line, one
/// longer than [`MATE_MAX_PLIES`], or a mate the opponent (not `mover`)
/// delivers.
pub(super) fn detect_mate_pattern(pre: &Position, pv: &[Move], mover: Color) -> Option<MateInfo> {
    let n = pv.len();
    // Odd length ⇒ the final (even-indexed) move is the mover's.
    if n == 0 || n > MATE_MAX_PLIES || n % 2 == 0 {
        return None;
    }
    let mut board = pre.clone();
    for &mv in pv {
        board.do_move(mv);
    }
    if !is_checkmate(&board) {
        return None;
    }
    let mating = pv[n - 1];
    let king = board.king_square(!mover);

    let pattern = if is_smothered_mate(&board, mover, king) {
        MatePattern::Smothered
    } else if is_back_rank_mate(&board, mover, king) {
        MatePattern::BackRank
    } else if is_anastasia_mate(&board, mover, king, mating) {
        MatePattern::Anastasia
    } else if is_hook_mate(&board, mover, king, mating) {
        MatePattern::Hook
    } else if is_arabian_mate(&board, mover, king, mating) {
        MatePattern::Arabian
    } else if let Some(p) = boden_or_double_bishop_mate(&board, mover, king) {
        p
    } else if is_dovetail_mate(&board, mover, king, mating) {
        MatePattern::Dovetail
    } else {
        return None;
    };

    Some(MateInfo {
        pattern,
        mating_sq: mating.to(),
        king_sq: king,
        mating_ply: n - 1,
    })
}

/// Build the standalone [`TacticPattern::Checkmate`] hit for a forced mating
/// line on which no geometric pattern fired. `material_gain` stays honest
/// about material over the window, but confidence is always `High` — a forced
/// mate is the most certain outcome there is, material aside.
pub(super) fn synthesize_checkmate_hit(
    pre: &Position,
    pv: &[Move],
    mover: Color,
    base_ply: usize,
    info: MateInfo,
) -> TacticHit {
    TacticHit {
        pattern: TacticPattern::Checkmate,
        pv_ply: base_ply + info.mating_ply,
        primary_piece: info.mating_sq,
        targets: vec![info.king_sq],
        material_gain: super::line_material_gain(pre, pv, mover),
        confidence: Confidence::High,
        sacrifice: super::is_sacrifice(pre, pv, mover),
        mate_pattern: Some(info.pattern),
    }
}

// =========================================================================
// Sub-detectors (terminal board)
// =========================================================================

/// Back-rank mate — port of `cook.py:back_rank_mate`.
///
/// The mated king is on its own back rank; each of the (up to three) squares
/// directly in front of it is occupied by one of the king's *own* pieces and
/// not covered by the mating side (a pure self-block, the textbook pawn wall);
/// and at least one checker sits on that back rank.
fn is_back_rank_mate(board: &Position, mover: Color, king: Square) -> bool {
    let back_rank = if mover == Color::White {
        Rank::R8
    } else {
        Rank::R1
    };
    if king.rank() != back_rank {
        return false;
    }
    let fwd_rank = if mover == Color::White {
        Rank::R7
    } else {
        Rank::R2
    };
    let kf = king.file().index() as i32;
    for df in [-1i32, 0, 1] {
        let Some(file) = u8::try_from(kf + df).ok().and_then(File::from_index) else {
            continue;
        };
        let sq = Square::new(file, fwd_rank);
        match board.piece_on(sq) {
            None => return false,                          // an open flight in front
            Some(p) if p.color() == mover => return false, // blocked by the mating side, not own pawns
            Some(_) => {}                                  // the king's own piece — good
        }
        if pov_attackers(board, sq, mover).any() {
            return false; // covered by the mating side ⇒ not a pure back-rank box
        }
    }
    board
        .checkers()
        .into_iter()
        .any(|c| c.rank() == back_rank)
}

/// Smothered mate — port of `cook.py:smothered_mate`.
///
/// A knight gives check and every square a king-step away is occupied by one
/// of the king's own pieces, so it cannot flee and the knight check can't be
/// blocked.
fn is_smothered_mate(board: &Position, mover: Color, king: Square) -> bool {
    for checker in board.checkers() {
        if board.piece_on(checker).map(|p| p.kind()) != Some(PieceType::Knight) {
            continue;
        }
        // The first knight checker decides (cook returns inside the loop).
        return king_neighbors(king)
            .all(|sq| board.piece_on(sq).is_some_and(|p| p.color() != mover));
    }
    false
}

/// Anastasia's mate — port of `cook.py:anastasia_mate`.
///
/// The mated king is on an edge file (a/h) but off the corner ranks; the
/// mating move was a rook or queen onto the king's file; one square inward
/// along the king's rank holds the king's own piece, and the square two
/// further inward holds the mating side's knight.
fn is_anastasia_mate(board: &Position, mover: Color, king: Square, mating: Move) -> bool {
    let kf = king.file();
    if !matches!(kf, File::A | File::H) || matches!(king.rank(), Rank::R1 | Rank::R8) {
        return false;
    }
    if mating.to().file() != kf {
        return false;
    }
    if !matches!(
        board.piece_on(mating.to()).map(|p| p.kind()),
        Some(PieceType::Queen | PieceType::Rook)
    ) {
        return false;
    }
    // Step inward off the edge file: +1 from the a-file, -1 from the h-file.
    // (lichess flips the board to the a-file and uses +1 / +3.)
    let inward: i32 = if kf == File::A { 1 } else { -1 };
    let kfi = kf.index() as i32;
    let rank = king.rank();
    let Some(blocker_sq) = u8::try_from(kfi + inward)
        .ok()
        .and_then(File::from_index)
        .map(|f| Square::new(f, rank))
    else {
        return false;
    };
    let Some(knight_sq) = u8::try_from(kfi + 3 * inward)
        .ok()
        .and_then(File::from_index)
        .map(|f| Square::new(f, rank))
    else {
        return false;
    };
    if !board
        .piece_on(blocker_sq)
        .is_some_and(|p| p.color() != mover)
    {
        return false;
    }
    board
        .piece_on(knight_sq)
        .is_some_and(|p| p.color() == mover && p.kind() == PieceType::Knight)
}

/// Hook mate — port of `cook.py:hook_mate`.
///
/// A rook beside the king (the mating move), guarded by a knight that is also
/// beside the king, with that knight in turn guarded by a pawn.
fn is_hook_mate(board: &Position, mover: Color, king: Square, mating: Move) -> bool {
    let rook_sq = mating.to();
    if board.piece_on(rook_sq).map(|p| p.kind()) != Some(PieceType::Rook)
        || king_distance(rook_sq, king) != 1
    {
        return false;
    }
    for n_sq in pov_attackers(board, rook_sq, mover) {
        if board.piece_on(n_sq).map(|p| p.kind()) != Some(PieceType::Knight)
            || king_distance(n_sq, king) != 1
        {
            continue;
        }
        for p_sq in pov_attackers(board, n_sq, mover) {
            if board.piece_on(p_sq).map(|p| p.kind()) == Some(PieceType::Pawn) {
                return true;
            }
        }
    }
    false
}

/// Arabian mate — port of `cook.py:arabian_mate`.
///
/// King in a corner; a rook beside it (the mating move) guarded by a knight a
/// (2, 2) leap from the king.
fn is_arabian_mate(board: &Position, mover: Color, king: Square, mating: Move) -> bool {
    if !(matches!(king.file(), File::A | File::H) && matches!(king.rank(), Rank::R1 | Rank::R8)) {
        return false;
    }
    let rook_sq = mating.to();
    if board.piece_on(rook_sq).map(|p| p.kind()) != Some(PieceType::Rook)
        || king_distance(rook_sq, king) != 1
    {
        return false;
    }
    let kr = king.rank().index() as i32;
    let kf = king.file().index() as i32;
    for n_sq in pov_attackers(board, rook_sq, mover) {
        if board.piece_on(n_sq).map(|p| p.kind()) != Some(PieceType::Knight) {
            continue;
        }
        let dr = (n_sq.rank().index() as i32 - kr).abs();
        let df = (n_sq.file().index() as i32 - kf).abs();
        if dr == 2 && df == 2 {
            return true;
        }
    }
    false
}

/// Boden's / double-bishop mate — port of `cook.py:boden_or_double_bishop_mate`.
///
/// The mating side has ≥ 2 bishops, and every square in the king's
/// neighbourhood (the king's own square included) is attacked by the mating
/// side *only* with bishops — a pure two-bishop net. The two flavours differ
/// by whether the bishops straddle the king's file (criss-cross = Boden) or
/// sit on the same side (parallel = double-bishop).
fn boden_or_double_bishop_mate(
    board: &Position,
    mover: Color,
    king: Square,
) -> Option<MatePattern> {
    let bishops: Vec<Square> = (board.pieces(PieceType::Bishop) & board.pieces_by_color(mover))
        .into_iter()
        .collect();
    if bishops.len() < 2 {
        return None;
    }
    for sq in std::iter::once(king).chain(king_neighbors(king)) {
        for a_sq in pov_attackers(board, sq, mover) {
            if board.piece_on(a_sq).map(|p| p.kind()) != Some(PieceType::Bishop) {
                return None;
            }
        }
    }
    let kf = king.file().index() as i32;
    let b0 = bishops[0].file().index() as i32;
    let b1 = bishops[1].file().index() as i32;
    if (b0 < kf) == (b1 > kf) {
        Some(MatePattern::Boden)
    } else {
        Some(MatePattern::DoubleBishop)
    }
}

/// Dovetail (Cozio's) mate — port of `cook.py:dovetail_mate`.
///
/// A centre king (off every edge), a queen on a diagonally-adjacent square
/// (the mating move), and the king boxed so that each remaining flight is
/// either covered by the queen alone (and empty) or left to one of the king's
/// own pieces — never covered by another mating-side piece.
fn is_dovetail_mate(board: &Position, mover: Color, king: Square, mating: Move) -> bool {
    if matches!(king.file(), File::A | File::H) || matches!(king.rank(), Rank::R1 | Rank::R8) {
        return false;
    }
    let q_sq = mating.to();
    if board.piece_on(q_sq).map(|p| p.kind()) != Some(PieceType::Queen) {
        return false;
    }
    // Diagonally adjacent: distance 1, off the king's file and rank.
    if q_sq.file() == king.file() || q_sq.rank() == king.rank() || king_distance(q_sq, king) > 1 {
        return false;
    }
    for sq in king_neighbors(king) {
        if sq == q_sq {
            continue;
        }
        let attackers = pov_attackers(board, sq, mover);
        if attackers.popcount() == 1 && attackers.contains(q_sq) {
            // A square only the queen covers must be empty (a flight she holds).
            if board.piece_on(sq).is_some() {
                return false;
            }
        } else if attackers.any() {
            // Covered by something besides (or in addition to) the queen.
            return false;
        }
        // Else: unattacked by the mating side — assumed blocked by the king's
        // own piece (the dovetail "tail").
    }
    true
}

// =========================================================================
// Helpers
// =========================================================================

/// The mating side's pieces attacking `sq`.
fn pov_attackers(board: &Position, sq: Square, pov: Color) -> Bitboard {
    board.attackers_to(sq, board.occupied()) & board.pieces_by_color(pov)
}

/// Whether the side to move in `pos` is checkmated.
fn is_checkmate(pos: &Position) -> bool {
    if !pos.checkers().any() {
        return false;
    }
    let mut scratch = pos.clone();
    legal_moves_vec(&mut scratch).is_empty()
}

/// The on-board squares a king-step away (Chebyshev distance 1).
fn king_neighbors(king: Square) -> impl Iterator<Item = Square> {
    let kf = king.file().index() as i32;
    let kr = king.rank().index() as i32;
    let mut out = Vec::with_capacity(8);
    for dr in [-1i32, 0, 1] {
        for df in [-1i32, 0, 1] {
            if dr == 0 && df == 0 {
                continue;
            }
            if let (Some(f), Some(r)) = (
                u8::try_from(kf + df).ok().and_then(File::from_index),
                u8::try_from(kr + dr).ok().and_then(Rank::from_index),
            ) {
                out.push(Square::new(f, r));
            }
        }
    }
    out.into_iter()
}
