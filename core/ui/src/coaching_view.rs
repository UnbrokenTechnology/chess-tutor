//! Live-coaching view: features-to-notice on the position the user
//! is about to move from. Never names a move — surfaces structural
//! observations (their piece is undefended, your pawn is backward,
//! the d5 square is contested) so the student picks the move
//! themselves.
//!
//! Distinct from the retrospective:
//! - **Retrospective** describes what *changed* on the last move.
//!   Inputs: pre-move and post-move positions plus a `MoveAnalysis`.
//! - **Coaching** describes the *current* position. Inputs: just the
//!   `Position`. No search runs; the snapshot uses pawn / threats
//!   helpers directly.
//!
//! Compute is intentionally cheap (sub-ms in release) so it can rebuild
//! every frame without any worker round-trip. If a future Coached
//! feature needs a real search (e.g. "this outpost is reachable by
//! …"), we can promote to the worker thread the same way the hint
//! panel does.

use std::collections::HashSet;

use chess_tutor_engine::analysis::{
    classify_tactical_mode, find_overloaded, list_hanging, CheckFollowup, Confidence,
    HangingPiece, LatentThreat, MatePattern, OverloadedPiece, PriorMove, TacticHit, TacticPattern,
    TacticalReason,
};
use chess_tutor_engine::movegen::legal_moves_vec;
use chess_tutor_engine::pawns::PawnsEval;
use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::{Color, Move, MoveKind, PieceType, Square};

use crate::view::{
    AnnotationKind, BoardAnnotation, CoachingItem, CoachingViewModel, RetrospectiveCategory,
    Sentiment,
};

/// Build the structured coaching view from the live position. `user_color`
/// is the side the student is playing — surface "their" cards as
/// opportunities and "ours" cards as risks. The view model is empty
/// (no items) when nothing notable is on the board, in which case
/// the renderer paints an encouraging neutral message.
///
/// `tactic_hint` is an optional named tactic the analytical engine
/// flagged on the user's predicted best line (pre-move tactic surface).
/// When present and of [`Confidence::High`], a "There's a … available"
/// card surfaces first — pattern named, location withheld (per the
/// pedagogical rule that coaching never names squares). Medium-
/// confidence hits are dropped here; the retrospective is where the
/// student studies the actual line.
pub fn build_coaching_view(
    pos: &Position,
    user_color: Color,
    tactic_hint: Option<&TacticHit>,
    prior_move: Option<PriorMove>,
) -> CoachingViewModel {
    let mut items: Vec<CoachingItem> = Vec::new();

    // The tactical-mode gate (PLAN §1/§2): a detectors-only scan of the
    // live position. When it fires, opponent-threat cards lead and the
    // positional (pawn-weakness) cards are demoted under a muted fold.
    // When it does not fire, behaviour is exactly as before — positional
    // cards lead and nothing is demoted. The gate is pure/static/sub-ms,
    // safe to run every frame alongside the rest of coaching.
    let state = classify_tactical_mode(pos, user_color, prior_move);
    let live = state.live;

    // Tactic card first when one fires — it's the strongest signal
    // ("look for a fork here") and the rest of the panel becomes
    // secondary context once the student knows there's a combination
    // to find. The High-confidence gate keeps misfires off this
    // pre-move surface (see HANDOFF-ux Tactic library design brief).
    //
    // This is the OurTactic surface. We route it through the existing
    // session-fed `tactic_hint` (PV-reuse + static scan, richer than the
    // gate's bare `find_best_tactic_in_position`) and therefore SKIP the
    // gate's own `OurTactic` reason below to avoid emitting two tactic
    // cards for the same combination.
    if let Some(hit) = tactic_hint {
        if hit.confidence == Confidence::High {
            items.push(tactic_card(hit));
        }
    }

    // New opponent-threat cards, emitted from the gate's `reasons` in
    // priority order (the vec is already sorted: InCheck <
    // OpponentLatentThreat < OpponentCheckFollowup < ForcingCheckChain <
    // OurTactic < LoosePiece). We render only the *new* threat cards
    // here. InCheck / LoosePiece / OurTactic are already served by the
    // existing richer builders below (check_card, opportunity/risk/
    // en-passant/overloaded, tactic_hint), so we skip those reasons to
    // avoid double-emission. The new cards lead because the gate places
    // them above the existing low-priority loose-piece/positional cards.
    for reason in &state.reasons {
        match reason {
            TacticalReason::OpponentLatentThreat(threat) => {
                items.push(latent_threat_card(threat));
            }
            TacticalReason::OpponentCheckFollowup(cf) => {
                items.push(check_followup_card(cf));
            }
            TacticalReason::ForcingCheckChain { depth } => {
                items.push(king_hunt_card(*depth));
            }
            // InCheck -> existing check_card (emitted below, with its
            // richer checker/response-count detail).
            // OurTactic -> existing tactic_hint card (emitted above).
            // LoosePiece -> existing opportunity/risk/en-passant cards
            //   (emitted below, legal-filtered and with attacker info).
            TacticalReason::InCheck
            | TacticalReason::OurTactic(_)
            | TacticalReason::LoosePiece { .. } => {}
        }
    }

    // Compute legal moves once. Used for:
    //   - Surfacing the "you're in check" card with concrete
    //     check-addressing context.
    //   - Filtering "Look for a capture" to only opportunities a
    //     legal move can actually take. Without this, the static
    //     hanging-piece list overcounts: pinned attackers and being-
    //     in-check are both cases where we *attack* a square but
    //     can't legally capture there.
    let mut scratch = pos.clone();
    let legal_moves = legal_moves_vec(&mut scratch);
    let legal_destinations: HashSet<Square> =
        legal_moves.iter().map(|m| m.to()).collect();

    // Check card first — when the king is under attack, that's the
    // only thing the student should be thinking about. Everything
    // else is filtered to keep the panel honest.
    if pos.in_check() && pos.side_to_move() == user_color {
        items.push(check_card(pos, &legal_moves));
    }

    // En passant: high-signal, easy to miss, deserves its own card
    // when available. The static `list_hanging` scan won't flag an
    // en-passant-capturable pawn because en passant isn't a normal
    // attack — the captured pawn sits on a square no enemy piece
    // directly attacks. We have to detect it from the legal-move
    // list itself.
    let en_passant_moves: Vec<Move> = legal_moves
        .iter()
        .copied()
        .filter(|m| m.kind() == MoveKind::EnPassant)
        .collect();
    if !en_passant_moves.is_empty() {
        items.push(en_passant_card(&en_passant_moves));
    }

    // Opportunity scan: undefended opponent pieces *we can actually
    // reach* with a legal move. The static `list_hanging` only checks
    // attacker bitboards, so a pinned attacker or a check-blocked
    // capture would still appear — we filter those out here.
    let theirs_hanging = list_hanging(pos, !user_color);
    let theirs_capturable: Vec<HangingPiece> = theirs_hanging
        .into_iter()
        .filter(|h| legal_destinations.contains(&h.location.square))
        .collect();
    if !theirs_capturable.is_empty() {
        items.push(opportunity_card(&theirs_capturable));
    }

    // Overloaded enemy pieces: a defender doing two jobs at once. A
    // pre-move structural observation (not a found combination), so
    // we name the squares the same way `opportunity_card` does — the
    // student still has to find the *move* that forces the choice.
    // The strict sole-defender-of-≥2 predicate keeps misfires low (see
    // overloading.rs //!); even so the surface is conservative for
    // now — no retrospective version yet, just coaching.
    let theirs_overloaded = find_overloaded(pos, !user_color);
    if !theirs_overloaded.is_empty() {
        items.push(overloaded_card(pos, &theirs_overloaded));
    }

    // Risk scan: our undefended pieces. We don't legal-move-filter
    // these because the threat is about the opponent's *next* turn
    // (which the engine can't fully evaluate without searching) —
    // a loose piece warrants the student's attention regardless of
    // whose turn it is right now.
    let ours_hanging = list_hanging(pos, user_color);
    if !ours_hanging.is_empty() {
        items.push(risk_card(&ours_hanging));
    }

    // Pawn weakness scan for both sides. Builds at most one card per
    // side per weakness kind (doubled / isolated / backward), so the
    // panel stays scannable. These are *positional* cards — when the
    // gate is live they are demoted (rendered after the tactical cards,
    // collapsed under a muted "Quiet-position notes" fold).
    let pawns = chess_tutor_engine::pawns::evaluate(pos);
    let mut positional: Vec<CoachingItem> = Vec::new();
    positional.extend(pawn_weakness_cards(&pawns, user_color, true));
    positional.extend(pawn_weakness_cards(&pawns, !user_color, false));
    if live {
        for card in &mut positional {
            card.demoted = true;
        }
    }
    items.extend(positional);

    CoachingViewModel { items }
}

/// Build the "There's a fork available" pre-move tactic card.
///
/// **No square annotations** by design — the pedagogical rule for
/// pre-move coaching is to name the pattern (so the student knows
/// what shape to look for) without telling them where it is (so they
/// do the work of finding it). The detail text echoes the
/// retrospective's per-pattern lesson so the surface terminology stays
/// consistent across the two panels.
fn tactic_card(hit: &TacticHit) -> CoachingItem {
    let heading = if hit.pattern == TacticPattern::Checkmate {
        match hit.mate_pattern {
            Some(MatePattern::BackRank) => "Look for a back-rank mate".to_string(),
            Some(MatePattern::Smothered) => "Look for a smothered mate".to_string(),
            _ => "There's a forced mate".to_string(),
        }
    } else {
        format!("There's {} available", coaching_pattern_phrase(hit.pattern))
    };
    let summary = if hit.pattern == TacticPattern::Checkmate {
        "the engine sees a forced mate from here".to_string()
    } else if hit.material_gain.is_some_and(|g| g > 0) {
        "the engine sees a winning combination".to_string()
    } else {
        "the engine sees the pattern in this position".to_string()
    };
    let detail = format!(
        "{} Look for it before you move. If you can't find it, play the best \
         move you can — you can review this position afterwards to study what \
         was there.",
        coaching_pattern_lesson(hit.pattern)
    );
    CoachingItem {
        category: RetrospectiveCategory::Tactic,
        heading,
        summary,
        detail,
        sentiment: Sentiment::Positive,
        // Pedagogically: no annotations on the coaching surface — the
        // student should locate the pattern themselves.
        annotations: Vec::new(),
        demoted: false,
    }
}

fn coaching_pattern_phrase(pattern: TacticPattern) -> &'static str {
    // Coaching prose mirrors the retrospective's pattern_phrase, kept
    // separate so the two surfaces can iterate independently if needed
    // (e.g. coaching might add "you might have"/"you may have a … here"
    // hedges).
    match pattern {
        TacticPattern::Fork => "a fork",
        TacticPattern::HangingCapture => "a free piece",
        TacticPattern::RemovingDefender => "a removing-the-defender tactic",
        TacticPattern::TrappedPiece => "a trapped piece",
        TacticPattern::Pin => "a pin",
        TacticPattern::RelativePin => "a relative pin",
        TacticPattern::Skewer => "a skewer",
        TacticPattern::DiscoveredAttack => "a discovered attack",
        TacticPattern::DiscoveredCheck => "a discovered check",
        TacticPattern::DoubleCheck => "a double-check tactic",
        TacticPattern::Sacrifice => "a sound sacrifice",
        TacticPattern::Intermezzo => "an in-between move",
        TacticPattern::Deflection => "a deflection",
        TacticPattern::Attraction => "an attraction",
        TacticPattern::Interference => "an interference tactic",
        TacticPattern::Clearance => "a clearance",
        TacticPattern::XRay => "an x-ray battery",
        TacticPattern::AttackingF2F7 => "an attack on f2/f7",
        TacticPattern::UnderPromotion => "an under-promotion",
        // Mate is handled at the call site (different heading shape).
        TacticPattern::Checkmate => "checkmate",
    }
}

fn coaching_pattern_lesson(pattern: TacticPattern) -> &'static str {
    // One-sentence reminder of the pattern shape. Distinct from the
    // retrospective's longer lesson — keep coaching brief so the
    // student isn't reading more than thinking.
    match pattern {
        TacticPattern::Fork => "A single piece can attack two of your opponent's pieces at once.",
        TacticPattern::HangingCapture => "One of your opponent's pieces is attacked and undefended.",
        TacticPattern::RemovingDefender => "If you capture the only defender of an enemy piece, that piece falls next.",
        TacticPattern::TrappedPiece => "An enemy piece has no safe square — every move it can make loses material.",
        TacticPattern::Pin => "A piece in your opponent's army can't move at all — its king is directly behind it.",
        TacticPattern::RelativePin => "A piece in your opponent's army shouldn't move — a more valuable piece sits behind it — but it legally can, so watch for a forcing move that breaks the pin.",
        TacticPattern::Skewer => "Two enemy pieces line up — the more valuable one in front must move, exposing what's behind.",
        TacticPattern::DiscoveredAttack => "Moving a piece can unmask an attack from a friend behind it. Two threats land at once.",
        TacticPattern::DiscoveredCheck => "Moving a piece can unmask a check from a friend behind it. The moving piece is free to do something extra.",
        TacticPattern::DoubleCheck => "Two pieces deliver check at once — the king must move; blocking and capturing don't work.",
        TacticPattern::Sacrifice => "Give up material now to gain something more important later — winning attack, decisive position, more material.",
        TacticPattern::Intermezzo => "Instead of the expected recapture, insert a forcing move first. The opponent must respond before the trade resumes.",
        TacticPattern::Deflection => "Pull an enemy defender off its duty — what it was guarding falls.",
        TacticPattern::Attraction => "Lure an enemy piece (often the king) onto a square where it can be attacked decisively.",
        TacticPattern::Interference => "Block the line between an enemy piece and what it's defending — the defender's reach is broken.",
        TacticPattern::Clearance => "Move a piece off its square to clear the line for a piece behind it.",
        TacticPattern::XRay => "Stack two of your pieces on the same file or diagonal — when the front one captures, the back one recaptures.",
        TacticPattern::AttackingF2F7 => "f2 and f7 are the weak points by an uncastled king — only the king itself defends them.",
        TacticPattern::UnderPromotion => "Promoting to a knight (or rook/bishop) is sometimes stronger than to a queen — usually for an immediate mate or to avoid stalemate.",
        TacticPattern::Checkmate => "There's a forced mating sequence available from here.",
    }
}

/// Build the "Your king is in check" card. Highlights the king and
/// every checking piece, and counts how many of the legal moves are
/// king moves vs. blocks/captures so the student knows what kind of
/// response options exist. Never names a specific move.
fn check_card(pos: &Position, legal_moves: &[Move]) -> CoachingItem {
    let us = pos.side_to_move();
    let king_sq = pos.king_square(us);
    let checkers = pos.checkers();

    let mut annotations = Vec::new();
    annotations.push(BoardAnnotation::SquareHighlight {
        square: king_sq,
        kind: AnnotationKind::Threat,
    });
    let mut checker_locations: Vec<(Square, PieceType)> = Vec::new();
    for sq in checkers {
        if let Some(piece) = pos.piece_on(sq) {
            checker_locations.push((sq, piece.kind()));
            annotations.push(BoardAnnotation::Arrow {
                from: sq,
                to: king_sq,
                kind: AnnotationKind::Attacker,
            });
        }
    }

    // Count response shapes from the legal moves. King moves =
    // "run." Other moves = "block or capture the checker." We don't
    // distinguish block vs capture in the count because the student
    // shouldn't need that split to reason — the categories are
    // really "move the king" vs "everything else."
    let king_move_count = legal_moves
        .iter()
        .filter(|m| m.from() == king_sq)
        .count();
    let other_count = legal_moves.len() - king_move_count;

    let summary = if checker_locations.len() == 1 {
        let (sq, piece) = checker_locations[0];
        format!(
            "checked by {} on {}",
            piece_name(piece),
            sq.to_algebraic()
        )
    } else {
        format!("double check by {} pieces", checker_locations.len())
    };

    let response_text = match (king_move_count, other_count) {
        (0, 0) => "No legal moves — this is checkmate.".to_string(),
        (k, 0) => format!(
            "{} king move{} available — the king must move (double check, or no piece can block or capture).",
            k,
            if k == 1 { "" } else { "s" }
        ),
        (0, n) => format!(
            "{} blocking-or-capturing move{} available — the king has nowhere safe to go.",
            n,
            if n == 1 { "" } else { "s" }
        ),
        (k, n) => format!(
            "{} response{} total: {} king move{} and {} block-or-capture move{}.",
            k + n,
            if k + n == 1 { "" } else { "s" },
            k,
            if k == 1 { "" } else { "s" },
            n,
            if n == 1 { "" } else { "s" }
        ),
    };

    let detail = format!(
        "Your king is in check — the only legal responses are moving the king \
         to safety, blocking the line of attack, or capturing the checking piece. \
         {} Other observations on the position only matter once the check is \
         addressed.",
        response_text
    );

    CoachingItem {
        category: RetrospectiveCategory::KingSafety,
        heading: "Your king is in check".to_string(),
        summary,
        detail,
        sentiment: Sentiment::Negative,
        annotations,
        demoted: false,
    }
}

/// Build the **opponent-latent-threat** card (PLAN §2). The opponent
/// has a tactic *loaded* against the student — a discovered attack,
/// pin, skewer, or removing-the-defender alignment that fires if the
/// student's move doesn't address it.
///
/// **Names the pattern, withholds the squares** — same pedagogical rule
/// as `tactic_card`: the student is told *what shape* the opponent has
/// so they know to defuse it, but they find *where* it is themselves.
/// No board annotations for that reason. Sentiment::Negative — this is
/// a danger to the student, not an opportunity.
fn latent_threat_card(threat: &LatentThreat) -> CoachingItem {
    let pattern_name = latent_pattern_name(threat.pattern);
    let heading = format!("Your opponent has {} loaded", pattern_name);
    let summary = "address it before you do anything else".to_string();
    let detail = format!(
        "{} They have it set up right now — a move that doesn't disrupt it lets \
         them fire it on their turn. Before you play, ask what your opponent has \
         ready against you, and make sure your move takes it away (capture the \
         piece that makes it work, block the line, or defend the target). \
         A natural-looking move that ignores it hands the game over.",
        latent_pattern_lesson(threat.pattern)
    );
    CoachingItem {
        category: RetrospectiveCategory::Threats,
        heading,
        summary,
        detail,
        sentiment: Sentiment::Negative,
        // No square annotations — the student locates the alignment.
        annotations: Vec::new(),
        demoted: false,
    }
}

/// Human-readable pattern name for a latent (opponent-loaded) threat,
/// with an indefinite article so it reads in the heading
/// ("Your opponent has a discovered attack loaded").
fn latent_pattern_name(pattern: TacticPattern) -> &'static str {
    match pattern {
        TacticPattern::DiscoveredAttack => "a discovered attack",
        TacticPattern::DiscoveredCheck => "a discovered check",
        TacticPattern::Pin => "a pin",
        TacticPattern::RelativePin => "a relative pin",
        TacticPattern::Skewer => "a skewer",
        TacticPattern::RemovingDefender => "a removing-the-defender threat",
        // The latent-threat detector only produces the alignment
        // patterns above; anything else falls back to a neutral phrase.
        _ => "a tactic",
    }
}

/// One-sentence reminder of how the loaded pattern works — mirrors
/// `coaching_pattern_lesson` but framed defensively (the opponent has
/// it; you must take it away).
fn latent_pattern_lesson(pattern: TacticPattern) -> &'static str {
    match pattern {
        TacticPattern::DiscoveredAttack => "One of their pieces is blocking an attack from a piece behind it — when they move the front piece (often with a check or capture you have to answer), the attack behind it lands.",
        TacticPattern::DiscoveredCheck => "One of their pieces is blocking a check from a piece behind it — when they move the front piece, the check lands and that piece is free to grab something.",
        TacticPattern::Pin => "One of your pieces can't move — your king is directly behind it — so a piece of theirs is bearing down on it for free.",
        TacticPattern::RelativePin => "One of your pieces shouldn't move — something more valuable sits behind it — so they can pile on the pinned piece.",
        TacticPattern::Skewer => "Two of your pieces line up; the more valuable one in front will have to move and expose what's behind it.",
        TacticPattern::RemovingDefender => "One of your pieces is the only thing defending another — if they take or chase that defender away, the piece it was guarding falls.",
        _ => "They have a tactic set up against you.",
    }
}

/// Build the **opponent-check-followup** card (PLAN §2,
/// `double-fork-after-qd8`). Their check isn't a stall: one ply past it,
/// after the student's forced reply, they have a follow-up tactic (a
/// two-step fork). Tell the student to look *past* the check before
/// reacting to it.
///
/// **No squares** — same pre-move pedagogical rule. We name the
/// follow-up pattern when the detector identified one. Sentiment::Negative.
fn check_followup_card(cf: &CheckFollowup) -> CoachingItem {
    // Name the follow-up pattern from the first reply that has one.
    let followup_pattern = cf
        .replies
        .iter()
        .find_map(|r| r.followup.as_ref())
        .map(|hit| coaching_pattern_phrase(hit.pattern))
        .unwrap_or("a follow-up tactic");
    let summary = "look one ply past the check".to_string();
    let detail = format!(
        "Their check isn't just a stall — look one move past it. After you answer \
         the check, they have {}. Don't react to the check on its own; work out \
         what comes *after* your forced reply and defuse that first, because once \
         you're committed to answering the check you may not get another chance.",
        followup_pattern
    );
    CoachingItem {
        category: RetrospectiveCategory::Threats,
        heading: "Their check is the first half of a tactic".to_string(),
        summary,
        detail,
        sentiment: Sentiment::Negative,
        annotations: Vec::new(),
        demoted: false,
    }
}

/// Build the **king-hunt** card (PLAN §2, `mating-net-after-ng5`).
/// SOFT and **mechanism-free** by design: the student's king faces a
/// forcing check sequence that self-replenishes several moves deep.
/// These tend to end in a mating net or a perpetual.
///
/// Critically this card **never names a mate, a line, or a tactic** —
/// the whole point of the ng5 case study is that the engine sees a long
/// forced sequence the student can't be expected to calculate, so we
/// give a directional nudge ("look for a more defensive move") without
/// fabricating a mechanism the student would then try to verify and
/// fail to find. No squares, no annotations. Sentiment::Negative.
fn king_hunt_card(depth: u8) -> CoachingItem {
    let _ = depth; // mechanism-free: depth drives the gate, not the prose.
    let summary = "look for a safer, more defensive move".to_string();
    let detail =
        "Your king faces a forcing check sequence several moves deep — each check \
         leads into another. Sequences like this tend to end in a mating net or a \
         perpetual, even when no single move looks losing. This is a sign to stop \
         attacking and look for a more defensive move that gives your king some \
         air, rather than one that walks further into the checks."
            .to_string();
    CoachingItem {
        category: RetrospectiveCategory::KingSafety,
        heading: "Your king is being hunted".to_string(),
        summary,
        detail,
        sentiment: Sentiment::Negative,
        annotations: Vec::new(),
        demoted: false,
    }
}

/// Build the "En passant capture available" card from one or more
/// legal en-passant moves. Almost always one move in practice
/// (two pawns rarely have simultaneous en-passant chances), but we
/// handle the list generically.
///
/// Geometry: an en-passant move's `to()` is the empty square *behind*
/// the captured pawn from the captured side's POV. The captured pawn
/// itself sits on `Square::new(to.file(), from.rank())` — same file
/// as the destination, same rank as the capturing pawn before it
/// moved.
///
/// The card never names the capturing move; it just points at the
/// pawn that can be taken and reminds the student that en passant
/// is one-shot — only available on the move immediately after the
/// opponent's two-square push.
fn en_passant_card(moves: &[Move]) -> CoachingItem {
    let mut annotations: Vec<BoardAnnotation> = Vec::new();
    let mut captured_squares: Vec<Square> = Vec::new();
    for m in moves {
        let captured_sq = Square::new(m.to().file(), m.from().rank());
        captured_squares.push(captured_sq);
        // Highlight the captured pawn — that's what the student
        // needs to *see*. Also draw the attacker arrow so the
        // geometry of the en-passant move is visually obvious.
        annotations.push(BoardAnnotation::SquareHighlight {
            square: captured_sq,
            kind: AnnotationKind::GoodPiece,
        });
        annotations.push(BoardAnnotation::Arrow {
            from: m.from(),
            to: m.to(),
            kind: AnnotationKind::Attacker,
        });
        // Also highlight the en-passant destination square subtly so
        // the student sees where the capturing pawn would actually
        // land — en passant geometry surprises people because the
        // capturing piece doesn't end up on the captured piece's
        // square.
        annotations.push(BoardAnnotation::SquareHighlight {
            square: m.to(),
            kind: AnnotationKind::NewMobility,
        });
    }
    let summary = if captured_squares.len() == 1 {
        format!("pawn on {}", captured_squares[0].to_algebraic())
    } else {
        let strs: Vec<String> = captured_squares
            .iter()
            .map(|s| s.to_algebraic().to_string())
            .collect();
        format!("pawns on {}", strs.join(", "))
    };
    let detail = "Your opponent's last move was a two-square pawn push that landed \
                  alongside one of your pawns. You can capture it *as if* it had \
                  only moved one square — your pawn moves diagonally to the square \
                  the pawn would have stopped on if it'd only pushed one. \
                  \n\nEn passant is one-shot: it's only legal on the move \
                  immediately following the opponent's two-square push. If you \
                  don't take it now, the opportunity is gone."
        .to_string();
    CoachingItem {
        category: RetrospectiveCategory::Threats,
        heading: "En passant capture available".to_string(),
        summary,
        detail,
        sentiment: Sentiment::Positive,
        annotations,
        demoted: false,
    }
}

fn opportunity_card(hangs: &[HangingPiece]) -> CoachingItem {
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
        annotations.push(BoardAnnotation::SquareHighlight {
            square: h.location.square,
            kind: AnnotationKind::GoodPiece,
        });
        for a in &h.attackers {
            annotations.push(BoardAnnotation::Arrow {
                from: a.square,
                to: h.location.square,
                kind: AnnotationKind::Attacker,
            });
        }
        let attacker_text: Vec<String> = h
            .attackers
            .iter()
            .map(|a| format!("{} on {}", piece_name(a.piece), a.square.to_algebraic()))
            .collect();
        detail_lines.push(format!(
            "{} on {} — attacked by {}, no defenders. Check whether you can capture, \
             and whether the opponent can defend, counter-attack, or threaten back.",
            capitalize(piece_name(h.location.piece)),
            h.location.square.to_algebraic(),
            join_with_and(&attacker_text)
        ));
    }
    CoachingItem {
        category: RetrospectiveCategory::Threats,
        heading: "Look for a capture".to_string(),
        summary,
        detail: detail_lines.join("\n"),
        sentiment: Sentiment::Positive,
        annotations,
        demoted: false,
    }
}

/// Overloaded enemy defender(s) — a piece holding up ≥ 2 of its own
/// pieces under attack. The card names the defender + the squares it
/// is the *sole* protector of, leaving the student to find the move
/// that forces the choice (deflection, capture-and-pin, etc.).
///
/// Single-card-for-all-defenders even if multiple fire, so the panel
/// stays scannable. Annotations: BadPiece on each overloaded defender
/// (the piece about to lose its juggling act) + Threat on each duty
/// (our opportunity squares).
fn overloaded_card(pos: &Position, overloaded: &[OverloadedPiece]) -> CoachingItem {
    let mut annotations: Vec<BoardAnnotation> = Vec::new();
    let mut detail_lines = Vec::new();
    for op in overloaded {
        // Defender — the overloaded piece itself.
        annotations.push(BoardAnnotation::SquareHighlight {
            square: op.piece,
            kind: AnnotationKind::BadPiece,
        });
        // Duties — pieces the defender is the only thing protecting.
        for &duty in &op.duties {
            annotations.push(BoardAnnotation::SquareHighlight {
                square: duty,
                kind: AnnotationKind::Threat,
            });
            // Arrow from defender to duty so the load-bearing geometry
            // is visible at a glance.
            annotations.push(BoardAnnotation::Arrow {
                from: op.piece,
                to: duty,
                kind: AnnotationKind::Defender,
            });
        }
        let defender_name = pos
            .piece_on(op.piece)
            .map(|p| piece_name(p.kind()))
            .unwrap_or("piece");
        let duty_text: Vec<String> = op
            .duties
            .iter()
            .map(|&sq| {
                let dn = pos
                    .piece_on(sq)
                    .map(|p| piece_name(p.kind()))
                    .unwrap_or("piece");
                format!("{} on {}", dn, sq.to_algebraic())
            })
            .collect();
        detail_lines.push(format!(
            "Their {} on {} is the only defender of {}. If you force it off one \
             duty — by capturing it, pinning it, or attacking it strongly enough \
             that it has to move — the piece it was guarding falls.",
            defender_name,
            op.piece.to_algebraic(),
            join_with_and(&duty_text),
        ));
    }
    let summary = if overloaded.len() == 1 {
        format!(
            "{} on {} is doing two jobs",
            pos.piece_on(overloaded[0].piece)
                .map(|p| piece_name(p.kind()))
                .unwrap_or("piece"),
            overloaded[0].piece.to_algebraic(),
        )
    } else {
        format!("{} pieces are overloaded", overloaded.len())
    };
    CoachingItem {
        category: RetrospectiveCategory::Tactic,
        heading: "Their piece is overloaded".to_string(),
        summary,
        detail: detail_lines.join("\n"),
        sentiment: Sentiment::Positive,
        annotations,
        demoted: false,
    }
}

fn risk_card(hangs: &[HangingPiece]) -> CoachingItem {
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
        annotations.push(BoardAnnotation::SquareHighlight {
            square: h.location.square,
            kind: AnnotationKind::Threat,
        });
        for a in &h.attackers {
            annotations.push(BoardAnnotation::Arrow {
                from: a.square,
                to: h.location.square,
                kind: AnnotationKind::Attacker,
            });
        }
        let attacker_text: Vec<String> = h
            .attackers
            .iter()
            .map(|a| format!("{} on {}", piece_name(a.piece), a.square.to_algebraic()))
            .collect();
        detail_lines.push(format!(
            "{} on {} — attacked by {}, no defenders. Consider defending it, moving it, \
             or making a stronger counter-threat.",
            capitalize(piece_name(h.location.piece)),
            h.location.square.to_algebraic(),
            join_with_and(&attacker_text)
        ));
    }
    CoachingItem {
        category: RetrospectiveCategory::Threats,
        heading: "Watch your loose piece".to_string(),
        summary,
        detail: detail_lines.join("\n"),
        sentiment: Sentiment::Negative,
        annotations,
        demoted: false,
    }
}

/// Doubled / isolated / backward pawn snapshot. `is_ours` flips the
/// framing: our weakness reads as a risk, theirs reads as something
/// to exploit.
fn pawn_weakness_cards(
    eval: &PawnsEval,
    color: Color,
    is_ours: bool,
) -> Vec<CoachingItem> {
    let bd = &eval.breakdowns[color.index()];
    let mut items = Vec::new();
    // Threshold matches the forced-consequences card (8 cp) — same
    // pedagogical level: small but real structural facts that
    // matter long-term.
    const COACHING_PAWN_THRESHOLD_CP: i32 = -8;
    let (doubled_mg, isolated_mg, backward_mg) =
        (bd.doubled.mg().0, bd.isolated.mg().0, bd.backward.mg().0);
    let (heading_us, heading_them) = (
        ("Your pawns are weak", "Their pawns are weak"),
        ("Your pawns are strong", "Their pawns are strong"),
    );
    let _ = heading_them.1; // silence warning; future "strong pawn" card
    let mut weaknesses: Vec<&str> = Vec::new();
    if doubled_mg <= COACHING_PAWN_THRESHOLD_CP {
        weaknesses.push("doubled pawn");
    }
    if isolated_mg <= COACHING_PAWN_THRESHOLD_CP {
        weaknesses.push("isolated pawn");
    }
    if backward_mg <= COACHING_PAWN_THRESHOLD_CP {
        weaknesses.push("backward pawn");
    }
    if weaknesses.is_empty() {
        return items;
    }
    let (heading, sentiment) = if is_ours {
        (heading_us.0, Sentiment::Negative)
    } else {
        (heading_us.1, Sentiment::Positive)
    };
    let summary = weaknesses.join(", ");
    let detail = if is_ours {
        format!(
            "Your pawn structure has {}. These are long-term weaknesses — they're \
             hard to defend in the endgame. When you choose a move, consider \
             whether you can resolve the weakness or whether the move makes it \
             worse.",
            join_with_and(&weaknesses.iter().map(|s| (*s).to_string()).collect::<Vec<_>>())
        )
    } else {
        format!(
            "Their pawn structure has {}. These are targets you can pressure — \
             they're hard for the opponent to defend, especially as material \
             comes off the board.",
            join_with_and(&weaknesses.iter().map(|s| (*s).to_string()).collect::<Vec<_>>())
        )
    };
    items.push(CoachingItem {
        category: RetrospectiveCategory::PawnStructure,
        heading: heading.to_string(),
        summary,
        detail,
        sentiment,
        annotations: Vec::new(),
        demoted: false,
    });
    items
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

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn join_with_and(items: &[String]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].clone(),
        2 => format!("{} and {}", items[0], items[1]),
        _ => {
            let head = &items[..items.len() - 1];
            format!("{}, and {}", head.join(", "), items[items.len() - 1])
        }
    }
}

#[cfg(test)]
#[path = "coaching_view_tests.rs"]
mod tests;
