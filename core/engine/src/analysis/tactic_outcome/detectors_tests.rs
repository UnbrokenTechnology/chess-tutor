//! Per-detector fixtures for the wave-4 multi-ply patterns, ported from
//! lichess-puzzler's `tagger/test.py` (`make("id", FEN, "uci moves")`).
//!
//! These call each `detect_*` *in isolation* (the wave-4 detectors are
//! private to this module, so a child test module can reach them). That
//! matches lichess's own tests, which check one predicate at a time —
//! unlike the full priority chain, where a more immediate pattern can win
//! the slot. The first UCI move is the opponent's setup; the rest is the
//! solver's line, exactly as `uci_line` splits it.

use super::*;
use crate::analysis::test_support::uci_line;

/// Build `(boards, pv, mover)` for a fixture, ready to hand to a detector.
fn setup(fen: &str, moves: &str) -> (Vec<Position>, Vec<Move>, Color) {
    let (pre, pv) = uci_line(fen, moves);
    let mover = pre.side_to_move();
    let boards = line_boards(&pre, &pv, WAVE4_MAX_PLIES);
    (boards, pv, mover)
}

// ---- deflection -----------------------------------------------------

fn deflects(fen: &str, moves: &str) -> bool {
    let (boards, pv, mover) = setup(fen, moves);
    detect_deflection(&boards, &pv, mover, 0, None).is_some()
}

#[test]
fn deflection_true_cases() {
    // Queen captures on a square the king had to be deflected from.
    assert!(deflects(
        "r1bqkbnr/pp3p1p/6p1/2pBp3/4P3/2P1B3/PP3PPP/RN1QK2R b KQkq - 0 9",
        "g8f6 d5f7 e8f7 d1d8"
    ));
    assert!(deflects(
        "r1bqkb1r/4pp1p/p1pp1np1/4P3/P1B5/2N5/1PP2PPP/R1BQK2R b KQkq - 0 9",
        "d6e5 c4f7 e8f7 d1d8"
    ));
    // Promotion variant: the deflected rook guarded the push square.
    assert!(deflects("8/8/PR4K1/8/5k1P/r7/4p3/8 w - - 0 52", "b6e6 a3a6 e6a6 e2e1q"));
}

#[test]
fn deflection_false_cases() {
    assert!(!deflects(
        "rnb1k2r/pppp2p1/4p2p/5p2/1q1Pn2P/2NQPN2/PPP2PP1/R3KB1R w KQkq - 1 9",
        "a2a3 b4b2 a1b1 b2c3 d3c3 e4c3"
    ));
    assert!(!deflects(
        "8/1R4p1/p5rp/4bN2/5kP1/2P4K/PP6/8 b - - 0 40",
        "g6g4 b7b4 f4f5 b4g4"
    ));
    assert!(!deflects(
        "5rk1/3R4/p1p3pp/1p2b3/2P1n2q/4Q2P/PP3PP1/4R1K1 w - - 4 27",
        "e3e4 h4f2 g1h1 f2f1 e1f1 f8f1"
    ));
    assert!(!deflects(
        "r2k2r1/1b2nQb1/1p2p2p/p3Pp2/2P4q/P6P/NP2R1PN/2R4K b - - 0 26",
        "h4d4 a2c3 g8f8 f7g7 f8g8 g7h6"
    ));
}

// ---- attraction -----------------------------------------------------

fn attracts(fen: &str, moves: &str) -> bool {
    let (boards, pv, mover) = setup(fen, moves);
    detect_attraction(&boards, &pv, mover, 0, None).is_some()
}

#[test]
fn attraction_true_cases() {
    // Sac draws the king onto a square, then a check (king case).
    assert!(attracts(
        "r4rk1/pp3pp1/7p/b2Pn3/4N3/6RQ/P4PPP/q1B1R1K1 b - - 8 26",
        "a5e1 g3g7 g8g7 h3h6 g7g8 e4f6"
    ));
    assert!(attracts(
        "2kr1b1r/1p1b2pp/p1P1p2n/2P3N1/P4q2/5N2/4BKPP/R2Q3R b - - 2 18",
        "d7c6 d1d8 c8d8 g5e6 d8c8 e6f4"
    ));
}

#[test]
fn attraction_false_cases() {
    assert!(!attracts(
        "r1bq1rk1/ppp1bppp/2n2n2/4p1B1/4N1P1/3P1N1P/PPP2P2/R2QKB1R w KQ - 1 9",
        "d1d2 f6e4 d3e4 c6d4 e1c1 d4f3 d2d8 e7g5 d8g5 f3g5"
    ));
    assert!(!attracts(
        "4r1k1/1R3ppp/1N3n2/1bP5/1P6/3p3P/6P1/3R2K1 w - - 0 28",
        "b6d5 f6d5 b7b5 d5c3 d1d3 c3b5"
    ));
}

// ---- interference (player + self) -----------------------------------

fn interferes(fen: &str, moves: &str) -> bool {
    let (boards, pv, mover) = setup(fen, moves);
    detect_interference(&boards, &pv, mover, 0, None).is_some()
}

#[test]
fn interference_true_case() {
    // A white knight interposes on the f-file, cutting the rook's defense of
    // f3, then the rook captures there.
    assert!(interferes(
        "r5k1/ppp2r2/3p3p/3Pp3/1P2N1bb/R5N1/1P3P1K/6R1 b - - 5 25",
        "g4f3 g3f5 g8h7 a3f3"
    ));
}

#[test]
fn interference_false_case() {
    assert!(!interferes(
        "6k1/1b1q1pbp/4pnp1/2Pp4/rp1P1P2/3BPRNP/4Q1P1/4B1K1 b - - 1 26",
        "f6e4 d3b5 b7c6 b5a4"
    ));
}

// ---- x-ray ----------------------------------------------------------

fn x_rays(fen: &str, moves: &str) -> bool {
    let (boards, pv, mover) = setup(fen, moves);
    detect_x_ray(&boards, &pv, mover, 0, None).is_some()
}

#[test]
fn x_ray_true_case() {
    // Battery on g2: the back rook recaptures through the front one — resolves
    // at the mover's third move (pv[4]).
    assert!(x_rays(
        "5R2/8/p1p4p/1p1p2k1/6r1/1P2P1r1/P1PKR3/8 b - - 3 33",
        "g3g2 f8g8 g5f6 e2g2 g4g2 g8g2"
    ));
}

// ---- clearance ------------------------------------------------------
//
// lichess's only `test_clearance` case is commented out in `test.py`, and
// it genuinely fails the predicate (the promotion gives check while the
// opponent's prior move was a king move — a case `cook.clearance`'s guard
// excludes). So we exercise the detector with a clean hand-built line:
// the knight steps off e3, clearing the c1–g5 diagonal, and the bishop
// then slides through the vacated square to g5.

#[test]
fn clearance_true_case() {
    use crate::types::Square;
    let pre = Position::from_fen("6k1/p6p/8/8/8/4N3/7P/2B3K1 w - - 0 1").unwrap();
    let pv = vec![
        Move::normal(Square::E3, Square::D5), // Nd5 — clears e3 on the diagonal
        Move::normal(Square::A7, Square::A6), // quiet reply
        Move::normal(Square::C1, Square::G5), // Bg5 slides through the cleared square
    ];
    let boards = line_boards(&pre, &pv, WAVE4_MAX_PLIES);
    assert!(detect_clearance(&boards, &pv, pre.side_to_move(), 0, None).is_some());
}

// ---- intermezzo -----------------------------------------------------

#[test]
fn intermezzo_true_and_guard_cases() {
    // Black just played ...Bxd4 (capturing a knight); White inserts Bxf7+
    // instead of recapturing, and only after Kxf7 takes the bishop.
    let pre = Position::from_fen("6k1/5p2/8/8/3b4/1B6/8/3R2K1 w - - 0 1").unwrap();
    let pv = vec![
        Move::normal(Square::B3, Square::F7), // Bxf7+ (zwischenzug)
        Move::normal(Square::G8, Square::F7), // Kxf7
        Move::normal(Square::D1, Square::D4), // Rxd4 (delayed recapture)
    ];
    let boards = line_boards(&pre, &pv, WAVE4_MAX_PLIES);
    let mover = pre.side_to_move();
    let prior = PriorMove {
        mv: Move::normal(Square::E5, Square::D4),
        captured: Some(PieceType::Knight),
    };
    assert!(detect_intermezzo(&boards, &pv, mover, 0, None, Some(prior)).is_some());
    // Without the prior capture there's no in-between to recognize.
    assert!(detect_intermezzo(&boards, &pv, mover, 0, None, None).is_none());
}

// ---- attacking f2/f7 (wave 6) ---------------------------------------

#[test]
fn attacking_f2_f7_cases() {
    use crate::types::Color;
    // Bxf7+ with Black's king still home on e8 — the motif.
    let pre = Position::from_fen("4k3/5p2/8/8/2B5/8/8/4K3 w - - 0 1").unwrap();
    let key = Move::normal(Square::C4, Square::F7);
    let mut post = pre.clone();
    post.do_move(key);
    assert!(detect_attacking_f2_f7(&pre, &post, key, Color::White, 0, None).is_some());

    // Same capture, but the king isn't on e8 → not the motif.
    let pre2 = Position::from_fen("3k4/5p2/8/8/2B5/8/8/4K3 w - - 0 1").unwrap();
    let mut post2 = pre2.clone();
    post2.do_move(key);
    assert!(detect_attacking_f2_f7(&pre2, &post2, key, Color::White, 0, None).is_none());
}

// ---- under-promotion (wave 6) ---------------------------------------

#[test]
fn under_promotion_cases() {
    // Knight promotion, not mate → under-promotion.
    let pre = Position::from_fen("4k3/1P6/8/8/8/8/8/4K3 w - - 0 1").unwrap();
    let knight = vec![Move::promotion(Square::B7, Square::B8, PieceType::Knight)];
    let boards = line_boards(&pre, &knight, WAVE4_MAX_PLIES);
    assert!(detect_under_promotion(&boards, &knight, 0, None).is_some());

    // Queen promotion, not mate → not an under-promotion.
    let queen = vec![Move::promotion(Square::B7, Square::B8, PieceType::Queen)];
    let boards_q = line_boards(&pre, &queen, WAVE4_MAX_PLIES);
    assert!(detect_under_promotion(&boards_q, &queen, 0, None).is_none());
}

#[test]
fn under_promotion_mate_cases_lichess() {
    // Knight-promotion checkmate (the necessary `=N#`) → under-promotion.
    let (boards, pv, _) = setup(
        "3R3r/p1P1kp1b/4pnpp/7P/6P1/2p5/P4P2/3R2K1 b - - 0 31",
        "c3c2 c7c8n",
    );
    assert!(detect_under_promotion(&boards, &pv, 0, None).is_some());

    // Rook-promotion checkmate (an *unnecessary* under-promotion) → not flagged.
    let (boards2, pv2, _) = setup(
        "8/1Pp3p1/8/2p5/2P5/5kbp/3p4/7K w - - 0 52",
        "b7b8q d2d1r",
    );
    assert!(detect_under_promotion(&boards2, &pv2, 0, None).is_none());
}
