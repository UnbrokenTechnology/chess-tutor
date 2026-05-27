//! Test helpers shared across the `analysis::*` test modules. Private
//! to the crate; compiled only under `#[cfg(test)]`.

#![cfg(test)]

use super::MoveAnalysis;
use crate::eval::EvalTrace;
use crate::movegen::legal_moves_vec;
use crate::position::Position;
use crate::types::{File, Move, MoveKind, PieceType, Rank, Square, Value};

/// Build a bare [`MoveAnalysis`] with only `pv` and `settled_ply`
/// filled in. Every outcome that walks the user's PV reads only those
/// two fields; the rest can be zeroed.
pub(super) fn ma_with_pv(pv: Vec<Move>, settled_ply: Option<usize>) -> MoveAnalysis {
    ma_with_pv_score(pv, settled_ply, Value::ZERO)
}

/// Like [`ma_with_pv`] but with an explicit line `score` (root
/// side-to-move POV). Needed by tests that gate on the eval — e.g.
/// distinguishing a sound sacrifice (score ≥ 0) from a losing material
/// dump (score < 0).
pub(super) fn ma_with_pv_score(
    pv: Vec<Move>,
    settled_ply: Option<usize>,
    score: Value,
) -> MoveAnalysis {
    MoveAnalysis {
        mv: pv.first().copied().unwrap_or(Move::NONE),
        score,
        depth: 1,
        pv,
        ply_traces: Vec::new(),
        settled_ply,
        pre_move_trace: EvalTrace::zero(),
        pre_score: Value::ZERO,
        term_deltas: Vec::new(),
    }
}

/// Parse a lichess-puzzler test line: a starting FEN plus a space-separated
/// UCI move list whose **first** move is the opponent's setup and the rest
/// is the solver's line. Returns `(pre, pv)` where `pre` is the position
/// after the setup move (solver to move) and `pv` is the remaining moves —
/// exactly the framing `compute_tactic_outcome` / `detect_line_tactic`
/// expect (`pv[0]` played by `pre.side_to_move()`).
///
/// Panics if any token isn't a legal move in the running position, so a
/// typo'd fixture fails loudly.
pub(super) fn uci_line(fen: &str, uci_moves: &str) -> (Position, Vec<Move>) {
    let mut walk = Position::from_fen(fen).unwrap();
    let mut all = Vec::new();
    for tok in uci_moves.split_whitespace() {
        let mv = find_uci_move(&mut walk, tok);
        all.push(mv);
        walk.do_move(mv);
    }
    assert!(!all.is_empty(), "uci_line needs at least the setup move");
    let mut pre = Position::from_fen(fen).unwrap();
    pre.do_move(all[0]);
    (pre, all[1..].to_vec())
}

fn find_uci_move(pos: &mut Position, uci: &str) -> Move {
    let from = sq_from_str(&uci[0..2]);
    let to = sq_from_str(&uci[2..4]);
    let promo = uci.as_bytes().get(4).map(|&b| match b {
        b'q' => PieceType::Queen,
        b'r' => PieceType::Rook,
        b'b' => PieceType::Bishop,
        b'n' => PieceType::Knight,
        other => panic!("bad promotion char {}", other as char),
    });
    for mv in legal_moves_vec(pos) {
        if mv.from() != from || mv.to() != to {
            continue;
        }
        match promo {
            Some(p) => {
                if mv.kind() == MoveKind::Promotion && mv.promoted_to() == p {
                    return mv;
                }
            }
            None => return mv,
        }
    }
    panic!("no legal move matching {uci}");
}

fn sq_from_str(s: &str) -> Square {
    let b = s.as_bytes();
    let file = File::from_index(b[0] - b'a').expect("file");
    let rank = Rank::from_index(b[1] - b'1').expect("rank");
    Square::new(file, rank)
}
