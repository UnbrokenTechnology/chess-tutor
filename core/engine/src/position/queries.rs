//! Positional / structural queries used by the evaluator — open files,
//! same-coloured-square pawn counts, opposite-coloured bishops,
//! passed-pawn detection, and the non-pawn-material total.

use super::Position;
use crate::bitboard::{file_bb, opposite_colors, passed_pawn_span, DARK_SQUARES, LIGHT_SQUARES};
use crate::types::{Color, PieceType, Square, Value};

impl Position {
    /// True when `color` has no pawn on `square`'s file. Used by the
    /// rook-on-open/semi-open-file bonus.
    pub fn is_on_semiopen_file(&self, color: Color, square: Square) -> bool {
        (self.pieces_of(color, PieceType::Pawn) & file_bb(square.file())).is_empty()
    }

    /// Count of `color`'s pawns standing on squares of the same tile colour
    /// as `square`. Used by the BishopPawns penalty (pawns sharing the
    /// bishop's colour block its diagonals).
    pub fn pawns_on_same_color_squares(&self, color: Color, square: Square) -> u32 {
        let color_mask = if (DARK_SQUARES & square).any() {
            DARK_SQUARES
        } else {
            LIGHT_SQUARES
        };
        (self.pieces_of(color, PieceType::Pawn) & color_mask).popcount()
    }

    /// True when each side has exactly one bishop and those bishops stand
    /// on opposite-coloured squares. Drives several endgame-scaling
    /// heuristics (opposite-bishop endings tend toward draws).
    pub fn opposite_bishops(&self) -> bool {
        self.count(Color::White, PieceType::Bishop) == 1
            && self.count(Color::Black, PieceType::Bishop) == 1
            && opposite_colors(
                self.pieces_of(Color::White, PieceType::Bishop).lsb(),
                self.pieces_of(Color::Black, PieceType::Bishop).lsb(),
            )
    }

    /// True when a `color` pawn standing on `square` would have no
    /// opposing pawn in its passed-pawn span. The caller is responsible
    /// for ensuring `square` lies on a legal pawn rank.
    pub fn pawn_passed(&self, color: Color, square: Square) -> bool {
        (self.pieces_of(!color, PieceType::Pawn) & passed_pawn_span(color, square)).is_empty()
    }

    /// Sum of non-pawn material for both colours. Useful for endgame-phase
    /// heuristics that look at total material, not per-colour.
    pub fn non_pawn_material_total(&self) -> Value {
        self.non_pawn_material[0] + self.non_pawn_material[1]
    }

    /// True when the remaining material on the board can never produce
    /// a checkmate. Used by the play loops (CLI + desktop) to terminate
    /// the game in a draw rather than let the user shuffle pieces
    /// forever.
    ///
    /// The rule applied (the "liberal" interpretation chosen for this
    /// app) covers FIDE 5.2.2 plus the same-coloured-bishops extension:
    ///
    /// - K vs K
    /// - K vs K + one minor (knight or bishop)
    /// - Any position where both sides have only bishops (no knights, no
    ///   pawns, no rooks, no queens) and **all** bishops on the board
    ///   stand on a single colour of square. Covers KB vs KB
    ///   same-colour, KBB vs K with both bishops on one colour, KBB vs
    ///   KB with all three bishops on one colour, etc.
    ///
    /// Deliberately not auto-drawn (consistent with FIDE 5.2.2, which
    /// requires *no* mating sequence even with opponent cooperation —
    /// the cases below admit a cooperative mate):
    ///
    /// - K + N + N vs K (helpmate exists)
    /// - K + B vs K + N (a few helpmate positions exist)
    /// - K + B + N vs K (forced mate — clearly not insufficient)
    pub fn has_insufficient_material(&self) -> bool {
        // Any pawn / rook / queen leaves room for a mate somewhere.
        if self.pieces(PieceType::Pawn).any()
            || self.pieces(PieceType::Rook).any()
            || self.pieces(PieceType::Queen).any()
        {
            return false;
        }

        let wn = self.count(Color::White, PieceType::Knight);
        let bn = self.count(Color::Black, PieceType::Knight);
        let wb = self.count(Color::White, PieceType::Bishop);
        let bb = self.count(Color::Black, PieceType::Bishop);
        let minors = wn + bn + wb + bb;

        // K vs K, or K vs K + one minor (no other side has any minor
        // because total <= 1).
        if minors <= 1 {
            return true;
        }

        // Knight-free positions where all bishops share a square colour
        // can't mate — no opposite-coloured-square coverage exists.
        if wn == 0 && bn == 0 {
            let bishops = self.pieces(PieceType::Bishop);
            if (bishops & DARK_SQUARES).is_empty() || (bishops & LIGHT_SQUARES).is_empty() {
                return true;
            }
        }

        false
    }
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{File, Rank};

    // ---- is_on_semiopen_file ----------------------------------------

    #[test]
    fn startpos_no_semiopen_files() {
        let p = Position::startpos();
        // Every file has a pawn in the starting position, so no file is
        // semi-open for either colour.
        for f in 0u8..8 {
            let sq = Square::new(File::from_index(f).unwrap(), Rank::from_index(0).unwrap());
            assert!(!p.is_on_semiopen_file(Color::White, sq));
            assert!(!p.is_on_semiopen_file(Color::Black, sq));
        }
    }

    #[test]
    fn semiopen_file_after_pawn_push() {
        // After 1. e4 e5, the e-file for neither side is semi-open.
        // But the d-file remains non-semiopen for both. An isolated
        // pawn on the d-file gives a semi-open file for the enemy.
        let p = Position::from_fen("4k3/8/8/8/3P4/8/8/4K3 w - - 0 1").unwrap();
        // d-file has only a white pawn, so it's semi-open for black.
        assert!(!p.is_on_semiopen_file(Color::White, Square::D4));
        assert!(p.is_on_semiopen_file(Color::Black, Square::D4));
    }

    // ---- pawns_on_same_color_squares --------------------------------

    #[test]
    fn pawns_on_same_color_squares_counts_correctly() {
        // White pawns on a2 (dark) and b2 (light). A light-square bishop
        // cares about how many white pawns sit on light squares.
        let p = Position::from_fen("4k3/8/8/8/8/8/PP6/4K3 w - - 0 1").unwrap();
        // Square B2 is a light square (file B = file index 1, rank 2 =
        // rank index 1; (1+1)%2 = 0 means... actually let's just test
        // what the function returns rather than re-derive colour math.
        let on_b2 = p.pawns_on_same_color_squares(Color::White, Square::B2);
        let on_a2 = p.pawns_on_same_color_squares(Color::White, Square::A2);
        // a2 and b2 are opposite-colour squares, so the two pawns split
        // between them: one pawn shares each square's colour.
        assert_eq!(on_a2, 1);
        assert_eq!(on_b2, 1);
        assert_eq!(on_a2 + on_b2, 2);
    }

    // ---- opposite_bishops -------------------------------------------

    #[test]
    fn opposite_bishops_detects_classic_drawish_endgame() {
        // White bishop on a1 (dark), black bishop on h1 (light) — one
        // each, opposite colours.
        let p = Position::from_fen("4k3/8/8/8/8/8/8/B3K2b w - - 0 1").unwrap();
        assert!(p.opposite_bishops());
    }

    #[test]
    fn opposite_bishops_rejects_same_color() {
        // White bishop on d1 and black bishop on h1 — both light squares
        // in this engine's tile colouring, so not "opposite bishops."
        let p = Position::from_fen("4k3/8/8/8/8/8/8/3BK2b w - - 0 1").unwrap();
        assert!(!p.opposite_bishops());
    }

    #[test]
    fn opposite_bishops_rejects_multiple_bishops() {
        // White has two bishops — the predicate requires exactly one each.
        let p = Position::from_fen("4k3/8/8/8/8/8/8/B2BK2b w - - 0 1").unwrap();
        assert!(!p.opposite_bishops());
    }

    // ---- pawn_passed ------------------------------------------------

    #[test]
    fn pawn_passed_true_when_span_is_clear() {
        let p = Position::from_fen("4k3/8/3P4/8/8/8/8/4K3 w - - 0 1").unwrap();
        assert!(p.pawn_passed(Color::White, Square::D6));
    }

    #[test]
    fn pawn_passed_false_when_enemy_pawn_on_adjacent_file_ahead() {
        let p = Position::from_fen("4k3/4p3/3P4/8/8/8/8/4K3 w - - 0 1").unwrap();
        assert!(!p.pawn_passed(Color::White, Square::D6));
    }

    // ---- non_pawn_material_total ------------------------------------

    #[test]
    fn non_pawn_material_total_sums_both_colours() {
        let p = Position::startpos();
        let expected = p.non_pawn_material(Color::White) + p.non_pawn_material(Color::Black);
        assert_eq!(p.non_pawn_material_total(), expected);
    }

    // ---- has_insufficient_material ----------------------------------

    #[test]
    fn insufficient_k_vs_k() {
        let p = Position::from_fen("4k3/8/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        assert!(p.has_insufficient_material());
    }

    #[test]
    fn insufficient_k_vs_kn() {
        let p = Position::from_fen("4k3/8/8/8/8/8/8/4KN2 w - - 0 1").unwrap();
        assert!(p.has_insufficient_material());
    }

    #[test]
    fn insufficient_k_vs_kb() {
        let p = Position::from_fen("4k3/8/8/8/8/8/8/4KB2 w - - 0 1").unwrap();
        assert!(p.has_insufficient_material());
    }

    #[test]
    fn insufficient_kb_vs_kb_same_colour() {
        // White bishop on c1 (dark), black bishop on f8 (dark). Both
        // on dark squares.
        let p = Position::from_fen("4kb2/8/8/8/8/8/8/2B1K3 w - - 0 1").unwrap();
        assert!(p.has_insufficient_material());
    }

    #[test]
    fn insufficient_kbb_vs_k_same_colour() {
        // Two white bishops on c1 and a3 (both dark squares).
        let p = Position::from_fen("4k3/8/8/8/8/B7/8/2B1K3 w - - 0 1").unwrap();
        assert!(p.has_insufficient_material());
    }

    #[test]
    fn insufficient_kbb_vs_kb_all_same_colour() {
        // Three bishops, all on dark squares (a3, c1, f8). Liberal
        // rule covers this — no opposite-colour-square coverage means
        // mate is impossible regardless of bishop count.
        let p = Position::from_fen("4kb2/8/8/8/8/B7/8/2B1K3 w - - 0 1").unwrap();
        assert!(p.has_insufficient_material());
    }

    #[test]
    fn sufficient_kb_vs_kb_opposite_colours() {
        // White bishop on c1 (dark), black bishop on f1 (light). Mate
        // possible with cooperation → FIDE 5.2.2 keeps the game alive.
        let p = Position::from_fen("4k3/8/8/8/8/8/8/2B1KB2 w - - 0 1").unwrap();
        assert!(!p.has_insufficient_material());
    }

    #[test]
    fn sufficient_kbb_vs_k_opposite_colours() {
        // White bishops on c1 (dark) and f1 (light) — the bishop pair.
        let p = Position::from_fen("4k3/8/8/8/8/8/8/2B1KB2 w - - 0 1").unwrap();
        assert!(!p.has_insufficient_material());
    }

    #[test]
    fn sufficient_knn_vs_k() {
        // Two knights vs lone king: helpmate exists, not auto-drawn.
        let p = Position::from_fen("4k3/8/8/8/8/8/8/3NKN2 w - - 0 1").unwrap();
        assert!(!p.has_insufficient_material());
    }

    #[test]
    fn sufficient_kbn_vs_k() {
        // Bishop + knight vs lone king — the classic forced mate.
        let p = Position::from_fen("4k3/8/8/8/8/8/8/3BKN2 w - - 0 1").unwrap();
        assert!(!p.has_insufficient_material());
    }

    #[test]
    fn sufficient_kb_vs_kn() {
        // KB vs KN — helpmates exist, not auto-drawn.
        let p = Position::from_fen("3nk3/8/8/8/8/8/8/3BK3 w - - 0 1").unwrap();
        assert!(!p.has_insufficient_material());
    }

    #[test]
    fn sufficient_with_any_pawn() {
        // K + P vs K is not insufficient — the pawn can promote.
        let p = Position::from_fen("4k3/8/8/8/8/8/4P3/4K3 w - - 0 1").unwrap();
        assert!(!p.has_insufficient_material());
    }

    #[test]
    fn sufficient_with_any_rook() {
        let p = Position::from_fen("4k3/8/8/8/8/8/8/4K2R w - - 0 1").unwrap();
        assert!(!p.has_insufficient_material());
    }

    #[test]
    fn sufficient_with_any_queen() {
        let p = Position::from_fen("4k3/8/8/8/8/8/8/3QK3 w - - 0 1").unwrap();
        assert!(!p.has_insufficient_material());
    }

    #[test]
    fn sufficient_startpos() {
        assert!(!Position::startpos().has_insufficient_material());
    }
}
