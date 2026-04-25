//! Structured per-capture story along the user's PV.
//!
//! The engine produces raw `CaptureEvent`s; renderers (CLI, Swift,
//! Kotlin) turn them into whatever prose they want. Nothing about
//! the sequence — even-trade detection, piece grouping, summary
//! phrasing — happens at this layer; that all belongs in the
//! presentation layer.

use super::MoveAnalysis;
use crate::position::Position;
use crate::types::{Color, Move, MoveKind, PieceType, Square, Value};

/// One capture that resolves along the user's principal variation.
/// Ordered chronologically via `ply` (0 = the user's own move).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CaptureEvent {
    /// Index into the move-analysis PV: `0` is the user's move, `1`
    /// is the opponent's reply, etc.
    pub ply: usize,
    /// Color of the side whose move captured — *not* the captured
    /// piece's color. The captured piece is always the opposite
    /// colour.
    pub captor: Color,
    /// Kind of piece that made the capturing move. For a promotion
    /// capture this is the pre-promotion piece (`Pawn`); the
    /// promoted piece kind isn't captured data, it's post-move state.
    pub captor_piece: PieceType,
    /// Kind of piece that was captured.
    pub captured_piece: PieceType,
    /// Square on which the capture resolved — the move's `to` square
    /// for normal captures and for en passant (en passant's captured
    /// pawn sits on a different square but the move resolves at
    /// `to`).
    pub square: Square,
    /// Midgame piece-value of the captured piece (engine-cp).
    pub value_mg: i32,
    /// Endgame piece-value of the captured piece (engine-cp).
    pub value_eg: i32,
}

/// Aggregate material story along the user's PV through the settled
/// ply. Purely structured data — no prose — so platform renderers
/// can phrase it however they like.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterialOutcome {
    /// Every capture walked through, in ply order.
    pub events: Vec<CaptureEvent>,
    /// Net material change from `root_stm`'s POV using midgame piece
    /// values. Positive = we won material; negative = we lost it.
    /// In engine-cp (a full pawn is 128). Uses midgame values
    /// because captures are, by definition, a tactical
    /// phenomenon — the midgame table is the intuitive "point
    /// value" chart.
    pub net_mg_cp: i32,
    /// Net material change using endgame values, in engine-cp.
    /// Useful for scaling the phrasing in late endgames where a
    /// pawn is relatively more valuable (PAWN_EG = 213 vs
    /// PAWN_MG = 128).
    pub net_eg_cp: i32,
    /// Last ply index walked — normally `settled_ply`, or PV length
    /// minus one when the PV never formally "settled." UIs that
    /// want to render "by move N" lift this through the SAN lookup.
    pub last_ply: usize,
}

/// Walk `ma.pv` from `pre_move_pos` up through the settled ply (or
/// PV end if none), recording every capture. Returns a
/// [`MaterialOutcome`] summarizing the sequence from `root_stm`'s
/// POV.
///
/// `pre_move_pos` must be the position the user was about to move
/// from — same position `analyze_position` was called with. We
/// clone it internally before replaying; the caller's position is
/// not mutated.
///
/// Edge cases handled:
/// - **Empty PV** (terminal root, which shouldn't reach this path
///   but is cheap to guard): returns an outcome with no events and
///   zero net.
/// - **En passant**: captured pawn is on a different square than
///   `to`, but we record `square = to` (where the capture
///   resolved).
/// - **Promotion captures**: captured piece is read from `to`
///   before the move; captor is `Pawn` (the pre-promotion piece).
/// - **Castling**: never a capture — skipped.
pub fn compute_material_outcome(
    ma: &MoveAnalysis,
    pre_move_pos: &Position,
    root_stm: Color,
) -> MaterialOutcome {
    let mut events = Vec::new();
    let mut scratch = pre_move_pos.clone();

    let last_ply = match ma.settled_ply {
        Some(idx) if idx < ma.pv.len() => idx,
        _ => ma.pv.len().saturating_sub(1),
    };

    for (ply, &mv) in ma.pv.iter().enumerate() {
        // Resolve the capture *before* applying the move — after
        // do_move, both squares reflect post-move state.
        if let Some(event) = capture_event_for(ply, mv, &scratch) {
            events.push(event);
        }
        scratch.do_move(mv);
        if ply >= last_ply {
            break;
        }
    }

    let (net_mg_cp, net_eg_cp) = events.iter().fold((0, 0), |(mg, eg), ev| {
        let sign = if ev.captor == root_stm { 1 } else { -1 };
        (mg + sign * ev.value_mg, eg + sign * ev.value_eg)
    });

    MaterialOutcome {
        events,
        net_mg_cp,
        net_eg_cp,
        last_ply,
    }
}

fn capture_event_for(ply: usize, mv: Move, pos: &Position) -> Option<CaptureEvent> {
    let captor_piece = pos.piece_on(mv.from())?;
    let captor = captor_piece.color();
    match mv.kind() {
        MoveKind::Castling => None,
        MoveKind::EnPassant => {
            // En passant always captures a pawn; the captured pawn
            // is behind `to` relative to the captor's direction. We
            // don't need to resolve its square for narration —
            // `mv.to()` is where the capture *lands*, which is what
            // the student saw.
            Some(CaptureEvent {
                ply,
                captor,
                captor_piece: captor_piece.kind(),
                captured_piece: PieceType::Pawn,
                square: mv.to(),
                value_mg: Value::mg_of_piece(PieceType::Pawn).0,
                value_eg: Value::eg_of_piece(PieceType::Pawn).0,
            })
        }
        MoveKind::Normal | MoveKind::Promotion => {
            let captured = pos.piece_on(mv.to())?;
            Some(CaptureEvent {
                ply,
                captor,
                captor_piece: captor_piece.kind(),
                captured_piece: captured.kind(),
                square: mv.to(),
                value_mg: Value::mg_of_piece(captured.kind()).0,
                value_eg: Value::eg_of_piece(captured.kind()).0,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::ma_with_pv;
    use super::*;

    #[test]
    fn material_outcome_simple_even_recapture() {
        // White bishop on d5, black knight on f6, black to move.
        // Sequence: Nxd5 (captures bishop), exd5 (recaptures
        // knight). Net from black's POV: +bishop (825) - knight
        // (781) = +44 mg.
        let fen = "rnbqkb1r/ppp2ppp/5n2/3Bp3/4P3/5Q2/PPPP1PPP/RNB1K1NR b KQkq - 0 4";
        let pos = Position::from_fen(fen).unwrap();
        let nxd5 = Move::normal(Square::F6, Square::D5);
        let exd5 = Move::normal(Square::E4, Square::D5);
        let ma = ma_with_pv(vec![nxd5, exd5], Some(1));

        let outcome = compute_material_outcome(&ma, &pos, Color::Black);

        assert_eq!(outcome.events.len(), 2);
        assert_eq!(outcome.events[0].ply, 0);
        assert_eq!(outcome.events[0].captor, Color::Black);
        assert_eq!(outcome.events[0].captor_piece, PieceType::Knight);
        assert_eq!(outcome.events[0].captured_piece, PieceType::Bishop);
        assert_eq!(outcome.events[0].square, Square::D5);
        assert_eq!(outcome.events[0].value_mg, Value::BISHOP_MG.0);

        assert_eq!(outcome.events[1].ply, 1);
        assert_eq!(outcome.events[1].captor, Color::White);
        assert_eq!(outcome.events[1].captor_piece, PieceType::Pawn);
        assert_eq!(outcome.events[1].captured_piece, PieceType::Knight);

        assert_eq!(outcome.net_mg_cp, Value::BISHOP_MG.0 - Value::KNIGHT_MG.0);
    }

    #[test]
    fn material_outcome_non_capture_pv_has_zero_events() {
        let pos = Position::startpos();
        let e4 = Move::normal(Square::E2, Square::E4);
        let e5 = Move::normal(Square::E7, Square::E5);
        let ma = ma_with_pv(vec![e4, e5], Some(1));
        let outcome = compute_material_outcome(&ma, &pos, Color::White);
        assert!(outcome.events.is_empty());
        assert_eq!(outcome.net_mg_cp, 0);
        assert_eq!(outcome.net_eg_cp, 0);
    }

    #[test]
    fn material_outcome_stops_at_settled_ply() {
        let fen = "rnbqkb1r/ppp2ppp/5n2/3Bp3/4P3/5Q2/PPPP1PPP/RNB1K1NR b KQkq - 0 4";
        let pos = Position::from_fen(fen).unwrap();
        let nxd5 = Move::normal(Square::F6, Square::D5);
        let exd5 = Move::normal(Square::E4, Square::D5);
        let ma = ma_with_pv(vec![nxd5, exd5], Some(0));
        let outcome = compute_material_outcome(&ma, &pos, Color::Black);
        assert_eq!(outcome.events.len(), 1);
        assert_eq!(outcome.events[0].ply, 0);
        assert_eq!(outcome.last_ply, 0);
    }

    #[test]
    fn material_outcome_en_passant_records_pawn_capture() {
        let fen = "rnbqkbnr/ppp1p1pp/8/3pPp2/8/8/PPPP1PPP/RNBQKBNR w KQkq f6 0 3";
        let pos = Position::from_fen(fen).unwrap();
        let exf6_ep = Move::en_passant(Square::E5, Square::F6);
        let ma = ma_with_pv(vec![exf6_ep], Some(0));
        let outcome = compute_material_outcome(&ma, &pos, Color::White);
        assert_eq!(outcome.events.len(), 1);
        let ev = &outcome.events[0];
        assert_eq!(ev.captor, Color::White);
        assert_eq!(ev.captor_piece, PieceType::Pawn);
        assert_eq!(ev.captured_piece, PieceType::Pawn);
        assert_eq!(ev.value_mg, Value::PAWN_MG.0);
        assert_eq!(outcome.net_mg_cp, Value::PAWN_MG.0);
    }

    #[test]
    fn material_outcome_promotion_capture_records_captured_piece() {
        let fen = "1r2k3/P7/8/8/8/8/8/4K3 w - - 0 1";
        let pos = Position::from_fen(fen).unwrap();
        let axb8q = Move::promotion(Square::A7, Square::B8, PieceType::Queen);
        let ma = ma_with_pv(vec![axb8q], Some(0));
        let outcome = compute_material_outcome(&ma, &pos, Color::White);
        assert_eq!(outcome.events.len(), 1);
        let ev = &outcome.events[0];
        assert_eq!(
            ev.captor_piece,
            PieceType::Pawn,
            "captor is pre-promotion piece"
        );
        assert_eq!(ev.captured_piece, PieceType::Rook);
        assert_eq!(ev.value_mg, Value::ROOK_MG.0);
    }

    #[test]
    fn material_outcome_castling_is_not_a_capture() {
        let fen = "rnbqk2r/pppp1ppp/5n2/4p3/1bB1P3/2N2N2/PPPP1PPP/R1BQK2R w KQkq - 0 1";
        let pos = Position::from_fen(fen).unwrap();
        let castle = Move::castling(Square::E1, Square::G1);
        let ma = ma_with_pv(vec![castle], Some(0));
        let outcome = compute_material_outcome(&ma, &pos, Color::White);
        assert!(outcome.events.is_empty());
        assert_eq!(outcome.net_mg_cp, 0);
    }

    #[test]
    fn material_outcome_sign_flips_with_pov() {
        let fen = "rnbqkb1r/ppp2ppp/5n2/3Bp3/4P3/5Q2/PPPP1PPP/RNB1K1NR b KQkq - 0 4";
        let pos = Position::from_fen(fen).unwrap();
        let nxd5 = Move::normal(Square::F6, Square::D5);
        let ma = ma_with_pv(vec![nxd5], Some(0));

        let from_black = compute_material_outcome(&ma, &pos, Color::Black);
        assert_eq!(from_black.net_mg_cp, Value::BISHOP_MG.0);

        let from_white = compute_material_outcome(&ma, &pos, Color::White);
        assert_eq!(from_white.net_mg_cp, -Value::BISHOP_MG.0);
    }

    #[test]
    fn material_outcome_empty_pv_is_empty_outcome() {
        let pos = Position::startpos();
        let ma = ma_with_pv(Vec::new(), None);
        let outcome = compute_material_outcome(&ma, &pos, Color::White);
        assert!(outcome.events.is_empty());
        assert_eq!(outcome.net_mg_cp, 0);
        assert_eq!(outcome.last_ply, 0);
    }

    #[test]
    fn material_outcome_no_settled_falls_back_to_pv_end() {
        let fen = "rnbqkb1r/ppp2ppp/5n2/3Bp3/4P3/5Q2/PPPP1PPP/RNB1K1NR b KQkq - 0 4";
        let pos = Position::from_fen(fen).unwrap();
        let nxd5 = Move::normal(Square::F6, Square::D5);
        let exd5 = Move::normal(Square::E4, Square::D5);
        let ma = ma_with_pv(vec![nxd5, exd5], None);
        let outcome = compute_material_outcome(&ma, &pos, Color::Black);
        assert_eq!(outcome.events.len(), 2);
        assert_eq!(outcome.last_ply, 1);
    }
}
