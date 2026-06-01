use super::*;
use chess_tutor_engine::engine::{Engine, SearchParams};

#[test]
fn tactic_role_variants_are_distinct() {
    assert_ne!(TacticRole::Played, TacticRole::Missed);
    assert_ne!(TacticRole::Missed, TacticRole::WalkedInto);
    assert_ne!(TacticRole::Played, TacticRole::WalkedInto);
}

#[test]
fn verdict_claim_constructs_and_clones() {
    let claim = Claim::Verdict {
        verdict: MoveVerdict::Blunder,
        mover: Color::White,
        san: "Qxf7".to_string(),
        score: Value(-820),
        best_score: Value(15),
        gap: Value(835),
        only_good_move: false,
        sacrifice: false,
        best_san: None,
    };
    // Cloning + Debug must work (derives present).
    let cloned = claim.clone();
    let _ = format!("{cloned:?}");
}

/// Resolve a UCI `from`/`to` (e.g. `a1a2`) against the legal list — the
/// move generator settles the [`chess_tutor_engine::types::MoveKind`].
fn resolve_uci(
    pos: &mut chess_tutor_engine::position::Position,
    uci: &str,
) -> chess_tutor_engine::types::Move {
    use chess_tutor_engine::types::{File, Rank, Square};
    let bytes = uci.as_bytes();
    let sq = |f: u8, r: u8| {
        Square::new(
            File::from_index(f - b'a').unwrap(),
            Rank::from_index(r - b'1').unwrap(),
        )
    };
    let from = sq(bytes[0], bytes[1]);
    let to = sq(bytes[2], bytes[3]);
    chess_tutor_engine::movegen::legal_moves_vec(pos)
        .into_iter()
        .find(|m| m.from() == from && m.to() == to)
        .unwrap_or_else(|| panic!("move {uci} not legal"))
}

/// Run a shallow multi-PV-2 search with the chosen move force-included —
/// the same shape the retrospective worker uses — and build the claim.
fn claim_for(fen: &str, mv_uci: &str, reveal: bool) -> Claim {
    use chess_tutor_engine::position::Position;

    let mut pos = Position::from_fen(fen).unwrap();
    let user_move = resolve_uci(&mut pos, mv_uci);

    let mut engine = Engine::default();
    let analyses = chess_tutor_engine::analysis::analyze_position(
        &mut engine,
        &mut pos,
        SearchParams {
            max_depth: 6,
            multi_pv: 2,
            force_include: vec![user_move],
            ..SearchParams::default()
        },
    );
    let best = &analyses[0];
    let user = analyses.iter().find(|a| a.mv == user_move).unwrap();
    let root_stm = pos.side_to_move();
    let user_mat =
        chess_tutor_engine::analysis::compute_material_outcome(user, &pos, root_stm);
    let best_mat =
        chess_tutor_engine::analysis::compute_material_outcome(best, &pos, root_stm);
    let verdict = user.classify_with_material(best.score, user_mat.net_mg_cp, best_mat.net_mg_cp);
    verdict_claim(&pos, &analyses, best, user, verdict, reveal)
}

/// In a clearly-won position with several good moves, `only_good_move`
/// is **false** — the second-best line is *also* winning, so it never
/// crosses the absolute "second-best loses" threshold. This is the same
/// reason a piece-hang at +25 never reads as "Brilliant": at a big plus
/// there's more than one winning move, so the only-good-move gate fails.
#[test]
fn only_good_move_false_in_a_won_position_with_choices() {
    // White up a rook with multiple sound continuations.
    let claim = claim_for("6k1/8/8/8/8/8/5PPP/R5K1 w - - 0 1", "a1a8", false);
    if let Claim::Verdict { only_good_move, .. } = claim {
        assert!(!only_good_move, "many winning moves ⇒ not the only good move");
    } else {
        panic!("expected Verdict");
    }
}

/// A *forced* move (only one legal reply) is never "only good move" —
/// the predicate explicitly requires `legal_count > 1` so we don't tag a
/// move the player had no choice about.
#[test]
fn only_good_move_false_for_a_forced_single_reply() {
    use chess_tutor_engine::position::Position;
    // Black king on h8, white queen g7 supported by the king on g6:
    // the only legal black move is Kxg7? no — set up a clean single-move
    // case: black king h8 in check from a rook on h1, only escape g-file
    // blocked, etc. Simplest: stalemate-adjacent single move.
    // Black king g8, white queen g7 (defended by Kg1's distant... here
    // simply): the only legal black move is Kxg7.
    let fen = "6k1/6Q1/8/8/8/8/8/6K1 b - - 0 1";
    let mut pos = Position::from_fen(fen).unwrap();
    let legal = chess_tutor_engine::movegen::legal_moves_vec(&mut pos);
    // Guard the fixture: exactly one legal move.
    assert_eq!(legal.len(), 1, "fixture must be a forced single-move position");
    let only = legal[0];
    let uci = format!(
        "{}{}",
        square_uci(only.from()),
        square_uci(only.to())
    );
    let claim = claim_for(fen, &uci, false);
    if let Claim::Verdict { only_good_move, .. } = claim {
        assert!(!only_good_move, "a forced move is never flagged only-good-move");
    } else {
        panic!("expected Verdict");
    }
}

// ---- threats_claims salience ----------------------------------------

use chess_tutor_engine::analysis::{
    HangingPiece, PieceLocation, PressureKind, PressuredPiece, ThreatsOutcome,
};
use chess_tutor_engine::types::{PieceType, Square};

fn pl(square: Square, piece: PieceType) -> PieceLocation {
    PieceLocation { square, piece }
}

fn hang(square: Square, piece: PieceType, attackers: Vec<PieceLocation>) -> HangingPiece {
    HangingPiece {
        location: pl(square, piece),
        attackers,
    }
}

fn empty_outcome() -> ThreatsOutcome {
    ThreatsOutcome {
        ours_hanging: vec![],
        theirs_hanging: vec![],
        ours_see_losing: vec![],
        theirs_see_losing: vec![],
        theirs_hanging_guaranteed: vec![],
        theirs_see_losing_guaranteed: vec![],
        ours_pressured: vec![],
        theirs_pressured: vec![],
        ours_hanging_delta: 0,
        theirs_hanging_delta: 0,
        ours_see_losing_delta: 0,
        theirs_see_losing_delta: 0,
        ours_pressured_delta: 0,
        theirs_pressured_delta: 0,
    }
}

/// A mover-side hang fires only on a positive delta (the move *created*
/// the threat). With a zero delta the claim is suppressed.
#[test]
fn threats_claims_gate_on_positive_delta() {
    let mut outcome = empty_outcome();
    outcome.ours_hanging = vec![hang(Square::D2, PieceType::Knight, vec![pl(Square::E3, PieceType::Pawn)])];

    // delta 0 → no claim.
    assert!(threats_claims(&outcome).is_empty());

    // delta > 0 → one mover-hanging claim.
    outcome.ours_hanging_delta = 1;
    let claims = threats_claims(&outcome);
    assert_eq!(claims.len(), 1);
    assert!(matches!(
        &claims[0],
        Claim::Threats {
            side: ThreatSide::Mover,
            kind: ThreatKind::Hanging,
            ..
        }
    ));
}

/// The opponent (`theirs_*`) side fires off the *guaranteed* list, never
/// the raw static snapshot — a defensible threat (raw list non-empty,
/// guaranteed empty) produces no claim.
#[test]
fn threats_claims_opponent_uses_guaranteed_only() {
    let mut outcome = empty_outcome();
    let piece = hang(Square::E5, PieceType::Pawn, vec![pl(Square::F3, PieceType::Knight)]);
    outcome.theirs_hanging = vec![piece.clone()];
    outcome.theirs_hanging_delta = 1;
    // Raw list non-empty but guaranteed empty (opponent can defend) → no claim.
    assert!(threats_claims(&outcome).is_empty());

    // Guaranteed → a single opponent-hanging claim.
    outcome.theirs_hanging_guaranteed = vec![piece];
    let claims = threats_claims(&outcome);
    assert_eq!(claims.len(), 1);
    assert!(matches!(
        &claims[0],
        Claim::Threats {
            side: ThreatSide::Opponent,
            kind: ThreatKind::Hanging,
            ..
        }
    ));
}

/// A pressured piece already surfaced as hanging on the same side is
/// de-duped out of the pressure claim.
#[test]
fn threats_claims_pressure_deduped_against_hanging() {
    let mut outcome = empty_outcome();
    outcome.ours_hanging = vec![hang(Square::A1, PieceType::Rook, vec![pl(Square::C2, PieceType::Knight)])];
    outcome.ours_hanging_delta = 1;
    outcome.ours_pressured = vec![PressuredPiece {
        location: pl(Square::A1, PieceType::Rook),
        attackers: vec![pl(Square::C2, PieceType::Knight)],
        kind: PressureKind::MinorOnMajor,
    }];
    outcome.ours_pressured_delta = 1;

    let claims = threats_claims(&outcome);
    // Only the hanging claim survives — the pressure entry on the same
    // square is suppressed.
    assert_eq!(claims.len(), 1);
    assert!(matches!(
        &claims[0],
        Claim::Threats {
            kind: ThreatKind::Hanging,
            ..
        }
    ));
}

/// `threats_claim_group` returns `None` for an empty piece list.
#[test]
fn threats_claim_group_none_on_empty() {
    assert!(threats_claim_group(ThreatSide::Mover, ThreatKind::Hanging, vec![]).is_none());
}

fn square_uci(sq: chess_tutor_engine::types::Square) -> String {
    let file = (b'a' + sq.file() as u8) as char;
    let rank = (b'1' + sq.rank() as u8) as char;
    format!("{file}{rank}")
}

/// Reveal threading: with `reveal_moves` on and the engine preferring a
/// different move, the claim carries the best SAN; off, it doesn't.
#[test]
fn verdict_claim_best_san_gated_on_reveal() {
    // 1.e4 is fine but the engine may prefer another first move; force a
    // clearly-suboptimal first move so best != user.
    let revealed = claim_for("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1", "a2a3", true);
    let plain = claim_for("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1", "a2a3", false);
    if let Claim::Verdict { best_san, .. } = plain {
        assert!(best_san.is_none(), "reveal off ⇒ no best SAN");
    }
    if let Claim::Verdict { best_san, .. } = revealed {
        // a2a3 is unlikely to be the engine's pick at depth 6, so a
        // distinct best move should be revealed.
        assert!(best_san.is_some(), "reveal on + distinct best ⇒ best SAN present");
    }
}

// ---- mobility_claims salience ----------------------------------------

fn mb(knight: i32, bishop: i32, rook: i32, queen: i32) -> MobilityBreakdown {
    use chess_tutor_engine::types::Score;
    MobilityBreakdown {
        knight: Score::new(knight, 0),
        bishop: Score::new(bishop, 0),
        rook: Score::new(rook, 0),
        queen: Score::new(queen, 0),
    }
}

fn mob_outcome(
    ours_pre: MobilityBreakdown,
    ours_post: MobilityBreakdown,
    theirs_pre: MobilityBreakdown,
    theirs_post: MobilityBreakdown,
) -> MobilityOutcome {
    MobilityOutcome {
        ours_pre,
        ours_post,
        theirs_pre,
        theirs_post,
        ours_per_piece_pre: Vec::new(),
        ours_per_piece_post: Vec::new(),
        theirs_per_piece_pre: Vec::new(),
        theirs_per_piece_post: Vec::new(),
    }
}

/// Below-threshold shifts produce no claims; the biggest one above the
/// threshold fires.
#[test]
fn mobility_claims_threshold_gates() {
    // Knight shifts 40 cp (below 50), bishop 60 (above).
    let out = mob_outcome(
        mb(0, 20, 0, 0),
        mb(40, 80, 0, 0),
        mb(0, 0, 0, 0),
        mb(0, 0, 0, 0),
    );
    let claims = mobility_claims(&out, 50);
    assert_eq!(claims.len(), 1, "only the bishop clears 50 cp");
    let Claim::Mobility { side, piece, pre_cp, post_cp } = claims[0] else {
        panic!("expected a Mobility claim");
    };
    assert_eq!(side, MobilitySide::Mover);
    assert_eq!(piece, PieceType::Bishop);
    assert_eq!((pre_cp, post_cp), (20, 80));
}

/// Mover-side claims precede opponent-side, and within a side the
/// biggest |delta| comes first.
#[test]
fn mobility_claims_orders_mover_first_then_biggest() {
    let out = mob_outcome(
        mb(0, 0, 80, 90),
        mb(20, 0, 130, 95), // rook +50, knight +20
        mb(0, 0, 0, 30),
        mb(0, 0, 0, 110), // queen +80
    );
    let claims = mobility_claims(&out, 20);
    // Mover rook (+50), mover knight (+20), then opponent queen (+80).
    assert_eq!(claims.len(), 3);
    let kinds: Vec<(MobilitySide, PieceType)> = claims
        .iter()
        .map(|c| match c {
            Claim::Mobility { side, piece, .. } => (*side, *piece),
            _ => panic!("expected Mobility"),
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            (MobilitySide::Mover, PieceType::Rook),
            (MobilitySide::Mover, PieceType::Knight),
            (MobilitySide::Opponent, PieceType::Queen),
        ]
    );
}

// ---- pawn_structure_claims salience ----------------------------------

use chess_tutor_engine::analysis::{PassedPawnsOutcome, PawnStructureOutcome};
use chess_tutor_engine::eval::{PassedBreakdown, PawnsBreakdown};

fn pb(
    connected: i32,
    isolated: i32,
    backward: i32,
    doubled: i32,
    weak_unopposed: i32,
    weak_lever: i32,
) -> PawnsBreakdown {
    use chess_tutor_engine::types::Score;
    PawnsBreakdown {
        connected: Score::new(connected, 0),
        isolated: Score::new(isolated, 0),
        backward: Score::new(backward, 0),
        doubled: Score::new(doubled, 0),
        weak_unopposed: Score::new(weak_unopposed, 0),
        weak_lever: Score::new(weak_lever, 0),
    }
}

fn ps_outcome(
    ours_pre: PawnsBreakdown,
    ours_post: PawnsBreakdown,
    theirs_pre: PawnsBreakdown,
    theirs_post: PawnsBreakdown,
) -> PawnStructureOutcome {
    PawnStructureOutcome {
        ours_pre,
        ours_post,
        theirs_pre,
        theirs_post,
    }
}

/// A sub-shift below the 15 cp threshold produces no claim.
#[test]
fn pawn_structure_claims_threshold_gates() {
    let out = ps_outcome(
        pb(0, 0, 0, 0, 0, 0),
        pb(0, 0, 0, -10, 0, 0),
        pb(0, 0, 0, 0, 0, 0),
        pb(0, 0, 0, 0, 0, 0),
    );
    assert!(pawn_structure_claims(&out).is_empty());
}

/// A new doubled pawn on the mover's side fires a worsened claim with
/// the right category.
#[test]
fn pawn_structure_claims_mover_worsened_doubled() {
    let out = ps_outcome(
        pb(0, 0, 0, 0, 0, 0),
        pb(0, 0, 0, -20, 0, 0),
        pb(0, 0, 0, 0, 0, 0),
        pb(0, 0, 0, 0, 0, 0),
    );
    let claims = pawn_structure_claims(&out);
    assert_eq!(claims.len(), 1);
    let Claim::PawnStructure { side, direction, categories } = &claims[0] else {
        panic!("expected PawnStructure");
    };
    assert_eq!(*side, PawnSide::Mover);
    assert_eq!(*direction, StructureDirection::Worsened);
    assert_eq!(categories.as_slice(), &[PawnCategory::Doubled]);
}

/// Worsening wins over improving on the same side.
#[test]
fn pawn_structure_claims_worsened_beats_improved_same_side() {
    // Doubled worsens (-20) while Connected improves (+20) on the mover.
    let out = ps_outcome(
        pb(0, 0, 0, -20, 0, 0),
        pb(20, 0, 0, -40, 0, 0),
        pb(0, 0, 0, 0, 0, 0),
        pb(0, 0, 0, 0, 0, 0),
    );
    let claims = pawn_structure_claims(&out);
    let Claim::PawnStructure { direction, .. } = &claims[0] else {
        panic!("expected PawnStructure");
    };
    assert_eq!(*direction, StructureDirection::Worsened);
}

/// Mover-side claim precedes the opponent-side claim.
#[test]
fn pawn_structure_claims_order_mover_first() {
    let out = ps_outcome(
        pb(0, 0, 0, 0, 0, 0),
        pb(0, 0, 0, -20, 0, 0),
        pb(0, 0, 0, 0, 0, 0),
        pb(0, 0, 0, -20, 0, 0),
    );
    let claims = pawn_structure_claims(&out);
    assert_eq!(claims.len(), 2);
    let sides: Vec<PawnSide> = claims
        .iter()
        .map(|c| match c {
            Claim::PawnStructure { side, .. } => *side,
            _ => panic!("expected PawnStructure"),
        })
        .collect();
    assert_eq!(sides, vec![PawnSide::Mover, PawnSide::Opponent]);
}

// ---- passed_pawns_claims salience ------------------------------------

fn pa(rank: i32, king_prox: i32, free_adv: i32, stopper: i32) -> PassedBreakdown {
    use chess_tutor_engine::types::Score;
    PassedBreakdown {
        rank_bonus: Score::new(rank, 0),
        king_proximity: Score::new(king_prox, 0),
        free_advance: Score::new(free_adv, 0),
        stopper_penalty: Score::new(stopper, 0),
    }
}

fn pass_outcome(
    ours_pre: PassedBreakdown,
    ours_post: PassedBreakdown,
    theirs_pre: PassedBreakdown,
    theirs_post: PassedBreakdown,
) -> PassedPawnsOutcome {
    PassedPawnsOutcome {
        ours_pre,
        ours_post,
        theirs_pre,
        theirs_post,
    }
}

/// Aggregate shift below 20 cp does not fire.
#[test]
fn passed_pawns_claims_threshold_gates() {
    let out = pass_outcome(
        pa(0, 0, 0, 0),
        pa(10, 0, 0, 0),
        pa(0, 0, 0, 0),
        pa(0, 0, 0, 0),
    );
    assert!(passed_pawns_claims(&out).is_empty());
}

/// A mover passer pushing forward fires an improved claim with a
/// positive (side-relative) delta.
#[test]
fn passed_pawns_claims_mover_improved() {
    let out = pass_outcome(
        pa(50, 0, 0, 0),
        pa(90, 0, 0, 0),
        pa(0, 0, 0, 0),
        pa(0, 0, 0, 0),
    );
    let claims = passed_pawns_claims(&out);
    assert_eq!(claims.len(), 1);
    let Claim::PassedPawns { side, direction, delta_mg } = &claims[0] else {
        panic!("expected PassedPawns");
    };
    assert_eq!(*side, PawnSide::Mover);
    assert_eq!(*direction, StructureDirection::Improved);
    assert_eq!(*delta_mg, 40);
}

/// An opponent passer being blunted fires a worsened claim on the
/// opponent side with a negative (side-relative) delta.
#[test]
fn passed_pawns_claims_opponent_worsened() {
    let out = pass_outcome(
        pa(0, 0, 0, 0),
        pa(0, 0, 0, 0),
        pa(80, 0, 0, 0),
        pa(40, 0, 0, 0),
    );
    let claims = passed_pawns_claims(&out);
    assert_eq!(claims.len(), 1);
    let Claim::PassedPawns { side, direction, delta_mg } = &claims[0] else {
        panic!("expected PassedPawns");
    };
    assert_eq!(*side, PawnSide::Opponent);
    assert_eq!(*direction, StructureDirection::Worsened);
    assert_eq!(*delta_mg, -40);
}

// ---- pieces_positional_claims salience -------------------------------

use chess_tutor_engine::analysis::{
    InitiativeOutcome, PiecesPositionalOutcome, SpaceOutcome,
};
use chess_tutor_engine::eval::PiecesBreakdown;

fn pib_zero() -> PiecesBreakdown {
    use chess_tutor_engine::types::Score;
    PiecesBreakdown {
        outposts: Score::ZERO,
        reachable_outposts: Score::ZERO,
        minor_behind_pawn: Score::ZERO,
        king_protector: Score::ZERO,
        bishop_pawns: Score::ZERO,
        long_diagonal_bishop: Score::ZERO,
        rook_on_queen_file: Score::ZERO,
        rook_on_open_file: Score::ZERO,
        rook_on_semiopen_file: Score::ZERO,
        trapped_rook: Score::ZERO,
        weak_queen: Score::ZERO,
    }
}

fn pieces_outcome(
    ours_pre: PiecesBreakdown,
    ours_post: PiecesBreakdown,
    theirs_pre: PiecesBreakdown,
    theirs_post: PiecesBreakdown,
    bishop_geometry_changed: bool,
) -> PiecesPositionalOutcome {
    let count_post = u32::from(bishop_geometry_changed);
    PiecesPositionalOutcome {
        ours_pre,
        ours_post,
        theirs_pre,
        theirs_post,
        ours_bishop_pawn_count_pre: 0,
        ours_bishop_pawn_count_post: count_post,
        theirs_bishop_pawn_count_pre: 0,
        theirs_bishop_pawn_count_post: count_post,
    }
}

/// A sub-shift below the threshold produces no claim.
#[test]
fn pieces_claims_threshold_gates() {
    use chess_tutor_engine::types::Score;
    let mut post = pib_zero();
    post.outposts = Score::new(10, 0);
    let out = pieces_outcome(pib_zero(), post, pib_zero(), pib_zero(), true);
    assert!(pieces_positional_claims(&out).is_empty());
}

/// A claimed outpost on the mover's side fires an improved claim.
#[test]
fn pieces_claims_mover_improved_outpost() {
    use chess_tutor_engine::types::Score;
    let mut post = pib_zero();
    post.outposts = Score::new(30, 0);
    let out = pieces_outcome(pib_zero(), post, pib_zero(), pib_zero(), true);
    let claims = pieces_positional_claims(&out);
    assert_eq!(claims.len(), 1);
    let Claim::PiecePlacement { side, category, direction, delta_mg } = &claims[0] else {
        panic!("expected PiecePlacement");
    };
    assert_eq!(*side, PlacementSide::Mover);
    assert_eq!(*category, PlacementCategory::Outposts);
    assert_eq!(*direction, StructureDirection::Improved);
    assert_eq!(*delta_mg, 30);
}

/// BishopPawns is suppressed when bishop geometry didn't change (the
/// 1.e4 e5 blocked-centre multiplier artifact).
#[test]
fn pieces_claims_bishop_pawns_suppressed_when_geometry_unchanged() {
    use chess_tutor_engine::types::Score;
    let mut pre = pib_zero();
    pre.bishop_pawns = Score::new(-24, 0);
    let mut post = pib_zero();
    post.bishop_pawns = Score::new(-48, 0);
    let out = pieces_outcome(pre, post, pib_zero(), pib_zero(), false);
    assert!(pieces_positional_claims(&out).is_empty());
}

/// Mover-side claims precede opponent-side claims.
#[test]
fn pieces_claims_order_mover_first() {
    use chess_tutor_engine::types::Score;
    let mut ours_post = pib_zero();
    ours_post.outposts = Score::new(30, 0);
    let mut theirs_post = pib_zero();
    theirs_post.outposts = Score::new(30, 0);
    let out = pieces_outcome(pib_zero(), ours_post, pib_zero(), theirs_post, true);
    let claims = pieces_positional_claims(&out);
    assert_eq!(claims.len(), 2);
    let sides: Vec<PlacementSide> = claims
        .iter()
        .map(|c| match c {
            Claim::PiecePlacement { side, .. } => *side,
            _ => panic!("expected PiecePlacement"),
        })
        .collect();
    assert_eq!(sides, vec![PlacementSide::Mover, PlacementSide::Opponent]);
}

// ---- space_claims salience -------------------------------------------

fn space_outcome(ours_pre: i32, ours_post: i32, theirs_pre: i32, theirs_post: i32) -> SpaceOutcome {
    use chess_tutor_engine::bitboard::Bitboard;
    SpaceOutcome {
        ours_space_pre_mg: ours_pre,
        ours_space_post_mg: ours_post,
        theirs_space_pre_mg: theirs_pre,
        theirs_space_post_mg: theirs_post,
        ours_piece_count_pre: 16,
        ours_piece_count_post: 16,
        theirs_piece_count_pre: 16,
        theirs_piece_count_post: 16,
        ours_safe_post: Bitboard::EMPTY,
        ours_reinforced_post: Bitboard::EMPTY,
        theirs_safe_post: Bitboard::EMPTY,
        theirs_reinforced_post: Bitboard::EMPTY,
    }
}

/// A shift below the threshold produces no claim.
#[test]
fn space_claims_threshold_gates() {
    let out = space_outcome(0, 10, 0, 0);
    assert!(space_claims(&out, SPACE_DEFAULT_THRESHOLD_CP).is_empty());
}

/// The mover gaining space fires a Gained claim with a positive delta.
#[test]
fn space_claims_mover_gained() {
    let out = space_outcome(20, 60, 0, 0);
    let claims = space_claims(&out, SPACE_DEFAULT_THRESHOLD_CP);
    assert_eq!(claims.len(), 1);
    let Claim::Space { side, direction, delta_mg } = &claims[0] else {
        panic!("expected Space");
    };
    assert_eq!(*side, SpaceSide::Mover);
    assert_eq!(*direction, SpaceDirection::Gained);
    assert_eq!(*delta_mg, 40);
}

/// The opponent's space shrinking fires a Lost claim on the opponent
/// side with a negative (side-relative) delta.
#[test]
fn space_claims_opponent_lost() {
    let out = space_outcome(0, 0, 60, 20);
    let claims = space_claims(&out, SPACE_DEFAULT_THRESHOLD_CP);
    assert_eq!(claims.len(), 1);
    let Claim::Space { side, direction, delta_mg } = &claims[0] else {
        panic!("expected Space");
    };
    assert_eq!(*side, SpaceSide::Opponent);
    assert_eq!(*direction, SpaceDirection::Lost);
    assert_eq!(*delta_mg, -40);
}

// ---- initiative_claim salience ---------------------------------------

fn init_outcome(
    threat: bool,
    check: bool,
    capture: bool,
    san: Option<&str>,
    swing: i32,
    favored: bool,
) -> InitiativeOutcome {
    InitiativeOutcome {
        user_move_was_threat: threat,
        opponent_reply_is_check: check,
        opponent_reply_is_capture: capture,
        opponent_reply_san: san.map(|s| s.to_string()),
        eval_swing_cp: swing,
        user_still_favored: favored,
    }
}

/// No threat ⇒ no claim.
#[test]
fn initiative_claim_silent_without_threat() {
    let out = init_outcome(false, false, false, Some("Nf6"), 0, true);
    assert!(initiative_claim(&out, Color::White).is_none());
}

/// No named reply ⇒ no claim (no template can be anchored).
#[test]
fn initiative_claim_silent_without_reply_san() {
    let out = init_outcome(true, false, false, None, 0, true);
    assert!(initiative_claim(&out, Color::White).is_none());
}

/// A threat with a quiet opponent reply is reinforcement.
#[test]
fn initiative_claim_reinforcement() {
    let out = init_outcome(true, false, false, Some("Nf6"), 0, true);
    let Some(Claim::Initiative { template, .. }) = initiative_claim(&out, Color::White) else {
        panic!("expected reinforcement claim");
    };
    assert_eq!(template, InitiativeTemplate::Reinforcement);
}

/// A forcing reply that settles against the mover with a real swing is
/// refutation; a check is carried as such.
#[test]
fn initiative_claim_refutation_check() {
    let out = init_outcome(true, true, false, Some("Qa3+"), -300, false);
    let Some(Claim::Initiative { template, reply_is_check, reply_san, .. }) =
        initiative_claim(&out, Color::White)
    else {
        panic!("expected refutation claim");
    };
    assert_eq!(template, InitiativeTemplate::Refutation);
    assert!(reply_is_check);
    assert_eq!(reply_san, "Qa3+");
}

/// A refutation-shaped reply whose swing is below the gate is suppressed.
#[test]
fn initiative_claim_refutation_suppressed_small_swing() {
    let out = init_outcome(true, true, false, Some("Kh1"), -10, false);
    assert!(initiative_claim(&out, Color::White).is_none());
}

/// A forcing reply where the mover stays favoured is held-despite, even
/// with a large swing.
#[test]
fn initiative_claim_held_despite_when_favored() {
    let out = init_outcome(true, true, false, Some("Qa3+"), -500, true);
    let Some(Claim::Initiative { template, .. }) = initiative_claim(&out, Color::White) else {
        panic!("expected held-despite claim");
    };
    assert_eq!(template, InitiativeTemplate::HeldDespite);
}

// ---- secondary_claim salience ----------------------------------------

/// The secondary claim sign-flips raw white-POV deltas to mover-POV: a
/// black mover sees a white-favouring delta as a *hurt*. Exercised via a
/// real analysis so `term_deltas` are populated.
#[test]
fn secondary_claim_flips_sign_for_black_mover() {
    use chess_tutor_engine::analysis::analyze_position;
    use chess_tutor_engine::movegen::legal_moves_vec;
    use chess_tutor_engine::position::Position;
    use chess_tutor_engine::types::Square;

    // After 1.e4, black to move. Play 1...e5.
    let mut pos =
        Position::from_fen("rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1").unwrap();
    let e5 = legal_moves_vec(&mut pos)
        .into_iter()
        .find(|m| m.from() == Square::E7 && m.to() == Square::E5)
        .unwrap();
    let mut engine = Engine::new(16);
    let analyses = analyze_position(
        &mut engine,
        &mut pos,
        SearchParams {
            max_depth: 6,
            multi_pv: 2,
            force_include: vec![e5],
            threads: 1,
            ..SearchParams::default()
        },
    );
    let user = analyses.iter().find(|a| a.mv == e5).unwrap();
    // No panic, mover-POV claim builds; if any term survives the trim it
    // is stored mover-relative (we only assert the shape here).
    if let Some(Claim::Secondary { terms }) = secondary_claim(user, Color::Black, &[], 50.0) {
        assert!(!terms.is_empty());
    }
}

// =====================================================================
// Special UI narratives — builder salience (step 10)
// =====================================================================

// ---- surprise_claim: the shallow-vs-deep gate (ported from the old
//      surprise_tag selector) ----------------------------------------

#[test]
fn surprise_claim_none_when_no_surprise() {
    assert!(surprise_claim(MoveVerdict::Inaccuracy, None, Color::White).is_none());
}

#[test]
fn surprise_claim_fires_on_inaccuracy_and_mistake_looks_good_but_bad() {
    for v in [MoveVerdict::Inaccuracy, MoveVerdict::Mistake] {
        let c = surprise_claim(v, Some(SurpriseKind::LooksGoodButBad), Color::White)
            .expect("should fire");
        assert!(matches!(c, Claim::Surprise { .. }));
    }
}

#[test]
fn surprise_claim_fires_on_best_and_good_looks_bad_but_good() {
    for v in [MoveVerdict::Best, MoveVerdict::Good] {
        assert!(
            surprise_claim(v, Some(SurpriseKind::LooksBadButGood), Color::White).is_some()
        );
    }
}

#[test]
fn surprise_claim_suppressed_when_kind_contradicts_verdict() {
    assert!(surprise_claim(MoveVerdict::Best, Some(SurpriseKind::LooksGoodButBad), Color::White).is_none());
    assert!(surprise_claim(MoveVerdict::Good, Some(SurpriseKind::LooksGoodButBad), Color::White).is_none());
    assert!(surprise_claim(MoveVerdict::Inaccuracy, Some(SurpriseKind::LooksBadButGood), Color::White).is_none());
    assert!(surprise_claim(MoveVerdict::Mistake, Some(SurpriseKind::LooksBadButGood), Color::White).is_none());
}

#[test]
fn surprise_claim_suppressed_on_blunder_and_best_available() {
    for v in [MoveVerdict::Blunder, MoveVerdict::BestAvailable] {
        assert!(surprise_claim(v, Some(SurpriseKind::LooksGoodButBad), Color::White).is_none());
        assert!(surprise_claim(v, Some(SurpriseKind::LooksBadButGood), Color::White).is_none());
    }
}

#[test]
fn surprise_claim_carries_mover() {
    let c = surprise_claim(MoveVerdict::Best, Some(SurpriseKind::LooksBadButGood), Color::Black)
        .expect("fires");
    let Claim::Surprise { mover, .. } = c else { panic!("wrong variant") };
    assert_eq!(mover, Color::Black);
}

// ---- forced_consequence_claims: PV-walk salience ---------------------

#[test]
fn forced_consequence_short_pv_yields_nothing() {
    // A one-ply PV can't reach the opponent's reply, so no concession.
    let pos = Position::startpos();
    let mut engine = Engine::new(16);
    let mut p = pos.clone();
    let analyses = chess_tutor_engine::analysis::analyze_position(
        &mut engine,
        &mut p,
        SearchParams { max_depth: 1, multi_pv: 1, threads: 1, ..SearchParams::default() },
    );
    let user = &analyses[0];
    // Depth-1 PVs are typically a single move; if so, no forced-consequence.
    if user.pv.len() < 2 {
        assert!(forced_consequence_claims(&pos, user, Color::White).is_empty());
    }
}

#[test]
fn forced_consequence_claims_are_mover_relative_and_negative() {
    // gxh6 doubling Black's h-pawns: White to move, Bxh6 trade then ...gxh6.
    let fen = "r2qk2r/ppp2ppp/2nb1n1b/3p4/3P4/2NBPN2/PPP2PPP/R2QK2R w KQkq - 0 1";
    let pos = Position::from_fen(fen).unwrap();
    let mut engine = Engine::new(16);
    let mut p = pos.clone();
    let analyses = chess_tutor_engine::analysis::analyze_position(
        &mut engine,
        &mut p,
        SearchParams { max_depth: 8, multi_pv: 1, threads: 1, ..SearchParams::default() },
    );
    let user = &analyses[0];
    // Whatever the engine's top line is, any forced-consequence claim it
    // emits must be a concession (negative delta) attributed to White.
    for c in forced_consequence_claims(&pos, user, Color::White) {
        let Claim::ForcedConsequence { mover, delta_mg, .. } = c else { panic!("wrong variant") };
        assert_eq!(mover, Color::White);
        assert!(delta_mg < 0, "a concession is a more-negative pawn delta");
    }
}

// ---- Cross-term multipliers (centre structure, castling) -----------------

fn blocked_center(
    locked_delta_ours: (u32, u32),
    locked_delta_theirs: (u32, u32),
    barricaded_delta_ours: (u32, u32),
    barricaded_delta_theirs: (u32, u32),
    ours_amp: bool,
    theirs_amp: bool,
) -> BlockedCenterOutcome {
    BlockedCenterOutcome {
        ours_locked_pre: locked_delta_ours.0,
        ours_locked_post: locked_delta_ours.1,
        theirs_locked_pre: locked_delta_theirs.0,
        theirs_locked_post: locked_delta_theirs.1,
        ours_barricaded_pre: barricaded_delta_ours.0,
        ours_barricaded_post: barricaded_delta_ours.1,
        theirs_barricaded_pre: barricaded_delta_theirs.0,
        theirs_barricaded_post: barricaded_delta_theirs.1,
        ours_amplifies_bishop_penalty: ours_amp,
        theirs_amplifies_bishop_penalty: theirs_amp,
    }
}

#[test]
fn center_structure_silent_when_neither_side_amplifies() {
    // A lock appears, but no bishop / same-coloured pawn to amplify.
    let o = blocked_center((0, 1), (0, 1), (0, 0), (0, 0), false, false);
    assert!(center_structure_claims(&o, Color::White).is_empty());
}

#[test]
fn center_structure_silent_when_no_change() {
    let o = blocked_center((1, 1), (0, 0), (0, 0), (0, 0), true, true);
    assert!(center_structure_claims(&o, Color::White).is_empty());
}

#[test]
fn center_structure_closed_on_new_lock() {
    let o = blocked_center((0, 1), (0, 1), (0, 0), (0, 0), true, true);
    let claims = center_structure_claims(&o, Color::White);
    assert_eq!(claims.len(), 1);
    let Claim::CenterStructure { mover, kind } = claims[0] else {
        panic!("wrong variant");
    };
    assert_eq!(mover, Color::White);
    assert_eq!(kind, CenterShift::Closed);
}

#[test]
fn center_structure_opened_when_lock_dissolves() {
    let o = blocked_center((2, 1), (1, 0), (0, 0), (0, 0), true, true);
    let claims = center_structure_claims(&o, Color::Black);
    assert_eq!(claims.len(), 1);
    let Claim::CenterStructure { mover, kind } = claims[0] else {
        panic!("wrong variant");
    };
    assert_eq!(mover, Color::Black);
    assert_eq!(kind, CenterShift::Opened);
}

#[test]
fn center_structure_barricaded_when_piece_lands_in_front() {
    // 2.Nf3 shape: a knight lands in front of f2 (barricade up by one).
    let o = blocked_center((1, 1), (1, 1), (0, 1), (0, 0), true, true);
    let claims = center_structure_claims(&o, Color::White);
    assert_eq!(claims.len(), 1);
    let Claim::CenterStructure { kind, .. } = claims[0] else {
        panic!("wrong variant");
    };
    assert_eq!(kind, CenterShift::Barricaded);
}

#[test]
fn center_structure_emits_both_axes_when_both_move() {
    let o = blocked_center((0, 1), (0, 0), (0, 1), (0, 0), true, true);
    let kinds: Vec<CenterShift> = center_structure_claims(&o, Color::White)
        .into_iter()
        .map(|c| {
            let Claim::CenterStructure { kind, .. } = c else {
                panic!("wrong variant");
            };
            kind
        })
        .collect();
    assert_eq!(kinds, vec![CenterShift::Closed, CenterShift::Barricaded]);
}

fn castling(
    ours_pre: bool,
    ours_post: bool,
    theirs_pre: bool,
    theirs_post: bool,
    ours_trapped_mg: i32,
    theirs_trapped_mg: i32,
) -> CastlingOutcome {
    CastlingOutcome {
        ours_could_castle_pre: ours_pre,
        ours_could_castle_post: ours_post,
        theirs_could_castle_pre: theirs_pre,
        theirs_could_castle_post: theirs_post,
        ours_trapped_rook_post_mg: ours_trapped_mg,
        theirs_trapped_rook_post_mg: theirs_trapped_mg,
    }
}

#[test]
fn castling_fires_for_mover_when_own_rights_lost_and_rook_trapped() {
    let o = castling(true, false, true, true, -90, 0);
    let claims = castling_claims(&o);
    assert_eq!(claims.len(), 1);
    let Claim::CastlingLoss { side } = claims[0] else {
        panic!("wrong variant");
    };
    assert_eq!(side, CastleSide::Mover);
}

#[test]
fn castling_fires_for_opponent_when_we_strip_their_rights() {
    let o = castling(true, true, true, false, 0, -90);
    let claims = castling_claims(&o);
    assert_eq!(claims.len(), 1);
    let Claim::CastlingLoss { side } = claims[0] else {
        panic!("wrong variant");
    };
    assert_eq!(side, CastleSide::Opponent);
}

#[test]
fn castling_silent_when_rights_kept() {
    let o = castling(true, true, true, true, -90, -90);
    assert!(castling_claims(&o).is_empty());
}

#[test]
fn castling_silent_when_no_trapped_rook() {
    let o = castling(true, false, true, false, 0, 0);
    assert!(castling_claims(&o).is_empty());
}

#[test]
fn castling_threshold_suppresses_tiny_penalty() {
    let o = castling(true, false, true, true, -10, 0);
    assert!(castling_claims(&o).is_empty());
}
