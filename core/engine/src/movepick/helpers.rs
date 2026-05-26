//! Free move-scoring / validation helpers used by the
//! [`MovePicker`](super::MovePicker) FSM: pick-best selection, partial
//! insertion sort, MVV-LVA, captured-piece value, and pseudo-legality.

use super::ScoredMove;
use crate::movegen::{generate_pseudo_legal_moves, MoveList};
use crate::position::Position;
use crate::types::{Move, Value};

/// Find the index of the highest-scoring entry in `buf[start..]`. Returns
/// `start` when the slice is empty (shouldn't happen — callers guard).
pub(super) fn pick_best_index(buf: &[ScoredMove], start: usize) -> usize {
    let mut best = start;
    for i in (start + 1)..buf.len() {
        if buf[i].score > buf[best].score {
            best = i;
        }
    }
    best
}

/// Sort entries whose score meets `limit` into descending order at the
/// front of `buf`; leave the tail unsorted. Matches Stockfish 11's
/// `partial_insertion_sort` so ordering behaviour parallels the reference.
pub(super) fn partial_insertion_sort(buf: &mut [ScoredMove], limit: i32) {
    let mut sorted_end: usize = 0;
    let mut p = 1;
    while p < buf.len() {
        if buf[p].score >= limit {
            let tmp = buf[p];
            sorted_end += 1;
            buf[p] = buf[sorted_end];
            let mut q = sorted_end;
            while q > 0 && buf[q - 1].score < tmp.score {
                buf[q] = buf[q - 1];
                q -= 1;
            }
            buf[q] = tmp;
        }
        p += 1;
    }
}

/// MVV-LVA capture scoring: the victim's mid-game value scaled by 6 (MVV)
/// minus the attacker's mid-game value (LVA). High = big victim captured
/// cheaply.
///
/// NOTE: SF11 (`movepick.cpp:110-111`) uses pure MVV (`victim*6`) with no
/// static LVA term, relying on the learned capture-history table for the
/// attacker signal. We deliberately keep the `-attacker` LVA tiebreak: an
/// A/B during the parity audit (2026-05-26) showed dropping it *regressed*
/// our node count by 1.4% at d=14 1T (12.39M → 12.56M). Our capture
/// history is less developed within short searches, so the static LVA
/// still adds useful ordering signal that SF gets from its capture
/// history. Justified deviation from SF.
pub(super) fn mvv_lva(pos: &Position, mv: Move) -> i32 {
    let victim = captured_piece_value(pos, mv).0;
    let attacker = Value::mg_of_piece(pos.moved_piece(mv).kind()).0;
    victim * 6 - attacker
}

/// Middle-game value of the piece captured by `mv`. En-passant captures a
/// pawn; promotions/normal captures take the piece on the destination.
pub(super) fn captured_piece_value(pos: &Position, mv: Move) -> Value {
    use crate::types::MoveKind;
    match mv.kind() {
        MoveKind::EnPassant => Value::PAWN_MG,
        MoveKind::Normal | MoveKind::Promotion => pos
            .piece_on(mv.to())
            .map(|p| Value::mg_of_piece(p.kind()))
            .unwrap_or(Value::ZERO),
        MoveKind::Castling => Value::ZERO,
    }
}

/// Conservative pseudo-legality check: a move is pseudo-legal iff the
/// pseudo-legal generator would emit it. Slow (O(movegen)) but correct;
/// used only once per node for the TT move. When search profiling shows
/// this is hot, swap in a direct validator mirroring Stockfish's
/// `Position::pseudo_legal`.
pub(super) fn is_pseudo_legal(pos: &Position, mv: Move) -> bool {
    if !mv.is_valid() {
        return false;
    }
    let mut list = MoveList::new();
    generate_pseudo_legal_moves(pos, &mut list);
    list.contains(&mv)
}
