//! UCI move notation — format an engine [`Move`] as `e2e4` / `e7e8q`
//! and parse a UCI string into a [`Move`] by matching the string
//! against the current position's legal move list.
//!
//! Matching against legal moves, rather than synthesising a [`Move`]
//! from the string alone, is deliberate: the engine's `Move` encoding
//! needs a [`MoveKind`] tag (`Castling`, `EnPassant`, `Promotion`,
//! `Normal`) that a UCI string doesn't carry, so we let the move
//! generator settle those.

use chess_tutor_engine::movegen::legal_moves_vec;
use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::{Move, MoveKind, PieceType, Square};

/// Format a move as a UCI string (e.g. `e2e4`, `e7e8q`). Castling
/// follows the standard UCI convention of king-from / king-to
/// (`e1g1` for white king-side), which is how the engine encodes
/// the move internally.
pub fn format(mv: Move) -> String {
    let mut s = String::with_capacity(5);
    s.push_str(&mv.from().to_algebraic());
    s.push_str(&mv.to().to_algebraic());
    if mv.kind() == MoveKind::Promotion {
        s.push(match mv.promoted_to() {
            PieceType::Queen => 'q',
            PieceType::Rook => 'r',
            PieceType::Bishop => 'b',
            PieceType::Knight => 'n',
            _ => '?',
        });
    }
    s
}

/// Parse a UCI move string and return the matching legal [`Move`] for
/// the supplied position. Returns `Err` if the string is malformed or
/// no legal move matches.
pub fn parse(pos: &mut Position, s: &str) -> Result<Move, String> {
    let s = s.trim().to_ascii_lowercase();
    if !(s.len() == 4 || s.len() == 5) {
        return Err(format!("UCI moves are 4 or 5 characters, got {:?}", s));
    }
    let from =
        Square::from_algebraic(&s[0..2]).ok_or_else(|| format!("bad from-square in {:?}", s))?;
    let to = Square::from_algebraic(&s[2..4]).ok_or_else(|| format!("bad to-square in {:?}", s))?;
    let promo = if s.len() == 5 {
        Some(
            promo_from_char(s.as_bytes()[4] as char)
                .ok_or_else(|| format!("bad promotion piece in {:?}", s))?,
        )
    } else {
        None
    };

    let legal = legal_moves_vec(pos);
    for mv in legal {
        if mv.from() != from || mv.to() != to {
            continue;
        }
        match (mv.kind(), promo) {
            (MoveKind::Promotion, Some(p)) if mv.promoted_to() == p => return Ok(mv),
            (MoveKind::Promotion, None) => continue,
            (_, Some(_)) => continue,
            _ => return Ok(mv),
        }
    }
    Err(format!("no legal move matches {:?}", s))
}

fn promo_from_char(c: char) -> Option<PieceType> {
    Some(match c {
        'q' => PieceType::Queen,
        'r' => PieceType::Rook,
        'b' => PieceType::Bishop,
        'n' => PieceType::Knight,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_normal_move() {
        let mv = Move::normal(Square::E2, Square::E4);
        assert_eq!(format(mv), "e2e4");
    }

    #[test]
    fn format_promotion() {
        let mv = Move::promotion(Square::E7, Square::E8, PieceType::Queen);
        assert_eq!(format(mv), "e7e8q");
    }

    #[test]
    fn parse_roundtrip_startpos() {
        let mut pos = Position::startpos();
        let mv = parse(&mut pos, "e2e4").unwrap();
        assert_eq!(format(mv), "e2e4");
    }

    #[test]
    fn parse_rejects_illegal_move() {
        let mut pos = Position::startpos();
        assert!(parse(&mut pos, "e2e5").is_err());
    }

    #[test]
    fn parse_kingside_castle() {
        let mut pos =
            Position::from_fen("r3k2r/pppbqppp/2n2n2/3pp3/3PP3/2N2N2/PPPBQPPP/R3K2R w KQkq - 0 1")
                .unwrap();
        let mv = parse(&mut pos, "e1g1").unwrap();
        assert_eq!(mv.kind(), MoveKind::Castling);
    }

    #[test]
    fn parse_promotion_requires_piece_letter() {
        let mut pos = Position::from_fen("8/4P3/8/8/8/8/8/4K2k w - - 0 1").unwrap();
        assert!(
            parse(&mut pos, "e7e8").is_err(),
            "must specify promotion piece"
        );
        let mv = parse(&mut pos, "e7e8q").unwrap();
        assert_eq!(mv.kind(), MoveKind::Promotion);
        assert_eq!(mv.promoted_to(), PieceType::Queen);
    }
}
