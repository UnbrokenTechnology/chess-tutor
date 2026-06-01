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
use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::Color;

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
        .map(|claim| tactic_item(claim, &ctx, reveal_best_moves))
        .collect()
}

/// Map a single [`Claim::Tactic`] to a [`RetrospectiveItem`]: prose from
/// the translator, structured surface (sentiment, chip, summary,
/// annotations) computed here.
fn tactic_item(claim: &Claim, ctx: &PhrasingContext, reveal_best_moves: bool) -> RetrospectiveItem {
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
        tactic_annotations(hit)
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

fn tactic_annotations(hit: &TacticHit) -> Vec<BoardAnnotation> {
    let mut out = Vec::new();
    // The primary piece — for a played tactic this is *our* piece doing
    // the work; for missed/walked-into it's the line's primary attacker.
    // GoodPiece tints from any POV; the card sentiment already tells the
    // student whether this is good news or bad news.
    out.push(BoardAnnotation::SquareHighlight {
        square: hit.primary_piece,
        kind: AnnotationKind::GoodPiece,
    });
    for &target in &hit.targets {
        // Skip degenerate arrows (capture pattern: primary == target).
        if target != hit.primary_piece {
            out.push(BoardAnnotation::Arrow {
                from: hit.primary_piece,
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
        let card = tactic_item(&played_claim(fork_hit()), &player_ctx(false), false);
        assert_eq!(card.heading, "You played a fork");
        assert_eq!(card.sentiment, Sentiment::Positive);
        assert_eq!(card.score_delta_pawns, Some(3.0));
    }

    #[test]
    fn missed_card_inverts_chip_and_drops_annotations_when_reveal_off() {
        let card = tactic_item(&missed_claim(fork_hit()), &player_ctx(false), false);
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
        let card = tactic_item(&missed_claim(fork_hit()), &player_ctx(true), true);
        assert!(!card.annotations.is_empty());
    }

    #[test]
    fn mate_suffix_only_for_default_surfaced_patterns() {
        let mut hit = fork_hit();
        hit.mate_pattern = Some(MatePattern::BackRank);
        let card = tactic_item(&played_claim(hit.clone()), &player_ctx(false), false);
        assert!(card.heading.ends_with("back-rank mate"));

        hit.mate_pattern = Some(MatePattern::Anastasia);
        let card = tactic_item(&played_claim(hit), &player_ctx(false), false);
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
        let card = tactic_item(&played_claim(hit), &player_ctx(false), false);
        assert_eq!(card.heading, "You played checkmate — back-rank mate");
        assert_eq!(card.summary, "forced mate");
    }

    #[test]
    fn walked_into_card_is_negative_with_warning_framing() {
        let card = tactic_item(&walked_claim(fork_hit()), &player_ctx(false), false);
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
        let card = tactic_item(&claim, &player_ctx(false), false);
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
        let card = tactic_item(&claim, &player_ctx(false), false);
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
        let card = tactic_item(&claim, &player_ctx(false), false);
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
    fn annotations_include_arrow_per_target_plus_threat_highlights() {
        let anns = tactic_annotations(&fork_hit());
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
        let anns = tactic_annotations(&hit);
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
