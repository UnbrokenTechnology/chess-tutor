use super::*;
use crate::san;

/// A fresh position from `fen` plus a SAN move legal in it.
fn mv(fen: &str, san_str: &str) -> (Position, Move) {
    let mut pos = Position::from_fen(fen).expect("valid fen");
    let m = san::parse(&mut pos, san_str).expect("legal san");
    (Position::from_fen(fen).unwrap(), m)
}

/// The canonical silent-sequencing position (case study
/// `silent-sequencing-after-qc8`). Black to move; played `…Qc8`, best
/// `…Be5`. The deep gap is ~567 cp (we pass a representative value); the
/// shallow gap is ~74 cp (well under the threshold); and the move walks
/// into no name-able pattern that distinguishes it from `…Be5` (both
/// defuse the standing e-file pin equally). So the diagnostic must fire.
const QC8_FEN: &str = "1r1q2nr/p3k3/2Bbbpp1/7p/2Q5/8/PPPP1PPP/R1B1R1K1 b - - 0 1";

#[test]
fn qc8_is_silent_sequencing() {
    let (pre, qc8) = mv(QC8_FEN, "Qc8");
    let (_, be5) = mv(QC8_FEN, "Be5");
    // Deep gap from the case study's depth-14 row: Be5 +1.22 vs Qc8 -1.45
    // ≈ +2.67 pawns ≈ 567 cp (root-STM/Black POV best − candidate).
    let deep_gap_cp = 567;
    assert!(
        is_silent_sequencing(&pre, qc8, be5, deep_gap_cp, None),
        "the …Qc8 case study FEN must classify as silent sequencing"
    );
}

#[test]
fn small_deep_gap_is_not_silent_sequencing() {
    let (pre, qc8) = mv(QC8_FEN, "Qc8");
    let (_, be5) = mv(QC8_FEN, "Be5");
    // If the deep gap were small, neither pick is clearly wrong — there is
    // nothing to be humble about, so the diagnostic must NOT fire.
    assert!(
        !is_silent_sequencing(&pre, qc8, be5, 50, None),
        "a small deep gap means no hidden blunder — not silent sequencing"
    );
}

#[test]
fn same_move_is_not_silent_sequencing() {
    let (pre, be5) = mv(QC8_FEN, "Be5");
    assert!(
        !is_silent_sequencing(&pre, be5, be5, 567, None),
        "playing the best move can't be silent sequencing"
    );
}

#[test]
fn nameable_tactic_disqualifies() {
    // A position where the side to move has an obvious free-piece capture
    // available — `find_best_tactic_in_position` fires, so even a large
    // deep gap reads as a teachable tactic, not silent sequencing. White
    // to move; Qxd5 grabs an undefended queen.
    let fen = "4k3/8/8/3q4/3Q4/8/8/4K3 w - - 0 1";
    let (pre, best) = mv(fen, "Qxd5+");
    // A losing alternative that doesn't take the queen.
    let (_, other) = mv(fen, "Ke2");
    assert!(
        !is_silent_sequencing(&pre, other, best, 900, None),
        "a name-able tactic on the board disqualifies silent sequencing"
    );
}

