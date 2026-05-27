//! Fixtures for the named-mate detectors.
//!
//! Back-rank cases are ported verbatim from lichess-puzzler's
//! `tagger/test.py` (`make("id", FEN, "uci moves")`, the first move the
//! opponent's setup). lichess ships *no* fixtures for the other mates, so
//! those are hand-built minimal positions that exercise the predicate
//! geometry: smothered / anastasia / arabian / boden / double-bishop as real
//! mate-in-1 lines through [`detect_mate_pattern`]; hook and dovetail (whose
//! legal mate-in-1 setups are fiddly) as direct sub-detector checks on a
//! constructed terminal board, mirroring how lichess unit-tests each `cook`
//! predicate in isolation.

use super::*;
use crate::analysis::test_support::uci_line;
use crate::analysis::tactic_outcome::detect_line_tactic;

/// Replay a lichess fixture line (first move = opponent setup) and name the
/// mate, if any.
fn mates_line(fen: &str, moves: &str) -> Option<MatePattern> {
    let (pre, pv) = uci_line(fen, moves);
    let mover = pre.side_to_move();
    detect_mate_pattern(&pre, &pv, mover).map(|m| m.pattern)
}

/// Name the mate for a hand-built `(pre, pv)` where `pre.side_to_move()` is the
/// mover and `pv` is their full mating line.
fn mates_direct(fen: &str, pv: &[Move]) -> Option<MatePattern> {
    let pre = Position::from_fen(fen).unwrap();
    let mover = pre.side_to_move();
    detect_mate_pattern(&pre, pv, mover).map(|m| m.pattern)
}

/// Decompose a *terminal* checkmate FEN (the mated side to move) into the
/// pieces a sub-detector wants: board, the mover who delivered mate, and the
/// mated king.
fn terminal(fen: &str) -> (Position, Color, Square) {
    let board = Position::from_fen(fen).unwrap();
    let mover = !board.side_to_move();
    let king = board.king_square(board.side_to_move());
    (board, mover, king)
}

// ---- back-rank (lichess fixtures) -----------------------------------

#[test]
fn back_rank_true_cases() {
    assert_eq!(
        mates_line(
            "5r1k/4q1p1/p2pP2p/1p6/1P2Q3/PB6/1BP3PP/6K1 w - - 1 27",
            "e4g6 e7a7 b2d4 a7d4 g1h1 f8f1"
        ),
        Some(MatePattern::BackRank)
    );
    assert_eq!(
        mates_line(
            "r5k1/pQ3ppp/8/8/B1pp4/4q3/PP5P/5R1K b - - 0 26",
            "a8d8 b7f7 g8h8 f7f8 d8f8 f1f8"
        ),
        Some(MatePattern::BackRank)
    );
}

#[test]
fn back_rank_false_cases() {
    // A mate, but the king is not boxed purely by its own pieces on the rank.
    assert_ne!(
        mates_line(
            "3r2k1/1bQ3p1/p2p3p/3qp1b1/1p6/1P1B4/P1P3PP/1K3R2 b - - 4 25",
            "d5c6 c7f7 g8h8 f7f8 d8f8 f1f8"
        ),
        Some(MatePattern::BackRank)
    );
    // Mating bishop sits on a forward flight square (mover's own piece there).
    assert_ne!(
        mates_line(
            "3r2k1/1b4pp/1p2pr2/p5N1/8/PP2n1P1/1BR2bBP/4R2K w - - 1 27",
            "b2f6 b7g2"
        ),
        Some(MatePattern::BackRank)
    );
}

// ---- smothered ------------------------------------------------------

#[test]
fn smothered_mate_in_one() {
    // Black Kh8 boxed by Rg8 + pawns g7/h7; White knight g5–f7#.
    assert_eq!(
        mates_direct(
            "6rk/6pp/8/6N1/8/8/8/7K w - - 0 1",
            &[Move::normal(Square::G5, Square::F7)]
        ),
        Some(MatePattern::Smothered)
    );
}

// ---- anastasia ------------------------------------------------------

#[test]
fn anastasia_mate_in_one() {
    // King h7 hemmed by its own pawn g7; White Ne7 covers g6/g8; Rg1–h1#.
    assert_eq!(
        mates_direct(
            "8/4N1pk/8/8/8/8/8/2K3R1 w - - 0 1",
            &[Move::normal(Square::G1, Square::H1)]
        ),
        Some(MatePattern::Anastasia)
    );
}

// ---- arabian --------------------------------------------------------

#[test]
fn arabian_mate_in_one() {
    // Corner king h8; White Nf6 a (2,2) leap away guards the mating Ra7–h7#.
    assert_eq!(
        mates_direct(
            "7k/R7/5N2/8/8/8/8/2K5 w - - 0 1",
            &[Move::normal(Square::A7, Square::H7)]
        ),
        Some(MatePattern::Arabian)
    );
}

// ---- boden / double-bishop ------------------------------------------

#[test]
fn boden_mate_in_one() {
    // Bishops on opposite sides of the king's file (a6 + f4 around Kc8): Bb5–a6#.
    assert_eq!(
        mates_direct(
            "2kr4/3p4/8/1B6/5B2/8/8/7K w - - 0 1",
            &[Move::normal(Square::B5, Square::A6)]
        ),
        Some(MatePattern::Boden)
    );
}

#[test]
fn double_bishop_mate_in_one() {
    // Bishops on the same side of the king's file (e6 + f6 around Kh8): Bh4–f6#.
    assert_eq!(
        mates_direct(
            "7k/7p/4B3/8/7B/8/8/7K w - - 0 1",
            &[Move::normal(Square::H4, Square::F6)]
        ),
        Some(MatePattern::DoubleBishop)
    );
}

// ---- hook / dovetail (sub-detector geometry) ------------------------

#[test]
fn hook_mate_geometry() {
    // Ke8 mated: Rd8 (the mating rook) guarded by Nf7 (also beside the king),
    // the knight guarded by Pe6.
    let (board, mover, king) = terminal("3Rk3/5N2/4P3/8/8/8/8/7K b - - 0 1");
    assert!(is_hook_mate(
        &board,
        mover,
        king,
        Move::normal(Square::D1, Square::D8)
    ));
}

#[test]
fn dovetail_mate_geometry() {
    // Centre king e5 boxed by its own pawns d4/d5/e4; Qf6 (defended by Bg7)
    // covers the remaining flights diagonally.
    let (board, mover, king) = terminal("8/6B1/5Q2/3pk3/3pp3/8/8/7K b - - 0 1");
    assert!(is_dovetail_mate(
        &board,
        mover,
        king,
        Move::normal(Square::F3, Square::F6)
    ));
}

// ---- guards ---------------------------------------------------------

#[test]
fn non_mating_line_is_none() {
    // A quiet king step that doesn't checkmate anything.
    assert_eq!(
        mates_direct(
            "4k3/8/8/8/8/8/8/4K3 w - - 0 1",
            &[Move::normal(Square::E1, Square::E2)]
        ),
        None
    );
}

#[test]
fn opponent_delivered_mate_is_none() {
    // White (the mover) plays a waiting push, then Black mates on the back
    // rank. The mating move is the opponent's (even-length line), so the scan
    // declines — the `user_walked_into` slot is what names an opponent's mate
    // (calling with the opponent as mover).
    assert_eq!(
        mates_direct(
            "r3k3/8/8/8/8/8/1P3PPP/6K1 w - - 0 1",
            &[
                Move::normal(Square::B2, Square::B3),
                Move::normal(Square::A8, Square::A1),
            ]
        ),
        None
    );
}

// ---- chain integration + flag field ---------------------------------

#[test]
fn standalone_checkmate_hit_carries_named_pattern() {
    // The smothered mate-in-1 fires no geometric pattern, so the chain
    // synthesizes a standalone `Checkmate` hit carrying the mate's name.
    let pre = Position::from_fen("6rk/6pp/8/6N1/8/8/8/7K w - - 0 1").unwrap();
    let pv = [Move::normal(Square::G5, Square::F7)];
    let hit = detect_line_tactic(&pre, &pv, pre.side_to_move(), 0, None).unwrap();
    assert_eq!(hit.pattern, TacticPattern::Checkmate);
    assert_eq!(hit.mate_pattern, Some(MatePattern::Smothered));
}

// ---- surfaced-by-default policy -------------------------------------

#[test]
fn only_everyday_mates_surface_by_default() {
    assert!(MatePattern::BackRank.surfaced_by_default());
    assert!(MatePattern::Smothered.surfaced_by_default());
    for m in [
        MatePattern::Anastasia,
        MatePattern::Hook,
        MatePattern::Arabian,
        MatePattern::Boden,
        MatePattern::DoubleBishop,
        MatePattern::Dovetail,
    ] {
        assert!(!m.surfaced_by_default());
    }
}
