use super::*;
use crate::types::{MoveKind, Piece, Rank};

// ---- Pseudo-legal counts from startpos --------------------------

#[test]
fn startpos_has_20_pseudo_legal_moves() {
    let p = Position::startpos();
    let moves = pseudo_legal_moves_vec(&p);
    // 16 pawn moves (8 pawns × {single push, double push}) + 4 knight
    // moves (b1→{a3, c3}, g1→{f3, h3}). No other piece has a legal move
    // at start: bishops/queens/rooks/king are blocked by pawns, and
    // castling requires clear squares.
    assert_eq!(moves.len(), 20, "startpos pseudo-legal move count");
}

#[test]
fn startpos_has_8_pawn_single_pushes_and_8_double_pushes() {
    let p = Position::startpos();
    let moves = pseudo_legal_moves_vec(&p);
    let pawn_moves: Vec<_> = moves
        .iter()
        .filter(|m| p.piece_on(m.from()) == Some(Piece::WhitePawn))
        .collect();
    assert_eq!(pawn_moves.len(), 16);
    let single = pawn_moves
        .iter()
        .filter(|m| m.to().rank() == Rank::R3)
        .count();
    let double = pawn_moves
        .iter()
        .filter(|m| m.to().rank() == Rank::R4)
        .count();
    assert_eq!(single, 8);
    assert_eq!(double, 8);
}

#[test]
fn startpos_has_no_promotions() {
    let p = Position::startpos();
    assert!(!pseudo_legal_moves_vec(&p)
        .iter()
        .any(|m| m.kind() == MoveKind::Promotion));
}

// ---- Promotion generation ---------------------------------------

#[test]
fn pawn_on_seventh_rank_has_four_promotion_pushes() {
    // White pawn on a7 can push to a8 with four promotions.
    let p = Position::from_fen("4k3/P7/8/8/8/8/8/4K3 w - - 0 1").unwrap();
    let moves = pseudo_legal_moves_vec(&p);
    let promos: Vec<_> = moves
        .iter()
        .filter(|m| m.kind() == MoveKind::Promotion && m.from() == Square::A7)
        .collect();
    assert_eq!(promos.len(), 4);
    let pieces: Vec<_> = promos.iter().map(|m| m.promoted_to()).collect();
    assert!(pieces.contains(&PieceType::Queen));
    assert!(pieces.contains(&PieceType::Rook));
    assert!(pieces.contains(&PieceType::Bishop));
    assert!(pieces.contains(&PieceType::Knight));
}

#[test]
fn pawn_capturing_to_promote_emits_four_per_capture() {
    // White pawn a7 captures a rook on b8: a7xb8=Q/R/B/N. Also the
    // straight push a7→a8 (promo). Total: 4 capture-promos + 4 push-promos.
    let p = Position::from_fen("1r2k3/P7/8/8/8/8/8/4K3 w - - 0 1").unwrap();
    let moves = pseudo_legal_moves_vec(&p);
    let promos: Vec<_> = moves
        .iter()
        .filter(|m| m.kind() == MoveKind::Promotion && m.from() == Square::A7)
        .collect();
    assert_eq!(promos.len(), 8);
}

// ---- En passant --------------------------------------------------

#[test]
fn en_passant_generation_emits_one_ep_move_per_attacker() {
    // White pawn on e5, black pawn on d5 from a previous double push.
    // EP target d6. White should have an ep capture: e5xd6.
    let p = Position::from_fen("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 3").unwrap();
    let moves = pseudo_legal_moves_vec(&p);
    let ep: Vec<_> = moves
        .iter()
        .filter(|m| m.kind() == MoveKind::EnPassant)
        .collect();
    assert_eq!(ep.len(), 1);
    assert_eq!(ep[0].from(), Square::E5);
    assert_eq!(ep[0].to(), Square::D6);
}

// ---- Knight and slider generation -------------------------------

#[test]
fn lone_knight_in_center_has_eight_moves() {
    let p = Position::from_fen("4k3/8/8/4N3/8/8/8/4K3 w - - 0 1").unwrap();
    let moves = pseudo_legal_moves_vec(&p);
    let n_moves: Vec<_> = moves.iter().filter(|m| m.from() == Square::E5).collect();
    assert_eq!(n_moves.len(), 8);
}

#[test]
fn bishop_is_blocked_by_friendly_piece() {
    // White bishop on a1, white pawn on d4 blocks the diagonal.
    // Bishop attacks b2, c3 (blocked by d4 which is friendly).
    let p = Position::from_fen("4k3/8/8/8/3P4/8/8/B3K3 w - - 0 1").unwrap();
    let moves = pseudo_legal_moves_vec(&p);
    let b_moves: Vec<_> = moves.iter().filter(|m| m.from() == Square::A1).collect();
    let targets: Vec<_> = b_moves.iter().map(|m| m.to()).collect();
    assert!(targets.contains(&Square::B2));
    assert!(targets.contains(&Square::C3));
    // d4 is friendly: not a target.
    assert!(!targets.contains(&Square::D4));
    // Everything past d4 is unreachable.
    assert!(!targets.contains(&Square::E5));
}

#[test]
fn rook_captures_first_enemy_on_ray() {
    // White rook on a1, black knight on a5. Rook reaches a2..a5
    // (capturing the knight), and nothing beyond.
    let p = Position::from_fen("4k3/8/8/n7/8/8/8/R3K3 w - - 0 1").unwrap();
    let moves = pseudo_legal_moves_vec(&p);
    let r_moves: Vec<_> = moves.iter().filter(|m| m.from() == Square::A1).collect();
    let targets: Vec<_> = r_moves.iter().map(|m| m.to()).collect();
    assert!(targets.contains(&Square::A5));
    assert!(!targets.contains(&Square::A6));
    assert!(!targets.contains(&Square::A7));
}

// ---- Castling generation -----------------------------------------

#[test]
fn castling_is_generated_when_all_conditions_hold() {
    // Standard back-rank with nothing between king and rook(s).
    let p = Position::from_fen("4k3/8/8/8/8/8/8/R3K2R w KQ - 0 1").unwrap();
    let moves = pseudo_legal_moves_vec(&p);
    let castles: Vec<_> = moves
        .iter()
        .filter(|m| m.kind() == MoveKind::Castling)
        .collect();
    assert_eq!(castles.len(), 2);
    let tos: Vec<_> = castles.iter().map(|m| m.to()).collect();
    assert!(tos.contains(&Square::G1));
    assert!(tos.contains(&Square::C1));
}

#[test]
fn castling_blocked_by_piece_in_the_way() {
    // Friendly bishop on f1 blocks kingside castling. Queenside still fine.
    let p = Position::from_fen("4k3/8/8/8/8/8/8/R3KB1R w KQ - 0 1").unwrap();
    let moves = pseudo_legal_moves_vec(&p);
    let castles: Vec<_> = moves
        .iter()
        .filter(|m| m.kind() == MoveKind::Castling)
        .collect();
    assert_eq!(castles.len(), 1);
    assert_eq!(castles[0].to(), Square::C1);
}

#[test]
fn cannot_castle_while_in_check() {
    // Black rook on e3 checks white king on e1 up the e-file.
    // Castling should not be generated, even though the physical squares
    // are clear.
    let p = Position::from_fen("4k3/8/8/8/8/4r3/8/R3K2R w KQ - 0 1").unwrap();
    let moves = pseudo_legal_moves_vec(&p);
    let castles: Vec<_> = moves
        .iter()
        .filter(|m| m.kind() == MoveKind::Castling)
        .collect();
    assert!(castles.is_empty());
}

#[test]
fn cannot_castle_through_attacked_square() {
    // Black rook on f8 attacks f1 along the f-file. The white king
    // would pass through f1 to reach g1, so kingside castling is
    // illegal. Queenside is unaffected.
    let p = Position::from_fen("5r2/4k3/8/8/8/8/8/R3K2R w KQ - 0 1").unwrap();
    let moves = pseudo_legal_moves_vec(&p);
    let castles: Vec<_> = moves
        .iter()
        .filter(|m| m.kind() == MoveKind::Castling)
        .collect();
    assert_eq!(castles.len(), 1);
    assert_eq!(
        castles[0].to(),
        Square::C1,
        "kingside blocked, queenside ok"
    );
}

#[test]
fn can_castle_queenside_even_if_b_file_square_attacked() {
    // Black bishop on a3 attacks b2 (but b1 is only crossed by the
    // rook, not the king — so queenside castling is legal).
    // Attackers of b1 from black bishop on a3? Bishop on a3 attacks
    // b2, b4, c1, c5... actually bishop on a3 moves diagonally:
    // a3→b2, a3→b4, a3→c1 (hits c1 — attacking c1!).
    // So let me pick a different square. Bishop on h3 attacks g2,
    // g4, f1, f5, e6, d7, c8... attacks f1 (kingside problem).
    // Use: black bishop on a6. Attacks: b5, b7, c4, c8, d3, e2, f1.
    // So f1 is attacked → kingside problem, not queenside.
    // For a b-file-only attack on b1: black knight on d2 attacks
    // b1, b3, c4, e4, f3, f1. Wait f1 too, no good.
    // Simpler: rook on b7. Attacks b-file including b1. Nothing else.
    let p = Position::from_fen("4k3/1r6/8/8/8/8/8/R3K2R w KQ - 0 1").unwrap();
    let moves = pseudo_legal_moves_vec(&p);
    let castles: Vec<_> = moves
        .iter()
        .filter(|m| m.kind() == MoveKind::Castling)
        .collect();
    let tos: Vec<_> = castles.iter().map(|m| m.to()).collect();
    // Kingside is unrelated: should still be available. Queenside: b1
    // is attacked but the king doesn't pass through b1 — only d1 and
    // c1 matter for the king's safety.
    assert!(tos.contains(&Square::G1));
    assert!(tos.contains(&Square::C1));
}

// ---- Legal filter eliminates king-in-check moves ----------------

#[test]
fn legal_filter_removes_king_in_check_moves() {
    // White rook on e2 is pinned to its king on e1 by the black rook on
    // e6. If the white rook steps off the e-file, the king is exposed
    // — the legal filter must reject those moves.
    let p_start = Position::from_fen("4k3/8/4r3/8/8/8/4R3/4K3 w - - 0 1").unwrap();
    let pseudo = pseudo_legal_moves_vec(&p_start);
    let mut p = p_start.clone();
    let legal = legal_moves_vec(&mut p);
    assert!(
        legal.len() < pseudo.len(),
        "some pseudo-legal moves must be rejected by legality"
    );
    // Specifically: the white rook on e2 cannot move off the e-file
    // because it would expose the king.
    let rook_off_efile: Vec<_> = legal
        .iter()
        .filter(|m| m.from() == Square::E2 && m.to().file() != crate::types::File::E)
        .collect();
    assert!(
        rook_off_efile.is_empty(),
        "pinned rook cannot leave the e-file"
    );
}

// ---- Perft on known positions ------------------------------------

/// Perft values for the standard starting position, from the chess
/// programming wiki / Stockfish test suite:
/// d1=20, d2=400, d3=8902, d4=197281.
#[test]
fn perft_startpos_depth_1() {
    let mut p = Position::startpos();
    assert_eq!(perft(&mut p, 1), 20);
}

#[test]
fn perft_startpos_depth_2() {
    let mut p = Position::startpos();
    assert_eq!(perft(&mut p, 2), 400);
}

#[test]
fn perft_startpos_depth_3() {
    let mut p = Position::startpos();
    assert_eq!(perft(&mut p, 3), 8902);
}

/// "Position 2" (Kiwipete) from chessprogramming.org — a tactical
/// position that exercises captures, castling, en-passant, and
/// promotions. Known values: d1=48, d2=2039, d3=97862.
const KIWIPETE: &str = "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1";

#[test]
fn perft_kiwipete_depth_1() {
    let mut p = Position::from_fen(KIWIPETE).unwrap();
    assert_eq!(perft(&mut p, 1), 48);
}

#[test]
fn perft_kiwipete_depth_2() {
    let mut p = Position::from_fen(KIWIPETE).unwrap();
    assert_eq!(perft(&mut p, 2), 2039);
}

/// Position 3 from the same wiki page — an endgame with lots of
/// checks. Known values: d1=14, d2=191, d3=2812, d4=43238.
const POSITION_3: &str = "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1";

#[test]
fn perft_position_3_depth_3() {
    let mut p = Position::from_fen(POSITION_3).unwrap();
    assert_eq!(perft(&mut p, 3), 2812);
}

/// Position 4 (the "Talkchess" / "KiwiPete variant B" position) —
/// includes a stalemate trap. Known values: d1=6, d2=264, d3=9467.
const POSITION_4: &str = "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1";

#[test]
fn perft_position_4_depth_2() {
    let mut p = Position::from_fen(POSITION_4).unwrap();
    assert_eq!(perft(&mut p, 2), 264);
}
