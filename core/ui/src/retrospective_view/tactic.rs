//! Tactic card builder — "you played a fork", "you missed a pin",
//! "you walked into a fork".
//!
//! The prose (heading + per-pattern lesson detail + escape note + the
//! ALLOWED-not-MISSED reframe) is produced by the shared teaching
//! translator ([`chess_tutor_teaching`]) from a [`Claim::Tactic`]; this
//! builder owns only the *structured* card surface the translator
//! deliberately doesn't carry — sentiment, the white-POV score chip, the
//! structured material summary line, and the per-square board
//! annotations.
//!
//! [`tactic_claims`] handles the three-slot dispatch (played / missed /
//! walked-into), the escape resolution, and the ALLOWED reframe gate
//! internally; we map each [`Claim::Tactic`] it returns onto a
//! [`RetrospectiveItem`].
//!
//! Pedagogical rules in force here (per memory
//! `feedback_teaching_terminology`):
//! - Use chess vocabulary where it's precise (*"fork"*, *"pin"*,
//!   *"skewer"*); plain English where the engine's signal doesn't fit
//!   the technical meaning exactly. (Lives in the translator now.)
//! - When [`reveal_best_moves`] is off, the missed-tactic card surfaces
//!   the *concept* without naming the move or pointing at the squares —
//!   same posture as the headline best-move arrow.

use chess_tutor_engine::analysis::{MoveAnalysis, PriorMove, TacticHit, TacticPattern};
use chess_tutor_engine::attacks::between_bb;
use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::{Color, Square};

use chess_tutor_teaching::claim::{tactic_claims, Claim, TacticRole};
use chess_tutor_teaching::phrasing::{
    phrase, Locale, Perspective, PhrasingContext, Verbosity,
};

use crate::view::{
    AnnotationKind, BoardAnnotation, RetrospectiveCategory, RetrospectiveItem, Sentiment,
};

/// Build every tactic-related item for one analysed move — played,
/// missed, and walked-into — in display order. One [`tactic_claims`]
/// call covers all three slots.
///
/// `reveal_best_moves` controls whether the *missed-tactic* card emits
/// board annotations (which would reveal the engine's preferred move's
/// location). When off, the card still appears so the student knows a
/// concept was available, but with no spatial spoilers — same posture
/// as the headline's `best_move_annotation` gate. Played and walked-
/// into cards always paint their annotations (the student needs to
/// see *their own* tactic; the warning about an opponent's tactic is
/// pedagogically more useful with squares shown).
///
/// `perspective` selects "you" vs "they" in the translator's prose and
/// the student-POV sentiment / chip sign: under `Opponent` the mover is
/// the opponent, so a played tactic hurts the student and a missed /
/// walked-into one is the student's chance — all signs flip.
pub(super) fn build_tactic_items(
    pre_move_pos: &Position,
    best: &MoveAnalysis,
    user: &MoveAnalysis,
    root_stm: Color,
    prior_move: Option<PriorMove>,
    reveal_best_moves: bool,
    perspective: Perspective,
) -> Vec<RetrospectiveItem> {
    let ctx = PhrasingContext {
        perspective,
        locale: Locale::En,
        verbosity: Verbosity::Normal,
        reveal_moves: reveal_best_moves,
    };
    tactic_claims(pre_move_pos, best, user, root_stm, prior_move)
        .iter()
        .map(|claim| {
            let pin_rear = pin_rear_king(pre_move_pos, best, user, claim);
            let attacker = discovery_attacker(pre_move_pos, best, user, claim);
            tactic_item(claim, &ctx, reveal_best_moves, pin_rear, attacker)
        })
        .collect()
}

/// The king an absolute pin bears against — the rear of the pin line — so
/// the card can draw a single arrow *through* the pinned piece to the king
/// it's pinned to, rather than the bare attacker→pinned arrow that reads
/// like a relative pin ("something pinned *to* the queen").
///
/// Returns `None` for any non-[`TacticPattern::Pin`] claim, or when the
/// line is too short to replay to the position the pin stands in. We replay
/// the claim's own line — the user's PV for a played / walked-into pin, the
/// engine's for a missed one — up to and including the pin's key move, then
/// read the pinned side's king (the pinned piece pins to its *own* king, so
/// the king is whatever colour sits on the pinned square).
fn pin_rear_king(
    pre_move_pos: &Position,
    best: &MoveAnalysis,
    user: &MoveAnalysis,
    claim: &Claim,
) -> Option<Square> {
    let Claim::Tactic { role, hit, .. } = claim else {
        return None;
    };
    if hit.pattern != TacticPattern::Pin {
        return None;
    }
    let pinned_sq = *hit.targets.first()?;
    let pv = match role {
        TacticRole::Missed => &best.pv,
        TacticRole::Played | TacticRole::WalkedInto => &user.pv,
    };
    if pv.len() <= hit.pv_ply {
        return None;
    }
    let mut board = pre_move_pos.clone();
    for &mv in pv.iter().take(hit.pv_ply + 1) {
        board.do_move(mv);
    }
    let pinned = board.piece_on(pinned_sq)?;
    Some(board.king_square(pinned.color()))
}

/// The square of the piece that actually *delivers* a discovered attack /
/// check — the one the key move unmasks — so the card can draw the
/// discovery arrow from the real attacker (the rook a bishop's move
/// reveals) rather than from the moved piece, which never lies on the
/// attack line.
///
/// `TacticHit::primary_piece` stores the piece that *moved* for these
/// patterns, not the attacker, and the engine discards the attacker
/// square. We recover it the same way the engine's detector found it:
/// replay the claim's line to the position right after the unmasking move,
/// then take the attacker of the target whose ray *through the vacated
/// square* is now open. `None` for non-discovery patterns or when the
/// geometry can't be resolved (the caller falls back to `primary_piece`).
fn discovery_attacker(
    pre_move_pos: &Position,
    best: &MoveAnalysis,
    user: &MoveAnalysis,
    claim: &Claim,
) -> Option<Square> {
    let Claim::Tactic { role, hit, .. } = claim else {
        return None;
    };
    if !matches!(
        hit.pattern,
        TacticPattern::DiscoveredAttack
            | TacticPattern::DiscoveredCheck
            | TacticPattern::DoubleCheck
    ) {
        return None;
    }
    let key = hit.key_move?;
    let &target = hit.targets.first()?;
    let pv = match role {
        TacticRole::Missed => &best.pv,
        TacticRole::Played | TacticRole::WalkedInto => &user.pv,
    };
    if pv.len() <= hit.pv_ply {
        return None;
    }
    let mut board = pre_move_pos.clone();
    for &mv in pv.iter().take(hit.pv_ply + 1) {
        board.do_move(mv);
    }
    // The unmasked attacker now attacks `target` along a ray that passes
    // through the square the key move vacated. Skip the moved piece itself.
    let vacated = key.from();
    let occ = board.occupied();
    board
        .attackers_to(target, occ)
        .into_iter()
        .find(|&sq| sq != key.to() && between_bb(sq, target).contains(vacated))
}

/// Map a single [`Claim::Tactic`] to a [`RetrospectiveItem`]: prose from
/// the translator, structured surface (sentiment, chip, summary,
/// annotations) computed here.
fn tactic_item(
    claim: &Claim,
    ctx: &PhrasingContext,
    reveal_best_moves: bool,
    pin_rear: Option<Square>,
    attacker: Option<Square>,
) -> RetrospectiveItem {
    let Claim::Tactic { role, hit, .. } = claim else {
        unreachable!("tactic_claims only emits Claim::Tactic");
    };
    let phrasing = phrase(claim, ctx);

    // Sentiment + chip sign are role-driven *from the mover's side*: the
    // mover playing a tactic is the mover's gain; missing / walking into one
    // is the mover's concession. Then re-anchored to the student: under the
    // opponent perspective the mover is the opponent, so a mover gain is the
    // student's loss — flip both the sentiment and the chip sign.
    let mover_gain = match role {
        TacticRole::Played => true,
        TacticRole::Missed | TacticRole::WalkedInto => false,
    };
    let good_for_student = match ctx.perspective {
        Perspective::Player => mover_gain,
        Perspective::Opponent => !mover_gain,
    };
    let chip_magnitude = hit.material_gain.map(|cp| cp as f32 / 100.0);
    let (sentiment, chip) = if good_for_student {
        (Sentiment::Positive, chip_magnitude)
    } else {
        (Sentiment::Negative, chip_magnitude.map(|c| -c))
    };

    // The missed-tactic card suppresses spatial hints when reveal is off
    // (it would reveal the engine's preferred move's location); played
    // and walked-into cards always paint (the student needs to see their
    // own tactic / the opponent's punishing response).
    let annotations = if *role == TacticRole::Missed && !reveal_best_moves {
        Vec::new()
    } else {
        tactic_annotations(hit, *role, pin_rear, attacker)
    };

    RetrospectiveItem {
        category: RetrospectiveCategory::Tactic,
        heading: phrasing.summary,
        summary: tactic_summary_line(*role, hit),
        detail: phrasing.detail.unwrap_or_default(),
        score_delta_pawns: chip,
        sentiment,
        annotations,
    }
}

// ------------------------------------------------------------------------
// Structured summary line — the material read under the heading. Neutral
// (no "you"); the perspective prose is the heading from the translator.
// ------------------------------------------------------------------------

fn tactic_summary_line(role: TacticRole, hit: &TacticHit) -> String {
    match role {
        TacticRole::Played => played_summary(hit),
        TacticRole::Missed => missed_summary(hit),
        TacticRole::WalkedInto => walked_into_summary(hit),
    }
}

fn played_summary(hit: &TacticHit) -> String {
    match (hit.pattern, hit.material_gain, hit.sacrifice) {
        (TacticPattern::Checkmate, _, _) => "forced mate".into(),
        (_, Some(gain), true) if gain <= 0 => "sound sacrifice — full compensation".into(),
        (_, Some(gain), true) if gain > 0 => {
            format!("sacrifice — recovers material ({:+.2})", gain as f32 / 100.0)
        }
        (_, Some(gain), false) if gain > 0 => {
            format!("wins material ({:+.2})", gain as f32 / 100.0)
        }
        (_, Some(0), _) => "even material, positional gain".into(),
        _ => "positional pressure".into(),
    }
}

fn missed_summary(hit: &TacticHit) -> String {
    match (hit.pattern, hit.material_gain) {
        (TacticPattern::Checkmate, _) => "the engine had forced mate".into(),
        (_, Some(gain)) if gain > 0 => {
            format!("the engine's line wins material ({:+.2})", gain as f32 / 100.0)
        }
        _ => "the engine had a tactic available".into(),
    }
}

fn walked_into_summary(hit: &TacticHit) -> String {
    match (hit.pattern, hit.material_gain) {
        (TacticPattern::Checkmate, _) => "their reply forces mate".into(),
        (_, Some(gain)) if gain > 0 => {
            format!("their reply wins material ({:+.2})", gain as f32 / 100.0)
        }
        _ => "their reply lands a tactic".into(),
    }
}

// ------------------------------------------------------------------------
// Annotations — the spatial story painted on the board
// ------------------------------------------------------------------------

/// Paint a tactic's spatial story. Every walked-into card follows one
/// rule (per user guidance): draw the **trigger move** that springs the
/// tactic, *plus* the **tactic itself** (fork arrows, the pin line, the
/// discovery arrow, …).
///
/// - `role` gates the trigger arrow: a walked-into tactic's key move is
///   the opponent's *future* reply, so the acting piece isn't on its
///   tactic square on the displayed (post-user-move) board — the gold
///   trigger arrow names the move that puts it there. Played tactics are
///   drawn after the move is made (no trigger needed); missed tactics
///   suppress spatial hints unless revealed.
/// - `pin_rear` (the king) turns an absolute pin into one line *through*
///   the pinned piece, so it doesn't read as a relative pin.
/// - `attacker` overrides the arrow source for a discovery: the unmasked
///   piece (the rook), not the piece that moved (whose square never lies
///   on the attack line).
fn tactic_annotations(
    hit: &TacticHit,
    role: TacticRole,
    pin_rear: Option<Square>,
    attacker: Option<Square>,
) -> Vec<BoardAnnotation> {
    let mut out = Vec::new();

    // The move that springs the tactic — only for a walked-into card, where
    // the key move is the opponent's reply that hasn't been played yet.
    if role == TacticRole::WalkedInto {
        if let Some(km) = hit.key_move {
            if km.from() != km.to() {
                out.push(BoardAnnotation::Arrow {
                    from: km.from(),
                    to: km.to(),
                    kind: AnnotationKind::TriggerMove,
                });
            }
        }
    }

    // The piece that actually delivers the tactic: the unmasked attacker
    // for a discovery, otherwise the piece that did the work.
    let attacker_sq = attacker.unwrap_or(hit.primary_piece);

    // Absolute pin: draw the line from the pinning piece *through* the
    // pinned piece to the king it pins to, so it reads as an absolute pin
    // (queen pinned to king), not a relative one (something pinned to the
    // queen).
    if hit.pattern == TacticPattern::Pin {
        if let (Some(&pinned), Some(king)) = (hit.targets.first(), pin_rear) {
            out.push(BoardAnnotation::Arrow {
                from: attacker_sq,
                to: king,
                kind: AnnotationKind::Attacker,
            });
            out.push(BoardAnnotation::SquareHighlight {
                square: pinned,
                kind: AnnotationKind::Threat,
            });
            return out;
        }
    }

    // Generic: highlight the attacker, then an arrow from it to each victim.
    out.push(BoardAnnotation::SquareHighlight {
        square: attacker_sq,
        kind: AnnotationKind::GoodPiece,
    });
    for &target in &hit.targets {
        // Skip degenerate arrows (capture pattern: attacker == target).
        if target != attacker_sq {
            out.push(BoardAnnotation::Arrow {
                from: attacker_sq,
                to: target,
                kind: AnnotationKind::Attacker,
            });
        }
        out.push(BoardAnnotation::SquareHighlight {
            square: target,
            kind: AnnotationKind::Threat,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chess_tutor_engine::analysis::{Confidence, MatePattern};
    use chess_tutor_engine::types::Square;

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

    fn player_ctx(reveal: bool) -> PhrasingContext {
        PhrasingContext {
            perspective: Perspective::Player,
            locale: Locale::En,
            verbosity: Verbosity::Normal,
            reveal_moves: reveal,
        }
    }

    fn played_claim(hit: TacticHit) -> Claim {
        Claim::Tactic {
            mover: Color::White,
            role: TacticRole::Played,
            hit,
            escape: None,
            allowed: None,
        }
    }

    fn missed_claim(hit: TacticHit) -> Claim {
        Claim::Tactic {
            mover: Color::White,
            role: TacticRole::Missed,
            hit,
            escape: None,
            allowed: None,
        }
    }

    fn walked_claim(hit: TacticHit) -> Claim {
        Claim::Tactic {
            mover: Color::White,
            role: TacticRole::WalkedInto,
            hit,
            escape: None,
            allowed: None,
        }
    }

    #[test]
    fn played_card_heading_uses_translator_prose() {
        let card = tactic_item(&played_claim(fork_hit()), &player_ctx(false), false, None, None);
        assert_eq!(card.heading, "You played a fork");
        assert_eq!(card.sentiment, Sentiment::Positive);
        assert_eq!(card.score_delta_pawns, Some(3.0));
    }

    #[test]
    fn missed_card_inverts_chip_and_drops_annotations_when_reveal_off() {
        let card = tactic_item(&missed_claim(fork_hit()), &player_ctx(false), false, None, None);
        assert_eq!(card.heading, "You missed a fork");
        assert_eq!(card.sentiment, Sentiment::Negative);
        assert_eq!(card.score_delta_pawns, Some(-3.0));
        assert!(
            card.annotations.is_empty(),
            "reveal-off must not paint the engine's preferred line"
        );
    }

    #[test]
    fn missed_card_keeps_annotations_when_reveal_on() {
        let card = tactic_item(&missed_claim(fork_hit()), &player_ctx(true), true, None, None);
        assert!(!card.annotations.is_empty());
    }

    #[test]
    fn mate_suffix_only_for_default_surfaced_patterns() {
        let mut hit = fork_hit();
        hit.mate_pattern = Some(MatePattern::BackRank);
        let card = tactic_item(&played_claim(hit.clone()), &player_ctx(false), false, None, None);
        assert!(card.heading.ends_with("back-rank mate"));

        hit.mate_pattern = Some(MatePattern::Anastasia);
        let card = tactic_item(&played_claim(hit), &player_ctx(false), false, None, None);
        // Anastasia is engine-known but not surfaced_by_default; no suffix.
        assert_eq!(card.heading, "You played a fork");
    }

    #[test]
    fn checkmate_pattern_uses_mate_phrasing_in_summary() {
        let hit = TacticHit {
            pattern: TacticPattern::Checkmate,
            pv_ply: 0,
            primary_piece: Square::H7,
            targets: vec![Square::G8],
            material_gain: None,
            confidence: Confidence::High,
            sacrifice: false,
            mate_pattern: Some(MatePattern::BackRank),
            key_move: None,
        };
        let card = tactic_item(&played_claim(hit), &player_ctx(false), false, None, None);
        assert_eq!(card.heading, "You played checkmate — back-rank mate");
        assert_eq!(card.summary, "forced mate");
    }

    #[test]
    fn walked_into_card_is_negative_with_warning_framing() {
        let card = tactic_item(&walked_claim(fork_hit()), &player_ctx(false), false, None, None);
        assert_eq!(card.sentiment, Sentiment::Negative);
        assert!(card.heading.starts_with("You walked into"));
    }

    #[test]
    fn played_card_appends_opponent_escape_note() {
        let claim = Claim::Tactic {
            mover: Color::White,
            role: TacticRole::Played,
            hit: fork_hit(),
            escape: Some(chess_tutor_teaching::claim::TacticEscapeInfo {
                san: "Qxe5".to_string(),
                kind: chess_tutor_engine::analysis::EscapeKind::Zwischenzug,
            }),
            allowed: None,
        };
        let card = tactic_item(&claim, &player_ctx(false), false, None, None);
        assert!(card.detail.contains("Qxe5"), "{}", card.detail);
        assert!(card.detail.contains("wriggle out"), "{}", card.detail);
        assert!(card.detail.contains("in-between capture"), "{}", card.detail);
    }

    #[test]
    fn walked_into_card_frames_escape_as_good_news() {
        let claim = Claim::Tactic {
            mover: Color::White,
            role: TacticRole::WalkedInto,
            hit: fork_hit(),
            escape: Some(chess_tutor_teaching::claim::TacticEscapeInfo {
                san: "Nf6".to_string(),
                kind: chess_tutor_engine::analysis::EscapeKind::AdequateRetreat,
            }),
            allowed: None,
        };
        let card = tactic_item(&claim, &player_ctx(false), false, None, None);
        assert!(card.detail.contains("Nf6"), "{}", card.detail);
        assert!(card.detail.contains("good news"), "{}", card.detail);
        assert!(
            card.detail.contains("moving the attacked piece to safety"),
            "{}",
            card.detail
        );
    }

    #[test]
    fn allowed_reframe_leads_card_with_swing_and_continuation() {
        let claim = Claim::Tactic {
            mover: Color::White,
            role: TacticRole::WalkedInto,
            hit: fork_hit(),
            escape: None,
            allowed: Some(chess_tutor_teaching::claim::AllowedReframe {
                best_pawns: 2.04,
                played_pawns: -1.27,
                swing_pawns: 3.3,
                continuation: "Qc5+ Kf7 b3 Ne7".to_string(),
            }),
        };
        let card = tactic_item(&claim, &player_ctx(false), false, None, None);
        assert_eq!(card.sentiment, Sentiment::Negative);
        assert!(card.heading.starts_with("You allowed"), "{}", card.heading);
        // Swing comes first, then the punishing continuation, then the
        // per-pattern lesson.
        assert!(card.detail.contains("3.3-pawn swing"), "{}", card.detail);
        assert!(card.detail.contains("Qc5+ Kf7"), "{}", card.detail);
        let swing_at = card.detail.find("3.3-pawn").unwrap();
        let lesson_at = card.detail.find("fork is one piece").unwrap();
        assert!(swing_at < lesson_at, "swing must lead the lesson");
    }

    #[test]
    fn build_tactic_items_fires_allowed_reframe_on_qc5_case_study() {
        use chess_tutor_engine::san;
        use chess_tutor_engine::types::{Move, Value};

        // discovered-attack-after-qxe6 FEN. The user plays Qc5+ (giving away
        // a winning position) instead of Qxe6+; the standing discovered
        // attack on Re1 must surface as a walked-into card led by the
        // ALLOWED reframe.
        let fen = "1r4nr/p3k3/4qpp1/4b2p/2Q5/8/PPPP1PPP/R1B1R1K1 w - - 0 1";
        let mut pre = Position::from_fen(fen).unwrap();
        let qc5 = san::parse(&mut pre, "Qc5+").unwrap();
        let pre = Position::from_fen(fen).unwrap();
        // user line: Qc5+ then a forced king move (the discovery isn't
        // *played* in this short line — it's standing).
        let user_pv = vec![qc5, Move::normal(Square::E7, Square::F7)];
        let user = MoveAnalysis {
            mv: qc5,
            score: Value(-270), // ~-1.27 pawns, root (White) POV
            depth: 1,
            pv: user_pv,
            ply_traces: Vec::new(),
            settled_ply: Some(1),
            pre_move_trace: chess_tutor_engine::eval::EvalTrace::zero(),
            pre_score: Value::ZERO,
            term_deltas: Vec::new(),
        };
        let mut best_pos = Position::from_fen(fen).unwrap();
        let qxe6 = san::parse(&mut best_pos, "Qxe6+").unwrap();
        let best = MoveAnalysis {
            mv: qxe6,
            score: Value(434), // ~+2.04 pawns, root POV
            depth: 1,
            pv: vec![qxe6],
            ply_traces: Vec::new(),
            settled_ply: Some(0),
            pre_move_trace: chess_tutor_engine::eval::EvalTrace::zero(),
            pre_score: Value::ZERO,
            term_deltas: Vec::new(),
        };
        let items =
            build_tactic_items(&pre, &best, &user, Color::White, None, false, Perspective::Player);
        let walked = items
            .iter()
            .find(|it| it.heading.starts_with("You allowed"))
            .expect("Qc5+ must surface a walked-into card with the ALLOWED reframe");
        assert!(walked.detail.contains("swing in the opponent's favour"));
        assert!(
            walked.detail.to_lowercase().contains("discovered attack"),
            "{}",
            walked.detail
        );
    }

    #[test]
    fn walked_into_absolute_pin_draws_through_to_king_with_trigger() {
        use chess_tutor_engine::san;
        use chess_tutor_engine::types::{Move, Value};

        // Reported bug: after 11.Bxd4 (White, the user) the engine's reply
        // …Bf4 would pin White's queen (d2) to the king (c1) along
        // f4-e3-d2-c1. The card drew a lone f4→d2 arrow on the post-Bxd4
        // board — f4 empty, the bishop still on h2 — which read like a
        // *relative* pin (queen as the rear) standing on nothing. It must
        // instead draw the pin THROUGH the queen to the king, plus a trigger
        // arrow h2→f4 naming the move that springs it.
        let fen = "r1b1k2r/1pqp1ppp/p3pn2/8/3nP3/2N1BP2/PPPQB1Pb/2KR3R w - - 0 11";
        let mut pre = Position::from_fen(fen).unwrap();
        let bxd4 = san::parse(&mut pre, "Bxd4").unwrap();
        let pre = Position::from_fen(fen).unwrap();
        // The engine's line: 11.Bxd4 Bf4 12.Be3 Bxe3 13.Qxe3.
        let user_pv = vec![
            bxd4,
            Move::normal(Square::H2, Square::F4),
            Move::normal(Square::D4, Square::E3),
            Move::normal(Square::F4, Square::E3),
            Move::normal(Square::D2, Square::E3),
        ];
        let make = |pv: Vec<Move>| MoveAnalysis {
            mv: pv[0],
            score: Value(300), // ~+1.4 pawns, White still winning — no giveaway
            depth: 1,
            pv,
            ply_traces: Vec::new(),
            settled_ply: Some(0),
            pre_move_trace: chess_tutor_engine::eval::EvalTrace::zero(),
            pre_score: Value::ZERO,
            term_deltas: Vec::new(),
        };
        // Bxd4 is also the engine's best move, so best == user.
        let user = make(user_pv.clone());
        let best = make(user_pv);

        let items =
            build_tactic_items(&pre, &best, &user, Color::White, None, false, Perspective::Player);
        let pin = items
            .iter()
            .find(|it| it.heading.to_lowercase().contains("pin"))
            .expect("the walked-into pin card must still appear");

        // The trigger arrow: the bishop's springing move h2→f4 (gold).
        assert!(
            pin.annotations.iter().any(|a| matches!(
                a,
                BoardAnnotation::Arrow { from: Square::H2, to: Square::F4, kind: AnnotationKind::TriggerMove }
            )),
            "expected an h2→f4 trigger arrow, got {:?}",
            pin.annotations
        );
        // The pin line runs through the queen to the king (f4→c1).
        assert!(
            pin.annotations.iter().any(|a| matches!(
                a,
                BoardAnnotation::Arrow { from: Square::F4, to: Square::C1, kind: AnnotationKind::Attacker }
            )),
            "expected the pin line to run f4→(through d2)→c1, got {:?}",
            pin.annotations
        );
        // The pinned queen is flagged …
        assert!(pin.annotations.iter().any(|a| matches!(
            a,
            BoardAnnotation::SquareHighlight { square: Square::D2, kind: AnnotationKind::Threat }
        )));
        // … and crucially there is NO bare f4→d2 arrow stopping at the queen.
        assert!(
            !pin.annotations.iter().any(|a| matches!(
                a,
                BoardAnnotation::Arrow { to: Square::D2, .. }
            )),
            "the queen must not be the arrow's endpoint (that reads as a relative pin)"
        );
    }

    #[test]
    fn walked_into_discovered_attack_draws_real_attacker_and_trigger() {
        use chess_tutor_engine::types::{Move, Value};

        // Reported bug: after 16…Qxd6 (Black) White has Bxh7+ (d3→h7, check)
        // which unmasks the ROOK on d1 attacking the queen on d6 along the
        // d-file. The card drew a lone h7→d6 arrow — h7 still holds Black's
        // pawn on the displayed board, and h7 doesn't even attack d6 — which
        // is geometric nonsense. It must instead draw the discovery from the
        // real attacker (rook d1 → queen d6) plus a trigger arrow d3→h7
        // naming the move that springs it.
        let fen = "r1b2rk1/2q2ppp/p2Ppn2/1p6/8/2NBQP2/PPP3P1/2KR3R b - - 0 16";
        let pre = Position::from_fen(fen).unwrap();
        // The engine's line: 16…Qxd6 17.Bxh7+ Nxh7 18.Rxd6 (rook wins the queen).
        let user_pv = vec![
            Move::normal(Square::C7, Square::D6), // Qxd6
            Move::normal(Square::D3, Square::H7), // Bxh7+ — the discovering move
            Move::normal(Square::F6, Square::H7), // Nxh7
            Move::normal(Square::D1, Square::D6), // Rxd6 — rook collects the queen
        ];
        let ma = |pv: Vec<Move>, score: i32| MoveAnalysis {
            mv: pv[0],
            score: Value(score),
            depth: 1,
            pv,
            ply_traces: Vec::new(),
            settled_ply: Some(0),
            pre_move_trace: chess_tutor_engine::eval::EvalTrace::zero(),
            pre_score: Value::ZERO,
            term_deltas: Vec::new(),
        };
        // Scores are Black-POV (the mover): Qxd6 is losing, a clean giveaway.
        let user = ma(user_pv, -900);
        let best = ma(vec![Move::normal(Square::C7, Square::A7)], -100); // Qa7 holds

        let items =
            build_tactic_items(&pre, &best, &user, Color::Black, None, false, Perspective::Player);
        let card = items
            .iter()
            .find(|it| it.heading.to_lowercase().contains("discovered"))
            .expect("the walked-into discovered-attack card must appear");

        // Trigger arrow: the bishop's springing move d3→h7 (gold).
        assert!(
            card.annotations.iter().any(|a| matches!(
                a,
                BoardAnnotation::Arrow { from: Square::D3, to: Square::H7, kind: AnnotationKind::TriggerMove }
            )),
            "expected a d3→h7 trigger arrow, got {:?}",
            card.annotations
        );
        // The discovery itself: the unmasked rook d1 → the queen d6.
        assert!(
            card.annotations.iter().any(|a| matches!(
                a,
                BoardAnnotation::Arrow { from: Square::D1, to: Square::D6, kind: AnnotationKind::Attacker }
            )),
            "expected the discovery arrow rook d1 → queen d6, got {:?}",
            card.annotations
        );
        // The queen is the threatened piece …
        assert!(card.annotations.iter().any(|a| matches!(
            a,
            BoardAnnotation::SquareHighlight { square: Square::D6, kind: AnnotationKind::Threat }
        )));
        // … and crucially NO arrow originates from h7 (the moved piece, which
        // never lies on the attack line — the old, nonsensical rendering).
        assert!(
            !card.annotations.iter().any(|a| matches!(
                a,
                BoardAnnotation::Arrow { from: Square::H7, .. }
            )),
            "no arrow may start from the moved piece's square h7"
        );
    }

    #[test]
    fn annotations_include_arrow_per_target_plus_threat_highlights() {
        let anns = tactic_annotations(&fork_hit(), TacticRole::Played, None, None);
        // 1 primary highlight + 2 arrows + 2 target highlights = 5.
        assert_eq!(anns.len(), 5);
        let arrow_count = anns
            .iter()
            .filter(|a| matches!(a, BoardAnnotation::Arrow { .. }))
            .count();
        assert_eq!(arrow_count, 2);
    }

    #[test]
    fn annotations_skip_degenerate_arrow_when_primary_equals_target() {
        let hit = TacticHit {
            pattern: TacticPattern::HangingCapture,
            pv_ply: 0,
            primary_piece: Square::E5,
            targets: vec![Square::E5],
            material_gain: Some(300),
            confidence: Confidence::High,
            sacrifice: false,
            mate_pattern: None,
            key_move: None,
        };
        let anns = tactic_annotations(&hit, TacticRole::Played, None, None);
        // No degenerate arrow, but both highlights still emit.
        let arrow_count = anns
            .iter()
            .filter(|a| matches!(a, BoardAnnotation::Arrow { .. }))
            .count();
        assert_eq!(arrow_count, 0);
        let highlight_count = anns
            .iter()
            .filter(|a| matches!(a, BoardAnnotation::SquareHighlight { .. }))
            .count();
        assert_eq!(highlight_count, 2);
    }
}
