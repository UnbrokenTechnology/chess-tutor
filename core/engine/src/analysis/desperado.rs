//! Desperado detection — "the doomed piece grabs a pawn (with check) on
//! its way off the board."
//!
//! See `PLAN-teaching-gui.md` §4 (desperado-aware material narration) and
//! [`teaching-positions/positional-punish-after-qe6`] (the safety-net
//! table). When a piece is going to be lost, the material story isn't
//! complete until we ask: *can that piece cash itself for a pawn first via
//! a forcing in-between?* In the case study, after `…Nxe4` White's Nf5 is
//! doomed, but `Nxg7+` is a **check**, so it forces `…Bxg7` and buys the
//! single tempo White needs to recapture on e4 before `…Qxf5` ever
//! happens. The net swings from −1.0 ("you're down a clean pawn") to 0.0
//! ("even — the desperado grabbed a pawn on the way down"). The honest
//! narration is "−1.0 becomes 0.0 because of the desperado," not "you're
//! fine."
//!
//! ## Scope (the clear case)
//!
//! This implements the **same-tempo forcing capture-with-check** variant —
//! the doomed piece, on its own move, captures an enemy piece *and gives
//! check*, so the opponent must respond to the check before collecting the
//! doomed piece. That is the case study's `Nxg7+` and the most common
//! desperado a 1200 misses. A fuller treatment (non-checking zwischenzug
//! desperados, multi-step) is a documented follow-up — the boundary is
//! marked on [`find_desperado`].

use crate::movegen::legal_moves_vec;
use crate::position::Position;
use crate::types::{Color, PieceType, Square, Value};

#[cfg(test)]
#[path = "desperado_tests.rs"]
mod tests;

/// A desperado resource for a doomed piece: a capture-with-check the piece
/// can play *itself* before it falls, recovering material on the way down.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Desperado {
    /// The doomed piece's square (its current location).
    pub piece: Square,
    /// The square it captures on (and from which it gives check).
    pub captures_on: Square,
    /// The piece type captured by the desperado.
    pub captured: PieceType,
    /// Midgame cp value recovered by the desperado capture — what the
    /// "−X becomes −X+this" narration leans on. Always > 0 (we only report
    /// captures of a real piece).
    pub recovered_cp: i32,
}

/// Find a same-tempo desperado for the (presumed-doomed) piece on
/// `piece_sq`, owned by `owner`, in `pos`.
///
/// The premise — that the piece is *actually* doomed — is the caller's to
/// establish (a material card already narrating a loss of this piece). We
/// only answer the narrower, structural question: **does this piece have a
/// legal capture-with-check available right now?** If so, it can cash
/// itself for the captured material before the opponent collects it, which
/// is the desperado the narration must mention.
///
/// `owner` must be the side to move in `pos` (the desperado is *the
/// owner's* move). Returns the most valuable such capture, or `None` when
/// the piece has no capture-with-check.
///
/// Boundary (PLAN §4, "minimal version"): only the forcing
/// **capture-with-check** variant is detected. A non-checking zwischenzug
/// desperado (where some *other* forcing threat buys the tempo) is not
/// covered here and is a documented follow-up.
pub fn find_desperado(pos: &Position, piece_sq: Square, owner: Color) -> Option<Desperado> {
    if pos.side_to_move() != owner {
        return None;
    }
    // The piece must belong to the owner.
    match pos.piece_on(piece_sq) {
        Some(p) if p.color() == owner => {}
        _ => return None,
    }

    let mut scratch = pos.clone();
    let mut best: Option<Desperado> = None;
    for mv in legal_moves_vec(&mut scratch) {
        if mv.from() != piece_sq {
            continue;
        }
        if !pos.is_capture(mv) || !pos.gives_check(mv) {
            continue;
        }
        // What does it capture? En passant always takes a pawn; otherwise
        // the piece standing on the destination square.
        let captured = captured_kind(pos, mv);
        let Some(captured) = captured else {
            continue;
        };
        let recovered_cp = Value::mg_of_piece(captured).0;
        if recovered_cp <= 0 {
            continue;
        }
        let cand = Desperado {
            piece: piece_sq,
            captures_on: mv.to(),
            captured,
            recovered_cp,
        };
        best = Some(match best {
            None => cand,
            Some(prev) if cand.recovered_cp > prev.recovered_cp => cand,
            Some(prev) => prev,
        });
    }
    best
}

/// The kind of piece `mv` captures, resolved against `pos`. `None` for a
/// quiet move or castling. En passant always takes a pawn.
fn captured_kind(pos: &Position, mv: crate::types::Move) -> Option<PieceType> {
    use crate::types::MoveKind;
    match mv.kind() {
        MoveKind::Castling => None,
        MoveKind::EnPassant => Some(PieceType::Pawn),
        MoveKind::Normal | MoveKind::Promotion => pos.piece_on(mv.to()).map(|p| p.kind()),
    }
}
