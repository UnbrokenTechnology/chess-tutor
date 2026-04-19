//! Square-level analysis: attacker/defender maps, Static Exchange Evaluation,
//! candidate move annotation.

use serde::{Deserialize, Serialize};
use shakmaty::attacks;
use shakmaty::{Bitboard, Board, Chess, Color, Position, Role, Square};

/// Per-square attacker/defender data for the current position.
///
/// 64 entries indexed by `Square as usize` (a1 = 0 .. h8 = 63).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SquareData {
    pub squares: Vec<SquareReport>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SquareReport {
    pub white_attackers: u8,
    pub black_attackers: u8,
    /// SEE value for capturing on this square with the side to move.
    /// Populated in a later Phase 1 step.
    pub see: Option<i32>,
}

/// Per-square attacker counts, plus the ability to enumerate the actual
/// attacker squares when the UI needs them (e.g. for a "who attacks this
/// piece?" listing).
///
/// Pure function of position — no analysis state or configuration. Built via
/// [`AttackMap::from_position`].
#[derive(Debug, Clone)]
pub struct AttackMap {
    white_attackers: [Bitboard; 64],
    black_attackers: [Bitboard; 64],
}

impl AttackMap {
    pub fn from_position(pos: &Chess) -> Self {
        let board = pos.board();
        let occupied = board.occupied();

        let mut white_attackers = [Bitboard::EMPTY; 64];
        let mut black_attackers = [Bitboard::EMPTY; 64];

        for sq in Square::ALL {
            let idx = usize::from(sq);
            white_attackers[idx] = attackers_of(board, sq, Color::White, occupied);
            black_attackers[idx] = attackers_of(board, sq, Color::Black, occupied);
        }

        Self {
            white_attackers,
            black_attackers,
        }
    }

    /// Attackers of the given colour to the given square, as a bitboard of
    /// their source squares. Iterate with `.into_iter()` to get individual
    /// source squares.
    pub fn attackers(&self, target: Square, color: Color) -> Bitboard {
        match color {
            Color::White => self.white_attackers[usize::from(target)],
            Color::Black => self.black_attackers[usize::from(target)],
        }
    }

    pub fn count(&self, target: Square, color: Color) -> u8 {
        self.attackers(target, color).count() as u8
    }

    /// True if the piece on `sq` is hanging — i.e. attacked by more pieces of
    /// the opposing colour than it is defended by pieces of its own colour.
    /// Returns `false` for empty squares and the king (which can't be "hanging"
    /// in a meaningful sense — if attacked it's a check, not a capture
    /// opportunity).
    pub fn is_hanging(&self, board: &Board, sq: Square) -> bool {
        let Some(piece) = board.piece_at(sq) else {
            return false;
        };
        if piece.role == Role::King {
            return false;
        }
        let defenders = self.count(sq, piece.color);
        let attackers = self.count(sq, piece.color.other());
        attackers > defenders
    }

    /// Build a compact per-square report for serialisation and FFI.
    pub fn to_square_data(&self) -> SquareData {
        let squares = Square::ALL
            .map(|sq| {
                let idx = usize::from(sq);
                SquareReport {
                    white_attackers: self.white_attackers[idx].count() as u8,
                    black_attackers: self.black_attackers[idx].count() as u8,
                    see: None,
                }
            })
            .to_vec();
        SquareData { squares }
    }
}

/// Return all pieces of `color` that attack `target`, given the current
/// occupancy. Uses shakmaty's free attack-generation functions to avoid
/// depending on a specific `Board::attacks_to` signature.
///
/// The pawn trick: pawns of colour X attack `target` iff a pawn of the
/// opposite colour *at* `target` would attack their squares — so we use
/// `pawn_attacks(!color, target)` and intersect with our own pawns.
fn attackers_of(board: &Board, target: Square, color: Color, occupied: Bitboard) -> Bitboard {
    let by_color = board.by_color(color);
    let mut result = Bitboard::EMPTY;

    result |= attacks::pawn_attacks(color.other(), target) & board.pawns() & by_color;
    result |= attacks::knight_attacks(target) & board.knights() & by_color;
    result |= attacks::bishop_attacks(target, occupied)
        & (board.bishops() | board.queens())
        & by_color;
    result |= attacks::rook_attacks(target, occupied)
        & (board.rooks() | board.queens())
        & by_color;
    result |= attacks::king_attacks(target) & board.kings() & by_color;

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use shakmaty::fen::Fen;
    use shakmaty::{CastlingMode, Chess};

    fn pos(fen: &str) -> Chess {
        fen.parse::<Fen>()
            .unwrap()
            .into_position(CastlingMode::Standard)
            .unwrap()
    }

    #[test]
    fn startpos_attack_counts() {
        let p = pos("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
        let am = AttackMap::from_position(&p);

        // e4 from startpos is attacked by exactly one white piece (the Nf3?
        // no — knights are on g1/b1, neither attacks e4 from startpos).
        // Let's check squares we actually know:
        //
        // e3: attacked by pawns on d2, f2; knight on g1 — 3 white attackers.
        //     no black piece reaches it across the pawn wall.
        assert_eq!(am.count(Square::E3, Color::White), 3);
        assert_eq!(am.count(Square::E3, Color::Black), 0);

        // a3: attacked by pawn on b2, knight on b1 — 2 white attackers.
        assert_eq!(am.count(Square::A3, Color::White), 2);
    }

    #[test]
    fn hanging_piece_detected() {
        // White pawn on d5 attacked by c6 pawn, defended by nothing.
        let p = pos("8/8/2p5/3P4/8/8/8/4K2k w - - 0 1");
        let am = AttackMap::from_position(&p);
        assert!(am.is_hanging(p.board(), Square::D5));
    }

    #[test]
    fn defended_piece_not_hanging() {
        // White pawn on d5 attacked by c6 pawn, defended by e4 pawn.
        let p = pos("8/8/2p5/3P4/4P3/8/8/4K2k w - - 0 1");
        let am = AttackMap::from_position(&p);
        assert!(!am.is_hanging(p.board(), Square::D5));
    }

    #[test]
    fn king_is_never_hanging() {
        // White king in check from a rook, no defenders. `is_hanging` still
        // reports false — kings are checked, not hung.
        let p = pos("4k3/8/8/8/8/8/8/r3K3 w - - 0 1");
        let am = AttackMap::from_position(&p);
        assert!(!am.is_hanging(p.board(), Square::E1));
    }

    #[test]
    fn attacker_bitboard_lists_source_squares() {
        // White knight on c3 attacks e4. Black pawn on d5 does too.
        let p = pos("4k3/8/8/3p4/8/2N5/8/4K3 w - - 0 1");
        let am = AttackMap::from_position(&p);

        let white = am.attackers(Square::E4, Color::White);
        let black = am.attackers(Square::E4, Color::Black);

        assert_eq!(white, Bitboard::from_square(Square::C3));
        assert_eq!(black, Bitboard::from_square(Square::D5));
    }
}

/// A legal move annotated with the analysis hooks the explainer consumes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CandidateMove {
    pub uci: String,
    pub san: String,
    pub material_change: i32,
    pub see: Option<i32>,
    pub gives_check: bool,
    pub is_capture: bool,
    /// Names of tactical motifs this move creates or executes, e.g. "fork".
    pub tactics: Vec<String>,
    /// Names of positional features this move changes, e.g. "opens-d-file".
    pub positional: Vec<String>,
    pub rank: u32,
}
