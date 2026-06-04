use super::*;
use crate::claim::{
    material_claim, CastleSide, CenterShift, Claim, InitiativeTemplate, PlacementCategory,
    PlacementSide, SpaceDirection, SpaceSide, TacticRole, ThreatKind, ThreatSide, ThreatTarget,
};
use chess_tutor_engine::analysis::{CaptureEvent, MoveVerdict, PieceLocation, PressureKind, TermId};
use chess_tutor_engine::types::{Color, PieceType, Square, Value};

fn ctx(perspective: Perspective) -> PhrasingContext {
    PhrasingContext {
        perspective,
        locale: Locale::En,
        verbosity: Verbosity::Normal,
        reveal_moves: false,
    }
}

/// Build a `Claim::Verdict` with the given verdict and the two chess.com
/// tier inputs; the scores are placeholders unless the test cares.
fn verdict(verdict: MoveVerdict, only_good_move: bool, sacrifice: bool) -> Claim {
    Claim::Verdict {
        verdict,
        mover: Color::White,
        san: "Nf3".to_string(),
        score: Value(15),
        best_score: Value(15),
        gap: Value(0),
        only_good_move,
        sacrifice,
        best_san: None,
    }
}

// ---- verdict_tier_label: the chess.com tier remap --------------------

#[test]
fn tier_label_best_plain_is_best() {
    // Best but not the only good move → plain "Best", never a tier.
    assert_eq!(verdict_tier_label(MoveVerdict::Best, false, false), "Best");
    assert_eq!(verdict_tier_label(MoveVerdict::Best, false, true), "Best");
}

#[test]
fn tier_label_best_only_good_move_is_great() {
    assert_eq!(verdict_tier_label(MoveVerdict::Best, true, false), "Great");
}

#[test]
fn tier_label_best_only_good_move_sacrifice_is_brilliant() {
    assert_eq!(verdict_tier_label(MoveVerdict::Best, true, true), "Brilliant");
}

#[test]
fn tier_label_non_best_never_tiered() {
    // A piece-hang at +25 classifies as Blunder, never Best — so even if
    // some upstream flag leaked, the label stays "Blunder", never
    // "Great"/"Brilliant". Only `Best` is eligible for the tier remap.
    for v in [
        MoveVerdict::Good,
        MoveVerdict::Inaccuracy,
        MoveVerdict::Mistake,
        MoveVerdict::Blunder,
        MoveVerdict::Miss,
        MoveVerdict::BestAvailable,
    ] {
        let label = verdict_tier_label(v, true, true);
        assert_ne!(label, "Great");
        assert_ne!(label, "Brilliant");
    }
    assert_eq!(verdict_tier_label(MoveVerdict::Blunder, true, true), "Blunder");
}

// ---- phrase: both perspectives ---------------------------------------

#[test]
fn phrase_best_player_vs_opponent() {
    let c = verdict(MoveVerdict::Best, false, false);
    let player = phrase(&c, &ctx(Perspective::Player));
    let opp = phrase(&c, &ctx(Perspective::Opponent));
    assert!(player.summary.starts_with("You played Nf3 — Best"));
    assert!(opp.summary.starts_with("They played Nf3 — Best"));
    // A Best move carries no "your chance" reframe.
    assert!(!opp.summary.contains("Your chance"));
}

#[test]
fn phrase_brilliant_renders_tier_both_perspectives() {
    let c = verdict(MoveVerdict::Best, true, true);
    let player = phrase(&c, &ctx(Perspective::Player));
    let opp = phrase(&c, &ctx(Perspective::Opponent));
    assert!(player.summary.contains("Brilliant"), "{}", player.summary);
    assert!(opp.summary.contains("Brilliant"), "{}", opp.summary);
}

#[test]
fn phrase_blunder_reframes_for_opponent() {
    let c = Claim::Verdict {
        verdict: MoveVerdict::Blunder,
        mover: Color::White,
        san: "Qxf7".to_string(),
        score: Value(-820),
        best_score: Value(15),
        gap: Value(-835),
        only_good_move: false,
        sacrifice: false,
        best_san: None,
    };
    let player = phrase(&c, &ctx(Perspective::Player));
    let opp = phrase(&c, &ctx(Perspective::Opponent));

    // Player side: "you", no reframe; SAN carries the ?? glyph.
    assert!(player.summary.starts_with("You played Qxf7?? — Blunder"));
    assert!(!player.summary.contains("Your chance"));

    // Opponent side: "they" + the directional reframe to your benefit.
    assert!(opp.summary.starts_with("They played Qxf7?? — Blunder"));
    assert!(opp.summary.contains("Your chance"));
}

#[test]
fn phrase_best_available_carries_lost_position_note() {
    let c = Claim::Verdict {
        verdict: MoveVerdict::BestAvailable,
        mover: Color::Black,
        san: "Kh8".to_string(),
        score: Value(-1200),
        best_score: Value(-1200),
        gap: Value(0),
        only_good_move: false,
        sacrifice: false,
        best_san: None,
    };
    let p = phrase(&c, &ctx(Perspective::Player));
    assert!(p.summary.contains("Best available"));
    let detail = p.detail.expect("BestAvailable carries a note");
    assert!(detail.contains("Position was already lost"));
}

#[test]
fn phrase_miss_note_flips_with_perspective() {
    let c = Claim::Verdict {
        verdict: MoveVerdict::Miss,
        mover: Color::White,
        san: "Ra7".to_string(),
        score: Value(0),
        best_score: Value(1200),
        gap: Value(-1200),
        only_good_move: false,
        sacrifice: false,
        best_san: None,
    };
    let player = phrase(&c, &ctx(Perspective::Player)).detail.unwrap();
    let opp = phrase(&c, &ctx(Perspective::Opponent)).detail.unwrap();
    assert!(player.contains("this one let it slip"));
    assert!(opp.contains("they let it slip"));
}

#[test]
fn phrase_reveal_moves_appends_engine_preferred() {
    let c = Claim::Verdict {
        verdict: MoveVerdict::Inaccuracy,
        mover: Color::White,
        san: "h3".to_string(),
        score: Value(-40),
        best_score: Value(40),
        gap: Value(-80),
        only_good_move: false,
        sacrifice: false,
        best_san: Some("Nf3".to_string()),
    };
    let revealed = PhrasingContext {
        perspective: Perspective::Player,
        locale: Locale::En,
        verbosity: Verbosity::Normal,
        reveal_moves: true,
    };
    let out = phrase(&c, &revealed);
    let detail = out.detail.expect("reveal carries the engine-preferred line");
    assert!(detail.contains("Engine preferred Nf3"));
    // With reveal off, no engine-preferred line on an Inaccuracy.
    assert!(phrase(&c, &ctx(Perspective::Player)).detail.is_none());
}

#[test]
fn tactic_role_is_reachable_from_phrasing_module() {
    // Smoke: the IR types the translator will consume are in scope.
    let _ = TacticRole::WalkedInto;
}

// ---- phrase: Material, both perspectives -----------------------------

/// One capture event. `mg`/`eg` are the captured piece's cp values; the
/// exact magnitudes only matter for the even-trade cp-lean detail.
fn capture(captor: Color, captured: PieceType, mg: i32, eg: i32) -> CaptureEvent {
    CaptureEvent {
        ply: 0,
        captor,
        captor_piece: PieceType::Pawn,
        captured_piece: captured,
        square: Square::E4,
        value_mg: mg,
        value_eg: eg,
    }
}

#[test]
fn material_mover_gain_stays_theirs_for_opponent() {
    // White (the mover) wins a clean bishop, nothing given back.
    let events = [capture(Color::White, PieceType::Bishop, 825, 915)];
    let c = material_claim(&events, Color::White);

    let player = phrase(&c, &ctx(Perspective::Player));
    assert_eq!(player.summary, "You won a bishop");

    let opp = phrase(&c, &ctx(Perspective::Opponent));
    // A mover's *gain* is never reframed to the player's benefit — it
    // stays "they won …", no "you win material".
    assert_eq!(opp.summary, "They won a bishop");
    assert!(!opp.summary.contains("you win material"));
}

#[test]
fn material_mover_loss_reframes_to_player_benefit() {
    // White (the mover) loses a clean bishop — net is mover-relative
    // negative.
    let events = [capture(Color::Black, PieceType::Bishop, 825, 915)];
    let c = material_claim(&events, Color::White);

    // Player side: a plain loss, no reframe.
    let player = phrase(&c, &ctx(Perspective::Player));
    assert_eq!(player.summary, "You lost a bishop");

    // Opponent side: the mover's loss is *your* gain — the directional
    // reframe lives only here.
    let opp = phrase(&c, &ctx(Perspective::Opponent));
    assert_eq!(opp.summary, "They lost a bishop — you win material");
}

#[test]
fn material_points_ledger_when_both_sides_capture() {
    // White wins a rook (5) and gives back a bishop (3): net +2, both
    // piles non-empty ⇒ the points ledger, not "a piece".
    let events = [
        capture(Color::White, PieceType::Rook, 1276, 1380),
        capture(Color::Black, PieceType::Bishop, 825, 915),
    ];
    let c = material_claim(&events, Color::White);
    let player = phrase(&c, &ctx(Perspective::Player));
    assert_eq!(player.summary, "You won 2 points (rook for bishop)");
}

#[test]
fn material_pawn_swing_reads_a_pawn() {
    let events = [capture(Color::White, PieceType::Pawn, 128, 213)];
    let c = material_claim(&events, Color::White);
    assert_eq!(
        phrase(&c, &ctx(Perspective::Player)).summary,
        "You won a pawn"
    );
}

#[test]
fn material_even_point_trade_with_cp_lean_carries_detail() {
    // B-for-N: 3 = 3 by points, but a cp lean the engine still reads.
    let events = [
        capture(Color::White, PieceType::Knight, 781, 854),
        capture(Color::Black, PieceType::Bishop, 825, 915),
    ];
    let c = material_claim(&events, Color::White);
    let p = phrase(&c, &ctx(Perspective::Player));
    assert_eq!(p.summary, "Even trade");
    let detail = p.detail.expect("even-by-points cp lean carries a note");
    assert!(detail.contains("even by point value"), "{detail}");
}

#[test]
fn material_empty_events_is_even_trade_no_detail() {
    let c = material_claim(&[], Color::White);
    let p = phrase(&c, &ctx(Perspective::Player));
    assert_eq!(p.summary, "Even trade");
    assert!(p.detail.is_none());
}

// ---- phrase: Tactic, all three roles, both perspectives --------------

use crate::claim::{AllowedReframe, TacticEscapeInfo};
use chess_tutor_engine::analysis::{
    Confidence, EscapeKind, MatePattern, TacticHit, TacticPattern,
};

fn fork_hit() -> TacticHit {
    TacticHit {
        pattern: TacticPattern::Fork,
        pv_ply: 0,
        primary_piece: Square::F7,
        targets: vec![Square::E5, Square::D8],
        material_gain: Some(300),
        confidence: Confidence::High,
        sacrifice: false,
        mate_pattern: None,
        key_move: None,
    }
}

fn tactic(role: TacticRole, hit: TacticHit) -> Claim {
    Claim::Tactic {
        mover: Color::White,
        role,
        hit,
        escape: None,
        allowed: None,
    }
}

#[test]
fn tactic_played_player_vs_opponent() {
    let c = tactic(TacticRole::Played, fork_hit());
    // Player: the mover (you) executed the tactic.
    assert_eq!(
        phrase(&c, &ctx(Perspective::Player)).summary,
        "You played a fork"
    );
    // Opponent: the mover (they) executed it *against you* — never reframed
    // to your benefit (a played tactic is the mover's gain).
    assert_eq!(
        phrase(&c, &ctx(Perspective::Opponent)).summary,
        "They forked you"
    );
}

#[test]
fn tactic_played_opponent_falls_back_to_generic_frame() {
    // A pattern with no clean "<verb> you" form keeps "They played …".
    let mut hit = fork_hit();
    hit.pattern = TacticPattern::HangingCapture;
    let c = tactic(TacticRole::Played, hit);
    assert_eq!(
        phrase(&c, &ctx(Perspective::Opponent)).summary,
        "They played a free piece"
    );
}

#[test]
fn tactic_missed_both_directions() {
    let c = tactic(TacticRole::Missed, fork_hit());
    assert_eq!(
        phrase(&c, &ctx(Perspective::Player)).summary,
        "You missed a fork"
    );
    assert_eq!(
        phrase(&c, &ctx(Perspective::Opponent)).summary,
        "They missed a fork"
    );
}

#[test]
fn tactic_walked_into_reframes_for_opponent() {
    let c = tactic(TacticRole::WalkedInto, fork_hit());
    // Player: a plain warning — you walked into it.
    assert_eq!(
        phrase(&c, &ctx(Perspective::Player)).summary,
        "You walked into a fork"
    );
    // Opponent: the mover (they) walked into it — *your* opportunity. This
    // is the load-bearing "you get a chance" reframe.
    assert_eq!(
        phrase(&c, &ctx(Perspective::Opponent)).summary,
        "You get a chance — they walked into a fork"
    );
}

#[test]
fn tactic_carries_per_pattern_lesson_in_detail() {
    let c = tactic(TacticRole::Played, fork_hit());
    let detail = phrase(&c, &ctx(Perspective::Player)).detail.unwrap();
    assert!(detail.contains("fork is one piece"), "{detail}");
}

#[test]
fn tactic_mate_suffix_only_for_default_surfaced() {
    let mut hit = fork_hit();
    hit.mate_pattern = Some(MatePattern::BackRank);
    let c = tactic(TacticRole::Played, hit.clone());
    assert!(phrase(&c, &ctx(Perspective::Player))
        .summary
        .ends_with("back-rank mate"));

    // Non-default-surfaced mate ⇒ no heading suffix.
    hit.mate_pattern = Some(MatePattern::Anastasia);
    let c = tactic(TacticRole::Played, hit);
    assert_eq!(
        phrase(&c, &ctx(Perspective::Player)).summary,
        "You played a fork"
    );
}

#[test]
fn tactic_played_escape_note_is_opponents_out() {
    let c = Claim::Tactic {
        mover: Color::White,
        role: TacticRole::Played,
        hit: fork_hit(),
        escape: Some(TacticEscapeInfo {
            san: "Qxe5".to_string(),
            kind: EscapeKind::Zwischenzug,
        }),
        allowed: None,
    };
    let detail = phrase(&c, &ctx(Perspective::Player)).detail.unwrap();
    assert!(detail.contains("wriggle out with Qxe5"), "{detail}");
    assert!(detail.contains("in-between capture"), "{detail}");
}

#[test]
fn tactic_walked_into_escape_note_is_your_out() {
    let c = Claim::Tactic {
        mover: Color::White,
        role: TacticRole::WalkedInto,
        hit: fork_hit(),
        escape: Some(TacticEscapeInfo {
            san: "Nf6".to_string(),
            kind: EscapeKind::AdequateRetreat,
        }),
        allowed: None,
    };
    let detail = phrase(&c, &ctx(Perspective::Player)).detail.unwrap();
    assert!(detail.contains("good news"), "{detail}");
    assert!(detail.contains("Nf6"), "{detail}");
}

#[test]
fn tactic_allowed_reframe_leads_swing_both_perspectives() {
    let reframe = AllowedReframe {
        best_pawns: 2.04,
        played_pawns: -1.27,
        swing_pawns: 3.3,
        continuation: "Qc5+ Kf7 b3 Ne7".to_string(),
    };
    let c = Claim::Tactic {
        mover: Color::White,
        role: TacticRole::WalkedInto,
        hit: fork_hit(),
        escape: None,
        allowed: Some(reframe),
    };

    // Player perspective: "You allowed …" heading + a swing in the
    // opponent's favour, then the continuation, then the lesson.
    let player = phrase(&c, &ctx(Perspective::Player));
    assert_eq!(player.summary, "You allowed a fork");
    let pd = player.detail.unwrap();
    assert!(pd.contains("3.3-pawn swing in the opponent's favour"), "{pd}");
    assert!(pd.contains("Qc5+ Kf7"), "{pd}");
    let swing_at = pd.find("3.3-pawn").unwrap();
    let lesson_at = pd.find("fork is one piece").unwrap();
    assert!(swing_at < lesson_at, "swing must lead the lesson");

    // Opponent perspective: the same swing is *your chance*.
    let opp = phrase(&c, &ctx(Perspective::Opponent));
    assert_eq!(opp.summary, "You get a chance — they walked into a fork");
    let od = opp.detail.unwrap();
    assert!(od.contains("swing your way"), "{od}");
    assert!(od.contains("your chance"), "{od}");
}

// ---- phrase: Threats, both perspectives ------------------------------

fn pl(square: Square, piece: PieceType) -> PieceLocation {
    PieceLocation { square, piece }
}

fn target(square: Square, piece: PieceType, attackers: Vec<PieceLocation>) -> ThreatTarget {
    ThreatTarget {
        location: pl(square, piece),
        attackers,
    }
}

fn threats(side: ThreatSide, kind: ThreatKind, pieces: Vec<ThreatTarget>) -> Claim {
    Claim::Threats { side, kind, pieces }
}

/// A mover-side hanging piece: a warning to the user when they moved,
/// the user's opportunity when the opponent moved (the directional flip
/// keys off *whose* piece is threatened, not the raw side label).
#[test]
fn threats_mover_hanging_flips_warning_to_opportunity() {
    let c = threats(
        ThreatSide::Mover,
        ThreatKind::Hanging,
        vec![target(
            Square::D2,
            PieceType::Knight,
            vec![pl(Square::E3, PieceType::Pawn)],
        )],
    );

    // The user moved → their own knight hangs → a warning.
    let player = phrase(&c, &ctx(Perspective::Player));
    assert_eq!(player.summary, "Your knight on d2 is hanging");
    let pd = player.detail.unwrap();
    assert!(pd.contains("Attacked and undefended"), "{pd}");
    assert!(pd.contains("attacked by the e3 pawn"), "{pd}");

    // The opponent moved → the *mover* is them → their knight hangs →
    // the user's opportunity.
    let opp = phrase(&c, &ctx(Perspective::Opponent));
    assert_eq!(opp.summary, "You can win material — their knight on d2 is hanging");
    assert!(opp.detail.unwrap().contains("the win is real"));
}

/// An opponent-side hanging piece (the engine's `theirs_*` list): the
/// user's opportunity when they moved, a warning to the user when the
/// opponent moved.
#[test]
fn threats_opponent_hanging_flips_with_perspective() {
    let c = threats(
        ThreatSide::Opponent,
        ThreatKind::Hanging,
        vec![target(
            Square::D7,
            PieceType::Bishop,
            vec![pl(Square::E6, PieceType::Pawn)],
        )],
    );

    // The user moved → the other side (opponent) hangs → opportunity.
    let player = phrase(&c, &ctx(Perspective::Player));
    assert_eq!(player.summary, "You can win material — their bishop on d7 is hanging");

    // The opponent moved → the other side (= the user) hangs → warning.
    let opp = phrase(&c, &ctx(Perspective::Opponent));
    assert_eq!(opp.summary, "Your bishop on d7 is hanging");
}

#[test]
fn threats_see_losing_both_perspectives() {
    let c = threats(
        ThreatSide::Mover,
        ThreatKind::SeeLosing,
        vec![target(
            Square::E5,
            PieceType::Knight,
            vec![pl(Square::D6, PieceType::Pawn), pl(Square::G4, PieceType::Knight)],
        )],
    );
    let player = phrase(&c, &ctx(Perspective::Player));
    assert_eq!(player.summary, "Your knight on e5 loses to a trade");
    assert!(player.detail.unwrap().contains("still loses material"));

    let opp = phrase(&c, &ctx(Perspective::Opponent));
    // Opponent moved → the mover's (their) piece loses the trade → the
    // user's opportunity.
    assert_eq!(opp.summary, "Their knight on e5 loses to a trade");
    assert!(opp.detail.unwrap().contains("wins material for you"));
}

#[test]
fn threats_multiple_pieces_uses_count_heading() {
    let c = threats(
        ThreatSide::Mover,
        ThreatKind::Hanging,
        vec![
            target(Square::D2, PieceType::Knight, vec![pl(Square::E3, PieceType::Pawn)]),
            target(Square::F5, PieceType::Bishop, vec![pl(Square::G6, PieceType::Pawn)]),
        ],
    );
    let player = phrase(&c, &ctx(Perspective::Player));
    assert_eq!(player.summary, "2 of your pieces are hanging");
    let pd = player.detail.unwrap();
    // Both pieces appear in the per-piece detail.
    assert!(pd.contains("Knight on d2"), "{pd}");
    assert!(pd.contains("Bishop on f5"), "{pd}");
}

#[test]
fn threats_pressured_picks_pattern_verb() {
    // Minor-on-major → "harried".
    let c = threats(
        ThreatSide::Mover,
        ThreatKind::Pressured(PressureKind::MinorOnMajor),
        vec![target(Square::A1, PieceType::Rook, vec![pl(Square::C2, PieceType::Knight)])],
    );
    let player = phrase(&c, &ctx(Perspective::Player));
    assert_eq!(player.summary, "Your rook on a1 is being harried");

    // Safe-pawn-threat → "kicked"; opponent-side from the player's POV
    // reads "Their …" (it's the opponent's piece under pressure).
    let c2 = threats(
        ThreatSide::Opponent,
        ThreatKind::Pressured(PressureKind::SafePawnThreat),
        vec![target(Square::F6, PieceType::Knight, vec![pl(Square::E5, PieceType::Pawn)])],
    );
    let player2 = phrase(&c2, &ctx(Perspective::Player));
    assert_eq!(player2.summary, "Their knight on f6 is being kicked");
}


// ---- King safety -----------------------------------------------------

use crate::claim::{CountShift, KingSide, PressureShift, SafetyDirection, ShelterShift};

fn king_safety(
    side: KingSide,
    direction: SafetyDirection,
    attackers: Option<CountShift>,
    shield: Option<ShelterShift>,
    king_sq: Square,
) -> Claim {
    Claim::KingSafety {
        side,
        direction,
        attackers,
        shield,
        pressure: None,
        king_sq,
    }
}

/// Build a pressure-only king-safety claim (count flat, no shield) for
/// the "more pressure" wording tests.
fn king_safety_pressure(
    side: KingSide,
    direction: SafetyDirection,
    pressure: PressureShift,
    king_sq: Square,
) -> Claim {
    Claim::KingSafety {
        side,
        direction,
        attackers: None,
        shield: None,
        pressure: Some(pressure),
        king_sq,
    }
}

/// Mover-side exposure: warning to the player, opportunity to the
/// opponent (the king is theirs, from the opponent's POV).
#[test]
fn king_safety_mover_exposed_both_perspectives() {
    let c = king_safety(
        KingSide::Mover,
        SafetyDirection::MoreExposed,
        Some(CountShift { pre: 1, post: 3 }),
        None,
        Square::E1,
    );
    // Player moved → it's the player's own king (a warning).
    let player = phrase(&c, &ctx(Perspective::Player));
    assert_eq!(
        player.summary,
        "Your king is more exposed: 3 attackers on the king ring (up from 1)."
    );
    // Opponent moved → the exposed king is the opponent's own; from the
    // player's side that's the player's chance.
    let opp = phrase(&c, &ctx(Perspective::Opponent));
    assert_eq!(
        opp.summary,
        "You expose the opponent's king: 3 attackers on the king ring (up from 1)."
    );
}

/// Opponent-side exposure flips the same way: from the player's POV
/// it's "you expose the opponent's king"; from the opponent's POV the
/// shifted king is the opponent's own.
#[test]
fn king_safety_opponent_exposed_both_perspectives() {
    let c = king_safety(
        KingSide::Opponent,
        SafetyDirection::MoreExposed,
        Some(CountShift { pre: 0, post: 2 }),
        None,
        Square::E8,
    );
    let player = phrase(&c, &ctx(Perspective::Player));
    assert!(player.summary.starts_with("You expose the opponent's king"), "{}", player.summary);
    let opp = phrase(&c, &ctx(Perspective::Opponent));
    assert!(opp.summary.starts_with("Your king is more exposed"), "{}", opp.summary);
}

/// The pawn-shield verb is perspective-correct: weakening your own
/// king's shield reads "weakened"; cracking the opponent's reads
/// "cracked".
#[test]
fn king_safety_shield_verb_flips_with_owner() {
    let c = king_safety(
        KingSide::Mover,
        SafetyDirection::MoreExposed,
        None,
        Some(ShelterShift { pre_mg: 80, post_mg: 30 }),
        Square::E1,
    );
    let player = phrase(&c, &ctx(Perspective::Player));
    assert!(player.summary.contains("pawn shield weakened (+0.80 → +0.30)"), "{}", player.summary);
    // Opponent moved → it's the opponent's shield from the player's POV.
    let opp = phrase(&c, &ctx(Perspective::Opponent));
    assert!(opp.summary.contains("pawn shield cracked (+0.80 → +0.30)"), "{}", opp.summary);
}

/// A safer shift reads "strengthened" regardless of owner; the lead
/// flips per perspective.
#[test]
fn king_safety_safer_both_perspectives() {
    let c = king_safety(
        KingSide::Mover,
        SafetyDirection::Safer,
        Some(CountShift { pre: 3, post: 1 }),
        None,
        Square::E1,
    );
    let player = phrase(&c, &ctx(Perspective::Player));
    assert_eq!(player.summary, "Your king is safer: attackers down to 1 (from 3).");
    let opp = phrase(&c, &ctx(Perspective::Opponent));
    assert_eq!(opp.summary, "The opponent's king is safer: attackers down to 1 (from 3).");
}

/// The flank label names the side when the king sits on an outside
/// file (kingside after castling).
#[test]
fn king_safety_names_flank_after_castling() {
    let c = king_safety(
        KingSide::Mover,
        SafetyDirection::MoreExposed,
        Some(CountShift { pre: 0, post: 2 }),
        None,
        Square::G1,
    );
    let player = phrase(&c, &ctx(Perspective::Player));
    assert!(player.summary.contains("2 attackers on the kingside (up from 0)"), "{}", player.summary);
}

/// The detail carries the perspective-neutral pre→post numbers.
#[test]
fn king_safety_detail_has_pre_post_numbers() {
    let c = king_safety(
        KingSide::Mover,
        SafetyDirection::MoreExposed,
        Some(CountShift { pre: 1, post: 3 }),
        Some(ShelterShift { pre_mg: 80, post_mg: 30 }),
        Square::E1,
    );
    let detail = phrase(&c, &ctx(Perspective::Player)).detail.expect("detail");
    assert!(detail.contains("Attackers on the king ring: 1 → 3."), "{detail}");
    assert!(detail.contains("Pawn shield: +0.80 → +0.30."), "{detail}");
}

/// A pressure-only shift (attacker count flat) gets the number-free
/// "under more pressure" heading, perspective-flipped, with the
/// adjacent-attack numbers tucked into the expandable detail.
#[test]
fn king_safety_pressure_only_is_number_free_heading() {
    let c = king_safety_pressure(
        KingSide::Opponent,
        SafetyDirection::MoreExposed,
        PressureShift { pre: 2, post: 4 },
        Square::E8,
    );
    // The opponent's king under more pressure → from the player's POV
    // that's the player applying it (the chess.com reframe).
    let player = phrase(&c, &ctx(Perspective::Player));
    assert_eq!(
        player.summary,
        "You pile more pressure on the opponent's king."
    );
    assert_eq!(
        player.detail.as_deref(),
        Some("Attacks next to the king: 2 → 4.")
    );
    // From the opponent's POV the pressured king is their own.
    let opp = phrase(&c, &ctx(Perspective::Opponent));
    assert_eq!(opp.summary, "Your king is under more pressure.");
}

/// A danger-driven pressure shift can fire with a flat adjacent-attack
/// count; the heading still appears but the "2 → 2" detail is suppressed.
#[test]
fn king_safety_pressure_flat_adjacent_count_has_no_detail() {
    let c = king_safety_pressure(
        KingSide::Mover,
        SafetyDirection::MoreExposed,
        PressureShift { pre: 2, post: 2 },
        Square::G1,
    );
    let player = phrase(&c, &ctx(Perspective::Player));
    assert_eq!(player.summary, "Your king is under more pressure.");
    assert_eq!(player.detail, None);
}

// ---- Mobility --------------------------------------------------------

use crate::claim::MobilitySide;

fn mobility(side: MobilitySide, piece: PieceType, pre_cp: i32, post_cp: i32) -> Claim {
    Claim::Mobility {
        side,
        piece,
        pre_cp,
        post_cp,
    }
}

/// Mover-side improvement: it's the player's own piece getting more
/// active; from the opponent's POV the same gain is the opponent's
/// piece improving (a warning to the player).
#[test]
fn mobility_mover_improved_both_perspectives() {
    let c = mobility(MobilitySide::Mover, PieceType::Knight, 20, 80);
    let player = phrase(&c, &ctx(Perspective::Player));
    assert_eq!(player.summary, "Your knight is more active");
    let opp = phrase(&c, &ctx(Perspective::Opponent));
    assert_eq!(opp.summary, "The opponent's knight is more active");
}

/// Mover-side drop: the player's own piece lost reach; from the
/// opponent's POV that's the opponent's piece restricted — the
/// player's gain (the reframe).
#[test]
fn mobility_mover_dropped_both_perspectives() {
    let c = mobility(MobilitySide::Mover, PieceType::Bishop, 80, 20);
    let player = phrase(&c, &ctx(Perspective::Player));
    assert_eq!(player.summary, "Your bishop is less active");
    let opp = phrase(&c, &ctx(Perspective::Opponent));
    assert_eq!(opp.summary, "You restrict the opponent's bishop");
}

/// Opponent-side improvement flips the same way: from the player's POV
/// it's the opponent's piece getting active; from the opponent's POV
/// it's their own piece improving.
#[test]
fn mobility_opponent_improved_both_perspectives() {
    let c = mobility(MobilitySide::Opponent, PieceType::Rook, 30, 90);
    let player = phrase(&c, &ctx(Perspective::Player));
    assert_eq!(player.summary, "The opponent's rook is more active");
    let opp = phrase(&c, &ctx(Perspective::Opponent));
    assert_eq!(opp.summary, "Your rook is more active");
}

/// Opponent-side drop: from the player's POV restricting the
/// opponent's piece is the player's gain; from the opponent's POV it's
/// their own piece losing reach.
#[test]
fn mobility_opponent_dropped_both_perspectives() {
    let c = mobility(MobilitySide::Opponent, PieceType::Queen, 90, 20);
    let player = phrase(&c, &ctx(Perspective::Player));
    assert_eq!(player.summary, "You restrict the opponent's queen");
    let opp = phrase(&c, &ctx(Perspective::Opponent));
    assert_eq!(opp.summary, "Your queen is less active");
}

/// The detail carries the perspective-neutral pre→post numbers.
#[test]
fn mobility_detail_has_pre_post_numbers() {
    let c = mobility(MobilitySide::Mover, PieceType::Knight, 80, 20);
    let detail = phrase(&c, &ctx(Perspective::Player)).detail.expect("detail");
    assert!(detail.contains("Activity +0.80 → +0.20"), "{detail}");
}

// ---- pawn structure phrasing (both perspectives) ---------------------

use crate::claim::{PawnCategory, PawnSide, StructureDirection};

/// Player POV: a worsened mover-side structure is "your" warning.
#[test]
fn pawn_structure_mover_worsened_player_is_your_warning() {
    let claim = Claim::PawnStructure {
        side: PawnSide::Mover,
        direction: StructureDirection::Worsened,
        categories: vec![PawnCategory::Doubled],
    };
    let p = phrase(&claim, &ctx(Perspective::Player));
    assert_eq!(p.summary, "Your pawn structure weakened: doubled a pawn.");
}

/// Opponent POV: a worsened mover-side structure (the opponent moved and
/// weakened their own pawns) reframes to *your* gain — never "you".
#[test]
fn pawn_structure_mover_worsened_opponent_reframes_to_your_gain() {
    let claim = Claim::PawnStructure {
        side: PawnSide::Mover,
        direction: StructureDirection::Worsened,
        categories: vec![PawnCategory::Doubled],
    };
    let p = phrase(&claim, &ctx(Perspective::Opponent));
    // The opponent weakened *their own* structure, which is your gain.
    assert_eq!(
        p.summary,
        "You weakened the opponent's pawn structure: doubled a pawn."
    );
}

/// Player POV: weakening the opponent's structure is "you weakened …".
#[test]
fn pawn_structure_opponent_side_player_is_opportunity() {
    let claim = Claim::PawnStructure {
        side: PawnSide::Opponent,
        direction: StructureDirection::Worsened,
        categories: vec![PawnCategory::Isolated, PawnCategory::Doubled],
    };
    let p = phrase(&claim, &ctx(Perspective::Player));
    assert_eq!(
        p.summary,
        "You weakened the opponent's pawn structure: isolated a pawn, doubled a pawn."
    );
}

/// Opponent POV: an opponent-side claim (the non-mover, i.e. the user's
/// own structure when the opponent moved) is the user's warning.
#[test]
fn pawn_structure_opponent_side_opponent_is_your_warning() {
    let claim = Claim::PawnStructure {
        side: PawnSide::Opponent,
        direction: StructureDirection::Worsened,
        categories: vec![PawnCategory::Doubled],
    };
    let p = phrase(&claim, &ctx(Perspective::Opponent));
    assert_eq!(p.summary, "Your pawn structure weakened: doubled a pawn.");
}

/// Improved direction uses the repair vocabulary, both perspectives.
#[test]
fn pawn_structure_improved_uses_repair_vocab() {
    let claim = Claim::PawnStructure {
        side: PawnSide::Mover,
        direction: StructureDirection::Improved,
        categories: vec![PawnCategory::Doubled],
    };
    let player = phrase(&claim, &ctx(Perspective::Player));
    assert_eq!(
        player.summary,
        "Your pawn structure improved: resolved a doubled pawn."
    );
    let opp = phrase(&claim, &ctx(Perspective::Opponent));
    assert_eq!(
        opp.summary,
        "The opponent's pawn structure improved: resolved a doubled pawn."
    );
}

/// A Claim never encodes "you": the player/opponent strings differ for
/// the same claim, proving perspective lives only in `phrase`.
#[test]
fn pawn_structure_claim_is_perspective_free() {
    let claim = Claim::PawnStructure {
        side: PawnSide::Mover,
        direction: StructureDirection::Worsened,
        categories: vec![PawnCategory::Backward],
    };
    let player = phrase(&claim, &ctx(Perspective::Player)).summary;
    let opp = phrase(&claim, &ctx(Perspective::Opponent)).summary;
    assert_ne!(player, opp);
}

// ---- passed pawns phrasing (both perspectives) -----------------------

/// Player POV: the user's own passers advancing is good news.
#[test]
fn passed_pawns_mover_improved_player_advances() {
    let claim = Claim::PassedPawns {
        side: PawnSide::Mover,
        direction: StructureDirection::Improved,
        delta_mg: 40,
    };
    let p = phrase(&claim, &ctx(Perspective::Player));
    assert_eq!(p.summary, "Your passed pawns advanced.");
}

/// Opponent POV: the opponent advancing their own passers is *your*
/// warning ("The opponent's passed pawns advanced").
#[test]
fn passed_pawns_mover_improved_opponent_is_warning() {
    let claim = Claim::PassedPawns {
        side: PawnSide::Mover,
        direction: StructureDirection::Improved,
        delta_mg: 40,
    };
    let p = phrase(&claim, &ctx(Perspective::Opponent));
    assert_eq!(p.summary, "The opponent's passed pawns advanced.");
}

/// Player POV: blunting the opponent's passers is your gain.
#[test]
fn passed_pawns_opponent_worsened_player_blunts() {
    let claim = Claim::PassedPawns {
        side: PawnSide::Opponent,
        direction: StructureDirection::Worsened,
        delta_mg: -40,
    };
    let p = phrase(&claim, &ctx(Perspective::Player));
    assert_eq!(p.summary, "You blunted the opponent's passed pawns.");
}

/// Opponent POV: an opponent-side worsened claim (the user's own passers
/// losing ground while the opponent moved) is the user's warning.
#[test]
fn passed_pawns_opponent_worsened_opponent_is_your_loss() {
    let claim = Claim::PassedPawns {
        side: PawnSide::Opponent,
        direction: StructureDirection::Worsened,
        delta_mg: -40,
    };
    let p = phrase(&claim, &ctx(Perspective::Opponent));
    assert_eq!(p.summary, "Your passed pawns lost ground.");
}

/// The detail line carries the signed cp shift and never says "you".
#[test]
fn passed_pawns_detail_carries_delta_and_is_neutral() {
    let claim = Claim::PassedPawns {
        side: PawnSide::Mover,
        direction: StructureDirection::Improved,
        delta_mg: 40,
    };
    let p = phrase(&claim, &ctx(Perspective::Player));
    let detail = p.detail.expect("a detail line");
    assert!(detail.contains("+0.40"), "detail: {detail}");
    assert!(!detail.contains("You") && !detail.contains("you"), "detail: {detail}");
}

// =====================================================================
// Piece placement
// =====================================================================

/// Player POV, mover side: improving your own knight's outpost is good.
#[test]
fn placement_mover_improved_player_is_yours() {
    let claim = Claim::PiecePlacement {
        side: PlacementSide::Mover,
        category: PlacementCategory::Outposts,
        direction: StructureDirection::Improved,
        delta_mg: 30,
    };
    let p = phrase(&claim, &ctx(Perspective::Player));
    assert_eq!(p.summary, "Your knight reached an outpost");
}

/// Opponent POV, mover side: the opponent improving their own knight's
/// outpost is *your* warning.
#[test]
fn placement_mover_improved_opponent_is_warning() {
    let claim = Claim::PiecePlacement {
        side: PlacementSide::Mover,
        category: PlacementCategory::Outposts,
        direction: StructureDirection::Improved,
        delta_mg: 30,
    };
    let p = phrase(&claim, &ctx(Perspective::Opponent));
    assert_eq!(p.summary, "Opponent's knight reached an outpost");
}

/// Player POV, opponent side worsened: denying the opponent's outpost is
/// your gain — the reframe.
#[test]
fn placement_opponent_worsened_player_is_opportunity() {
    let claim = Claim::PiecePlacement {
        side: PlacementSide::Opponent,
        category: PlacementCategory::Outposts,
        direction: StructureDirection::Worsened,
        delta_mg: -30,
    };
    let p = phrase(&claim, &ctx(Perspective::Player));
    assert_eq!(p.summary, "You denied the opponent's knight an outpost");
}

/// Opponent POV, opponent side worsened: the user's own knight (the
/// non-mover) lost its outpost — a warning.
#[test]
fn placement_opponent_worsened_opponent_is_your_warning() {
    let claim = Claim::PiecePlacement {
        side: PlacementSide::Opponent,
        category: PlacementCategory::Outposts,
        direction: StructureDirection::Worsened,
        delta_mg: -30,
    };
    let p = phrase(&claim, &ctx(Perspective::Opponent));
    assert_eq!(p.summary, "Your knight lost its outpost");
}

/// The detail carries the signed cp shift and the concept gloss.
#[test]
fn placement_detail_carries_shift_and_concept() {
    let claim = Claim::PiecePlacement {
        side: PlacementSide::Mover,
        category: PlacementCategory::RookOnOpenFile,
        direction: StructureDirection::Improved,
        delta_mg: 45,
    };
    let p = phrase(&claim, &ctx(Perspective::Player));
    let detail = p.detail.expect("a detail line");
    assert!(detail.contains("+0.45"), "detail: {detail}");
    assert!(detail.to_lowercase().contains("open file"), "detail: {detail}");
}

// =====================================================================
// Space
// =====================================================================

/// Player POV, mover side gained: you gained space.
#[test]
fn space_mover_gained_player_is_yours() {
    let claim = Claim::Space {
        side: SpaceSide::Mover,
        direction: SpaceDirection::Gained,
        delta_mg: 40,
    };
    let p = phrase(&claim, &ctx(Perspective::Player));
    assert_eq!(p.summary, "You gained space");
}

/// Opponent POV, mover side gained: the opponent gained space — your
/// warning.
#[test]
fn space_mover_gained_opponent_is_warning() {
    let claim = Claim::Space {
        side: SpaceSide::Mover,
        direction: SpaceDirection::Gained,
        delta_mg: 40,
    };
    let p = phrase(&claim, &ctx(Perspective::Opponent));
    assert_eq!(p.summary, "The opponent gained space");
}

/// Player POV, opponent side lost: you squeezed the opponent's space —
/// the reframe.
#[test]
fn space_opponent_lost_player_is_opportunity() {
    let claim = Claim::Space {
        side: SpaceSide::Opponent,
        direction: SpaceDirection::Lost,
        delta_mg: -40,
    };
    let p = phrase(&claim, &ctx(Perspective::Player));
    assert_eq!(p.summary, "You squeezed the opponent's space");
}

/// Opponent POV, opponent side lost: the user (non-mover) lost their own
/// space.
#[test]
fn space_opponent_lost_opponent_is_your_loss() {
    let claim = Claim::Space {
        side: SpaceSide::Opponent,
        direction: SpaceDirection::Lost,
        delta_mg: -40,
    };
    let p = phrase(&claim, &ctx(Perspective::Opponent));
    assert_eq!(p.summary, "You lost space");
}

// =====================================================================
// Initiative (forcing hierarchy)
// =====================================================================

/// Player POV reinforcement: your move creates a threat the opponent
/// must address.
#[test]
fn initiative_reinforcement_player() {
    let claim = Claim::Initiative {
        mover: Color::White,
        template: InitiativeTemplate::Reinforcement,
        reply_san: "Nf6".to_string(),
        reply_is_check: false,
    };
    let p = phrase(&claim, &ctx(Perspective::Player));
    assert!(
        p.summary.starts_with("Your move creates a threat"),
        "{}",
        p.summary
    );
    assert!(
        p.summary.contains("the opponent must address"),
        "{}",
        p.summary
    );
}

/// Opponent POV reinforcement: their move creates a threat *you* must
/// address.
#[test]
fn initiative_reinforcement_opponent() {
    let claim = Claim::Initiative {
        mover: Color::White,
        template: InitiativeTemplate::Reinforcement,
        reply_san: "Nf6".to_string(),
        reply_is_check: false,
    };
    let p = phrase(&claim, &ctx(Perspective::Opponent));
    assert!(
        p.summary.starts_with("Their move creates a threat"),
        "{}",
        p.summary
    );
    assert!(p.summary.contains("you must address"), "{}", p.summary);
}

/// Refutation by check names the reply and the check-priority rule.
#[test]
fn initiative_refutation_check_player() {
    let claim = Claim::Initiative {
        mover: Color::White,
        template: InitiativeTemplate::Refutation,
        reply_san: "Qa3+".to_string(),
        reply_is_check: true,
    };
    let p = phrase(&claim, &ctx(Perspective::Player));
    assert!(p.summary.contains("Qa3+"), "{}", p.summary);
    assert!(
        p.summary.contains("a check that takes priority"),
        "{}",
        p.summary
    );
    assert!(p.summary.contains("Checks must be answered"), "{}", p.summary);
}

/// Held-despite by capture: names the reply and that the threat still
/// lands.
#[test]
fn initiative_held_despite_capture_player() {
    let claim = Claim::Initiative {
        mover: Color::White,
        template: InitiativeTemplate::HeldDespite,
        reply_san: "Rxd4".to_string(),
        reply_is_check: false,
    };
    let p = phrase(&claim, &ctx(Perspective::Player));
    assert!(p.summary.contains("Rxd4"), "{}", p.summary);
    assert!(
        p.summary.contains("addresses the material first"),
        "{}",
        p.summary
    );
    assert!(p.summary.contains("the threat still lands"), "{}", p.summary);
}

// =====================================================================
// Secondary (other shifts)
// =====================================================================

/// Helped / hurt split from the mover-POV deltas, biggest-first list.
#[test]
fn secondary_splits_helped_and_hurt() {
    let claim = Claim::Secondary {
        terms: vec![(TermId::KingPawnShield, 80), (TermId::MobilityKnight, -40)],
    };
    let p = phrase(&claim, &ctx(Perspective::Player));
    assert_eq!(p.summary, "1 helped, 1 hurt");
    let detail = p.detail.expect("detail");
    assert!(detail.contains("Also helped:"), "{detail}");
    assert!(detail.contains("Also hurt:"), "{detail}");
    assert!(detail.contains("+0.80"), "{detail}");
    assert!(detail.contains("-0.40"), "{detail}");
}

/// The deltas are mover-POV already, so the same claim phrases
/// identically under either perspective (the IR did the reframe by
/// sign-flipping at build time, not phrase time).
#[test]
fn secondary_content_is_perspective_stable() {
    let claim = Claim::Secondary {
        terms: vec![(TermId::KingPawnShield, 80)],
    };
    let player = phrase(&claim, &ctx(Perspective::Player));
    let opponent = phrase(&claim, &ctx(Perspective::Opponent));
    assert_eq!(player.summary, opponent.summary);
    assert_eq!(player.detail, opponent.detail);
}

// =====================================================================
// Special UI narratives (step 10)
// =====================================================================

use crate::claim::ForcedConcession;
use chess_tutor_engine::analysis::SurpriseKind;

// ---- ForcedConsequence -----------------------------------------------

#[test]
fn forced_consequence_player_lands_on_them() {
    let claim = Claim::ForcedConsequence {
        mover: Color::White,
        reply_san: "gxh6".to_string(),
        category: ForcedConcession::Doubled,
        delta_mg: -20,
    };
    let p = phrase(&claim, &ctx(Perspective::Player));
    // The concession lands on the replier (the opponent) — "they get …".
    assert!(p.summary.contains("If they reply gxh6"), "{}", p.summary);
    assert!(p.summary.contains("doubled pawns"), "{}", p.summary);
    // Never claims it "forces".
    assert!(!p.summary.to_lowercase().contains("forces"), "{}", p.summary);
}

#[test]
fn forced_consequence_opponent_lands_on_you() {
    let claim = Claim::ForcedConsequence {
        mover: Color::Black,
        reply_san: "gxh6".to_string(),
        category: ForcedConcession::Isolated,
        delta_mg: -15,
    };
    let p = phrase(&claim, &ctx(Perspective::Opponent));
    // When the opponent moved, the replier is the user — "if you reply …".
    assert!(p.summary.contains("If you reply gxh6"), "{}", p.summary);
    assert!(p.summary.contains("an isolated pawn"), "{}", p.summary);
}

// ---- Desperado -------------------------------------------------------

#[test]
fn desperado_player_is_you() {
    // recovered_cp = PAWN_MG (128) → one human pawn.
    let claim = Claim::Desperado {
        mover: Color::White,
        san: "Nxg7+".to_string(),
        recovered_cp: Value::PAWN_MG.0,
    };
    let p = phrase(&claim, &ctx(Perspective::Player));
    assert!(p.summary.starts_with("Desperado — Nxg7+"), "{}", p.summary);
    assert!(p.summary.contains("~1"), "{}", p.summary);
    let detail = p.detail.expect("detail");
    assert!(detail.contains("you trade it off"), "{detail}");
}

#[test]
fn desperado_opponent_is_they() {
    let claim = Claim::Desperado {
        mover: Color::Black,
        san: "Nxg2+".to_string(),
        recovered_cp: Value::PAWN_MG.0,
    };
    let p = phrase(&claim, &ctx(Perspective::Opponent));
    let detail = p.detail.expect("detail");
    assert!(detail.contains("they trade it off"), "{detail}");
    assert!(!detail.contains("you trade it off"), "{detail}");
}

// ---- OverrideNote ----------------------------------------------------

#[test]
fn override_note_never_calls_recommended_move_strong() {
    let claim = Claim::OverrideNote {
        mover: Color::White,
        static_pawns: 1.9,
        search_pawns: 0.7,
    };
    let p = phrase(&claim, &ctx(Perspective::Player));
    let blob = format!("{} {}", p.summary, p.detail.clone().unwrap());
    assert!(!blob.to_lowercase().contains("positionally strong"), "{blob}");
    assert!(blob.contains("search overrules") || blob.contains("trust the search"), "{blob}");
    // Player framing: "your move".
    assert!(p.summary.contains("your move"), "{}", p.summary);
}

#[test]
fn override_note_opponent_is_their_move() {
    let claim = Claim::OverrideNote {
        mover: Color::Black,
        static_pawns: 1.9,
        search_pawns: 0.7,
    };
    let p = phrase(&claim, &ctx(Perspective::Opponent));
    assert!(p.summary.contains("their move"), "{}", p.summary);
}

// ---- DepthHonesty ----------------------------------------------------

#[test]
fn depth_honesty_no_blunder_no_fake_mechanism_both_perspectives() {
    for persp in [Perspective::Player, Perspective::Opponent] {
        let claim = Claim::DepthHonesty { mover: Color::White };
        let p = phrase(&claim, &ctx(persp));
        let blob = format!("{} {}", p.summary, p.detail.clone().unwrap()).to_lowercase();
        assert!(blob.contains("calculation depth"), "{blob}");
        assert!(!blob.contains("blunder"), "{blob}");
        assert!(!blob.contains("walked into"), "{blob}");
    }
}

#[test]
fn depth_honesty_player_says_you_didnt_miss() {
    let claim = Claim::DepthHonesty { mover: Color::White };
    let p = phrase(&claim, &ctx(Perspective::Player));
    let detail = p.detail.expect("detail");
    assert!(detail.contains("you should feel you missed"), "{detail}");
}

// ---- Surprise tag ----------------------------------------------------

#[test]
fn surprise_positive_player_well_spotted() {
    let claim = Claim::Surprise {
        mover: Color::White,
        verdict: MoveVerdict::Best,
        kind: SurpriseKind::LooksBadButGood,
    };
    let p = phrase(&claim, &ctx(Perspective::Player));
    assert!(p.summary.starts_with("Well spotted"), "{}", p.summary);
}

#[test]
fn surprise_positive_opponent_is_they() {
    let claim = Claim::Surprise {
        mover: Color::Black,
        verdict: MoveVerdict::Good,
        kind: SurpriseKind::LooksBadButGood,
    };
    let p = phrase(&claim, &ctx(Perspective::Opponent));
    assert!(p.summary.contains("They found"), "{}", p.summary);
}

#[test]
fn surprise_negative_flips_subject() {
    let claim = Claim::Surprise {
        mover: Color::White,
        verdict: MoveVerdict::Mistake,
        kind: SurpriseKind::LooksGoodButBad,
    };
    let player = phrase(&claim, &ctx(Perspective::Player));
    let opponent = phrase(&claim, &ctx(Perspective::Opponent));
    assert!(player.summary.contains("you'll be on the defensive"), "{}", player.summary);
    assert!(opponent.summary.contains("they'll be on the defensive"), "{}", opponent.summary);
    // Avoids strong chess terminology.
    assert!(!player.summary.to_lowercase().contains("refute"), "{}", player.summary);
}

// ---- Centre structure (cross-term multiplier) ----------------------------

#[test]
fn center_structure_closed_flips_subject_by_perspective() {
    let claim = Claim::CenterStructure {
        mover: Color::White,
        kind: CenterShift::Closed,
    };
    let player = phrase(&claim, &ctx(Perspective::Player));
    let opponent = phrase(&claim, &ctx(Perspective::Opponent));
    assert!(player.summary.starts_with("You closed the center"), "{}", player.summary);
    assert!(opponent.summary.starts_with("They closed the center"), "{}", opponent.summary);
}

#[test]
fn center_structure_opened_is_subjectless_both_perspectives() {
    let claim = Claim::CenterStructure {
        mover: Color::Black,
        kind: CenterShift::Opened,
    };
    let player = phrase(&claim, &ctx(Perspective::Player));
    let opponent = phrase(&claim, &ctx(Perspective::Opponent));
    // The opening of the centre is a neutral board fact — no "you" / "they".
    assert!(player.summary.starts_with("The center opened"), "{}", player.summary);
    assert_eq!(player.summary, opponent.summary);
}

#[test]
fn center_structure_barricaded_and_cleared_render() {
    let bar = phrase(
        &Claim::CenterStructure { mover: Color::White, kind: CenterShift::Barricaded },
        &ctx(Perspective::Player),
    );
    assert!(bar.summary.starts_with("A piece now sits in front"), "{}", bar.summary);
    let clr = phrase(
        &Claim::CenterStructure { mover: Color::White, kind: CenterShift::Cleared },
        &ctx(Perspective::Player),
    );
    assert!(clr.summary.starts_with("A central pawn's path cleared"), "{}", clr.summary);
}

// ---- Castling loss × trapped rook (cross-term multiplier) ----------------

#[test]
fn castling_loss_mover_side_reframes_by_perspective() {
    // Mover lost its OWN castling: a warning to the player; the opponent's
    // gain when the opponent moved.
    let claim = Claim::CastlingLoss { side: CastleSide::Mover };
    let player = phrase(&claim, &ctx(Perspective::Player));
    let opponent = phrase(&claim, &ctx(Perspective::Opponent));
    assert!(player.summary.starts_with("You forfeited castling"), "{}", player.summary);
    assert!(
        opponent.summary.starts_with("You stripped the opponent of castling"),
        "{}",
        opponent.summary
    );
}

#[test]
fn castling_loss_opponent_side_reframes_by_perspective() {
    // Mover stripped the OPPONENT's castling: the player's gain; the
    // opponent's own warning when the opponent moved.
    let claim = Claim::CastlingLoss { side: CastleSide::Opponent };
    let player = phrase(&claim, &ctx(Perspective::Player));
    let opponent = phrase(&claim, &ctx(Perspective::Opponent));
    assert!(
        player.summary.starts_with("You stripped the opponent of castling"),
        "{}",
        player.summary
    );
    assert!(opponent.summary.starts_with("You forfeited castling"), "{}", opponent.summary);
}

// ---- PositionalWin: sound-sacrifice justification --------------------

/// Build a `Claim::PositionalWin` modelling the case study: White down a
/// point of material, king-danger swings hard.
fn positional_win() -> Claim {
    Claim::PositionalWin {
        mover: Color::White,
        sacrificed_points: -1, // down a point
        dominant_term: TermId::KingDanger,
        term_pre_cp: 286,
        term_post_cp: 3211,
    }
}

#[test]
fn positional_win_player_played_is_praise_leading_with_material_cost() {
    let p = phrase(&positional_win(), &ctx(Perspective::Player));
    // Leads with the material cost ("you give up a pawn"), then names the
    // compensating term. No raw search cp in the summary.
    assert!(p.summary.starts_with("Worth it: you give up a pawn"), "{}", p.summary);
    assert!(p.summary.contains("king safety"), "{}", p.summary);
    // The detail shows the pre→post swing in pawns, never engine cp.
    let detail = p.detail.expect("a detail line with the term swing");
    assert!(detail.contains("King safety goes"), "{}", detail);
    assert!(detail.contains("→"), "{}", detail);
}

#[test]
fn positional_win_opponent_reframes_to_their_sacrifice() {
    let o = phrase(&positional_win(), &ctx(Perspective::Opponent));
    // From the player's POV the opponent found the sacrifice: "they give
    // up …", and the term swings in *their* favour.
    assert!(o.summary.starts_with("Worth it for them: they give up a pawn"), "{}", o.summary);
    assert!(o.summary.contains("their favour"), "{}", o.summary);
    // Never says "you give up" in the opponent reframe.
    assert!(!o.summary.contains("you give up"), "{}", o.summary);
}

#[test]
fn positional_win_multi_point_sacrifice_reads_as_points() {
    // A two-point sacrifice (e.g. rook for bishop) reads "N points".
    let claim = Claim::PositionalWin {
        mover: Color::Black,
        sacrificed_points: -2,
        dominant_term: TermId::PiecesTrappedRook,
        term_pre_cp: 0,
        term_post_cp: 104,
    };
    let p = phrase(&claim, &ctx(Perspective::Player));
    assert!(p.summary.contains("you give up 2 points"), "{}", p.summary);
    assert!(p.summary.contains("trapped rook"), "{}", p.summary);
}

// ---- MissedProphylaxis: the defence you needed -----------------------

/// Build a `Claim::MissedProphylaxis` modelling the case study: Black (the
/// mover) skipped `Ra8`, allowing White's `Rxe7+`; king safety collapses.
/// `reveal` controls whether the prophylactic move is named.
fn missed_prophylaxis(reveal: bool) -> Claim {
    Claim::MissedProphylaxis {
        mover: Color::Black,
        prophylactic_san: reveal.then(|| "Ra8".to_string()),
        punisher_san: "Rxe7+".to_string(),
        exploded_term: TermId::KingDanger,
        swing_cp: 380,
    }
}

#[test]
fn missed_prophylaxis_player_names_defence_and_punisher() {
    let p = phrase(&missed_prophylaxis(true), &ctx(Perspective::Player));
    // Player wording leads with the defence they needed and the move it
    // stops, then the term that collapses.
    assert!(
        p.summary.starts_with("You needed Ra8 to stop Rxe7+"),
        "{}",
        p.summary
    );
    assert!(p.summary.contains("king safety"), "{}", p.summary);
    // Never says "they" in the player frame.
    assert!(!p.summary.contains("opponent"), "{}", p.summary);
    let detail = p.detail.expect("a detail line naming the punisher");
    assert!(detail.contains("Rxe7+"), "{}", detail);
}

#[test]
fn missed_prophylaxis_opponent_is_opportunity_reframe() {
    let o = phrase(&missed_prophylaxis(true), &ctx(Perspective::Opponent));
    // From the player's POV the opponent skipped the defence, so the
    // punisher is now *your* winning move.
    assert!(
        o.summary.starts_with("Your opponent skipped Ra8; Rxe7+ now wins"),
        "{}",
        o.summary
    );
    assert!(o.summary.contains("king safety"), "{}", o.summary);
    // Never scolds the user ("you needed") in the opponent reframe.
    assert!(!o.summary.contains("You needed"), "{}", o.summary);
}

#[test]
fn missed_prophylaxis_reveal_off_teaches_concept_without_naming_move() {
    let p = phrase(&missed_prophylaxis(false), &ctx(Perspective::Player));
    // No prophylactic SAN ⇒ the concept is taught, the defence not named,
    // but the punisher (the teaching) still is.
    assert!(!p.summary.contains("Ra8"), "{}", p.summary);
    assert!(p.summary.contains("Rxe7+"), "{}", p.summary);
    assert!(p.summary.contains("quiet move"), "{}", p.summary);
}
