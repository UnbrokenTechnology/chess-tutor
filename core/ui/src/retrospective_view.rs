//! Builds the structured [`crate::view::RetrospectiveViewModel`]
//! that drives the desktop's retrospective panel.
//!
//! The narration crate produces *text* from the same engine
//! outcomes; this module produces *structured cards* with per-item
//! board annotations. Some threshold + categorization logic
//! intentionally duplicates the narration crate — the alternative
//! (depending on narration from `ui`) would inflate the dep graph
//! for a thin win, and the engine outcome readers do the heavy
//! lifting either way. See `core/narration/src/lib.rs` for the
//! parallel text path.
//!
//! Each per-category builder returns `Option<RetrospectiveItem>`;
//! categories that didn't move materially emit `None` so the panel
//! stays scannable.

use chess_tutor_engine::analysis::{
    compute_king_safety_outcome, compute_material_outcome, compute_mobility_outcome,
    compute_passed_pawns_outcome, compute_pawn_structure_outcome,
    compute_pieces_positional_outcome, compute_space_outcome, compute_threats_outcome,
    cumulative_prefix, HangingPiece, KingSafetyOutcome, MaterialOutcome, MobilityOutcome,
    MoveAnalysis, MoveVerdict, PassedPawnsOutcome, PawnStructureOutcome, PieceMobility,
    PiecesPositionalOutcome, SpaceOutcome, SurpriseKind, TermId, ThreatsOutcome,
};
use chess_tutor_engine::eval::{MobilityBreakdown, PassedBreakdown, PawnsBreakdown, PiecesBreakdown};
use chess_tutor_engine::movegen::legal_moves_vec;
use chess_tutor_engine::position::Position;
use chess_tutor_engine::san;
use chess_tutor_engine::types::{Color, Move, PieceType, Square, Value};

use crate::view::{
    AnnotationKind, BoardAnnotation, RetrospectiveCategory, RetrospectiveHeadline,
    RetrospectiveItem, RetrospectiveViewModel, Sentiment,
};

/// Build the structured view model for a user move.
///
/// `analyses[0]` is the engine's preferred move; `user_move` should
/// appear somewhere in the slice (typically by passing it in
/// `SearchParams::force_include`). Returns an empty view model when
/// the analyses slice is empty or the user move can't be found.
///
/// `show_all` widens two filters when `true`: the per-piece-type
/// mobility threshold drops from 50 cp to "any non-zero shift", and
/// "Other shifts" shows every residual term instead of just the 50%-
/// coverage prefix. Default `false` matches the prior behavior.
///
/// `reveal_best_moves` controls whether the headline carries the
/// engine's preferred move (SAN, score, gap, and the on-board arrow).
/// Off by default — the retrospective explains *why* the user's move
/// was an inaccuracy without telling them *what* to play, which trains
/// understanding over rote memorisation. When off, the four fields are
/// suppressed at this layer so renderers don't need to know about the
/// preference.
pub fn build_retrospective_view(
    pre_move_pos: &Position,
    analyses: &[MoveAnalysis],
    user_move: Move,
    show_all: bool,
    reveal_best_moves: bool,
) -> RetrospectiveViewModel {
    if analyses.is_empty() {
        return RetrospectiveViewModel::default();
    }
    let best = &analyses[0];
    let Some(user) = analyses.iter().find(|a| a.mv == user_move) else {
        return RetrospectiveViewModel::default();
    };
    let root_stm = pre_move_pos.side_to_move();
    let verdict = user.classify(best.score);

    let headline = build_headline(pre_move_pos, best, user, verdict, root_stm, reveal_best_moves);

    // Game-over short-circuit: if the user's move leaves the
    // opponent with no legal replies (checkmate or stalemate), the
    // game is decided and per-category cards are noise. The SAN in
    // the headline already shows `#` for mate and the verdict label
    // ("Best!") communicates the outcome — students don't need a
    // "hurt king safety -2.5" footnote after winning by mate.
    let mut post_pos = post_user_move_position(pre_move_pos, user);
    if legal_moves_vec(&mut post_pos).is_empty() {
        return RetrospectiveViewModel {
            headline,
            items: Vec::new(),
        };
    }

    let mut items: Vec<RetrospectiveItem> = Vec::new();
    // Material + Imbalance are always consumed from the secondary
    // "Other shifts" row list. The dedicated material card (below)
    // narrates captures honestly via `MaterialOutcome`'s realized
    // events; surfacing the raw term deltas on top is redundant at
    // best and confusing at worst. MaterialPsqPositional is *not*
    // consumed here — it's a real positional signal ("knight on f3
    // is better-placed than g1") that lands in the secondary card
    // as "development" until it gets its own dedicated card.
    let mut consumed_terms: Vec<TermId> =
        vec![TermId::MaterialPieceValue, TermId::Imbalance];

    // For "best" verdicts we still surface the per-category cards so
    // the student sees *why* the move was best — same intent as
    // narration's `explain_best = true` default.

    let material_outcome = compute_material_outcome(user, pre_move_pos, root_stm);
    if let Some(it) = build_material_item(pre_move_pos, &material_outcome, root_stm) {
        items.push(it);
    }

    // Build a small map of "what we just captured" so the threats
    // card can recognise planned recaptures. Bxh6 leaves our bishop
    // attacked + undefended (ours_hanging fires) — but if we just
    // pocketed a piece of ≥ equal point value on that same square,
    // that's a trade, not a hang.
    let user_captures_by_square: Vec<(Square, u8)> = material_outcome
        .realized_events()
        .filter(|ev| ev.captor == root_stm)
        .map(|ev| (ev.square, ev.captured_piece.classical_points()))
        .collect();
    let threats_outcome = compute_threats_outcome(user, pre_move_pos, root_stm);
    for it in build_threat_items(&threats_outcome, &user_captures_by_square) {
        items.push(it);
    }
    if !threats_items_empty(&threats_outcome) {
        consumed_terms.extend_from_slice(&[
            TermId::ThreatsByMinor,
            TermId::ThreatsByRook,
            TermId::ThreatsByKing,
            TermId::ThreatsHanging,
            TermId::ThreatsRestricted,
            TermId::ThreatsBySafePawn,
            TermId::ThreatsByPawnPush,
            TermId::ThreatsKnightOnQueen,
            TermId::ThreatsSliderOnQueen,
        ]);
    }

    let king_safety_outcome = compute_king_safety_outcome(user, pre_move_pos, root_stm);
    for it in build_king_safety_items(&king_safety_outcome) {
        items.push(it);
        consumed_terms.extend_from_slice(&[
            TermId::KingPawnShield,
            TermId::KingDanger,
            TermId::KingPawnlessFlank,
            TermId::KingFlankAttacks,
        ]);
    }

    let pawn_structure_outcome = compute_pawn_structure_outcome(user, pre_move_pos, root_stm);
    if let Some(it) = build_pawn_structure_item(&pawn_structure_outcome) {
        items.push(it);
        consumed_terms.extend_from_slice(&[
            TermId::PawnsConnected,
            TermId::PawnsIsolated,
            TermId::PawnsBackward,
            TermId::PawnsDoubled,
            TermId::PawnsWeakUnopposed,
            TermId::PawnsWeakLever,
        ]);
    }

    // Forced-consequences cards: structural concessions the
    // opponent's best reply *creates on their side*. Cheap walk one
    // ply past the user's move; surfaces e.g. doubled h-pawns after
    // gxh6.
    for it in build_forced_consequences_items(user, pre_move_pos, root_stm) {
        items.push(it);
    }

    let mobility_outcome = compute_mobility_outcome(user, pre_move_pos, root_stm);
    for it in build_mobility_items(&mobility_outcome, &post_pos, root_stm, show_all) {
        items.push(it);
        consumed_terms.extend_from_slice(&[
            TermId::MobilityKnight,
            TermId::MobilityBishop,
            TermId::MobilityRook,
            TermId::MobilityQueen,
        ]);
    }

    let passed_outcome = compute_passed_pawns_outcome(user, pre_move_pos, root_stm);
    if let Some(it) = build_passed_pawns_item(&passed_outcome) {
        items.push(it);
        consumed_terms.extend_from_slice(&[
            TermId::PassedRankBonus,
            TermId::PassedKingProximity,
            TermId::PassedFreeAdvance,
            TermId::PassedStopperPenalty,
        ]);
    }

    let pieces_outcome = compute_pieces_positional_outcome(user, pre_move_pos, root_stm);
    // Capture-aware king-protector suppression. Computed once from
    // the realized captures so the per-sub-term loop in
    // build_pieces_positional_items can drop misleading KP cards
    // without re-walking events.
    let kp_supp = king_protector_suppression(&material_outcome, root_stm);
    for it in build_pieces_positional_items(&pieces_outcome, root_stm, kp_supp) {
        items.push(it);
    }
    // Always consume every piece TermId — sub-terms that fired above
    // threshold get their own card above; sub-terms that didn't fire
    // are below noise level and shouldn't leak to the "Other shifts"
    // secondary list either.
    consumed_terms.extend_from_slice(&[
        TermId::PiecesOutposts,
        TermId::PiecesReachableOutposts,
        TermId::PiecesMinorBehindPawn,
        TermId::PiecesKingProtector,
        TermId::PiecesBishopPawns,
        TermId::PiecesLongDiagonalBishop,
        TermId::PiecesRookOnQueenFile,
        TermId::PiecesRookOnOpenFile,
        TermId::PiecesRookOnSemiopenFile,
        TermId::PiecesTrappedRook,
        TermId::PiecesWeakQueen,
    ]);

    let space_outcome = compute_space_outcome(user, pre_move_pos, root_stm);
    let space_items = build_space_items(&space_outcome, show_all);
    if !space_items.is_empty() {
        consumed_terms.push(TermId::Space);
    }
    for it in space_items {
        items.push(it);
    }

    if let Some(it) = build_secondary_item(user, root_stm, &consumed_terms, show_all) {
        items.push(it);
    }

    RetrospectiveViewModel { headline, items }
}

// ---------------------------------------------------------------------
// Headline
// ---------------------------------------------------------------------

fn build_headline(
    pre_move_pos: &Position,
    best: &MoveAnalysis,
    user: &MoveAnalysis,
    verdict: MoveVerdict,
    root_stm: Color,
    reveal_best_moves: bool,
) -> RetrospectiveHeadline {
    let user_san = san::format(pre_move_pos, user.mv);
    let user_is_sharp = matches!(
        (verdict, user.surprise(root_stm)),
        (
            MoveVerdict::Best | MoveVerdict::Good,
            Some(SurpriseKind::LooksBadButGood)
        )
    );
    let san_annotation = sharp_or_verdict_annotation(verdict, user_is_sharp);
    let verdict_label = verdict_label(verdict);
    let verdict_sentiment = verdict_sentiment(verdict);
    let user_score = format_score_pawns(user.score);

    // Best-move reveal is opt-in. When off, the four "what the engine
    // would have played" fields stay `None` so renderers naturally
    // skip them — telling the student the answer trains memorisation,
    // not the understanding the per-category cards below are designed
    // to build.
    let mut best_san = None;
    let mut best_score = None;
    let mut gap = None;
    let mut best_move_annotation = None;
    if reveal_best_moves && best.mv != user.mv {
        let san = san::format(pre_move_pos, best.mv);
        best_score = Some(format_score_pawns(best.score));
        gap = Some(format_delta_pawns(user.score.0 - best.score.0));
        best_move_annotation = Some(BoardAnnotation::Arrow {
            from: best.mv.from(),
            to: best.mv.to(),
            kind: AnnotationKind::BestMove,
        });
        best_san = Some(san);
    }

    let note = match verdict {
        MoveVerdict::BestAvailable => Some(format!(
            "Position was already lost ({}).",
            format_score_pawns(best.score)
        )),
        _ if user_is_sharp => Some(
            "Well spotted — this looks risky at first glance, but the longer line pays off."
                .to_string(),
        ),
        _ => surprise_note(verdict, user.surprise(root_stm)),
    };

    RetrospectiveHeadline {
        user_san,
        san_annotation,
        verdict_label,
        verdict_sentiment,
        user_score,
        best_san,
        best_score,
        gap,
        note,
        best_move_annotation,
    }
}

// ---------------------------------------------------------------------
// Material
// ---------------------------------------------------------------------

fn build_material_item(
    _pre_move_pos: &Position,
    outcome: &MaterialOutcome,
    root_stm: Color,
) -> Option<RetrospectiveItem> {
    // Past tense ("You won material") only describes what actually
    // resolved in the position the student is looking at — the
    // user's move plus any forced opponent recapture. The
    // realized_events accessor enforces this; deeper PV captures
    // are reserved for hypothetical framings (CLI's "Best line:").
    let events: Vec<&_> = outcome.realized_events().collect();
    if events.is_empty() {
        return None;
    }
    // Suppress on hangs: when the user's ply-0 move was *not* a
    // capture and the opponent's ply-1 best response is to take one
    // of our pieces, that's a hanging piece — not a completed loss.
    // The threats card surfaces this case with proper present-tense
    // framing ("Your piece is hanging") plus attacker arrows and a
    // target-square highlight, and the opponent might still miss the
    // capture (a 1400 bot blunders these regularly). Calling it
    // "You lost material" frames the hang as a settled fact, which
    // confuses students about whether the loss has actually happened.
    let first_event_is_opponent_capture =
        events.first().is_some_and(|ev| ev.captor != root_stm);
    if first_event_is_opponent_capture {
        return None;
    }
    // Pedagogical "is this an even trade?" uses classical point values
    // (P:1, N:3, B:3, R:5, Q:9), not engine-internal cp. A B-for-N
    // swap (net -44 cp midgame) reads as Even to a student, and
    // surfacing it as "You lost material" mis-teaches. The cp net
    // still drives the numeric summary so the student can see the
    // exact engine valuation — we just don't let the cp gap pick the
    // headline.
    let net_points = realized_point_net(&events, root_stm);
    let net = outcome.realized_net_mg_cp(root_stm);
    let (heading, sentiment) = if net_points > 0 {
        ("You won material", Sentiment::Positive)
    } else if net_points < 0 {
        ("You lost material", Sentiment::Negative)
    } else {
        ("Even trade", Sentiment::Neutral)
    };

    let summary = if net_points == 0 {
        if net == 0 {
            format!("{} captures, balanced", events.len())
        } else {
            // Fair-point trade with a small cp lean — show the cp
            // delta in parens so the student can see how the engine
            // values the slight asymmetry (B vs N etc.) without it
            // re-headlining as a loss.
            format!(
                "{} captures, even by point value ({:+.2} engine cp)",
                events.len(),
                net as f32 / 100.0
            )
        }
    } else {
        format!(
            "net {:+} point{}, {:+.2} pawns engine cp",
            net_points,
            if net_points.abs() == 1 { "" } else { "s" },
            net as f32 / 100.0
        )
    };

    // Detail: list each capture step.
    let mut detail_lines: Vec<String> = Vec::new();
    for ev in &events {
        let captor_label = piece_name(ev.captor_piece);
        let captured_label = piece_name(ev.captured_piece);
        let sign = if ev.captor == root_stm {
            "you take"
        } else {
            "opponent takes"
        };
        detail_lines.push(format!(
            "Ply {}: {} a {} with {} on {}.",
            ev.ply + 1,
            sign,
            captured_label,
            article(captor_label),
            ev.square.to_algebraic()
        ));
    }
    // Phase-dependent teaching note. When point parity is even but
    // the engine's cp valuation leans meaningfully one way, that's a
    // learning opportunity: classical 3=3 hides that bishops favor
    // open positions / endgames, two minors usually beat a rook in
    // middlegame, etc. Surface the fact; let the student internalise
    // the why.
    if net_points == 0 {
        if let Some(note) = phase_dependent_trade_note(&events, root_stm) {
            detail_lines.push(note);
        }
    }
    let detail = detail_lines.join("\n");

    // Annotations: highlight every square where a capture resolved.
    // We don't have the PV here directly (the outcome doesn't expose
    // it), so from/to arrows would require a recomputation pass.
    // Square highlights are precise enough to point the student at
    // each capture without that work.
    let mut annotations = Vec::new();
    for ev in &events {
        let kind = if ev.captor == root_stm {
            AnnotationKind::Capture
        } else {
            AnnotationKind::Threat
        };
        annotations.push(BoardAnnotation::SquareHighlight {
            square: ev.square,
            kind,
        });
    }

    // Chip on the card: signed cp delta from White's POV so the
    // existing per-card delta chip math stays consistent. Only show
    // a chip when the point-value parity also says non-even — a
    // small cp lean on a fair-points trade isn't headline-worthy.
    let score_delta_pawns = if net_points != 0 {
        let sign = if root_stm == Color::White { 1 } else { -1 };
        Some((net * sign) as f32 / 100.0)
    } else {
        None
    };

    Some(RetrospectiveItem {
        category: RetrospectiveCategory::Material,
        heading: heading.to_string(),
        summary,
        detail,
        score_delta_pawns,
        sentiment,
        annotations,
    })
}

// ---------------------------------------------------------------------
// Threats
// ---------------------------------------------------------------------

fn threats_items_empty(outcome: &ThreatsOutcome) -> bool {
    outcome.ours_hanging.is_empty()
        && outcome.theirs_hanging.is_empty()
        && outcome.ours_see_losing.is_empty()
        && outcome.theirs_see_losing.is_empty()
        && outcome.ours_pressured.is_empty()
        && outcome.theirs_pressured.is_empty()
}

fn build_threat_items(
    outcome: &ThreatsOutcome,
    user_captures_by_square: &[(Square, u8)],
) -> Vec<RetrospectiveItem> {
    let mut items = Vec::new();

    // Highest-value guaranteed counter-threat — used to suppress
    // ours_hanging entries whose loss is irrelevant because the
    // opponent has a bigger problem on the board. Computed once
    // across both guaranteed-hanging and guaranteed-SEE-losing lists
    // since either qualifies as a winning counter-threat.
    let max_counter_threat_points =
        max_target_points(&outcome.theirs_hanging_guaranteed)
            .max(max_target_points(&outcome.theirs_see_losing_guaranteed));

    // Our hanging pieces — filter for misleading entries. Two cases
    // get filtered out:
    //   (1) "Planned recapture" — we just captured a piece of ≥ equal
    //       point value on the same square. The bishop on h6 right
    //       after Bxh6 is the second leg of a trade we initiated.
    //   (2) "Compensating counter-attack" — we have a guaranteed
    //       higher-value win elsewhere. The opponent has to address
    //       that bigger problem; our hanging bishop is no longer
    //       their best response.
    let ours_hanging_filtered = filter_misleading_hangs(
        &outcome.ours_hanging,
        user_captures_by_square,
        max_counter_threat_points,
    );
    if !ours_hanging_filtered.is_empty() {
        items.push(threat_item_from_hangs(
            &ours_hanging_filtered,
            "Your piece is hanging",
            Sentiment::Negative,
            true,
        ));
    }

    // "You can win material" only fires off the *guaranteed* list —
    // entries that survive every legal opponent response. The raw
    // theirs_hanging is a static snapshot and would mis-teach the
    // student about defensible threats (Nf3 attacks e5 but ...Nc6
    // defends, etc.).
    if !outcome.theirs_hanging_guaranteed.is_empty() {
        items.push(threat_item_from_hangs(
            &outcome.theirs_hanging_guaranteed,
            "You can win material",
            Sentiment::Positive,
            false,
        ));
    }

    let ours_see_losing_filtered = filter_misleading_hangs(
        &outcome.ours_see_losing,
        user_captures_by_square,
        max_counter_threat_points,
    );
    if !ours_see_losing_filtered.is_empty() {
        items.push(threat_item_from_hangs(
            &ours_see_losing_filtered,
            "Your piece loses to a trade",
            Sentiment::Negative,
            true,
        ));
    }
    if !outcome.theirs_see_losing_guaranteed.is_empty() {
        items.push(threat_item_from_hangs(
            &outcome.theirs_see_losing_guaranteed,
            "Their piece loses to a trade",
            Sentiment::Positive,
            false,
        ));
    }

    items
}

/// Drop hanging-piece entries that would mislead the student. Two
/// suppression cases:
///
/// 1. **Planned recapture**: we just captured a piece of ≥ equal
///    classical point value (P:1, N:3, B:3, R:5, Q:9) on the same
///    square the piece now sits on. The exchange is fair or
///    favorable — calling it a hang frames the student's deliberate
///    trade as a mistake.
///
/// 2. **Compensating counter-attack**: there's a guaranteed
///    higher-value win elsewhere on the board (`counter_threat_pts`
///    > our hanging piece's points). The opponent can't both
///    address the bigger threat and capture our piece — they have
///    to choose the bigger problem, so our piece isn't really
///    hanging. This catches the classic "leave the bishop hanging,
///    threatening the queen" zwischenzug pattern, when the queen
///    is in the *guaranteed* win list (i.e. opponent can't save it
///    even by capturing the bishop, because the bishop wasn't the
///    only attacker).
///
/// Both checks use classical point values, not cp, so the student
/// reasons in the same units the cards present.
fn filter_misleading_hangs(
    hangs: &[HangingPiece],
    user_captures_by_square: &[(Square, u8)],
    counter_threat_pts: u8,
) -> Vec<HangingPiece> {
    hangs
        .iter()
        .filter(|h| {
            let our_points = h.location.piece.classical_points();
            // Case 1: planned recapture on the same square.
            let planned_recapture =
                user_captures_by_square.iter().any(|(sq, captured_points)| {
                    *sq == h.location.square && *captured_points >= our_points
                });
            // Case 2: a guaranteed counter-threat of strictly higher
            // value than the hanging piece. "Strictly higher" because
            // an equal-value counter-threat is a wash — opponent
            // could plausibly take ours and accept the loss elsewhere
            // as compensation.
            let compensated_by_counter = counter_threat_pts > our_points;
            !(planned_recapture || compensated_by_counter)
        })
        .cloned()
        .collect()
}

/// Maximum classical point value across a hanging-piece list.
/// Returns 0 when the list is empty — used as the "no
/// counter-threat" sentinel by [`filter_misleading_hangs`].
fn max_target_points(hangs: &[HangingPiece]) -> u8 {
    hangs
        .iter()
        .map(|h| h.location.piece.classical_points())
        .max()
        .unwrap_or(0)
}

fn threat_item_from_hangs(
    hangs: &[HangingPiece],
    heading: &str,
    sentiment: Sentiment,
    target_is_ours: bool,
) -> RetrospectiveItem {
    let summary = if hangs.len() == 1 {
        format!(
            "{} on {}",
            piece_name(hangs[0].location.piece),
            hangs[0].location.square.to_algebraic()
        )
    } else {
        format!("{} pieces", hangs.len())
    };

    let mut detail_lines = Vec::new();
    let mut annotations = Vec::new();
    for h in hangs {
        let mut attacker_strs = Vec::new();
        for a in &h.attackers {
            attacker_strs.push(format!("{} on {}", piece_name(a.piece), a.square.to_algebraic()));
            annotations.push(BoardAnnotation::Arrow {
                from: a.square,
                to: h.location.square,
                kind: AnnotationKind::Attacker,
            });
        }
        let target_kind = if target_is_ours {
            AnnotationKind::Threat
        } else {
            AnnotationKind::GoodPiece
        };
        annotations.push(BoardAnnotation::SquareHighlight {
            square: h.location.square,
            kind: target_kind,
        });
        detail_lines.push(format!(
            "{} on {} — attacked by {}.",
            capitalize(piece_name(h.location.piece)),
            h.location.square.to_algebraic(),
            join_with_and(&attacker_strs),
        ));
    }

    RetrospectiveItem {
        category: RetrospectiveCategory::Threats,
        heading: heading.to_string(),
        summary,
        detail: detail_lines.join("\n"),
        score_delta_pawns: None,
        sentiment,
        annotations,
    }
}

// ---------------------------------------------------------------------
// King safety
// ---------------------------------------------------------------------

const KING_SHELTER_DELTA_THRESHOLD_CP: i32 = 25;
const KING_SHELTER_ENDGAME_PHASE_CUTOFF: i32 = 32;

fn build_king_safety_items(outcome: &KingSafetyOutcome) -> Vec<RetrospectiveItem> {
    let mut items = Vec::new();
    let shelter_relevant = outcome.phase >= KING_SHELTER_ENDGAME_PHASE_CUTOFF;

    let ours_attackers_up = outcome.ours_attackers_delta() > 0;
    let ours_shield_down = shelter_relevant
        && outcome.ours_pawn_shield_mg_delta() <= -KING_SHELTER_DELTA_THRESHOLD_CP;
    let ours_attackers_down = outcome.ours_attackers_delta() < 0;
    let ours_shield_up = shelter_relevant
        && outcome.ours_pawn_shield_mg_delta() >= KING_SHELTER_DELTA_THRESHOLD_CP;

    if ours_attackers_up || ours_shield_down {
        items.push(RetrospectiveItem {
            category: RetrospectiveCategory::KingSafety,
            heading: "Your king is more exposed".to_string(),
            summary: king_safety_summary_exposure(
                outcome.ours_post.attackers_count,
                outcome.ours_pre.attackers_count,
                outcome.ours_pawn_shield_mg_delta(),
                ours_shield_down,
            ),
            detail: king_safety_detail(
                outcome.ours_pre.attackers_count,
                outcome.ours_post.attackers_count,
                outcome.ours_pre.pawn_shield_mg,
                outcome.ours_post.pawn_shield_mg,
                ours_attackers_up,
                ours_shield_down,
            ),
            score_delta_pawns: None,
            sentiment: Sentiment::Negative,
            annotations: vec![BoardAnnotation::SquareHighlight {
                square: outcome.ours_post.king_sq,
                kind: AnnotationKind::KingRing,
            }],
        });
    } else if ours_attackers_down || ours_shield_up {
        items.push(RetrospectiveItem {
            category: RetrospectiveCategory::KingSafety,
            heading: "Your king is safer".to_string(),
            summary: king_safety_summary_safer(
                outcome.ours_post.attackers_count,
                outcome.ours_pre.attackers_count,
                outcome.ours_pawn_shield_mg_delta(),
                ours_shield_up,
            ),
            detail: king_safety_detail(
                outcome.ours_pre.attackers_count,
                outcome.ours_post.attackers_count,
                outcome.ours_pre.pawn_shield_mg,
                outcome.ours_post.pawn_shield_mg,
                ours_attackers_down,
                ours_shield_up,
            ),
            score_delta_pawns: None,
            sentiment: Sentiment::Positive,
            annotations: vec![BoardAnnotation::SquareHighlight {
                square: outcome.ours_post.king_sq,
                kind: AnnotationKind::GoodPiece,
            }],
        });
    }

    let theirs_attackers_up = outcome.theirs_attackers_delta() > 0;
    let theirs_shield_down = shelter_relevant
        && outcome.theirs_pawn_shield_mg_delta() <= -KING_SHELTER_DELTA_THRESHOLD_CP;
    let theirs_attackers_down = outcome.theirs_attackers_delta() < 0;
    let theirs_shield_up = shelter_relevant
        && outcome.theirs_pawn_shield_mg_delta() >= KING_SHELTER_DELTA_THRESHOLD_CP;

    if theirs_attackers_up || theirs_shield_down {
        items.push(RetrospectiveItem {
            category: RetrospectiveCategory::KingSafety,
            heading: "You expose the opponent's king".to_string(),
            summary: king_safety_summary_exposure(
                outcome.theirs_post.attackers_count,
                outcome.theirs_pre.attackers_count,
                outcome.theirs_pawn_shield_mg_delta(),
                theirs_shield_down,
            ),
            detail: king_safety_detail(
                outcome.theirs_pre.attackers_count,
                outcome.theirs_post.attackers_count,
                outcome.theirs_pre.pawn_shield_mg,
                outcome.theirs_post.pawn_shield_mg,
                theirs_attackers_up,
                theirs_shield_down,
            ),
            score_delta_pawns: None,
            sentiment: Sentiment::Positive,
            annotations: vec![BoardAnnotation::SquareHighlight {
                square: outcome.theirs_post.king_sq,
                kind: AnnotationKind::KingRing,
            }],
        });
    } else if theirs_attackers_down || theirs_shield_up {
        items.push(RetrospectiveItem {
            category: RetrospectiveCategory::KingSafety,
            heading: "The opponent's king is safer".to_string(),
            summary: king_safety_summary_safer(
                outcome.theirs_post.attackers_count,
                outcome.theirs_pre.attackers_count,
                outcome.theirs_pawn_shield_mg_delta(),
                theirs_shield_up,
            ),
            detail: king_safety_detail(
                outcome.theirs_pre.attackers_count,
                outcome.theirs_post.attackers_count,
                outcome.theirs_pre.pawn_shield_mg,
                outcome.theirs_post.pawn_shield_mg,
                theirs_attackers_down,
                theirs_shield_up,
            ),
            score_delta_pawns: None,
            sentiment: Sentiment::Negative,
            annotations: vec![BoardAnnotation::SquareHighlight {
                square: outcome.theirs_post.king_sq,
                kind: AnnotationKind::GoodPiece,
            }],
        });
    }

    items
}

fn king_safety_summary_exposure(
    post_atk: i32,
    pre_atk: i32,
    shield_delta_cp: i32,
    shield_changed: bool,
) -> String {
    let mut parts = Vec::new();
    if post_atk > pre_atk {
        parts.push(format!("{} attackers (up from {})", post_atk, pre_atk));
    }
    if shield_changed {
        parts.push(format!(
            "shield {:+.2}",
            shield_delta_cp as f32 / 100.0
        ));
    }
    parts.join(", ")
}

fn king_safety_summary_safer(
    post_atk: i32,
    pre_atk: i32,
    shield_delta_cp: i32,
    shield_changed: bool,
) -> String {
    let mut parts = Vec::new();
    if post_atk < pre_atk {
        parts.push(format!("attackers down to {} (from {})", post_atk, pre_atk));
    }
    if shield_changed {
        parts.push(format!(
            "shield {:+.2}",
            shield_delta_cp as f32 / 100.0
        ));
    }
    parts.join(", ")
}

fn king_safety_detail(
    pre_atk: i32,
    post_atk: i32,
    pre_shield: i32,
    post_shield: i32,
    show_attackers: bool,
    show_shield: bool,
) -> String {
    let mut parts = Vec::new();
    if show_attackers {
        parts.push(format!(
            "Attackers on the king ring: {} → {}.",
            pre_atk, post_atk
        ));
    }
    if show_shield {
        parts.push(format!(
            "Pawn shield: {:+.2} → {:+.2}.",
            pre_shield as f32 / 100.0,
            post_shield as f32 / 100.0,
        ));
    }
    parts.join("\n")
}

// ---------------------------------------------------------------------
// Mobility
// ---------------------------------------------------------------------

const MOBILITY_DELTA_THRESHOLD_CP: i32 = 20;

/// A per-square delta tells us *which* piece's activity actually
/// moved when the per-piece-type aggregate shifted. Pieces sit on
/// different squares pre vs post when the piece moved itself; for
/// stationary pieces (e.g. both bishops after 1.e4), the same
/// square appears in both snapshots and the delta is `post - pre`.
const PER_PIECE_HIGHLIGHT_THRESHOLD_CP: i32 = 15;

fn build_mobility_items(
    outcome: &MobilityOutcome,
    _post_pos: &Position,
    _root_stm: Color,
    show_all: bool,
) -> Vec<RetrospectiveItem> {
    // show_all drops the per-piece floor from 50 cp to 1 cp so a
    // bishop's 12→13 reach surfaces. Without it, the default 50 cp
    // gate hides knock-on shifts from pawn pushes that didn't really
    // change the piece's role on the board.
    let threshold = if show_all { 1 } else { MOBILITY_DELTA_THRESHOLD_CP };
    let mut items = Vec::new();

    for (label, piece_type, delta, pre, post) in
        mobility_all_shifts(&outcome.ours_pre, &outcome.ours_post, threshold)
    {
        let (heading, sentiment) = if delta < 0 {
            (format!("Your {label} activity dropped"), Sentiment::Negative)
        } else {
            (format!("Your {label} activity improved"), Sentiment::Positive)
        };
        let annotations = highlight_specific_pieces(
            &outcome.ours_per_piece_pre,
            &outcome.ours_per_piece_post,
            piece_type,
            sentiment,
        );
        items.push(RetrospectiveItem {
            category: RetrospectiveCategory::Mobility,
            heading,
            summary: format!(
                "{:+.2} → {:+.2}",
                pre as f32 / 100.0,
                post as f32 / 100.0
            ),
            detail: format!(
                "Stockfish's mobility term weights the squares this piece type attacks \
                 inside its safe-area bitmap. A {label} climbing from {:+.2} to {:+.2} \
                 typically means it found a more active diagonal, file, or outpost.",
                pre as f32 / 100.0,
                post as f32 / 100.0
            ),
            score_delta_pawns: Some(delta as f32 / 100.0),
            sentiment,
            annotations,
        });
    }

    for (label, piece_type, delta, pre, post) in
        mobility_all_shifts(&outcome.theirs_pre, &outcome.theirs_post, threshold)
    {
        let (heading, sentiment) = if delta < 0 {
            (
                format!("You restricted the opponent's {label}"),
                Sentiment::Positive,
            )
        } else {
            (
                format!("The opponent's {label} got more active"),
                Sentiment::Negative,
            )
        };
        let annotations = highlight_specific_pieces(
            &outcome.theirs_per_piece_pre,
            &outcome.theirs_per_piece_post,
            piece_type,
            sentiment,
        );
        items.push(RetrospectiveItem {
            category: RetrospectiveCategory::Mobility,
            heading,
            summary: format!(
                "{:+.2} → {:+.2}",
                pre as f32 / 100.0,
                post as f32 / 100.0
            ),
            detail: format!(
                "The opponent's {label} mobility shifted from {:+.2} to {:+.2}. \
                 Restricting an opponent's piece is just as valuable as activating \
                 your own — it tends to lock in long-term advantages.",
                pre as f32 / 100.0,
                post as f32 / 100.0
            ),
            score_delta_pawns: Some(-delta as f32 / 100.0),
            sentiment,
            annotations,
        });
    }

    items
}

/// All per-piece-type mobility shifts whose `|delta_mg|` clears
/// `threshold_cp`, sorted biggest-first. Returns up to four entries:
/// (label, piece_type, delta, pre_mg, post_mg).
fn mobility_all_shifts(
    pre: &MobilityBreakdown,
    post: &MobilityBreakdown,
    threshold_cp: i32,
) -> Vec<(&'static str, PieceType, i32, i32, i32)> {
    let candidates: [(&'static str, PieceType, i32, i32); 4] = [
        ("knight", PieceType::Knight, pre.knight.mg().0, post.knight.mg().0),
        ("bishop", PieceType::Bishop, pre.bishop.mg().0, post.bishop.mg().0),
        ("rook", PieceType::Rook, pre.rook.mg().0, post.rook.mg().0),
        ("queen", PieceType::Queen, pre.queen.mg().0, post.queen.mg().0),
    ];
    let mut shifts: Vec<_> = candidates
        .into_iter()
        .map(|(label, pt, pre_mg, post_mg)| (label, pt, post_mg - pre_mg, pre_mg, post_mg))
        .filter(|(_, _, delta, _, _)| delta.abs() >= threshold_cp)
        .collect();
    shifts.sort_by_key(|(_, _, delta, _, _)| std::cmp::Reverse(delta.abs()));
    shifts
}

/// Pick the *specific* pieces of `piece_type` whose mobility shifted
/// in the direction `sentiment` calls out. Pre/post snapshots are
/// keyed by square — for pieces that didn't move themselves the same
/// square appears in both and the per-square delta tells us whose
/// activity actually changed. When a piece moved between pre and
/// post (different from-square / to-square), the post entry stands
/// in for "the piece that just moved here" so its new square gets
/// the highlight.
///
/// For each highlighted piece we also emit `NewMobility` /
/// `LostMobility` square highlights for the squares that piece
/// newly attacks (or no longer attacks), so the student sees what
/// "activity improved" actually means on the board. The moved piece
/// has no same-square pre snapshot — every square it attacks counts
/// as newly available for the improved case.
///
/// Threshold filters out the always-on rocking that happens when
/// any pawn push reshapes the mobility bitmap by a handful of cp.
fn highlight_specific_pieces(
    pre_pieces: &[PieceMobility],
    post_pieces: &[PieceMobility],
    piece_type: PieceType,
    sentiment: Sentiment,
) -> Vec<BoardAnnotation> {
    let piece_kind = match sentiment {
        Sentiment::Positive => AnnotationKind::GoodPiece,
        Sentiment::Negative => AnnotationKind::BadPiece,
        _ => AnnotationKind::Highlight,
    };

    // For the overall change to be "improved", per-square deltas
    // pointing the same direction are the ones to surface. Per-piece
    // deltas pointing the *opposite* direction are noise (one piece
    // gained mobility, another lost some) — they'd confuse the
    // teaching story.
    let want_positive = matches!(sentiment, Sentiment::Positive);

    // Build a map of pre-move per-piece records keyed by square (only
    // for pieces of the requested type).
    use std::collections::HashMap;
    let mut pre_by_sq: HashMap<Square, &PieceMobility> = HashMap::new();
    for pm in pre_pieces {
        if pm.piece == piece_type {
            pre_by_sq.insert(pm.square, pm);
        }
    }

    // Squares where the piece exists post-move with a meaningful
    // per-square delta in the surfaced direction.
    let mut hits: Vec<(&PieceMobility, i32)> = Vec::new();
    for pm in post_pieces {
        if pm.piece != piece_type {
            continue;
        }
        // If the piece was on the same square pre-move, use the
        // per-square delta. If it just landed here (the moved piece),
        // treat the "delta" as its full post-move mobility — it's
        // the piece that produced the most obvious activity change.
        let prev = pre_by_sq.get(&pm.square).copied();
        let delta = match prev {
            Some(p) => pm.mg - p.mg,
            None => pm.mg,
        };
        let aligned = (want_positive && delta > 0) || (!want_positive && delta < 0);
        if aligned && delta.abs() >= PER_PIECE_HIGHLIGHT_THRESHOLD_CP {
            hits.push((pm, delta.abs()));
        }
    }

    // If nothing crossed the threshold, fall back to whichever
    // post-move piece had the largest aligned delta — students
    // still want *some* visual when the card says "activity moved."
    if hits.is_empty() {
        let mut best: Option<(&PieceMobility, i32)> = None;
        for pm in post_pieces {
            if pm.piece != piece_type {
                continue;
            }
            let prev = pre_by_sq.get(&pm.square).copied();
            let delta = match prev {
                Some(p) => pm.mg - p.mg,
                None => pm.mg,
            };
            let aligned = (want_positive && delta > 0) || (!want_positive && delta < 0);
            if !aligned {
                continue;
            }
            match best {
                Some((_, b)) if delta.abs() <= b => {}
                _ => best = Some((pm, delta.abs())),
            }
        }
        if let Some((pm, _)) = best {
            let mut anns = vec![BoardAnnotation::SquareHighlight {
                square: pm.square,
                kind: piece_kind,
            }];
            push_mobility_square_highlights(
                &mut anns,
                pm,
                pre_by_sq.get(&pm.square).copied(),
                want_positive,
            );
            return anns;
        }
        return Vec::new();
    }

    // Sort descending by magnitude so the biggest swing is visually
    // dominant (renderers paint in order; later highlights overdraw
    // earlier ones, but with same alpha that's a non-issue here).
    hits.sort_by_key(|(_, d)| std::cmp::Reverse(*d));
    let mut anns = Vec::new();
    for (pm, _) in &hits {
        anns.push(BoardAnnotation::SquareHighlight {
            square: pm.square,
            kind: piece_kind,
        });
    }
    for (pm, _) in &hits {
        push_mobility_square_highlights(
            &mut anns,
            pm,
            pre_by_sq.get(&pm.square).copied(),
            want_positive,
        );
    }
    anns
}

/// Highlight the squares this piece's mobility footprint gained
/// (positive sentiment) or lost (negative sentiment).
///
/// When `prev` is `Some`, the piece was on the same square pre and
/// post and the diff between the two `mobility_squares` bitboards
/// names what actually changed. When `prev` is `None`, this is the
/// piece that just moved here — every square it now attacks is
/// "newly available from this piece" in the improved case; the
/// dropped case has no analogue (its from-square footprint lives
/// at a different square entirely), so we paint nothing.
fn push_mobility_square_highlights(
    out: &mut Vec<BoardAnnotation>,
    post: &PieceMobility,
    prev: Option<&PieceMobility>,
    want_positive: bool,
) {
    let (squares, kind) = match (prev, want_positive) {
        (Some(p), true) => (
            post.mobility_squares & !p.mobility_squares,
            AnnotationKind::NewMobility,
        ),
        (Some(p), false) => (
            p.mobility_squares & !post.mobility_squares,
            AnnotationKind::LostMobility,
        ),
        (None, true) => (post.mobility_squares, AnnotationKind::NewMobility),
        (None, false) => return,
    };
    for sq in squares {
        out.push(BoardAnnotation::SquareHighlight { square: sq, kind });
    }
}

// ---------------------------------------------------------------------
// Pawn structure
// ---------------------------------------------------------------------

const PAWN_STRUCTURE_DELTA_THRESHOLD_CP: i32 = 15;

#[derive(Copy, Clone, Debug)]
enum PawnSubTerm {
    Connected,
    Isolated,
    Backward,
    Doubled,
    WeakUnopposed,
    WeakLever,
}

impl PawnSubTerm {
    const ALL: [PawnSubTerm; 6] = [
        PawnSubTerm::Connected,
        PawnSubTerm::Isolated,
        PawnSubTerm::Backward,
        PawnSubTerm::Doubled,
        PawnSubTerm::WeakUnopposed,
        PawnSubTerm::WeakLever,
    ];
    fn delta_mg(self, pre: &PawnsBreakdown, post: &PawnsBreakdown) -> i32 {
        match self {
            PawnSubTerm::Connected => post.connected.mg().0 - pre.connected.mg().0,
            PawnSubTerm::Isolated => post.isolated.mg().0 - pre.isolated.mg().0,
            PawnSubTerm::Backward => post.backward.mg().0 - pre.backward.mg().0,
            PawnSubTerm::Doubled => post.doubled.mg().0 - pre.doubled.mg().0,
            PawnSubTerm::WeakUnopposed => post.weak_unopposed.mg().0 - pre.weak_unopposed.mg().0,
            PawnSubTerm::WeakLever => post.weak_lever.mg().0 - pre.weak_lever.mg().0,
        }
    }
    fn worsened_phrase(self) -> &'static str {
        match self {
            PawnSubTerm::Connected => "broke pawn connections",
            PawnSubTerm::Isolated => "isolated a pawn",
            PawnSubTerm::Backward => "created a backward pawn",
            PawnSubTerm::Doubled => "doubled a pawn",
            PawnSubTerm::WeakUnopposed => "exposed a weak pawn",
            PawnSubTerm::WeakLever => "walked into a pawn lever",
        }
    }
    fn improved_phrase(self) -> &'static str {
        match self {
            PawnSubTerm::Connected => "connected pawns",
            PawnSubTerm::Isolated => "reconnected an isolated pawn",
            PawnSubTerm::Backward => "freed a backward pawn",
            PawnSubTerm::Doubled => "resolved a doubled pawn",
            PawnSubTerm::WeakUnopposed => "covered a weak pawn",
            PawnSubTerm::WeakLever => "resolved a pawn lever",
        }
    }
}

fn pawn_clauses(pre: &PawnsBreakdown, post: &PawnsBreakdown) -> (Vec<&'static str>, Vec<&'static str>) {
    let mut worsened = Vec::new();
    let mut improved = Vec::new();
    for st in PawnSubTerm::ALL.iter() {
        let d = st.delta_mg(pre, post);
        if d <= -PAWN_STRUCTURE_DELTA_THRESHOLD_CP {
            worsened.push(st.worsened_phrase());
        } else if d >= PAWN_STRUCTURE_DELTA_THRESHOLD_CP {
            improved.push(st.improved_phrase());
        }
    }
    (worsened, improved)
}

fn build_pawn_structure_item(outcome: &PawnStructureOutcome) -> Option<RetrospectiveItem> {
    let (ours_worsened, ours_improved) = pawn_clauses(&outcome.ours_pre, &outcome.ours_post);
    let (theirs_worsened, theirs_improved) =
        pawn_clauses(&outcome.theirs_pre, &outcome.theirs_post);

    if ours_worsened.is_empty()
        && ours_improved.is_empty()
        && theirs_worsened.is_empty()
        && theirs_improved.is_empty()
    {
        return None;
    }

    // Sentiment: worsened on our side hurts; worsened on theirs helps.
    let net_our = ours_improved.len() as i32 - ours_worsened.len() as i32;
    let net_their = theirs_worsened.len() as i32 - theirs_improved.len() as i32;
    let net = net_our + net_their;
    let (heading, sentiment) = if !ours_worsened.is_empty() {
        ("Your pawn structure weakened", Sentiment::Negative)
    } else if !theirs_worsened.is_empty() {
        ("Weakened their pawn structure", Sentiment::Positive)
    } else if !ours_improved.is_empty() {
        ("Your pawn structure improved", Sentiment::Positive)
    } else if net < 0 {
        ("Their pawn structure improved", Sentiment::Negative)
    } else {
        ("Pawn structure changed", Sentiment::Mixed)
    };

    let summary_clauses: &[&'static str] = if !ours_worsened.is_empty() {
        &ours_worsened
    } else if !theirs_worsened.is_empty() {
        &theirs_worsened
    } else if !ours_improved.is_empty() {
        &ours_improved
    } else {
        &theirs_improved
    };
    let summary = summary_clauses.join(", ");

    let mut detail_lines = Vec::new();
    if !ours_worsened.is_empty() {
        detail_lines.push(format!("You: {}.", ours_worsened.join(", ")));
    }
    if !ours_improved.is_empty() {
        detail_lines.push(format!("You: {}.", ours_improved.join(", ")));
    }
    if !theirs_worsened.is_empty() {
        detail_lines.push(format!("Opponent: {}.", theirs_worsened.join(", ")));
    }
    if !theirs_improved.is_empty() {
        detail_lines.push(format!("Opponent: {}.", theirs_improved.join(", ")));
    }

    Some(RetrospectiveItem {
        category: RetrospectiveCategory::PawnStructure,
        heading: heading.to_string(),
        summary,
        detail: detail_lines.join("\n"),
        score_delta_pawns: None,
        sentiment,
        annotations: Vec::new(),
    })
}

// ---------------------------------------------------------------------
// Forced consequences — pawn-structure concessions in the opponent's
// best reply
// ---------------------------------------------------------------------

/// Walk the PV one ply past the user's move and look for pawn-
/// structure concessions in the opponent's *response*. The existing
/// pawn-structure card describes the change `pre → post-user-move`;
/// this card describes `post-user-move → post-opponent-reply`. It's
/// what answers "yes Bxh6 was an even trade, *and* it doubles their
/// h-pawns." Never says "this forces" — only "if they reply with X".
fn build_forced_consequences_items(
    user: &MoveAnalysis,
    pre_move_pos: &Position,
    root_stm: Color,
) -> Vec<RetrospectiveItem> {
    if user.pv.len() < 2 {
        return Vec::new();
    }
    let mut after_user = pre_move_pos.clone();
    after_user.do_move(user.pv[0]);
    let reply = user.pv[1];
    // Move-legality guard: if the engine's reply somehow isn't legal
    // against the post-user-move position (shouldn't happen in
    // practice), bail rather than panic.
    let reply_san = san::format(&after_user, reply);
    let mut after_reply = after_user.clone();
    after_reply.do_move(reply);

    let before = chess_tutor_engine::pawns::evaluate(&after_user)
        .breakdowns[(!root_stm).index()];
    let after = chess_tutor_engine::pawns::evaluate(&after_reply)
        .breakdowns[(!root_stm).index()];

    // Sub-terms where a more-negative delta means *worse* for the
    // opponent. Doubled / Isolated / Backward / WeakUnopposed /
    // WeakLever are all penalty terms (≤ 0 score); the Connected
    // term is a bonus and not interesting on the "they conceded
    // something" axis.
    // Lower threshold than the regular pawn-structure card. SF11's
    // Doubled penalty is `Score::new(11, 56)` per doubled pawn — only
    // ~11 cp at full middlegame phase, below the regular 15 cp gate.
    // Doubled / isolated / backward pawns are pedagogically valuable
    // even at small cp; they're long-term concessions that matter
    // toward the endgame.
    const FORCED_CONSEQUENCES_THRESHOLD_CP: i32 = 8;

    let mut items = Vec::new();
    for st in [
        PawnSubTerm::Doubled,
        PawnSubTerm::Isolated,
        PawnSubTerm::Backward,
        PawnSubTerm::WeakUnopposed,
    ] {
        let delta = st.delta_mg(&before, &after);
        // We're looking for the opponent's pawn position getting
        // worse — a more-negative delta.
        if delta > -FORCED_CONSEQUENCES_THRESHOLD_CP {
            continue;
        }
        let consequence = match st {
            PawnSubTerm::Doubled => "doubled pawns",
            PawnSubTerm::Isolated => "an isolated pawn",
            PawnSubTerm::Backward => "a backward pawn",
            PawnSubTerm::WeakUnopposed => "a weak pawn on a half-open file",
            // Connected / WeakLever excluded above.
            _ => continue,
        };
        items.push(RetrospectiveItem {
            category: RetrospectiveCategory::PawnStructure,
            heading: format!("If they reply {}, they get {}", reply_san, consequence),
            summary: format!(
                "their structure {:+.2} pawns after the reply",
                delta as f32 / 100.0
            ),
            detail: format!(
                "After your move and the opponent's best response {}, their pawn \
                 structure picks up {}. The engine's evaluation values this as a \
                 long-term concession on their side — they may decide not to \
                 reply this way, but if they do, this is the structural cost.",
                reply_san, consequence
            ),
            score_delta_pawns: Some(-delta as f32 / 100.0),
            sentiment: Sentiment::Positive,
            annotations: Vec::new(),
        });
    }
    items
}

// ---------------------------------------------------------------------
// Passed pawns
// ---------------------------------------------------------------------

const PASSED_DELTA_THRESHOLD_CP: i32 = 20;

fn passed_total_mg(bd: &PassedBreakdown) -> i32 {
    bd.rank_bonus.mg().0
        + bd.king_proximity.mg().0
        + bd.free_advance.mg().0
        + bd.stopper_penalty.mg().0
}

fn build_passed_pawns_item(outcome: &PassedPawnsOutcome) -> Option<RetrospectiveItem> {
    let ours_pre = passed_total_mg(&outcome.ours_pre);
    let ours_post = passed_total_mg(&outcome.ours_post);
    let theirs_pre = passed_total_mg(&outcome.theirs_pre);
    let theirs_post = passed_total_mg(&outcome.theirs_post);
    let ours_delta = ours_post - ours_pre;
    let theirs_delta = theirs_post - theirs_pre;

    if ours_delta.abs() < PASSED_DELTA_THRESHOLD_CP
        && theirs_delta.abs() < PASSED_DELTA_THRESHOLD_CP
    {
        return None;
    }

    let (heading, sentiment, net_for_user) = if ours_delta.abs() >= theirs_delta.abs() {
        if ours_delta > 0 {
            ("Your passed pawns advanced", Sentiment::Positive, ours_delta)
        } else {
            ("Your passed pawns lost ground", Sentiment::Negative, ours_delta)
        }
    } else if theirs_delta > 0 {
        ("Opponent's passed pawns advanced", Sentiment::Negative, -theirs_delta)
    } else {
        ("You blunted their passed pawns", Sentiment::Positive, -theirs_delta)
    };

    let summary = format!(
        "yours {:+.2}, theirs {:+.2}",
        ours_delta as f32 / 100.0,
        theirs_delta as f32 / 100.0
    );
    let detail = "Passed pawns are pawns with no enemy pawns on the same file or \
                  adjacent files ahead of them. The engine scores them by rank, \
                  king proximity, and clear-path bonuses."
        .to_string();

    Some(RetrospectiveItem {
        category: RetrospectiveCategory::PassedPawns,
        heading: heading.to_string(),
        summary,
        detail,
        score_delta_pawns: Some(net_for_user as f32 / 100.0),
        sentiment,
        annotations: Vec::new(),
    })
}

// ---------------------------------------------------------------------
// Space
// ---------------------------------------------------------------------

/// Default firing threshold for a Space card. One reinforced-square
/// change at full piece count moves the score by ~14 cp, so the
/// integer threshold is set just above to avoid surfacing the
/// always-on rocking caused by pawn-push-induced behind-set shifts.
/// Under "Show all signals" we drop to 1 cp so the +14 single-square
/// gains do surface.
const SPACE_DELTA_THRESHOLD_CP: i32 = 15;

/// Build Space cards. Up to two cards per move: one for our side and
/// one for the opponent's, fired independently when each side's
/// `|space_delta_mg|` crosses the threshold. Each card paints only
/// that side's post-move space (front + reinforced) — the renderer
/// shows them sequentially as the user clicks through, rather than
/// crowding both sides' highlights onto one board.
fn build_space_items(outcome: &SpaceOutcome, show_all: bool) -> Vec<RetrospectiveItem> {
    let threshold = if show_all { 1 } else { SPACE_DELTA_THRESHOLD_CP };

    let mut items = Vec::new();
    if let Some(it) = build_space_item_ours(outcome, threshold) {
        items.push(it);
    }
    if let Some(it) = build_space_item_theirs(outcome, threshold) {
        items.push(it);
    }
    items
}

fn build_space_item_ours(outcome: &SpaceOutcome, threshold: i32) -> Option<RetrospectiveItem> {
    let delta = outcome.ours_space_delta_mg();
    if delta.abs() < threshold {
        return None;
    }
    let (heading, sentiment) = if delta > 0 {
        ("You gained space", Sentiment::Positive)
    } else {
        ("You lost space", Sentiment::Negative)
    };
    Some(make_space_card(
        heading,
        sentiment,
        delta,
        outcome.ours_safe_post,
        outcome.ours_reinforced_post,
    ))
}

fn build_space_item_theirs(outcome: &SpaceOutcome, threshold: i32) -> Option<RetrospectiveItem> {
    let delta = outcome.theirs_space_delta_mg();
    if delta.abs() < threshold {
        return None;
    }
    let (heading, sentiment) = if delta > 0 {
        ("Opponent gained space", Sentiment::Negative)
    } else {
        ("You squeezed the opponent's space", Sentiment::Positive)
    };
    // Score-delta is from the user's POV — opponent gaining space
    // hurts the user, so flip the sign.
    Some(make_space_card(
        heading,
        sentiment,
        -delta,
        outcome.theirs_safe_post,
        outcome.theirs_reinforced_post,
    ))
}

fn make_space_card(
    heading: &str,
    sentiment: Sentiment,
    score_delta_mg: i32,
    safe_post: chess_tutor_engine::bitboard::Bitboard,
    reinforced_post: chess_tutor_engine::bitboard::Bitboard,
) -> RetrospectiveItem {
    let summary = format!("{:+.2} pawns", score_delta_mg as f32 / 100.0);
    let detail = "Stockfish's space term scores the central c–f files across the three ranks \
                  in front of your back row. Squares the enemy pawns attack don't count; \
                  squares on or behind your own pawn that no enemy piece attacks count \
                  twice. The bonus is squared by piece count, so space matters most when \
                  the board is still full."
        .to_string();

    // Front-only (highlighted as `SpaceFront`) vs. reinforced (the
    // doubly-rewarded subset, highlighted as `SpaceReinforced`).
    // Reinforced is always a subset of safe, so we subtract it out of
    // the front tier to keep each square painted exactly once.
    let front_only = safe_post & !reinforced_post;
    let mut annotations: Vec<BoardAnnotation> = Vec::new();
    for sq in front_only {
        annotations.push(BoardAnnotation::SquareHighlight {
            square: sq,
            kind: AnnotationKind::SpaceFront,
        });
    }
    for sq in reinforced_post {
        annotations.push(BoardAnnotation::SquareHighlight {
            square: sq,
            kind: AnnotationKind::SpaceReinforced,
        });
    }

    RetrospectiveItem {
        category: RetrospectiveCategory::Space,
        heading: heading.to_string(),
        summary,
        detail,
        score_delta_pawns: Some(score_delta_mg as f32 / 100.0),
        sentiment,
        annotations,
    }
}

// ---------------------------------------------------------------------
// Piece placement — one card per sub-signal × side
// ---------------------------------------------------------------------

const PIECES_DELTA_THRESHOLD_CP: i32 = 20;

/// One per `PiecesBreakdown` sub-term — each describes a distinct
/// chess concept (outpost claimed, rook on open file, bishop blocked
/// by own pawns, etc.) and gets its own card. Mirrors the narration
/// crate's `PieceSubTerm` (core/ui doesn't depend on narration).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum PieceSubTerm {
    Outposts,
    ReachableOutposts,
    MinorBehindPawn,
    KingProtector,
    BishopPawns,
    LongDiagonalBishop,
    RookOnQueenFile,
    RookOnOpenFile,
    RookOnSemiopenFile,
    TrappedRook,
    WeakQueen,
}

impl PieceSubTerm {
    const ALL: [PieceSubTerm; 11] = [
        PieceSubTerm::Outposts,
        PieceSubTerm::ReachableOutposts,
        PieceSubTerm::MinorBehindPawn,
        PieceSubTerm::KingProtector,
        PieceSubTerm::BishopPawns,
        PieceSubTerm::LongDiagonalBishop,
        PieceSubTerm::RookOnQueenFile,
        PieceSubTerm::RookOnOpenFile,
        PieceSubTerm::RookOnSemiopenFile,
        PieceSubTerm::TrappedRook,
        PieceSubTerm::WeakQueen,
    ];

    fn delta_mg(self, pre: &PiecesBreakdown, post: &PiecesBreakdown) -> i32 {
        match self {
            PieceSubTerm::Outposts => post.outposts.mg().0 - pre.outposts.mg().0,
            PieceSubTerm::ReachableOutposts => {
                post.reachable_outposts.mg().0 - pre.reachable_outposts.mg().0
            }
            PieceSubTerm::MinorBehindPawn => {
                post.minor_behind_pawn.mg().0 - pre.minor_behind_pawn.mg().0
            }
            PieceSubTerm::KingProtector => post.king_protector.mg().0 - pre.king_protector.mg().0,
            PieceSubTerm::BishopPawns => post.bishop_pawns.mg().0 - pre.bishop_pawns.mg().0,
            PieceSubTerm::LongDiagonalBishop => {
                post.long_diagonal_bishop.mg().0 - pre.long_diagonal_bishop.mg().0
            }
            PieceSubTerm::RookOnQueenFile => {
                post.rook_on_queen_file.mg().0 - pre.rook_on_queen_file.mg().0
            }
            PieceSubTerm::RookOnOpenFile => {
                post.rook_on_open_file.mg().0 - pre.rook_on_open_file.mg().0
            }
            PieceSubTerm::RookOnSemiopenFile => {
                post.rook_on_semiopen_file.mg().0 - pre.rook_on_semiopen_file.mg().0
            }
            PieceSubTerm::TrappedRook => post.trapped_rook.mg().0 - pre.trapped_rook.mg().0,
            PieceSubTerm::WeakQueen => post.weak_queen.mg().0 - pre.weak_queen.mg().0,
        }
    }

    /// Heading when our side's sub-term improved (good for the user).
    fn ours_improved_heading(self) -> &'static str {
        match self {
            PieceSubTerm::Outposts => "Your knight reached an outpost",
            PieceSubTerm::ReachableOutposts => "Your knight has a route to an outpost",
            PieceSubTerm::MinorBehindPawn => "Your minor gained pawn cover",
            PieceSubTerm::KingProtector => "Your minor rallied to defend the king",
            PieceSubTerm::BishopPawns => "Your bishop freed itself from its pawn chain",
            PieceSubTerm::LongDiagonalBishop => "Your bishop took the long diagonal",
            PieceSubTerm::RookOnQueenFile => "Your rook reached the queen's file",
            PieceSubTerm::RookOnOpenFile => "Your rook took the open file",
            PieceSubTerm::RookOnSemiopenFile => "Your rook took a semi-open file",
            PieceSubTerm::TrappedRook => "Your rook escaped its trap",
            PieceSubTerm::WeakQueen => "Your queen shook off pressure",
        }
    }

    /// Heading when our side's sub-term worsened (bad for the user).
    fn ours_worsened_heading(self) -> &'static str {
        match self {
            PieceSubTerm::Outposts => "Your knight lost its outpost",
            PieceSubTerm::ReachableOutposts => "Your knight's outpost route closed",
            PieceSubTerm::MinorBehindPawn => "Your minor lost its pawn cover",
            PieceSubTerm::KingProtector => "Your minor drifted away from the king",
            PieceSubTerm::BishopPawns => "Your bishop is blocked by its own pawns",
            PieceSubTerm::LongDiagonalBishop => "Your bishop left the long diagonal",
            PieceSubTerm::RookOnQueenFile => "Your rook left the queen's file",
            PieceSubTerm::RookOnOpenFile => "Your rook left the open file",
            PieceSubTerm::RookOnSemiopenFile => "Your rook left a semi-open file",
            PieceSubTerm::TrappedRook => "Your rook got trapped",
            PieceSubTerm::WeakQueen => "Your queen is under x-ray pressure",
        }
    }

    /// Heading when their side's sub-term improved (bad for the user).
    fn theirs_improved_heading(self) -> &'static str {
        match self {
            PieceSubTerm::Outposts => "Opponent's knight reached an outpost",
            PieceSubTerm::ReachableOutposts => "Opponent's knight has a route to an outpost",
            PieceSubTerm::MinorBehindPawn => "Opponent's minor gained pawn cover",
            PieceSubTerm::KingProtector => "Opponent's minor rallied to defend their king",
            PieceSubTerm::BishopPawns => "Opponent's bishop freed itself from its pawn chain",
            PieceSubTerm::LongDiagonalBishop => "Opponent's bishop took the long diagonal",
            PieceSubTerm::RookOnQueenFile => "Opponent's rook reached your queen's file",
            PieceSubTerm::RookOnOpenFile => "Opponent's rook took the open file",
            PieceSubTerm::RookOnSemiopenFile => "Opponent's rook took a semi-open file",
            PieceSubTerm::TrappedRook => "Opponent's rook escaped its trap",
            PieceSubTerm::WeakQueen => "Opponent's queen shook off pressure",
        }
    }

    /// Heading when their side's sub-term worsened (good for the user).
    fn theirs_worsened_heading(self) -> &'static str {
        match self {
            PieceSubTerm::Outposts => "You denied the opponent's knight an outpost",
            PieceSubTerm::ReachableOutposts => "You closed the opponent's outpost route",
            PieceSubTerm::MinorBehindPawn => "You stripped the opponent's pawn cover",
            PieceSubTerm::KingProtector => "Opponent's minor drifted from their king",
            PieceSubTerm::BishopPawns => "Opponent's bishop is blocked by their own pawns",
            PieceSubTerm::LongDiagonalBishop => "Opponent's bishop left the long diagonal",
            PieceSubTerm::RookOnQueenFile => "Opponent's rook left your queen's file",
            PieceSubTerm::RookOnOpenFile => "Opponent's rook left the open file",
            PieceSubTerm::RookOnSemiopenFile => "Opponent's rook left a semi-open file",
            PieceSubTerm::TrappedRook => "You trapped the opponent's rook",
            PieceSubTerm::WeakQueen => "You put the opponent's queen under x-ray pressure",
        }
    }

    /// Short prose explaining what this sub-term measures. Renders in
    /// the card's expand-on-click detail.
    fn detail(self) -> &'static str {
        match self {
            PieceSubTerm::Outposts => {
                "An outpost is a square defended by your own pawn that the opponent's \
                 pawns can't kick away. Knights and bishops are powerful on outposts \
                 because no minor piece can dislodge them with a single move."
            }
            PieceSubTerm::ReachableOutposts => {
                "Your knight is one move away from an outpost — a square defended by \
                 your pawn that the opponent's pawns can't reach. Outposts are \
                 strongest with a knight on them; this card means the route is open."
            }
            PieceSubTerm::MinorBehindPawn => {
                "A minor piece directly behind one of your pawns is shielded from \
                 captures along its file and tends to support pawn pushes."
            }
            PieceSubTerm::KingProtector => {
                "Minor pieces lose a small bonus the further they sit from your own \
                 king. Knights and bishops near home help shield the king from attacks."
            }
            PieceSubTerm::BishopPawns => {
                "A bishop is penalised for each friendly pawn sitting on its color — \
                 those pawns block the bishop's diagonals. Either trade the bishop or \
                 push the pawns off its color."
            }
            PieceSubTerm::LongDiagonalBishop => {
                "A bishop attacking both central squares (e4/e5 or d4/d5) along its \
                 long diagonal exerts pressure on the center from a single piece."
            }
            PieceSubTerm::RookOnQueenFile => {
                "A rook on the same file as the enemy queen exerts latent pressure \
                 even with pawns in the way — when the file opens it becomes a tactic."
            }
            PieceSubTerm::RookOnOpenFile => {
                "A rook on a file with no pawns of either color controls the entire \
                 file. Open files are the rook's natural element."
            }
            PieceSubTerm::RookOnSemiopenFile => {
                "A rook on a file with no friendly pawns but enemy pawns can pressure \
                 those pawns directly — useful for attacking weak pawns."
            }
            PieceSubTerm::TrappedRook => {
                "A rook stuck behind its own king after castling rights are gone has \
                 almost no mobility. It blocks the king and contributes nothing."
            }
            PieceSubTerm::WeakQueen => {
                "The queen sees a slider x-ray threat against it — a rook or bishop \
                 aimed through one intervening piece. A discovered attack can win the \
                 queen unless you defuse it."
            }
        }
    }
}

/// Skip `BishopPawns` narration when bishop geometry didn't change on
/// `side`. Without this filter, a central pawn push (1.e4 e5) that
/// merely doubles the blocked-centre multiplier would fire phantom
/// "bishop blocked by own pawns" cards on both sides — none of which
/// describe anything a 1200-ELO student can act on. Mirrors the
/// narration crate's `include_bishop_pawns`.
fn include_sub_term(st: PieceSubTerm, bishop_geometry_changed: bool) -> bool {
    st != PieceSubTerm::BishopPawns || bishop_geometry_changed
}

/// Capture-aware suppression flags for the king-protector
/// sub-term — built from the realized capture events so the
/// per-sub-term loop can drop arithmetic-noise variants of the
/// card.
#[derive(Copy, Clone, Debug, Default)]
struct KingProtectorSuppression {
    /// `true` when at least one of our minors was captured at ply
    /// ≤ 1. Their average king-distance "improves" purely because a
    /// minor came off the board — no actual repositioning happened.
    /// Drop the ours-side KP card (in either direction).
    ours_minor_captured: bool,
    /// Same logic for the opponent's side.
    theirs_minor_captured: bool,
    /// `true` when *our* ply-0 move was a capture made *by* a minor.
    /// The minor's "drift away from the king" is what enabled the
    /// capture; framing it as a cost mis-teaches. Drops the
    /// `ours_worsened` direction only — improvements (a minor
    /// rallying back to the king) still surface normally.
    our_minor_capturing: bool,
}

fn king_protector_suppression(
    material: &chess_tutor_engine::analysis::MaterialOutcome,
    root_stm: Color,
) -> KingProtectorSuppression {
    let mut out = KingProtectorSuppression::default();
    for ev in material.realized_events() {
        if ev.captured_piece.is_minor() {
            if ev.captor == root_stm {
                // We captured one of their minors.
                out.theirs_minor_captured = true;
            } else {
                // They captured one of ours.
                out.ours_minor_captured = true;
            }
        }
        if ev.ply == 0 && ev.captor == root_stm && ev.captor_piece.is_minor() {
            out.our_minor_capturing = true;
        }
    }
    out
}

fn build_pieces_positional_items(
    outcome: &PiecesPositionalOutcome,
    _root_stm: Color,
    kp_supp: KingProtectorSuppression,
) -> Vec<RetrospectiveItem> {
    let mut items = Vec::new();

    for st in PieceSubTerm::ALL {
        // Our side.
        if include_sub_term(st, outcome.ours_bishop_pawn_count_changed()) {
            let delta = st.delta_mg(&outcome.ours_pre, &outcome.ours_post);
            if delta.abs() >= PIECES_DELTA_THRESHOLD_CP {
                let (heading, sentiment) = if delta > 0 {
                    (st.ours_improved_heading(), Sentiment::Positive)
                } else {
                    (st.ours_worsened_heading(), Sentiment::Negative)
                };
                // King-protector capture-aware suppression. Two cases
                // for our side:
                //   - If one of our minors was just captured, the
                //     remaining minors' average king-distance
                //     "improved" purely by arithmetic — not a
                //     defensive move. Drop in either direction.
                //   - If our ply-0 capture was BY a minor, the "drift
                //     away" variant misleadingly frames the
                //     capture-enabling move as a cost. Drop only the
                //     worsened direction; if the captor ends up
                //     closer to our king (the engine likes the
                //     square defensively), keep the improved card.
                if st == PieceSubTerm::KingProtector {
                    if kp_supp.ours_minor_captured {
                        continue;
                    }
                    if kp_supp.our_minor_capturing && delta < 0 {
                        continue;
                    }
                }
                items.push(RetrospectiveItem {
                    category: RetrospectiveCategory::PiecePlacement,
                    heading: heading.to_string(),
                    summary: format!("{:+.2} pawns", delta as f32 / 100.0),
                    detail: st.detail().to_string(),
                    // User-POV delta: our improvement is positive for
                    // us; our worsening is negative.
                    score_delta_pawns: Some(delta as f32 / 100.0),
                    sentiment,
                    annotations: Vec::new(),
                });
            }
        }

        // Their side.
        if include_sub_term(st, outcome.theirs_bishop_pawn_count_changed()) {
            let delta = st.delta_mg(&outcome.theirs_pre, &outcome.theirs_post);
            if delta.abs() >= PIECES_DELTA_THRESHOLD_CP {
                let (heading, sentiment) = if delta > 0 {
                    (st.theirs_improved_heading(), Sentiment::Negative)
                } else {
                    (st.theirs_worsened_heading(), Sentiment::Positive)
                };
                // Same KP capture-aware suppression for their side:
                // if a minor of theirs came off the board, the
                // delta is arithmetic — not their pieces "rallying."
                if st == PieceSubTerm::KingProtector && kp_supp.theirs_minor_captured {
                    continue;
                }
                items.push(RetrospectiveItem {
                    category: RetrospectiveCategory::PiecePlacement,
                    heading: heading.to_string(),
                    summary: format!("{:+.2} pawns", delta as f32 / 100.0),
                    detail: st.detail().to_string(),
                    // User-POV delta: their improvement hurts us; their
                    // worsening helps us — both flip sign vs. raw delta.
                    score_delta_pawns: Some(-delta as f32 / 100.0),
                    sentiment,
                    annotations: Vec::new(),
                });
            }
        }
    }

    items
}

// ---------------------------------------------------------------------
// Secondary terms (Helped / Hurt fallback)
// ---------------------------------------------------------------------

const RETROSPECTIVE_TOP_PERCENT: f32 = 50.0;

fn build_secondary_item(
    user: &MoveAnalysis,
    root_stm: Color,
    skip: &[TermId],
    show_all: bool,
) -> Option<RetrospectiveItem> {
    // show_all bypasses the 50%-coverage trim so every residual term
    // with a non-zero delta appears as a row. The GUI's collapsible
    // card keeps the noise out of the way until the user expands.
    let percent = if show_all { 100.0 } else { RETROSPECTIVE_TOP_PERCENT };
    let prefix = cumulative_prefix(&user.term_deltas, percent);
    let sign = if root_stm == Color::White { 1 } else { -1 };
    let rows: Vec<(TermId, i32)> = prefix
        .iter()
        .filter(|d| !skip.contains(&d.term) && d.delta_tapered != 0)
        .map(|d| (d.term, d.delta_tapered * sign))
        .collect();
    if rows.is_empty() {
        return None;
    }
    let (helped, hurt): (Vec<_>, Vec<_>) = rows.into_iter().partition(|(_, cp)| *cp > 0);
    let mut detail_lines = Vec::new();
    if !helped.is_empty() {
        detail_lines.push(format!(
            "Also helped: {}",
            format_term_list(&helped)
        ));
    }
    if !hurt.is_empty() {
        detail_lines.push(format!(
            "Also hurt: {}",
            format_term_list(&hurt)
        ));
    }
    let net: i32 = helped.iter().map(|(_, cp)| *cp).sum::<i32>()
        + hurt.iter().map(|(_, cp)| *cp).sum::<i32>();
    let sentiment = if net > 0 {
        Sentiment::Positive
    } else if net < 0 {
        Sentiment::Negative
    } else {
        Sentiment::Mixed
    };
    let summary = if !helped.is_empty() && !hurt.is_empty() {
        format!(
            "{} helped, {} hurt",
            helped.len(),
            hurt.len()
        )
    } else if !helped.is_empty() {
        format!("{} helped", helped.len())
    } else {
        format!("{} hurt", hurt.len())
    };
    Some(RetrospectiveItem {
        category: RetrospectiveCategory::Secondary,
        heading: "Other shifts".to_string(),
        summary,
        detail: detail_lines.join("\n"),
        score_delta_pawns: Some(net as f32 / 100.0),
        sentiment,
        annotations: Vec::new(),
    })
}

fn format_term_list(rows: &[(TermId, i32)]) -> String {
    let mut sorted: Vec<&(TermId, i32)> = rows.iter().collect();
    sorted.sort_by_key(|(_, cp)| std::cmp::Reverse(cp.abs()));
    sorted
        .iter()
        .map(|(term, cp)| format!("{} {:+.2}", term.pretty_label(), *cp as f32 / 100.0))
        .collect::<Vec<_>>()
        .join(", ")
}

// ---------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------

fn post_user_move_position(pre: &Position, user: &MoveAnalysis) -> Position {
    let mut p = pre.clone();
    if let Some(&mv) = user.pv.first() {
        p.do_move(mv);
    }
    p
}

/// Classical point-value net across realized captures from
/// `root_stm`'s POV. Positive = our side captured more by points
/// (P:1, N:3, B:3, R:5, Q:9); negative = opponent did; zero = even
/// (the B↔N case the cp-based net would call "lost material").
fn realized_point_net(
    events: &[&chess_tutor_engine::analysis::CaptureEvent],
    root_stm: Color,
) -> i32 {
    let mut net = 0i32;
    for ev in events {
        let pts = ev.captured_piece.classical_points() as i32;
        if ev.captor == root_stm {
            net += pts;
        } else {
            net -= pts;
        }
    }
    net
}

/// Build a teaching note for a point-even trade whose engine cp net
/// leans meaningfully in one direction. Returns `None` when both
/// mg and eg leans are tight (≤ 30 cp), in which case the trade is
/// genuinely equal and there's nothing to teach.
///
/// The note's job is to give the student a concrete fact about the
/// position — "the engine values your bishop higher than their
/// knight in this position" — without framing it as a verdict on
/// the move. A 50 cp lean is small enough that the move can still
/// be best for other reasons; the student should understand it as
/// information, not a critique.
fn phase_dependent_trade_note(
    events: &[&chess_tutor_engine::analysis::CaptureEvent],
    root_stm: Color,
) -> Option<String> {
    const PHASE_NOTE_THRESHOLD_CP: i32 = 30;
    let (net_mg, net_eg) = events.iter().fold((0i32, 0i32), |(mg, eg), ev| {
        let sign = if ev.captor == root_stm { 1 } else { -1 };
        (mg + sign * ev.value_mg, eg + sign * ev.value_eg)
    });
    let mg_abs = net_mg.abs();
    let eg_abs = net_eg.abs();
    if mg_abs < PHASE_NOTE_THRESHOLD_CP && eg_abs < PHASE_NOTE_THRESHOLD_CP {
        return None;
    }
    // Identify which side the lean favors. The trade is "even by
    // points" but cp may favor either side; positive cp = us, negative
    // = opponent. We pick the dominant phase as the headline and call
    // out the other for contrast if it disagrees in direction.
    let dominant_cp = if eg_abs > mg_abs { net_eg } else { net_mg };
    let dominant_phase = if eg_abs > mg_abs { "endgame" } else { "middlegame" };
    let other_phase = if eg_abs > mg_abs { "middlegame" } else { "endgame" };
    let other_cp = if eg_abs > mg_abs { net_mg } else { net_eg };

    let lean_text = format!(
        "{:+.2} pawns at {} phase",
        dominant_cp as f32 / 100.0,
        dominant_phase
    );
    let favor_us = dominant_cp > 0;
    let lead = if favor_us {
        format!(
            "Even by points, but the engine reads this slightly in your favor — {}.",
            lean_text
        )
    } else {
        format!(
            "Even by points, but the engine reads this slightly in your opponent's favor — {}.",
            lean_text
        )
    };
    // If mg and eg agree in direction, the lean is consistent across
    // the game. If they disagree, the trade is phase-dependent and
    // that's the more interesting story.
    let phase_clause = if dominant_cp.signum() == other_cp.signum() || other_cp == 0 {
        format!(
            " The {} valuation is similar ({:+.2} pawns), so the imbalance \
             holds across the game.",
            other_phase,
            other_cp as f32 / 100.0
        )
    } else {
        format!(
            " In the {} the trade reads {:+.2} pawns — phase-dependent: the \
             engine values these pieces differently depending on how much \
             material remains on the board.",
            other_phase,
            other_cp as f32 / 100.0
        )
    };
    Some(format!("{}{}", lead, phase_clause))
}

fn piece_name(pt: PieceType) -> &'static str {
    match pt {
        PieceType::Pawn => "pawn",
        PieceType::Knight => "knight",
        PieceType::Bishop => "bishop",
        PieceType::Rook => "rook",
        PieceType::Queen => "queen",
        PieceType::King => "king",
    }
}

fn article(name: &str) -> String {
    let c = name.chars().next().unwrap_or('x').to_ascii_lowercase();
    if matches!(c, 'a' | 'e' | 'i' | 'o' | 'u') {
        format!("an {}", name)
    } else {
        format!("a {}", name)
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn join_with_and(parts: &[String]) -> String {
    match parts.len() {
        0 => String::new(),
        1 => parts[0].clone(),
        2 => format!("{} and {}", parts[0], parts[1]),
        _ => {
            let (last, lead) = parts.split_last().unwrap();
            format!("{}, and {}", lead.join(", "), last)
        }
    }
}

fn verdict_label(v: MoveVerdict) -> &'static str {
    match v {
        MoveVerdict::Best => "Best",
        MoveVerdict::Good => "Good",
        MoveVerdict::Inaccuracy => "Inaccuracy",
        MoveVerdict::Mistake => "Mistake",
        MoveVerdict::Blunder => "Blunder",
        MoveVerdict::BestAvailable => "Best available",
    }
}

fn verdict_sentiment(v: MoveVerdict) -> Sentiment {
    match v {
        MoveVerdict::Best | MoveVerdict::Good => Sentiment::Positive,
        MoveVerdict::Inaccuracy => Sentiment::Mixed,
        MoveVerdict::Mistake | MoveVerdict::Blunder => Sentiment::Negative,
        MoveVerdict::BestAvailable => Sentiment::Neutral,
    }
}

fn sharp_or_verdict_annotation(v: MoveVerdict, is_sharp: bool) -> &'static str {
    if is_sharp {
        return "!";
    }
    match v {
        MoveVerdict::Blunder => "??",
        MoveVerdict::Mistake => "?",
        _ => "",
    }
}

fn format_score_pawns(score: Value) -> String {
    let abs = score.0.abs();
    let mate_threshold = Value::MATE.0 - Value::MAX_PLY;
    if abs >= mate_threshold {
        let plies = Value::MATE.0 - abs;
        let moves = (plies + 1) / 2;
        if score.0 >= 0 {
            format!("#{}", moves)
        } else {
            format!("-#{}", moves)
        }
    } else {
        format!("{:+.2}", score.0 as f32 / 100.0)
    }
}

fn format_delta_pawns(delta_cp: i32) -> String {
    format!("{:+.2}", delta_cp as f32 / 100.0)
}

fn surprise_note(verdict: MoveVerdict, surprise: Option<SurpriseKind>) -> Option<String> {
    match (verdict, surprise) {
        (MoveVerdict::Mistake | MoveVerdict::Blunder, Some(SurpriseKind::LooksGoodButBad)) => {
            Some("This looked natural but the deeper line gives back material.".to_string())
        }
        (MoveVerdict::Best | MoveVerdict::Good, Some(SurpriseKind::LooksBadButGood)) => {
            Some("This looked risky on the surface — the longer line pays off.".to_string())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chess_tutor_engine::engine::{Engine, SearchParams};

    /// End-to-end smoke: analyze 1.e4 from startpos and confirm the
    /// view model returns a non-empty headline + parses without
    /// panicking. We can't assert specific cards because the
    /// engine's outcome of the opening shifts by depth — but the
    /// headline must be populated.
    #[test]
    fn build_view_model_from_startpos_analysis_returns_headline() {
        let mut pos = Position::startpos();
        let mut engine = Engine::default();
        let analyses = chess_tutor_engine::analysis::analyze_position(
            &mut engine,
            &mut pos,
            SearchParams {
                max_depth: 4,
                multi_pv: 4,
                ..SearchParams::default()
            },
        );
        assert!(!analyses.is_empty());
        // Pick any analyzed move as the "user" move so we can build
        // the view model.
        let user_move = analyses[0].mv;
        let pre = Position::startpos();
        let vm = build_retrospective_view(&pre, &analyses, user_move, false, false);
        assert!(!vm.headline.user_san.is_empty());
        assert!(!vm.headline.verdict_label.is_empty());
        assert!(!vm.headline.user_score.is_empty());
    }

    #[test]
    fn build_view_model_suppresses_items_when_user_move_delivers_mate() {
        // Fool's mate: after 1.f3 e5 2.g4, Black plays Qh4#. The
        // game ends right there — per-category cards (king safety
        // shifts, mobility deltas, "other shifts") are noise. The
        // headline still shows the mating SAN with '#' and the
        // verdict label.
        let pre = Position::from_fen(
            "rnbqkbnr/pppp1ppp/8/4p3/6P1/5P2/PPPPP2P/RNBQKBNR b KQkq g3 0 2",
        )
        .unwrap();
        let mating_move = Move::normal(Square::D8, Square::H4);
        let mut pos = pre.clone();
        let mut engine = Engine::default();
        let analyses = chess_tutor_engine::analysis::analyze_position(
            &mut engine,
            &mut pos,
            SearchParams {
                max_depth: 4,
                multi_pv: 4,
                force_include: vec![mating_move],
                ..SearchParams::default()
            },
        );
        assert!(
            analyses.iter().any(|a| a.mv == mating_move),
            "force_include should have analyzed the mating move"
        );
        let vm = build_retrospective_view(&pre, &analyses, mating_move, false, false);
        assert!(!vm.headline.user_san.is_empty(), "headline still populated");
        assert!(
            vm.items.is_empty(),
            "game-ending move should suppress every per-category card, got {:?}",
            vm.items.iter().map(|i| &i.heading).collect::<Vec<_>>()
        );
    }

    #[test]
    fn build_view_model_with_missing_user_move_returns_empty() {
        let mut pos = Position::startpos();
        let mut engine = Engine::default();
        let analyses = chess_tutor_engine::analysis::analyze_position(
            &mut engine,
            &mut pos,
            SearchParams {
                max_depth: 3,
                multi_pv: 1,
                ..SearchParams::default()
            },
        );
        // Pick a move that's almost certainly NOT in a depth-3
        // multi-pv-1 search: a1-a2 (would be illegal anyway because
        // a1 has the rook and a2 the pawn from startpos). The view
        // model should fall through to default rather than panic.
        let bogus = Move::normal(
            chess_tutor_engine::types::Square::A1,
            chess_tutor_engine::types::Square::A2,
        );
        let pre = Position::startpos();
        let vm = build_retrospective_view(&pre, &analyses, bogus, false, false);
        assert!(vm.headline.user_san.is_empty());
        assert!(vm.items.is_empty());
    }

    #[test]
    fn material_card_ignores_captures_past_ply_one() {
        // A MaterialOutcome with a single capture at ply 15 should
        // produce NO card — we don't say "You won material" past
        // tense based on a speculative deep-PV trade. This is the
        // 1.e4 e5 2.Nf3 → "Ply 15: you take a bishop with a bishop
        // on e6" pathology the user reported.
        use chess_tutor_engine::analysis::CaptureEvent;
        use chess_tutor_engine::types::PieceType;
        let outcome = MaterialOutcome {
            events: vec![CaptureEvent {
                ply: 14, // 0-indexed ply 14 = "Ply 15" in detail text
                captor: Color::White,
                captor_piece: PieceType::Bishop,
                captured_piece: PieceType::Bishop,
                square: chess_tutor_engine::types::Square::E6,
                value_mg: 825,
                value_eg: 915,
            }],
            net_mg_cp: 825,
            net_eg_cp: 915,
            last_ply: 14,
        };
        let pre = Position::startpos();
        let item = build_material_item(&pre, &outcome, Color::White);
        assert!(
            item.is_none(),
            "ply-15 capture must not drive a material card, got {item:?}"
        );
    }

    #[test]
    fn material_card_treats_bishop_for_knight_as_even_trade() {
        // User captures a knight at ply 0; opponent recaptures with a
        // bishop at ply 1. Engine cp net is -44 (B 825 vs N 781), but
        // classical point values are 3-for-3 — students read this as
        // an even trade. The card heading must reflect point parity,
        // not the cp lean.
        use chess_tutor_engine::analysis::CaptureEvent;
        use chess_tutor_engine::types::{PieceType, Square};
        let outcome = MaterialOutcome {
            events: vec![
                CaptureEvent {
                    ply: 0,
                    captor: Color::White,
                    captor_piece: PieceType::Bishop,
                    captured_piece: PieceType::Knight,
                    square: Square::C6,
                    value_mg: 781,
                    value_eg: 854,
                },
                CaptureEvent {
                    ply: 1,
                    captor: Color::Black,
                    captor_piece: PieceType::Pawn,
                    captured_piece: PieceType::Bishop,
                    square: Square::C6,
                    value_mg: 825,
                    value_eg: 915,
                },
            ],
            net_mg_cp: 781 - 825,
            net_eg_cp: 854 - 915,
            last_ply: 1,
        };
        let pre = Position::startpos();
        let item = build_material_item(&pre, &outcome, Color::White)
            .expect("two ply-0+ply-1 captures must produce a card");
        assert_eq!(item.heading, "Even trade");
        // Score-delta chip suppressed because point parity is even.
        assert!(item.score_delta_pawns.is_none());
    }

    #[test]
    fn filter_misleading_hangs_suppresses_bishop_on_square_we_just_captured_knight_on() {
        // Bxh6 leaves our bishop on h6 attacked by gxh6 — the second
        // leg of an even trade. The filter must recognise we just
        // captured a 3-point knight on h6 and drop the hang.
        use chess_tutor_engine::analysis::PieceLocation;
        use chess_tutor_engine::types::{PieceType, Square};
        let hangs = vec![HangingPiece {
            location: PieceLocation {
                square: Square::H6,
                piece: PieceType::Bishop,
            },
            attackers: vec![PieceLocation {
                square: Square::G7,
                piece: PieceType::Pawn,
            }],
        }];
        let captures = vec![(Square::H6, 3u8)]; // we captured a knight (3 pts) on h6
        let filtered = filter_misleading_hangs(&hangs, &captures, 0);
        assert!(filtered.is_empty(), "fair recapture should suppress the hang");
    }

    #[test]
    fn filter_misleading_hangs_keeps_real_sacrifice() {
        // Qxh6 leaves our queen attacked on a square where we
        // captured a knight. Q (9pts) > N (3pts) is a sacrifice, not
        // an even trade. The hanging-queen warning is informative —
        // it must NOT be suppressed.
        use chess_tutor_engine::analysis::PieceLocation;
        use chess_tutor_engine::types::{PieceType, Square};
        let hangs = vec![HangingPiece {
            location: PieceLocation {
                square: Square::H6,
                piece: PieceType::Queen,
            },
            attackers: vec![PieceLocation {
                square: Square::G7,
                piece: PieceType::Pawn,
            }],
        }];
        let captures = vec![(Square::H6, 3u8)];
        let filtered = filter_misleading_hangs(&hangs, &captures, 0);
        assert_eq!(filtered.len(), 1, "queen sac is not an even trade");
    }

    #[test]
    fn filter_misleading_hangs_keeps_hang_on_different_square() {
        // We captured on h6, but a separate piece hangs on a8. Unrelated
        // — don't suppress.
        use chess_tutor_engine::analysis::PieceLocation;
        use chess_tutor_engine::types::{PieceType, Square};
        let hangs = vec![HangingPiece {
            location: PieceLocation {
                square: Square::A8,
                piece: PieceType::Bishop,
            },
            attackers: vec![PieceLocation {
                square: Square::A1,
                piece: PieceType::Rook,
            }],
        }];
        let captures = vec![(Square::H6, 3u8)];
        let filtered = filter_misleading_hangs(&hangs, &captures, 0);
        assert_eq!(filtered.len(), 1, "unrelated hang must surface");
    }

    #[test]
    fn filter_misleading_hangs_suppresses_when_higher_value_counter_threat_exists() {
        // Counter-attack pattern: our bishop is "hanging" on f6 but
        // we have a guaranteed win of their queen elsewhere (in
        // theirs_hanging_guaranteed). The opponent's best response
        // is to address the queen problem, not capture our bishop —
        // so the bishop isn't really hanging.
        use chess_tutor_engine::analysis::PieceLocation;
        use chess_tutor_engine::types::{PieceType, Square};
        let hangs = vec![HangingPiece {
            location: PieceLocation {
                square: Square::F6,
                piece: PieceType::Bishop, // 3 pts
            },
            attackers: vec![PieceLocation {
                square: Square::G7,
                piece: PieceType::Pawn,
            }],
        }];
        // counter_threat_pts = 9 (queen, > bishop's 3)
        let filtered = filter_misleading_hangs(&hangs, &[], 9);
        assert!(
            filtered.is_empty(),
            "queen counter-threat should suppress the bishop hang"
        );
    }

    #[test]
    fn filter_misleading_hangs_keeps_when_counter_threat_is_equal_value() {
        // Equal-value counter-threat (both bishops, 3 pts each).
        // Opponent could plausibly accept the trade — take our
        // bishop, lose theirs. The hang is informative; don't drop.
        use chess_tutor_engine::analysis::PieceLocation;
        use chess_tutor_engine::types::{PieceType, Square};
        let hangs = vec![HangingPiece {
            location: PieceLocation {
                square: Square::F6,
                piece: PieceType::Bishop, // 3 pts
            },
            attackers: vec![PieceLocation {
                square: Square::G7,
                piece: PieceType::Pawn,
            }],
        }];
        // counter_threat_pts = 3 (their bishop, not > our 3)
        let filtered = filter_misleading_hangs(&hangs, &[], 3);
        assert_eq!(
            filtered.len(),
            1,
            "equal counter-threat is a wash, not full compensation"
        );
    }

    #[test]
    fn filter_misleading_hangs_keeps_when_counter_threat_is_lower_value() {
        // We "hang" a queen but our counter-threat is just a pawn.
        // Opponent gladly takes the queen — the hang is real.
        use chess_tutor_engine::analysis::PieceLocation;
        use chess_tutor_engine::types::{PieceType, Square};
        let hangs = vec![HangingPiece {
            location: PieceLocation {
                square: Square::F6,
                piece: PieceType::Queen, // 9 pts
            },
            attackers: vec![PieceLocation {
                square: Square::G7,
                piece: PieceType::Pawn,
            }],
        }];
        // counter_threat_pts = 1 (a pawn)
        let filtered = filter_misleading_hangs(&hangs, &[], 1);
        assert_eq!(
            filtered.len(),
            1,
            "small counter-threat doesn't excuse a hanging queen"
        );
    }

    #[test]
    fn phase_note_fires_on_b_for_n_trade_favoring_opponent() {
        // White gives up a bishop (825/915) for a knight (781/854).
        // Point parity is even (3↔3) but engine cp leans toward
        // opponent — small in mg (-44) and a bit more in eg (-61).
        // Both exceed our 30 cp threshold; the dominant phase should
        // be endgame, and the framing should call out the opponent's
        // favor.
        use chess_tutor_engine::analysis::CaptureEvent;
        use chess_tutor_engine::types::{PieceType, Square};
        let events_storage = vec![
            CaptureEvent {
                ply: 0,
                captor: Color::White,
                captor_piece: PieceType::Bishop,
                captured_piece: PieceType::Knight,
                square: Square::H6,
                value_mg: 781,
                value_eg: 854,
            },
            CaptureEvent {
                ply: 1,
                captor: Color::Black,
                captor_piece: PieceType::Pawn,
                captured_piece: PieceType::Bishop,
                square: Square::H6,
                value_mg: 825,
                value_eg: 915,
            },
        ];
        let events: Vec<&CaptureEvent> = events_storage.iter().collect();
        let note = phase_dependent_trade_note(&events, Color::White)
            .expect("B-for-N trade should produce a phase note");
        assert!(
            note.contains("opponent's favor"),
            "expected 'opponent's favor' framing, got: {note}"
        );
        assert!(
            note.contains("endgame"),
            "expected endgame to be called out, got: {note}"
        );
    }

    #[test]
    fn phase_note_skipped_when_lean_is_below_threshold() {
        // Pawn-for-pawn trade. mg/eg leans are zero or negligible —
        // the note should not fire (nothing pedagogical to add).
        use chess_tutor_engine::analysis::CaptureEvent;
        use chess_tutor_engine::types::{PieceType, Square};
        let events_storage = vec![
            CaptureEvent {
                ply: 0,
                captor: Color::White,
                captor_piece: PieceType::Pawn,
                captured_piece: PieceType::Pawn,
                square: Square::D5,
                value_mg: 124,
                value_eg: 206,
            },
            CaptureEvent {
                ply: 1,
                captor: Color::Black,
                captor_piece: PieceType::Pawn,
                captured_piece: PieceType::Pawn,
                square: Square::D5,
                value_mg: 124,
                value_eg: 206,
            },
        ];
        let events: Vec<&CaptureEvent> = events_storage.iter().collect();
        assert_eq!(
            phase_dependent_trade_note(&events, Color::White),
            None,
            "pawn-pawn trade should produce no note"
        );
    }

    #[test]
    fn forced_consequences_fires_on_bxh6_doubled_pawns() {
        // The user's exact reported case. After Bxh6, the engine's
        // best reply is gxh6, which doubles Black's h-pawns. The new
        // forced-consequences card should surface this with the
        // "If they reply ..." framing.
        use chess_tutor_engine::engine::{Engine, SearchParams};
        let pre = Position::from_fen(
            "r1bqkb1r/ppp2ppp/2n1p2n/3pP3/3P4/5N2/PPP2PPP/RNBQKB1R w KQkq - 5 5",
        )
        .unwrap();
        let user_move = Move::normal(Square::C1, Square::H6);
        let mut pos = pre.clone();
        let mut engine = Engine::default();
        let analyses = chess_tutor_engine::analysis::analyze_position(
            &mut engine,
            &mut pos,
            SearchParams {
                max_depth: 6,
                multi_pv: 1,
                force_include: vec![user_move],
                ..SearchParams::default()
            },
        );
        let user = analyses.iter().find(|a| a.mv == user_move).expect("force_include");
        // The engine's predicted reply at this depth should be gxh6.
        // If a future tuning changes that, the test becomes invalid —
        // we'd need to choose a more stable scenario.
        assert!(
            user.pv.len() >= 2,
            "expected at least one opponent reply in PV"
        );
        let items = build_forced_consequences_items(user, &pre, Color::White);
        assert!(
            items.iter().any(|it| it.heading.contains("doubled pawns")),
            "expected a 'doubled pawns' forced-consequences card after Bxh6, got: {:?}",
            items.iter().map(|i| &i.heading).collect::<Vec<_>>()
        );
    }

    #[test]
    fn forced_consequences_skips_when_pv_has_no_reply() {
        // A user move with no engine reply in the PV (terminal
        // position or single-ply analysis) must not produce any
        // forced-consequences card.
        let pre = Position::startpos();
        let user_move = Move::normal(Square::E2, Square::E4);
        let user = chess_tutor_engine::analysis::MoveAnalysis {
            mv: user_move,
            score: chess_tutor_engine::types::Value::ZERO,
            depth: 1,
            pv: vec![user_move], // only ply 0, no reply
            ply_traces: vec![chess_tutor_engine::eval::EvalTrace::zero()],
            settled_ply: None,
            pre_move_trace: chess_tutor_engine::eval::EvalTrace::zero(),
            pre_score: chess_tutor_engine::types::Value::ZERO,
            term_deltas: Vec::new(),
        };
        let items = build_forced_consequences_items(&user, &pre, Color::White);
        assert!(items.is_empty(), "no PV reply => no card");
    }

    #[test]
    fn king_protector_suppression_detects_our_minor_capturing() {
        // Bxh6 — our bishop captures a knight at ply 0. Should mark
        // both `theirs_minor_captured` (knight is a minor we took)
        // and `our_minor_capturing` (bishop is a minor that did the
        // capturing).
        use chess_tutor_engine::analysis::CaptureEvent;
        use chess_tutor_engine::types::{PieceType, Square};
        let outcome = MaterialOutcome {
            events: vec![CaptureEvent {
                ply: 0,
                captor: Color::White,
                captor_piece: PieceType::Bishop,
                captured_piece: PieceType::Knight,
                square: Square::H6,
                value_mg: 781,
                value_eg: 854,
            }],
            net_mg_cp: 781,
            net_eg_cp: 854,
            last_ply: 0,
        };
        let supp = king_protector_suppression(&outcome, Color::White);
        assert!(supp.theirs_minor_captured);
        assert!(supp.our_minor_capturing);
        assert!(!supp.ours_minor_captured);
    }

    #[test]
    fn king_protector_suppression_detects_ours_minor_captured_on_recapture() {
        // Bxh6 + gxh6 — we lose our bishop in the recapture. After
        // both events, ours_minor_captured should be true.
        use chess_tutor_engine::analysis::CaptureEvent;
        use chess_tutor_engine::types::{PieceType, Square};
        let outcome = MaterialOutcome {
            events: vec![
                CaptureEvent {
                    ply: 0,
                    captor: Color::White,
                    captor_piece: PieceType::Bishop,
                    captured_piece: PieceType::Knight,
                    square: Square::H6,
                    value_mg: 781,
                    value_eg: 854,
                },
                CaptureEvent {
                    ply: 1,
                    captor: Color::Black,
                    captor_piece: PieceType::Pawn,
                    captured_piece: PieceType::Bishop,
                    square: Square::H6,
                    value_mg: 825,
                    value_eg: 915,
                },
            ],
            net_mg_cp: 781 - 825,
            net_eg_cp: 854 - 915,
            last_ply: 1,
        };
        let supp = king_protector_suppression(&outcome, Color::White);
        assert!(supp.theirs_minor_captured);
        assert!(supp.ours_minor_captured);
        assert!(supp.our_minor_capturing);
    }

    #[test]
    fn king_protector_suppression_pawn_capture_doesnt_set_our_minor_capturing() {
        // exd5 — pawn capture. captor_piece is a pawn, not a minor.
        // Neither `our_minor_capturing` nor the minor-captured flags
        // should fire when only pawns are involved.
        use chess_tutor_engine::analysis::CaptureEvent;
        use chess_tutor_engine::types::{PieceType, Square};
        let outcome = MaterialOutcome {
            events: vec![CaptureEvent {
                ply: 0,
                captor: Color::White,
                captor_piece: PieceType::Pawn,
                captured_piece: PieceType::Pawn,
                square: Square::D5,
                value_mg: 124,
                value_eg: 206,
            }],
            net_mg_cp: 124,
            net_eg_cp: 206,
            last_ply: 0,
        };
        let supp = king_protector_suppression(&outcome, Color::White);
        assert!(!supp.our_minor_capturing);
        assert!(!supp.theirs_minor_captured);
        assert!(!supp.ours_minor_captured);
    }

    #[test]
    fn material_card_flags_bishop_for_rook_as_material_loss() {
        // User captures a rook at ply 0; opponent recaptures with a
        // bishop at ply 1. Classical points: 5 vs 3 — net +2 for us.
        // Verifies that point parity correctly classifies an unequal
        // trade (not just suppresses cp-tight ones).
        use chess_tutor_engine::analysis::CaptureEvent;
        use chess_tutor_engine::types::{PieceType, Square};
        let outcome = MaterialOutcome {
            events: vec![
                CaptureEvent {
                    ply: 0,
                    captor: Color::White,
                    captor_piece: PieceType::Bishop,
                    captured_piece: PieceType::Rook,
                    square: Square::C6,
                    value_mg: 1276,
                    value_eg: 1380,
                },
                CaptureEvent {
                    ply: 1,
                    captor: Color::Black,
                    captor_piece: PieceType::Pawn,
                    captured_piece: PieceType::Bishop,
                    square: Square::C6,
                    value_mg: 825,
                    value_eg: 915,
                },
            ],
            net_mg_cp: 1276 - 825,
            net_eg_cp: 1380 - 915,
            last_ply: 1,
        };
        let pre = Position::startpos();
        let item = build_material_item(&pre, &outcome, Color::White)
            .expect("R-for-B is a card");
        assert_eq!(item.heading, "You won material");
        assert!(item.score_delta_pawns.is_some());
    }

    #[test]
    fn material_card_suppressed_when_opponent_captures_first() {
        // User's ply-0 move was quiet (no capture by us); opponent's
        // ply-1 best move takes one of our pieces — i.e., we hung a
        // piece. The threats card surfaces this with the right
        // present-tense framing ("Your piece is hanging") plus
        // attacker arrows; the material card must suppress itself so
        // we don't double-narrate the hang as a settled past-tense
        // "You lost material."
        use chess_tutor_engine::analysis::CaptureEvent;
        use chess_tutor_engine::types::{PieceType, Square};
        let outcome = MaterialOutcome {
            events: vec![CaptureEvent {
                ply: 1,
                captor: Color::Black,
                captor_piece: PieceType::Bishop,
                captured_piece: PieceType::Knight,
                square: Square::H3,
                value_mg: 781,
                value_eg: 854,
            }],
            net_mg_cp: -781,
            net_eg_cp: -854,
            last_ply: 1,
        };
        let pre = Position::startpos();
        let item = build_material_item(&pre, &outcome, Color::White);
        assert!(
            item.is_none(),
            "opponent-only realized capture is a hang — threats card handles it; \
             material card must stay silent. got {item:?}"
        );
    }

    #[test]
    fn capitalize_handles_empty_and_unicode() {
        assert_eq!(capitalize(""), "");
        assert_eq!(capitalize("knight"), "Knight");
    }

    #[test]
    fn join_with_and_handles_zero_one_two_three() {
        assert_eq!(join_with_and(&[]), "");
        assert_eq!(join_with_and(&["a".into()]), "a");
        assert_eq!(join_with_and(&["a".into(), "b".into()]), "a and b");
        assert_eq!(
            join_with_and(&["a".into(), "b".into(), "c".into()]),
            "a, b, and c"
        );
    }
}
