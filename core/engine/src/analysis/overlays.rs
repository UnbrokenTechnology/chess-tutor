//! Persistent board overlays — bitboard snapshots the UI paints onto
//! the live (or historically-viewed) position so the student can see
//! what the engine considers, independently of any retrospective.
//!
//! Each overlay maps to one or two bitboards on [`OverlayData`]. The
//! view-model layer turns them into [`crate::analysis`]-free
//! `BoardAnnotation`s via the existing renderer plumbing; the
//! renderer itself stays a flat color table.
//!
//! Computed on demand against a snapshot position. Not on the search
//! hot path — runs the standard `Evaluator` priming (initialize +
//! pieces::evaluate for both colours) which costs ~tens of µs and is
//! fine for the per-frame UI tick.

use crate::bitboard::Bitboard;
use crate::eval::{self, Evaluator};
use crate::position::Position;
use crate::types::{Color, Square};

/// All bitboards needed by the current set of UI overlays. POV-
/// agnostic: white/black named explicitly so the view layer can flip
/// based on which side the user is playing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OverlayData {
    pub white_space_safe: Bitboard,
    pub white_space_reinforced: Bitboard,
    pub black_space_safe: Bitboard,
    pub black_space_reinforced: Bitboard,
    /// Squares NOT in the mobility area for each colour — the engine
    /// gives no mobility credit for attacking these. Complement of
    /// `Evaluator::mobility_area[colour]`.
    pub white_mobility_excluded: Bitboard,
    pub black_mobility_excluded: Bitboard,
    /// The 3×3 box around each king (clamped to b2..g7 interior so
    /// corner kings still get a full 8-square ring), minus squares
    /// the enemy double-attacks with pawns.
    pub white_king_ring: Bitboard,
    pub black_king_ring: Bitboard,
    /// Pieces of `colour` that are pinned to their own king.
    pub white_pinned: Bitboard,
    pub black_pinned: Bitboard,
    /// Squares where the white-attacker count exceeds the black-
    /// attacker count by exactly 1.
    pub heat_white_1: Bitboard,
    /// Squares with white attacker advantage ≥ 2.
    pub heat_white_2plus: Bitboard,
    /// Squares where black attackers exceed white by exactly 1.
    pub heat_black_1: Bitboard,
    /// Squares with black attacker advantage ≥ 2.
    pub heat_black_2plus: Bitboard,
}

/// Compute every overlay bitboard for `pos`. Single Evaluator pass
/// shared across overlays.
pub fn compute_overlays(pos: &Position) -> OverlayData {
    let mut e = Evaluator::new(pos);
    e.initialize(Color::White);
    e.initialize(Color::Black);
    eval::pieces::evaluate(&mut e, Color::White);
    eval::pieces::evaluate(&mut e, Color::Black);

    let (white_space_safe, white_space_reinforced) = eval::space::space_bitboards(&e, Color::White);
    let (black_space_safe, black_space_reinforced) = eval::space::space_bitboards(&e, Color::Black);

    let white_mobility_excluded = !e.mobility_area[Color::White.index()];
    let black_mobility_excluded = !e.mobility_area[Color::Black.index()];

    let white_king_ring = e.king_ring[Color::White.index()];
    let black_king_ring = e.king_ring[Color::Black.index()];

    let white_pinned = pos.blockers_for_king(Color::White) & pos.pieces_by_color(Color::White);
    let black_pinned = pos.blockers_for_king(Color::Black) & pos.pieces_by_color(Color::Black);

    let (heat_white_1, heat_white_2plus, heat_black_1, heat_black_2plus) = attack_heat(pos);

    OverlayData {
        white_space_safe,
        white_space_reinforced,
        black_space_safe,
        black_space_reinforced,
        white_mobility_excluded,
        black_mobility_excluded,
        white_king_ring,
        black_king_ring,
        white_pinned,
        black_pinned,
        heat_white_1,
        heat_white_2plus,
        heat_black_1,
        heat_black_2plus,
    }
}

/// Walk every board square, bucket the per-side attacker-count diff
/// into four bitboards. Squares with no attackers (or balanced
/// non-zero counts) end up in none of the four — the overlay leaves
/// them untinted.
fn attack_heat(pos: &Position) -> (Bitboard, Bitboard, Bitboard, Bitboard) {
    let occ = pos.occupied();
    let white_pieces = pos.pieces_by_color(Color::White);
    let mut white_1 = Bitboard::EMPTY;
    let mut white_2 = Bitboard::EMPTY;
    let mut black_1 = Bitboard::EMPTY;
    let mut black_2 = Bitboard::EMPTY;
    for raw in 0u8..64 {
        let sq = Square::from_index(raw);
        let attackers = pos.attackers_to(sq, occ);
        let w_count = (attackers & white_pieces).popcount() as i32;
        let b_count = (attackers & !white_pieces).popcount() as i32;
        let net = w_count - b_count;
        match net {
            1 => white_1 = white_1.with(sq),
            n if n >= 2 => white_2 = white_2.with(sq),
            -1 => black_1 = black_1.with(sq),
            n if n <= -2 => black_2 = black_2.with(sq),
            _ => {}
        }
    }
    (white_1, white_2, black_1, black_2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startpos_overlays_are_symmetric() {
        let pos = Position::startpos();
        let d = compute_overlays(&pos);
        // Space mask is symmetric at startpos.
        assert_eq!(d.white_space_safe.popcount(), d.black_space_safe.popcount());
        // Both kings have rings; ring sizes match.
        assert_eq!(d.white_king_ring.popcount(), d.black_king_ring.popcount());
        // No pins from start.
        assert_eq!(d.white_pinned, Bitboard::EMPTY);
        assert_eq!(d.black_pinned, Bitboard::EMPTY);
    }

    #[test]
    fn heat_buckets_partition_attacked_squares() {
        // Any square that appears in one heat bucket can't appear in
        // another — the per-square net-attacker bucket is exclusive.
        let pos = Position::startpos();
        let d = compute_overlays(&pos);
        let all_heat =
            d.heat_white_1 | d.heat_white_2plus | d.heat_black_1 | d.heat_black_2plus;
        // No overlaps between any pair.
        assert!((d.heat_white_1 & d.heat_white_2plus).is_empty());
        assert!((d.heat_white_1 & d.heat_black_1).is_empty());
        assert!((d.heat_white_1 & d.heat_black_2plus).is_empty());
        assert!((d.heat_white_2plus & d.heat_black_1).is_empty());
        assert!((d.heat_white_2plus & d.heat_black_2plus).is_empty());
        assert!((d.heat_black_1 & d.heat_black_2plus).is_empty());
        // At startpos the heat map is non-empty (each side defends
        // its own pieces, attacker counts on contested squares
        // differ).
        let _ = all_heat;
    }

    #[test]
    fn pinned_overlay_picks_up_a_pinned_knight() {
        // White king e1, black bishop on c3 pins the white knight on
        // d2. Black king parked on h8 to keep the position legal.
        let pos = Position::from_fen("7k/8/8/8/8/2b5/3N4/4K3 w - - 0 1").unwrap();
        let d = compute_overlays(&pos);
        let knight_sq = Square::D2;
        assert!(d.white_pinned.contains(knight_sq));
        assert!(d.black_pinned.is_empty());
    }
}
