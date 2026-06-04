//! [`PiecesPositionalOutcome`] — pre/post snapshots of the
//! 11-sub-term per-piece positional
//! [`crate::eval::PiecesBreakdown`] on both sides.

use super::{post_user_move, MoveAnalysis};
use crate::eval::PiecesBreakdown;
use crate::position::Position;
use crate::types::{Color, PieceType, Square};

/// Pre/post snapshots of the 11-sub-term per-piece positional
/// breakdown on both sides. "Post" is the position immediately after
/// the user's move. The CLI diffs sub-term by sub-term for phrases
/// like *"a minor claimed an outpost"* / *"a rook claimed the open
/// file"*.
///
/// POV convention: `ours_*` refers to the user's pieces
/// (`root_stm`); `theirs_*` to the opponent's.
///
/// `bishop_pawn_count_*` fields track pawns sharing a colour with one
/// of that side's bishops — summed across each of that side's
/// bishops. When pre equals post, any `bishop_pawns` Score delta is
/// driven purely by the blocked-centre multiplier (a central pawn
/// push / block) rather than by a genuine bishop-vs-own-pawn
/// geometry change, and narration for the BishopPawns sub-term
/// should be suppressed to avoid phrases like *"a bishop got stuck
/// behind its pawn chain"* firing when no bishop actually moved and
/// no pawn on its colour appeared or disappeared.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PiecesPositionalOutcome {
    pub ours_pre: PiecesBreakdown,
    pub ours_post: PiecesBreakdown,
    pub theirs_pre: PiecesBreakdown,
    pub theirs_post: PiecesBreakdown,
    pub ours_bishop_pawn_count_pre: u32,
    pub ours_bishop_pawn_count_post: u32,
    pub theirs_bishop_pawn_count_pre: u32,
    pub theirs_bishop_pawn_count_post: u32,
}

impl PiecesPositionalOutcome {
    /// True when our side's raw bishop-vs-own-pawn count changed
    /// (i.e., at least one of our bishops has a different number of
    /// pawns on its colour after the move). When false, any
    /// `bishop_pawns` Score delta on this side is multiplier-only and
    /// narration for `BishopPawns` should be suppressed.
    pub fn ours_bishop_pawn_count_changed(&self) -> bool {
        self.ours_bishop_pawn_count_pre != self.ours_bishop_pawn_count_post
    }

    /// Mirror of [`Self::ours_bishop_pawn_count_changed`] for the
    /// opponent.
    pub fn theirs_bishop_pawn_count_changed(&self) -> bool {
        self.theirs_bishop_pawn_count_pre != self.theirs_bishop_pawn_count_post
    }
}

/// Sum `pawns_on_same_color_squares` across every bishop `side`
/// owns. Zero when the side has no bishops.
fn bishop_pawn_count(pos: &Position, side: Color) -> u32 {
    pos.pieces_of(side, PieceType::Bishop)
        .into_iter()
        .map(|sq| pos.pawns_on_same_color_squares(side, sq))
        .sum()
}

/// Build a fresh `Evaluator` for `pos`, prime it with `initialize`,
/// and run `pieces::evaluate` for white and black in order — the
/// same sequence the main evaluator uses. Returns both colours'
/// breakdowns so callers can assign to `ours` / `theirs` from any
/// POV without a second eval pass.
fn snapshot_pieces_both(pos: &Position) -> (PiecesBreakdown, PiecesBreakdown) {
    let mut e = crate::eval::Evaluator::new(pos);
    e.initialize(Color::White);
    e.initialize(Color::Black);
    let w = crate::eval::pieces::evaluate(&mut e, Color::White);
    let b = crate::eval::pieces::evaluate(&mut e, Color::Black);
    (w, b)
}

/// Each `side` knight that can hop to an outpost square, paired with the
/// outpost it reaches — exactly the `(knight, outpost)` pairs the
/// `reachable_outposts` eval term scored (it primes the opt-in tracker,
/// so the result can't diverge from the score). The retrospective diffs
/// pre vs post to draw the route the knight *gained* (or lost). One pair
/// per reachable outpost; a knight eyeing two outposts yields two.
pub fn reachable_outpost_squares(pos: &Position, side: Color) -> Vec<(Square, Square)> {
    let mut e = crate::eval::Evaluator::new(pos);
    e.per_piece_reachable_outpost = Some(Vec::new());
    e.initialize(Color::White);
    e.initialize(Color::Black);
    let _ = crate::eval::pieces::evaluate(&mut e, Color::White);
    let _ = crate::eval::pieces::evaluate(&mut e, Color::Black);
    e.per_piece_reachable_outpost
        .take()
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, c, _)| *c == side)
        .map(|(knight, _, outpost)| (knight, outpost))
        .collect()
}

/// Each `side` minor sitting directly behind a pawn, paired with the
/// covering pawn — exactly the `(minor, pawn)` pairs the
/// `minor_behind_pawn` eval term scored (it primes the opt-in tracker, so
/// the result matches the score). The retrospective diffs pre vs post to
/// highlight *which* minor gained / lost its pawn cover.
pub fn minor_behind_pawn_squares(pos: &Position, side: Color) -> Vec<(Square, Square)> {
    let mut e = crate::eval::Evaluator::new(pos);
    e.per_piece_minor_behind_pawn = Some(Vec::new());
    e.initialize(Color::White);
    e.initialize(Color::Black);
    let _ = crate::eval::pieces::evaluate(&mut e, Color::White);
    let _ = crate::eval::pieces::evaluate(&mut e, Color::Black);
    e.per_piece_minor_behind_pawn
        .take()
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, c, _)| *c == side)
        .map(|(minor, _, pawn)| (minor, pawn))
        .collect()
}

/// Snapshot piece-positional terms at the pre-move position and at
/// the position immediately after the user's move.
pub fn compute_pieces_positional_outcome(
    ma: &MoveAnalysis,
    pre_move_pos: &Position,
    root_stm: Color,
) -> PiecesPositionalOutcome {
    let (w_pre, b_pre) = snapshot_pieces_both(pre_move_pos);
    let (ours_pre, theirs_pre) = if root_stm == Color::White {
        (w_pre, b_pre)
    } else {
        (b_pre, w_pre)
    };
    let ours_bishop_pawn_count_pre = bishop_pawn_count(pre_move_pos, root_stm);
    let theirs_bishop_pawn_count_pre = bishop_pawn_count(pre_move_pos, !root_stm);

    let scratch = post_user_move(pre_move_pos, ma);

    let (w_post, b_post) = snapshot_pieces_both(&scratch);
    let (ours_post, theirs_post) = if root_stm == Color::White {
        (w_post, b_post)
    } else {
        (b_post, w_post)
    };
    let ours_bishop_pawn_count_post = bishop_pawn_count(&scratch, root_stm);
    let theirs_bishop_pawn_count_post = bishop_pawn_count(&scratch, !root_stm);

    PiecesPositionalOutcome {
        ours_pre,
        ours_post,
        theirs_pre,
        theirs_post,
        ours_bishop_pawn_count_pre,
        ours_bishop_pawn_count_post,
        theirs_bishop_pawn_count_pre,
        theirs_bishop_pawn_count_post,
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::ma_with_pv;
    use super::*;
    use crate::types::{Move, Square};

    #[test]
    fn pieces_positional_outcome_snapshots_match_direct_eval() {
        let pre = Position::startpos();
        let mv = Move::normal(Square::G1, Square::F3);
        let ma = ma_with_pv(vec![mv], Some(0));
        let outcome = compute_pieces_positional_outcome(&ma, &pre, Color::White);

        let (w_pre, b_pre) = snapshot_pieces_both(&pre);
        assert_eq!(outcome.ours_pre, w_pre);
        assert_eq!(outcome.theirs_pre, b_pre);

        let mut post = pre.clone();
        post.do_move(mv);
        let (w_post, b_post) = snapshot_pieces_both(&post);
        assert_eq!(outcome.ours_post, w_post);
        assert_eq!(outcome.theirs_post, b_post);
    }

    #[test]
    fn pieces_positional_outcome_startpos_is_symmetric() {
        let pos = Position::startpos();
        let ma = ma_with_pv(Vec::new(), None);
        let outcome = compute_pieces_positional_outcome(&ma, &pos, Color::White);
        assert_eq!(outcome.ours_pre, outcome.ours_post);
        assert_eq!(outcome.theirs_pre, outcome.theirs_post);
        assert_eq!(outcome.ours_pre, outcome.theirs_pre);
    }

    #[test]
    fn pieces_positional_outcome_respects_root_stm_pov() {
        let pre_fen = "rnbqkbnr/pppppppp/8/8/8/5N2/PPPPPPPP/RNBQKB1R b KQkq - 0 1";
        let pre = Position::from_fen(pre_fen).unwrap();
        let ma = ma_with_pv(Vec::new(), None);
        let white_pov = compute_pieces_positional_outcome(&ma, &pre, Color::White);
        let black_pov = compute_pieces_positional_outcome(&ma, &pre, Color::Black);
        assert_eq!(white_pov.ours_pre, black_pov.theirs_pre);
        assert_eq!(white_pov.theirs_pre, black_pov.ours_pre);
    }

    #[test]
    fn bishop_pawn_count_unchanged_after_central_push_locks_centre() {
        // 1.e4 e5 from startpos: the central pawn push creates a
        // blocked centre on both sides but doesn't change the raw
        // count of pawns sharing a colour with either side's bishops.
        // Both `*_changed()` flags must be false so the narrator can
        // suppress phantom "a bishop got stuck" narration.
        let pre = Position::startpos();
        let mut after_e4 = pre.clone();
        after_e4.do_move(Move::normal(Square::E2, Square::E4));
        let ma = ma_with_pv(vec![Move::normal(Square::E7, Square::E5)], Some(0));
        let outcome = compute_pieces_positional_outcome(&ma, &after_e4, Color::Black);
        assert!(
            !outcome.ours_bishop_pawn_count_changed(),
            "1...e5 should not change black's bishop-pawn count, got pre={} post={}",
            outcome.ours_bishop_pawn_count_pre,
            outcome.ours_bishop_pawn_count_post,
        );
        assert!(
            !outcome.theirs_bishop_pawn_count_changed(),
            "1...e5 should not change white's bishop-pawn count, got pre={} post={}",
            outcome.theirs_bishop_pawn_count_pre,
            outcome.theirs_bishop_pawn_count_post,
        );
    }

    #[test]
    fn minor_behind_pawn_squares_finds_the_minor_and_its_pawn() {
        // White bishop on e2 with a white pawn directly in front on e3.
        let pos = Position::from_fen("4k3/8/8/8/8/4P3/4B3/4K3 w - - 0 1").unwrap();
        let ours = minor_behind_pawn_squares(&pos, Color::White);
        assert_eq!(ours, vec![(Square::E2, Square::E3)]);
        // Black has no such minor.
        assert!(minor_behind_pawn_squares(&pos, Color::Black).is_empty());
    }

    #[test]
    fn reachable_outpost_squares_finds_knight_and_target() {
        // White knight on e4, pawn on b4 guarding c5; Black has no pawns,
        // so c5 is a clean outpost the knight can hop to.
        let pos = Position::from_fen("4k3/8/8/8/1P2N3/8/8/4K3 w - - 0 1").unwrap();
        let ours = reachable_outpost_squares(&pos, Color::White);
        assert!(
            ours.contains(&(Square::E4, Square::C5)),
            "expected the e4 knight to have a route to the c5 outpost, got {ours:?}"
        );
    }

    #[test]
    fn bishop_pawn_count_changed_when_capture_removes_same_colour_pawn() {
        // Black has a dark-squared bishop on c5 and a dark-squared
        // pawn on e5 (5+5=10, dark). White knight on d3 captures e5,
        // dropping black's bishop-pawn count by 1.
        let pre_fen = "4k3/8/8/2b1p3/8/3N4/8/4K3 w - - 0 1";
        let pre = Position::from_fen(pre_fen).unwrap();
        let mv = Move::normal(Square::D3, Square::E5);
        let ma = ma_with_pv(vec![mv], Some(0));
        let outcome = compute_pieces_positional_outcome(&ma, &pre, Color::White);
        assert_eq!(outcome.theirs_bishop_pawn_count_pre, 1);
        assert_eq!(outcome.theirs_bishop_pawn_count_post, 0);
        assert!(outcome.theirs_bishop_pawn_count_changed());
    }
}
