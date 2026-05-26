use super::*;
use crate::types::Color;

// ---- Black mirrors white -----------------------------------------

#[test]
fn black_psq_is_white_mirrored_and_negated() {
    // For every piece type and square, black's psq on the vertically-
    // flipped square must equal the negation of white's.
    for pt in [
        PieceType::Pawn,
        PieceType::Knight,
        PieceType::Bishop,
        PieceType::Rook,
        PieceType::Queen,
        PieceType::King,
    ] {
        let white = Piece::new(Color::White, pt);
        let black = Piece::new(Color::Black, pt);
        for i in 0u8..64 {
            let sq = Square::from_index(i);
            let flipped = sq.flip_vertical();
            let w = psq_score(white, sq);
            let b = psq_score(black, flipped);
            assert_eq!(
                w.mg().0,
                -b.mg().0,
                "mg mismatch: {:?} on {} vs {:?} on {}",
                white,
                sq.to_algebraic(),
                black,
                flipped.to_algebraic()
            );
            assert_eq!(
                w.eg().0,
                -b.eg().0,
                "eg mismatch: {:?} on {} vs {:?} on {}",
                white,
                sq.to_algebraic(),
                black,
                flipped.to_algebraic()
            );
        }
    }
}

// ---- File symmetry for non-pawns ---------------------------------

#[test]
fn non_pawn_scores_are_file_symmetric() {
    // A knight on b4 and a knight on g4 should score identically
    // (they're mirror images across the d/e file boundary).
    for pt in [
        PieceType::Knight,
        PieceType::Bishop,
        PieceType::Rook,
        PieceType::Queen,
        PieceType::King,
    ] {
        let piece = Piece::new(Color::White, pt);
        for r in 0u8..8 {
            for f in 0u8..4 {
                let left = Square::from_index(r * 8 + f);
                let right = Square::from_index(r * 8 + (7 - f));
                assert_eq!(
                    psq_score(piece, left),
                    psq_score(piece, right),
                    "{:?} not file-symmetric at rank {}",
                    pt,
                    r + 1
                );
            }
        }
    }
}

#[test]
fn pawn_scores_are_not_file_symmetric() {
    // Pawns use PBonus (asymmetric). Confirm that at least one
    // rank-2 square disagrees with its mirror.
    let wp = Piece::new(Color::White, PieceType::Pawn);
    let a2 = psq_score(wp, Square::A2);
    let h2 = psq_score(wp, Square::H2);
    assert_ne!(a2, h2, "pawn psq should be asymmetric across files");
}

// ---- Known reference entries -------------------------------------

#[test]
fn knight_on_a1_has_deep_penalty() {
    // Reference: Bonus[Knight][rank 1][file a] = S(-175, -96).
    // psq[W_KNIGHT][A1] = knight_value + bonus
    //                   = (781, 854) + (-175, -96) = (606, 758)
    let s = psq_score(Piece::WhiteKnight, Square::A1);
    assert_eq!(s.mg().0, 781 - 175);
    assert_eq!(s.eg().0, 854 - 96);
}

#[test]
fn king_on_e1_matches_reference_entry() {
    // Reference: Bonus[King][rank 1][file folded(E) = 3] = S(198, 76).
    // Kings have no material contribution.
    let s = psq_score(Piece::WhiteKing, Square::E1);
    assert_eq!(s.mg().0, 198);
    assert_eq!(s.eg().0, 76);
}

#[test]
fn white_pawn_on_d4_matches_reference_entry() {
    // Reference: PBonus[rank 4][file D] = S(20, -4).
    // Plus pawn material = (128, 213). Total = (148, 209).
    let s = psq_score(Piece::WhitePawn, Square::D4);
    assert_eq!(s.mg().0, 128 + 20);
    assert_eq!(s.eg().0, 213 - 4);
}

// ---- Sum over startpos is zero -----------------------------------

#[test]
fn startpos_psq_sum_is_zero() {
    // White's pieces mirror black's, so summing psq over every square
    // of the starting position gives zero in both phases.
    let startpos: [(Piece, Square); 32] = [
        (Piece::WhiteRook, Square::A1),
        (Piece::WhiteKnight, Square::B1),
        (Piece::WhiteBishop, Square::C1),
        (Piece::WhiteQueen, Square::D1),
        (Piece::WhiteKing, Square::E1),
        (Piece::WhiteBishop, Square::F1),
        (Piece::WhiteKnight, Square::G1),
        (Piece::WhiteRook, Square::H1),
        (Piece::WhitePawn, Square::A2),
        (Piece::WhitePawn, Square::B2),
        (Piece::WhitePawn, Square::C2),
        (Piece::WhitePawn, Square::D2),
        (Piece::WhitePawn, Square::E2),
        (Piece::WhitePawn, Square::F2),
        (Piece::WhitePawn, Square::G2),
        (Piece::WhitePawn, Square::H2),
        (Piece::BlackPawn, Square::A7),
        (Piece::BlackPawn, Square::B7),
        (Piece::BlackPawn, Square::C7),
        (Piece::BlackPawn, Square::D7),
        (Piece::BlackPawn, Square::E7),
        (Piece::BlackPawn, Square::F7),
        (Piece::BlackPawn, Square::G7),
        (Piece::BlackPawn, Square::H7),
        (Piece::BlackRook, Square::A8),
        (Piece::BlackKnight, Square::B8),
        (Piece::BlackBishop, Square::C8),
        (Piece::BlackQueen, Square::D8),
        (Piece::BlackKing, Square::E8),
        (Piece::BlackBishop, Square::F8),
        (Piece::BlackKnight, Square::G8),
        (Piece::BlackRook, Square::H8),
    ];
    let sum: Score = startpos
        .iter()
        .fold(Score::ZERO, |acc, (p, s)| acc + psq_score(*p, *s));
    assert_eq!(sum, Score::ZERO);
}
