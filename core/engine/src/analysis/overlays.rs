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

use super::tactic_util::trapped_cage;
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
    /// Pieces of `colour` that are trapped — attacked, with no safe square
    /// and no favourable trade out. Computed with `colour` to move (via a
    /// null-move turn-flip when it isn't), so a trapped enemy piece shows
    /// even while it's *your* move — the flagship case. Empty for the
    /// not-to-move side when the side to move is in check (a null move is
    /// illegal there). For the richer per-piece "cage" (the dead escape
    /// squares closing in), call [`trapped_cages`].
    pub white_trapped: Bitboard,
    pub black_trapped: Bitboard,
    /// Union of every trapped white piece's "cage" — the squares it could
    /// legally move to but which are all unsafe, closing in on it. Pairs
    /// with [`Self::white_trapped`] so renderers can paint the box around
    /// the doomed piece in one pass without re-running [`trapped_cages`].
    /// Use [`trapped_cages`] when you need the per-piece breakdown
    /// (e.g. drawing arrows from each cage to its owning piece).
    pub white_trapped_cage: Bitboard,
    pub black_trapped_cage: Bitboard,
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

    let (white_trapped, white_trapped_cage) = trapped_pieces_and_cage(pos, Color::White);
    let (black_trapped, black_trapped_cage) = trapped_pieces_and_cage(pos, Color::Black);

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
        white_trapped,
        black_trapped,
        white_trapped_cage,
        black_trapped_cage,
        heat_white_1,
        heat_white_2plus,
        heat_black_1,
        heat_black_2plus,
    }
}

/// Run `f` against a position in which `colour` is the side to move.
///
/// [`crate::analysis::tactic_util::is_trapped`] reasons only about the
/// side to move, so to ask "is this *enemy* piece trapped" on our turn we
/// flip the turn with a null move. Returns `None` (the caller substitutes
/// an empty result) when `colour` is not on move *and* the side that is on
/// move is in check — a null move is illegal there, and the question is
/// moot mid-check anyway. The position is cloned; the caller's is never
/// mutated.
fn with_colour_to_move<R>(
    pos: &Position,
    colour: Color,
    f: impl FnOnce(&Position) -> R,
) -> Option<R> {
    if pos.side_to_move() == colour {
        Some(f(pos))
    } else if pos.checkers().any() {
        None
    } else {
        let mut flipped = pos.clone();
        let _ = flipped.do_null_move();
        Some(f(&flipped))
    }
}

/// Bitboards for every trapped piece of `colour` paired with the union of
/// their cages (dead escape squares), in one walk. Handles the side-to-move
/// requirement via [`with_colour_to_move`]. The piece set and the cage union
/// are always disjoint: a trapped piece's cage is the set of squares it
/// could legally move *to*, never its own square.
fn trapped_pieces_and_cage(pos: &Position, colour: Color) -> (Bitboard, Bitboard) {
    with_colour_to_move(pos, colour, |p| {
        let mut pieces = Bitboard::EMPTY;
        let mut cage = Bitboard::EMPTY;
        for sq in p.pieces_by_color(colour) {
            if let Some(c) = trapped_cage(p, sq) {
                pieces = pieces.with(sq);
                cage |= c;
            }
        }
        (pieces, cage)
    })
    .unwrap_or((Bitboard::EMPTY, Bitboard::EMPTY))
}

/// Per-trapped-piece "cage": for each trapped piece of `colour`, its
/// square paired with the bitboard of squares it could legally move to but
/// which are all unsafe. This is the rich overlay surface — paint the
/// piece plus the ring of dead squares closing in on it — whereas
/// [`OverlayData::white_trapped`] / `black_trapped` are the cheap highlight.
/// Same turn-flip handling as the bitboards (works for the not-to-move
/// side; empty when the side to move is in check).
pub fn trapped_cages(pos: &Position, colour: Color) -> Vec<(Square, Bitboard)> {
    with_colour_to_move(pos, colour, |p| {
        let mut out = Vec::new();
        for sq in p.pieces_by_color(colour) {
            if let Some(cage) = trapped_cage(p, sq) {
                out.push((sq, cage));
            }
        }
        out
    })
    .unwrap_or_default()
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

    // Black knight a8 fenced by white pawns c5/d6 and attacked by Bd5;
    // its only squares (b6, c7) are both covered. Black to move.
    const TRAPPED_KNIGHT_FEN: &str = "n6k/8/3P4/2PB4/8/8/8/6K1 b - - 0 1";

    #[test]
    fn trapped_overlay_picks_up_a_trapped_knight_for_side_to_move() {
        let pos = Position::from_fen(TRAPPED_KNIGHT_FEN).unwrap();
        let d = compute_overlays(&pos);
        assert!(d.black_trapped.contains(Square::A8));
        assert!(d.white_trapped.is_empty());
    }

    #[test]
    fn trapped_overlay_sees_enemy_piece_via_turn_flip() {
        // Same position, but White to move: the black knight is the
        // not-to-move side's piece, yet the null-move turn-flip still
        // reports it trapped — the flagship "you can win their piece" case.
        let pos = Position::from_fen("n6k/8/3P4/2PB4/8/8/8/6K1 w - - 0 1").unwrap();
        let d = compute_overlays(&pos);
        assert!(d.black_trapped.contains(Square::A8));
    }

    #[test]
    fn trapped_cages_reports_the_dead_escape_squares() {
        let pos = Position::from_fen(TRAPPED_KNIGHT_FEN).unwrap();
        let cages = trapped_cages(&pos, Color::Black);
        assert_eq!(cages.len(), 1);
        let (sq, dead) = cages[0];
        assert_eq!(sq, Square::A8);
        // The knight's only moves are b6 and c7 — both unsafe.
        assert_eq!(dead.popcount(), 2);
        assert!(dead.contains(Square::B6));
        assert!(dead.contains(Square::C7));
    }

    #[test]
    fn no_trapped_pieces_at_startpos() {
        let d = compute_overlays(&Position::startpos());
        assert!(d.white_trapped.is_empty());
        assert!(d.black_trapped.is_empty());
        assert!(d.white_trapped_cage.is_empty());
        assert!(d.black_trapped_cage.is_empty());
    }

    #[test]
    fn trapped_cage_bitboard_collects_every_dead_square() {
        // Same trapped-knight fixture: the knight is on a8 and its only
        // legal moves (b6, c7) are both covered. The cage bitboard must
        // hold exactly those two squares, and stay disjoint from the
        // piece's own square.
        let pos = Position::from_fen(TRAPPED_KNIGHT_FEN).unwrap();
        let d = compute_overlays(&pos);
        assert!(d.black_trapped.contains(Square::A8));
        assert_eq!(d.black_trapped_cage.popcount(), 2);
        assert!(d.black_trapped_cage.contains(Square::B6));
        assert!(d.black_trapped_cage.contains(Square::C7));
        // Cage is always disjoint from the trapped piece's own square.
        assert!((d.black_trapped & d.black_trapped_cage).is_empty());
        assert!(d.white_trapped_cage.is_empty());
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
