use super::*;
use crate::position::Position;

/// Run the full piece-evaluation pipeline for `us` and return the
/// per-sub-term breakdown. Mirrors the bootstrap pattern described in
/// the eval-module tests: build the scratchpad, initialize both
/// colours' attack tables, then evaluate.
fn piece_breakdown(fen: &str, us: Color) -> PiecesBreakdown {
    let pos = Position::from_fen(fen).unwrap();
    let mut e = Evaluator::new(&pos);
    e.initialize(Color::White);
    e.initialize(Color::Black);
    // Evaluate both colours so any cross-colour state (attack tables)
    // is fully populated — matches how the main evaluator runs.
    let _other = evaluate(&mut e, !us);
    evaluate(&mut e, us)
}

// ---- Breakdown totals stay consistent with the aggregate -----------

#[test]
fn breakdown_total_sums_every_sub_term() {
    // The per-colour total() must equal the sum of every public
    // sub-term field. A future refactor that adds a field but forgets
    // to update total() would silently drift; this test catches that.
    let fen = "r1bqkb1r/ppp2ppp/2np1n2/4p3/2B1P3/2N2N2/PPPP1PPP/R1BQK2R w KQkq - 0 5";
    let b = piece_breakdown(fen, Color::White);
    let manual = b.outposts
        + b.reachable_outposts
        + b.minor_behind_pawn
        + b.king_protector
        + b.bishop_pawns
        + b.long_diagonal_bishop
        + b.rook_on_queen_file
        + b.rook_on_open_file
        + b.rook_on_semiopen_file
        + b.trapped_rook
        + b.weak_queen;
    assert_eq!(b.total(), manual);
}

// ---- rook_on_open_file vs rook_on_semiopen_file attribution --------

#[test]
fn rook_on_fully_open_file_lands_on_open_field_only() {
    // White rook on e1, no pawns on the e-file for either colour.
    // Pawns off-file so the rook is on a fully open file.
    // Matches Stockfish's ROOK_ON_FILE[1] = Score(47, 25).
    let fen = "4k3/1pp3pp/8/8/8/8/PPP3PP/4R2K w - - 0 1";
    let b = piece_breakdown(fen, Color::White);
    assert_eq!(b.rook_on_open_file, ROOK_ON_FILE[1]);
    assert_eq!(b.rook_on_semiopen_file, Score::ZERO);
}

#[test]
fn rook_on_semiopen_file_lands_on_semiopen_field_only() {
    // White rook on e1, white has no pawn on e-file but black does
    // (e5). ROOK_ON_FILE[0] fires; the fully-open field stays zero.
    let fen = "4k3/1pp3pp/8/4p3/8/8/PPP3PP/4R2K w - - 0 1";
    let b = piece_breakdown(fen, Color::White);
    assert_eq!(b.rook_on_semiopen_file, ROOK_ON_FILE[0]);
    assert_eq!(b.rook_on_open_file, Score::ZERO);
}

// ---- outposts vs reachable_outposts attribution --------------------

#[test]
fn knight_on_outpost_lands_on_outposts_field() {
    // White knight on d5, supported by white pawn on c4. Black has
    // no pawns on c or e files ahead (ranks 6+), so d5 is an outpost.
    // Knight multiplier is ×2, so the field holds OUTPOST * 2.
    let fen = "4k3/8/8/3N4/2P5/8/8/4K3 w - - 0 1";
    let b = piece_breakdown(fen, Color::White);
    assert_eq!(b.outposts, OUTPOST * 2);
    assert_eq!(b.reachable_outposts, Score::ZERO);
}

#[test]
fn knight_reaching_outpost_lands_on_reachable_outposts_field() {
    // Knight on f3 can jump to d4 / e5 — both supported by a pawn on
    // c3 / d4? We want exactly one reachable outpost and no direct
    // outpost. Knight on f3, pawn on c4, knight sees e5 but e5 isn't
    // an outpost without c4 supporting... Simpler: knight on c3 with
    // pawn on c4 supports d5 as an outpost target; the knight can
    // jump to d5 or b5. d5 is attacked by c4, b5 is attacked by c4
    // too — so they're both outposts. The knight is not on an
    // outpost itself.
    let fen = "4k3/8/8/8/2P5/2N5/8/4K3 w - - 0 1";
    let b = piece_breakdown(fen, Color::White);
    assert_eq!(b.outposts, Score::ZERO);
    assert_eq!(b.reachable_outposts, REACHABLE_OUTPOST);
}

// ---- weak_queen attribution ----------------------------------------

#[test]
fn weak_queen_fires_under_xray_slider_threat() {
    // White queen on d1, black rook on d8 — aligned with one piece
    // (white pawn on d5) between them. The rook x-rays the queen:
    // remove the pawn and the rook pins/attacks the queen. Expect
    // -WEAK_QUEEN in the breakdown.
    let fen = "3rk3/8/8/3P4/8/8/8/3QK3 w - - 0 1";
    let b = piece_breakdown(fen, Color::White);
    assert_eq!(b.weak_queen, Score::ZERO - WEAK_QUEEN);
}

#[test]
fn weak_queen_absent_without_xray_threat() {
    // White queen on d1 with no aligned enemy rook/bishop.
    let fen = "4k3/8/8/8/8/8/8/3QK3 w - - 0 1";
    let b = piece_breakdown(fen, Color::White);
    assert_eq!(b.weak_queen, Score::ZERO);
}

// ---- long_diagonal_bishop attribution ------------------------------

#[test]
fn long_diagonal_bishop_lands_on_its_own_field() {
    // Bishop on b2 sees the long a1-h8 diagonal through pawns — its
    // pawns-only x-ray hits both d4 and e5 (the CENTER squares). No
    // friendly pawn is standing on the diagonal to block the test.
    let fen = "4k3/8/8/8/8/8/1B6/4K3 w - - 0 1";
    let b = piece_breakdown(fen, Color::White);
    assert_eq!(b.long_diagonal_bishop, LONG_DIAGONAL_BISHOP);
}

// ---- Per-colour symmetry mirrors -----------------------------------

#[test]
fn mirrored_positions_produce_mirrored_breakdowns() {
    // Colour-flipped mirror positions should produce equal breakdowns
    // for the relevant side. The total() on both sides must agree.
    let white_fen = "4k3/8/8/3N4/2P5/8/8/4K3 w - - 0 1";
    let black_fen = "4k3/8/8/2p5/3n4/8/8/4K3 w - - 0 1";
    let w = piece_breakdown(white_fen, Color::White);
    let b = piece_breakdown(black_fen, Color::Black);
    assert_eq!(w.outposts, b.outposts);
    assert_eq!(w.minor_behind_pawn, b.minor_behind_pawn);
    assert_eq!(w.total(), b.total());
}
