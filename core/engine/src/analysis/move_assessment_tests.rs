use super::*;
use crate::eval::EvalTrace;
use crate::types::{Square, Value};

use super::super::{PriorMove, TermDelta};

fn make_delta(term: TermId, white_pov_tapered: i32) -> TermDelta {
    TermDelta {
        term,
        delta_mg: white_pov_tapered,
        delta_eg: white_pov_tapered,
        delta_tapered: white_pov_tapered,
        piece_involved: None,
    }
}

/// Build a minimal `MoveAnalysis` suitable for assess_teaching
/// tests. PV is just the single user move so `compute_material_*`
/// paths (when called) see no captures.
fn make_analysis(mv: Move, score_cp: i32, term_deltas: Vec<TermDelta>) -> MoveAnalysis {
    MoveAnalysis {
        mv,
        score: Value(score_cp),
        depth: 8,
        pv: vec![mv],
        ply_traces: vec![EvalTrace::zero()],
        settled_ply: Some(0),
        pre_move_trace: EvalTrace::zero(),
        pre_score: Value::ZERO,
        term_deltas,
    }
}

fn quiet_move() -> Move {
    // a2-a3 — legal from startpos, never a capture.
    Move::normal(Square::A2, Square::A3)
}

fn other_quiet_move() -> Move {
    // h2-h3 — legal from startpos, distinct from a2-a3.
    Move::normal(Square::H2, Square::H3)
}

// ---- assess_teaching: noise floor + dominance gate -------------

#[test]
fn teaching_fires_on_single_term_dominance() {
    // User move drops 80 cp on the user-side; one TermId carries
    // 70/80 = 87.5%. White-to-move so root_stm is White; negative
    // white-POV tapered deltas are user-side drops.
    let best = make_analysis(other_quiet_move(), 60, vec![]);
    let user = make_analysis(
        quiet_move(),
        -20,
        vec![
            make_delta(TermId::KingDanger, -70),
            make_delta(TermId::KingPawnShield, -10),
        ],
    );
    let info = assess_teaching(&best, &user, Color::White, &GatingConfig::default())
        .expect("dominant king-safety drop should fire");
    assert_eq!(info.dominant.term, TermId::KingDanger);
    assert_eq!(info.dominant.severity_cp, 70);
    assert!((info.dominant.share_of_drop - 70.0 / 80.0).abs() < 1e-6);
    // 70 cp is above the absolute-severity escape (50 cp), so this
    // would have fired via that path even without a 60% share.
    // The dominance-share path takes precedence when both pass.
    // Either way, single-signal → no secondary.
    assert!(info.secondary.is_none());
}

#[test]
fn teaching_skipped_when_drop_spread_within_a_family() {
    // 40 cp total drop spread 15/13/12 across three piece-placement
    // sub-terms. Per-family gating would have fired ("40 cp of
    // piece placement!"); per-term gating doesn't, because no
    // single TermId carries 60% AND none crosses the absolute-
    // severity escape (50 cp). The Nc3-in-Four-Knights case.
    let best = make_analysis(other_quiet_move(), 60, vec![]);
    let user = make_analysis(
        quiet_move(),
        20,
        vec![
            make_delta(TermId::PiecesKingProtector, -15),
            make_delta(TermId::PiecesBishopPawns, -13),
            make_delta(TermId::PiecesMinorBehindPawn, -12),
        ],
    );
    assert_eq!(
        assess_teaching(&best, &user, Color::White, &GatingConfig::default()),
        None
    );
}

#[test]
fn teaching_fires_via_absolute_escape_when_no_single_term_dominates() {
    // 100 cp total drop split 55/30/15. Top term doesn't hit the
    // 60% share gate (55/100), but it does clear the 50 cp
    // absolute-severity escape. Fire on the single dominant term.
    let best = make_analysis(other_quiet_move(), 80, vec![]);
    let user = make_analysis(
        quiet_move(),
        -20,
        vec![
            make_delta(TermId::KingDanger, -55),
            make_delta(TermId::ThreatsHanging, -30),
            make_delta(TermId::PiecesBishopPawns, -15),
        ],
    );
    let info = assess_teaching(&best, &user, Color::White, &GatingConfig::default())
        .expect("absolute-severity escape should fire on 55 cp signal");
    assert_eq!(info.dominant.term, TermId::KingDanger);
    assert_eq!(info.dominant.severity_cp, 55);
    // 55+30 = 85 ≥ 75 — multi-term wins over the escape path.
    // This codifies the priority order.
    assert_eq!(
        info.secondary.map(|s| s.term),
        Some(TermId::ThreatsHanging)
    );
}

#[test]
fn teaching_fires_multi_term_on_two_real_signals() {
    // 100 cp total drop split 40/40/20. Neither term dominates
    // (each is 40% of the drop), but both individually clear the
    // 25 cp severity floor and together cover 80% — two real,
    // teachable signals. Surface both.
    let best = make_analysis(other_quiet_move(), 80, vec![]);
    let user = make_analysis(
        quiet_move(),
        -20,
        vec![
            make_delta(TermId::PiecesRookOnOpenFile, -40),
            make_delta(TermId::KingPawnShield, -40),
            make_delta(TermId::MobilityBishop, -20),
        ],
    );
    let info = assess_teaching(&best, &user, Color::White, &GatingConfig::default())
        .expect("multi-term gate should fire on 40/40 case");
    assert_eq!(info.dominant.term, TermId::PiecesRookOnOpenFile);
    assert_eq!(info.dominant.severity_cp, 40);
    let secondary = info.secondary.expect("secondary present");
    assert_eq!(secondary.term, TermId::KingPawnShield);
    assert_eq!(secondary.severity_cp, 40);
}

#[test]
fn teaching_skipped_when_drop_distributed_across_families() {
    // 80 cp total drop split 30/25/25 across three terms in
    // different families. No single term hits the 60% share gate
    // (30/80 = 37.5%), none crosses the 50 cp escape, and the
    // top-two coverage is only 55/80 = 69% — below the 75%
    // multi-term threshold. Genuine noise — skip.
    let best = make_analysis(other_quiet_move(), 60, vec![]);
    let user = make_analysis(
        quiet_move(),
        -20,
        vec![
            make_delta(TermId::KingDanger, -30),
            make_delta(TermId::PiecesOutposts, -25),
            make_delta(TermId::MobilityKnight, -25),
        ],
    );
    assert_eq!(
        assess_teaching(&best, &user, Color::White, &GatingConfig::default()),
        None
    );
}

#[test]
fn teaching_skipped_when_multi_term_secondary_below_severity_floor() {
    // 100 cp drop split 75/15/5/5. The top hits the absolute
    // escape and would fire single-term. The second is below the
    // 25 cp severity floor, so the multi-term branch doesn't
    // surface it. Result: single-term intervention.
    let best = make_analysis(other_quiet_move(), 80, vec![]);
    let user = make_analysis(
        quiet_move(),
        -20,
        vec![
            make_delta(TermId::KingDanger, -75),
            make_delta(TermId::MobilityKnight, -15),
            make_delta(TermId::ThreatsHanging, -5),
            make_delta(TermId::PiecesBishopPawns, -5),
        ],
    );
    let info = assess_teaching(&best, &user, Color::White, &GatingConfig::default())
        .expect("fires");
    assert_eq!(info.dominant.term, TermId::KingDanger);
    assert!(info.secondary.is_none(), "secondary too small to surface");
}

#[test]
fn teaching_skipped_when_drop_below_noise_floor() {
    // 20 cp drop, entirely king-safety — but below the default
    // 30 cp noise floor. No prompt.
    let best = make_analysis(other_quiet_move(), 30, vec![]);
    let user = make_analysis(
        quiet_move(),
        10,
        vec![make_delta(TermId::KingDanger, -20)],
    );
    assert_eq!(
        assess_teaching(&best, &user, Color::White, &GatingConfig::default()),
        None
    );
}

#[test]
fn teaching_skipped_when_dominant_term_severity_below_min() {
    // 35 cp total drop, all in one term (100% share!) but the
    // term-severity gate (25 cp default) still passes because
    // 35 ≥ 25. Tighten the threshold to verify the gate works.
    let best = make_analysis(other_quiet_move(), 30, vec![]);
    let user = make_analysis(
        quiet_move(),
        -10,
        vec![make_delta(TermId::KingDanger, -35)],
    );
    let strict = GatingConfig {
        teaching_term_severity_min_cp: 50,
        ..GatingConfig::default()
    };
    assert_eq!(assess_teaching(&best, &user, Color::White, &strict), None);
}

#[test]
fn teaching_skipped_when_position_already_hopeless() {
    // best.score is -600 — past the -500 default hopeless cap.
    // Even a real teaching dimension shouldn't fire mid-loss.
    let best = make_analysis(other_quiet_move(), -600, vec![]);
    let user = make_analysis(
        quiet_move(),
        -700,
        vec![make_delta(TermId::KingDanger, -100)],
    );
    assert_eq!(
        assess_teaching(&best, &user, Color::White, &GatingConfig::default()),
        None
    );
}

#[test]
fn teaching_skipped_when_user_is_best_move() {
    // user.score == best.score → drop is zero → noise floor.
    let best = make_analysis(quiet_move(), 60, vec![]);
    let user = make_analysis(
        quiet_move(),
        60,
        vec![make_delta(TermId::KingDanger, -100)], // shouldn't matter
    );
    assert_eq!(
        assess_teaching(&best, &user, Color::White, &GatingConfig::default()),
        None
    );
}

#[test]
fn teaching_skipped_when_dominant_term_is_material_piece_value() {
    // Material piece-value drops are handled by the blunder gate.
    // A pure piece-value drop here would otherwise pass the
    // share+severity gates, but we explicitly exclude it so we
    // don't double-narrate ("teaching: material" alongside
    // "blunder: lost N cp").
    let best = make_analysis(other_quiet_move(), 60, vec![]);
    let user = make_analysis(
        quiet_move(),
        -40,
        vec![make_delta(TermId::MaterialPieceValue, -100)],
    );
    assert_eq!(
        assess_teaching(&best, &user, Color::White, &GatingConfig::default()),
        None
    );
}

#[test]
fn teaching_picks_largest_negative_term() {
    // Two negative deltas; the prompt's dominant.term should be
    // whichever single TermId carried more. (Both are in the same
    // family here, which is fine — the gate is per-term, but the
    // chosen term is just whichever has the largest magnitude.)
    let best = make_analysis(other_quiet_move(), 60, vec![]);
    let user = make_analysis(
        quiet_move(),
        -20,
        vec![
            make_delta(TermId::KingDanger, -30),
            make_delta(TermId::KingPawnShield, -50),
        ],
    );
    let info = assess_teaching(&best, &user, Color::White, &GatingConfig::default())
        .expect("fires");
    assert_eq!(info.dominant.term, TermId::KingPawnShield);
}

#[test]
fn teaching_root_stm_black_flips_sign() {
    // root_stm is Black, so a *positive* white-POV delta is a
    // user-side drop. Same scenario as the dominance test but with
    // signs flipped.
    let best = make_analysis(other_quiet_move(), 60, vec![]);
    let user = make_analysis(
        quiet_move(),
        -20,
        vec![
            make_delta(TermId::KingDanger, 70),
            make_delta(TermId::KingPawnShield, 10),
        ],
    );
    let info = assess_teaching(&best, &user, Color::Black, &GatingConfig::default())
        .expect("black-side drop should fire");
    assert_eq!(info.dominant.term, TermId::KingDanger);
    assert_eq!(info.dominant.severity_cp, 70);
}

// ---- term_family mapping coverage ------------------------------

#[test]
fn term_family_every_term_id_has_a_mapping() {
    // Exhaustive sweep: every TermId returns a family without
    // panicking. Catches future TermId additions that forget to
    // extend `TermFamily::of`.
    for &t in &TermId::ALL {
        let _ = TermFamily::of(t);
    }
}

#[test]
fn term_family_groups_king_subterms_together() {
    assert_eq!(TermFamily::of(TermId::KingDanger), TermFamily::KingSafety);
    assert_eq!(
        TermFamily::of(TermId::KingPawnShield),
        TermFamily::KingSafety
    );
    assert_eq!(
        TermFamily::of(TermId::KingFlankAttacks),
        TermFamily::KingSafety
    );
}

// ---- classify_user_move: end-to-end on a real position ---------

#[test]
fn classify_returns_fine_when_user_move_not_in_analyses() {
    let pre = Position::startpos();
    let analyses: Vec<MoveAnalysis> = Vec::new();
    let assessment = classify_user_move(
        &pre,
        &analyses,
        quiet_move(),
        &GatingConfig::default(),
        None,
    );
    assert!(assessment.is_fine());
}

/// Real position where Black is to move and can hang the queen
/// to a knight pickup. We run a small search, force the hanging
/// move into the analyses, and confirm the classifier flags it
/// as a blunder.
#[test]
fn classify_flags_hung_queen_as_blunder() {
    use crate::engine::{Engine, SearchParams};

    // White: K e1, N f3. Black: K e8, Q d8. Black plays Qd4 and
    // White's Nxd4 wins the queen — a 900+ cp realized loss.
    let mut pre = Position::from_fen(
        "3qk3/8/8/8/8/5N2/8/4K3 b - - 0 1",
    )
    .expect("valid FEN");
    let hang = Move::normal(Square::D8, Square::D4);

    let mut engine = Engine::default();
    let analyses = super::super::analyze_position(
        &mut engine,
        &mut pre,
        SearchParams {
            max_depth: 4,
            multi_pv: 4,
            force_include: vec![hang],
            ..SearchParams::default()
        },
    );
    let pre = Position::from_fen("3qk3/8/8/8/8/5N2/8/4K3 b - - 0 1").unwrap();
    let assessment = classify_user_move(&pre, &analyses, hang, &GatingConfig::default(), None);
    let blunder = assessment.blunder.expect("Qd4 should trip blunder gate");
    // Queen midgame value is well above 300 cp.
    assert!(
        blunder.material_loss_cp >= 700,
        "expected ≥ 700 cp loss, got {}",
        blunder.material_loss_cp
    );
    // The hanging piece lands on d4 after Nxd4.
    assert_eq!(blunder.lost_piece_square, Some(Square::D4));
}

// ---- ALLOWED-not-MISSED pause (PLAN §3) ------------------------

/// Run a full multi-PV analysis on `fen`, force-including `user`, and
/// classify it. Shared by the case-study pause tests below — they need
/// real search scores (the ALLOWED gate keys on the cp swing) and a
/// real PV (the detector chain walks it).
///
/// `depth = 12`, `multi_pv = 3`, single-thread — matches the production
/// retrospective worker (`RETROSPECTIVE_MULTI_PV = 3`, single-threaded
/// for determinism). The classifier runs against exactly this analysis
/// in the GUI, so the tests must use the same configuration: the
/// MultiPV setting materially moves the scores the ALLOWED giveaway gate
/// keys on (memory `project_multipv_mate_pathology`), so a depth/MultiPV
/// that doesn't match production would test a configuration that never
/// runs.
fn classify_on_fen(fen: &str, user: Move, prior: Option<PriorMove>) -> MoveAssessment {
    use crate::engine::{Engine, SearchParams};
    let mut pre = Position::from_fen(fen).expect("valid FEN");
    let mut engine = Engine::default();
    let analyses = super::super::analyze_position(
        &mut engine,
        &mut pre,
        SearchParams {
            max_depth: 12,
            multi_pv: 3,
            force_include: vec![user],
            threads: 1,
            ..SearchParams::default()
        },
    );
    let pre = Position::from_fen(fen).unwrap();
    classify_user_move(&pre, &analyses, user, &GatingConfig::default(), prior)
}

#[test]
fn allowed_pause_fires_on_discovered_attack_qc5() {
    use super::super::TacticPattern;
    // discovered-attack-after-qxe6: White (the user) plays the natural
    // Qc5+ (c4c5) instead of the defusing Qxe6+. Qc5+ leaves Black's
    // qe6/be5/Re1 e-file alignment loaded; the eval collapses from
    // winning to losing for White. The pause must use the ALLOWED
    // framing (a detector explains the swing) rather than a bare
    // missed-move teaching prompt.
    //
    // The *named* pattern is the alignment-collapse the search realizes
    // on Black's best reply line. At the production config that line
    // cashes the e-file via a Clearance (…Bxe5 then …Rxc2), so the slot
    // reports Clearance rather than the bare standing DiscoveredAttack
    // the static `danger:` scan names — both describe the same loaded
    // e-file, and either is a legitimate "what you allowed." We assert
    // ALLOWED fires with a real material pattern and a positive
    // concession, not a specific pattern id (search-config-dependent).
    const FEN: &str = "1r4nr/p3k3/4qpp1/4b2p/2Q5/8/PPPP1PPP/R1B1R1K1 w - - 0 1";
    let qc5 = Move::normal(Square::C4, Square::C5);
    let assessment = classify_on_fen(FEN, qc5, None);
    let allowed = assessment
        .allowed
        .expect("Qc5+ gave away the win to a standing e-file tactic — ALLOWED must fire");
    assert!(
        matches!(
            allowed.walked_into.pattern,
            TacticPattern::DiscoveredAttack
                | TacticPattern::Clearance
                | TacticPattern::RemovingDefender
                | TacticPattern::Skewer
                | TacticPattern::Fork
                | TacticPattern::HangingCapture
        ),
        "expected a material-winning pattern naming what Qc5+ allowed, got {:?}",
        allowed.walked_into.pattern,
    );
    assert!(
        allowed.conceded_cp > 0,
        "conceded a positive amount, got {}",
        allowed.conceded_cp
    );
}

#[test]
fn allowed_pause_fires_on_positional_punish_oo() {
    use super::super::TacticPattern;
    // positional-punish-after-qe6: White (the user) plays O-O (e1g1)
    // instead of Ne3, allowing the standing RemovingDefender against Nf5
    // (…Nxe4 removes the e4-pawn that guards it). At the production
    // retrospective config (depth 12, MultiPV 3) the best line is ~+1.8
    // and O-O drops to ~+0.2 — conceded ~1.6 pawns, no longer winning —
    // so the shared `gave_away_advantage` predicate fires and a detector
    // (the surviving alignment) explains it: ALLOWED, not MISSED.
    //
    // PLAN §5's matrix lists this row as "pause on O-O (allowed …Nxe4)".
    // (NOTE: a single-PV `critique` reports O-O at +0.99 — above the
    // giveaway floor — because MultiPV shifts the scores; the GUI runs
    // MultiPV 3, which is what this test reproduces.)
    const FEN: &str = "r1b1kb1r/1p3ppp/p2pqn2/4pNB1/4P3/2N5/PPP2PPP/R2QK2R w KQkq - 0 1";
    let castle = Move::castling(Square::E1, Square::G1);
    let assessment = classify_on_fen(FEN, castle, None);
    let allowed = assessment
        .allowed
        .expect("O-O concedes the bind and allows the standing tactic — ALLOWED must fire");
    assert_eq!(
        allowed.walked_into.pattern,
        TacticPattern::RemovingDefender,
        "the reframe must name the remove-the-defender motif (…Nxe4 unguards Nf5)",
    );
    assert!(allowed.conceded_cp > 0);
}

#[test]
fn no_pause_on_silent_sequencing_qc8() {
    // silent-sequencing-after-qc8: Black (the user) plays …Qc8. The
    // engine hates it, but the verdict only emerges past human
    // calculation depth and NO named detector fires. The suppressor must
    // keep the pause silent — neither teaching nor allowed survives.
    const FEN: &str = "1r1q2nr/p3k3/2Bbbpp1/7p/2Q5/8/PPPP1PPP/R1B1R1K1 b - - 0 1";
    let qc8 = Move::normal(Square::D8, Square::C8);
    let assessment = classify_on_fen(FEN, qc8, None);
    assert!(
        assessment.allowed.is_none(),
        "no detector explains …Qc8 — ALLOWED must not fire",
    );
    assert!(
        assessment.teaching.is_none(),
        "the silent-sequencing suppressor must clear the teaching pause for …Qc8",
    );
}
