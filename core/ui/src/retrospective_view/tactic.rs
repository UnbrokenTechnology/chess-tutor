//! Tactic card builder — "you played a fork", "you missed a pin".
//!
//! Consumes [`compute_tactic_outcome`] and emits up to two
//! [`RetrospectiveItem`]s: a played-tactic card and a missed-tactic
//! card. The "walked-into" slot is handled separately by
//! [`super::forced_consequences::build_forced_consequences_items`] so
//! it co-exists with the structural-concession surface (HANDOFF-ux
//! "Three surfaces" decision).
//!
//! Pedagogical rules in force here (per memory
//! `feedback_teaching_terminology`):
//! - Use chess vocabulary where it's precise (*"fork"*, *"pin"*,
//!   *"skewer"*); plain English where the engine's signal doesn't fit
//!   the technical meaning exactly.
//! - When [`LearningPreferences::reveal_best_moves`] is off, the
//!   missed-tactic card surfaces the *concept* without naming the move
//!   or pointing at the squares — same posture as the headline best-
//!   move arrow.

use chess_tutor_engine::analysis::{
    compute_tactic_outcome, MatePattern, MoveAnalysis, PriorMove, TacticHit, TacticPattern,
};
use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::Color;

use crate::view::{
    AnnotationKind, BoardAnnotation, RetrospectiveCategory, RetrospectiveItem, Sentiment,
};

/// Build every tactic-related item for one analysed move — played,
/// missed, and walked-into — in display order. One
/// [`compute_tactic_outcome`] call covers all three slots.
///
/// `reveal_best_moves` controls whether the *missed-tactic* card emits
/// board annotations (which would reveal the engine's preferred move's
/// location). When off, the card still appears so the student knows a
/// concept was available, but with no spatial spoilers — same posture
/// as the headline's `best_move_annotation` gate. Played and walked-
/// into cards always paint their annotations (the student needs to
/// see *their own* tactic; the warning about an opponent's tactic is
/// pedagogically more useful with squares shown).
pub(super) fn build_tactic_items(
    pre_move_pos: &Position,
    best: &MoveAnalysis,
    user: &MoveAnalysis,
    root_stm: Color,
    prior_move: Option<PriorMove>,
    reveal_best_moves: bool,
) -> Vec<RetrospectiveItem> {
    let outcome = compute_tactic_outcome(best, user, pre_move_pos, root_stm, prior_move);
    let mut items = Vec::new();
    if let Some(hit) = outcome.user_played_tactic {
        items.push(played_item(&hit));
    }
    if let Some(hit) = outcome.user_missed_tactic {
        items.push(missed_item(&hit, reveal_best_moves));
    }
    if let Some(hit) = outcome.user_walked_into {
        items.push(walked_into_item(&hit));
    }
    items
}

// ------------------------------------------------------------------------
// Card builders
// ------------------------------------------------------------------------

fn played_item(hit: &TacticHit) -> RetrospectiveItem {
    RetrospectiveItem {
        category: RetrospectiveCategory::Tactic,
        heading: played_heading(hit),
        summary: played_summary(hit),
        detail: played_detail(hit),
        score_delta_pawns: hit.material_gain.map(|cp| cp as f32 / 100.0),
        sentiment: Sentiment::Positive,
        annotations: tactic_annotations(hit),
    }
}

fn missed_item(hit: &TacticHit, reveal_best_moves: bool) -> RetrospectiveItem {
    RetrospectiveItem {
        category: RetrospectiveCategory::Tactic,
        heading: missed_heading(hit),
        summary: missed_summary(hit),
        detail: missed_detail(hit, reveal_best_moves),
        // Show the gap as a negative number — the student missed *that*
        // much material.
        score_delta_pawns: hit.material_gain.map(|cp| -(cp as f32) / 100.0),
        sentiment: Sentiment::Negative,
        // Suppress spatial hints when reveal_best_moves is off — see
        // module doc.
        annotations: if reveal_best_moves {
            tactic_annotations(hit)
        } else {
            Vec::new()
        },
    }
}

fn walked_into_item(hit: &TacticHit) -> RetrospectiveItem {
    // "Walked into" framing: the opponent now has the tactic on the
    // best reply. Pattern names the lesson; targets are *our* pieces
    // (from the opponent's POV) that the tactic hits. Always surface
    // annotations — unlike missed-tactic, this isn't revealing the
    // *student's* missed move; it's warning about the opponent's
    // response, which the student needs to see to learn the cost of
    // their own move.
    RetrospectiveItem {
        category: RetrospectiveCategory::Tactic,
        heading: walked_into_heading(hit),
        summary: walked_into_summary(hit),
        detail: walked_into_detail(hit),
        score_delta_pawns: hit.material_gain.map(|cp| -(cp as f32) / 100.0),
        sentiment: Sentiment::Negative,
        annotations: tactic_annotations(hit),
    }
}

// ------------------------------------------------------------------------
// Headings — short card title
// ------------------------------------------------------------------------

fn played_heading(hit: &TacticHit) -> String {
    base_heading_with_mate(hit, "You played")
}

fn missed_heading(hit: &TacticHit) -> String {
    base_heading_with_mate(hit, "You missed")
}

fn walked_into_heading(hit: &TacticHit) -> String {
    // "If they reply, they get …" — mirrors the existing forced-
    // consequences card framing without saying "this forces".
    let pattern_name = pattern_phrase(hit.pattern);
    let suffix = mate_suffix(hit);
    format!("If they reply, they get {pattern_name}{suffix}")
}

/// "{prefix} a fork", "{prefix} checkmate (back-rank)".
fn base_heading_with_mate(hit: &TacticHit, prefix: &str) -> String {
    let pattern_name = pattern_phrase(hit.pattern);
    let suffix = mate_suffix(hit);
    format!("{prefix} {pattern_name}{suffix}")
}

/// Lower-case sentence-fragment form of the pattern's name, with the
/// article ("a fork", "the bishop pair sacrifice") — used inside heading
/// templates ("You played {…}"). [`TacticPattern::heading()`] returns
/// the title-case standalone form ("Fork", "Free piece") which doesn't
/// flow inside a sentence.
fn pattern_phrase(pattern: TacticPattern) -> &'static str {
    match pattern {
        TacticPattern::Fork => "a fork",
        TacticPattern::HangingCapture => "a free piece",
        TacticPattern::RemovingDefender => "removing the defender",
        TacticPattern::TrappedPiece => "a trapped piece",
        TacticPattern::Pin => "a pin",
        TacticPattern::Skewer => "a skewer",
        TacticPattern::DiscoveredAttack => "a discovered attack",
        TacticPattern::DiscoveredCheck => "a discovered check",
        TacticPattern::DoubleCheck => "double check",
        TacticPattern::Sacrifice => "a sound sacrifice",
        TacticPattern::Intermezzo => "an in-between move",
        TacticPattern::Deflection => "a deflection",
        TacticPattern::Attraction => "an attraction",
        TacticPattern::Interference => "interference",
        TacticPattern::Clearance => "a clearance",
        TacticPattern::XRay => "an x-ray",
        TacticPattern::AttackingF2F7 => "an attack on f2/f7",
        TacticPattern::UnderPromotion => "an under-promotion",
        TacticPattern::Checkmate => "checkmate",
    }
}

/// " — back-rank mate" / " — smothered mate" suffix when the line
/// terminates in a named everyday mate. Other recognised mates exist
/// in the library but stay engine-only until the named-mates UI lands
/// (see [`MatePattern::surfaced_by_default`]).
fn mate_suffix(hit: &TacticHit) -> String {
    match hit.mate_pattern {
        Some(mp) if mp.surfaced_by_default() => {
            format!(" — {}", mate_phrase(mp))
        }
        _ => String::new(),
    }
}

fn mate_phrase(mate: MatePattern) -> &'static str {
    match mate {
        MatePattern::BackRank => "back-rank mate",
        MatePattern::Smothered => "smothered mate",
        // The non-default-surfaced patterns are still emitted as text
        // when they ride alongside a played-tactic card (rare, but
        // happens — e.g. a deflection that delivers Anastasia's). 1200
        // student probably doesn't know the name, so we keep the call
        // site behind `surfaced_by_default` for now; this exists for
        // the eventual richer-mates pass.
        MatePattern::Anastasia => "Anastasia's mate",
        MatePattern::Hook => "hook mate",
        MatePattern::Arabian => "Arabian mate",
        MatePattern::Boden => "Boden's mate",
        MatePattern::DoubleBishop => "double-bishop mate",
        MatePattern::Dovetail => "dovetail mate",
    }
}

// ------------------------------------------------------------------------
// Summaries — one-line subtitle under the heading
// ------------------------------------------------------------------------

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
        (_, Some(gain)) if gain > 0 => format!(
            "the engine's line wins material ({:+.2})",
            gain as f32 / 100.0
        ),
        _ => "the engine had a tactic available".into(),
    }
}

fn walked_into_summary(hit: &TacticHit) -> String {
    match (hit.pattern, hit.material_gain) {
        (TacticPattern::Checkmate, _) => "their reply forces mate".into(),
        (_, Some(gain)) if gain > 0 => format!(
            "their reply wins material ({:+.2})",
            gain as f32 / 100.0
        ),
        _ => "their reply lands a tactic".into(),
    }
}

// ------------------------------------------------------------------------
// Details — expanded prose shown when the card is selected
// ------------------------------------------------------------------------

fn played_detail(hit: &TacticHit) -> String {
    let lesson = pattern_lesson(hit.pattern);
    let mate = mate_detail(hit);
    let sac = if hit.sacrifice && hit.pattern != TacticPattern::Sacrifice {
        " The line also gives up material — a sacrifice — but the engine \
         confirms the combination is sound: you're at least equal once it \
         resolves."
    } else {
        ""
    };
    format!("{lesson}{mate}{sac}")
}

fn missed_detail(hit: &TacticHit, reveal_best_moves: bool) -> String {
    let lesson = pattern_lesson(hit.pattern);
    let mate = mate_detail(hit);
    let location = if reveal_best_moves {
        " The engine's preferred move sets it up — see the highlighted \
         pieces below."
    } else {
        " Look for this pattern in your own moves next time — the engine \
         saw one here."
    };
    format!("{lesson}{mate}{location}")
}

fn walked_into_detail(hit: &TacticHit) -> String {
    let lesson = pattern_lesson(hit.pattern);
    let mate = mate_detail(hit);
    format!(
        "After your move, the opponent's best response lands this pattern. \
         {lesson}{mate} They may decide not to play it, but if they do, \
         this is the cost."
    )
}

/// Per-pattern teaching paragraph. Kept short — one or two sentences;
/// the goal is for the student to recognise the *shape*, not memorise
/// a definition.
fn pattern_lesson(pattern: TacticPattern) -> &'static str {
    match pattern {
        TacticPattern::Fork => {
            "A fork is one piece attacking two or more enemy pieces at \
             once — they can't all be defended, so one falls."
        }
        TacticPattern::HangingCapture => {
            "An enemy piece was attacked and undefended — a free piece. \
             Spotting hanging pieces is the most reliable source of \
             material at any level."
        }
        TacticPattern::RemovingDefender => {
            "Capturing the only piece defending another enemy piece — \
             the defender goes, the piece it was guarding falls next."
        }
        TacticPattern::TrappedPiece => {
            "An enemy piece with no safe square: every move it can make \
             loses material or is impossible. Often the player who lands \
             the trapped piece will recover the cost of any sacrifice \
             needed to seal off escape squares."
        }
        TacticPattern::Pin => {
            "A pinned piece can't move without exposing a more valuable \
             piece (or its king) behind it. Pin a defender and the piece \
             it defends becomes free; pin an attacker and its threat \
             goes away."
        }
        TacticPattern::Skewer => {
            "A piece attacks two enemy pieces in a line — the more \
             valuable one in front must move, exposing the one behind. \
             The opposite of a pin geometrically; usually decisive when \
             the front piece is the king."
        }
        TacticPattern::DiscoveredAttack => {
            "Moving one piece unmasks an attack from a friend behind it. \
             Discovered attacks combine the threat of the moved piece \
             with the threat of the unmasked one — both must be \
             addressed at once."
        }
        TacticPattern::DiscoveredCheck => {
            "Moving one piece unmasks a check from a friend behind it. \
             Because the king must respond to the check, the moved \
             piece is free to grab something — discovered checks are \
             one of the most powerful tactical motifs."
        }
        TacticPattern::DoubleCheck => {
            "Two pieces give check at the same time. The king must \
             move — blocking and capturing don't work against double \
             check — which usually narrows the response to a single \
             move (or none)."
        }
        TacticPattern::Sacrifice => {
            "Giving up material to gain something more important: \
             a winning attack, a decisive position, or recovering more \
             material later. The engine confirms this combination is \
             sound — you're at least equal after it resolves."
        }
        TacticPattern::Intermezzo => {
            "An in-between move: instead of making the expected \
             recapture, you insert a forcing move elsewhere first. The \
             opponent must respond to the threat before you complete \
             the original sequence — often turning an even trade into a \
             material gain."
        }
        TacticPattern::Deflection => {
            "Pulling an enemy defender off its duty square. The piece \
             it was guarding then falls, or the square it was \
             controlling becomes available."
        }
        TacticPattern::Attraction => {
            "Drawing an enemy piece — usually the king — onto a square \
             where it can be checked or attacked decisively. Often by \
             offering a piece the opponent can't resist taking."
        }
        TacticPattern::Interference => {
            "Cutting the line between an enemy piece and what it was \
             defending. The defender's reach is broken; the piece it \
             was guarding becomes vulnerable."
        }
        TacticPattern::Clearance => {
            "Moving a piece off a square to clear the line for a \
             friend behind it. The clearing move usually carries a \
             threat of its own so the opponent can't take advantage of \
             the tempo lost."
        }
        TacticPattern::XRay => {
            "A battery: two of your pieces line up on the same file or \
             diagonal so when the front one captures, the back one is \
             ready to recapture — usually winning the exchange."
        }
        TacticPattern::AttackingF2F7 => {
            "The f2 (white) and f7 (black) squares are the weakest \
             points near an uncastled king — only the king itself \
             defends them. A piece landing on f7 with the enemy king \
             on e8 is the classic beginner combination."
        }
        TacticPattern::UnderPromotion => {
            "Promoting to a knight (or rook / bishop) instead of a \
             queen — used when the under-promoted piece does something \
             a queen can't: a knight giving immediate mate, or a rook / \
             bishop avoiding stalemate."
        }
        TacticPattern::Checkmate => {
            "A forced sequence ending the game. From here, every \
             opponent reply is met by a continuation that leads to mate."
        }
    }
}

fn mate_detail(hit: &TacticHit) -> &'static str {
    match hit.mate_pattern {
        Some(MatePattern::BackRank) => {
            " The mate uses the back-rank pattern — the enemy king is \
             trapped on its first rank by its own pieces, with the \
             mating piece sliding along that rank."
        }
        Some(MatePattern::Smothered) => {
            " The mate is smothered — the enemy king has no flight \
             squares because every neighbour is occupied by its own \
             pieces, and a knight delivers the final blow."
        }
        // Other named mates exist in the engine but aren't surfaced by
        // default for a 1200 student — they show up as the heading
        // suffix when present (see `mate_suffix`) but don't add detail
        // text yet. When the named-mates expansion lands, this match
        // becomes exhaustive.
        _ => "",
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
    use chess_tutor_engine::analysis::Confidence;
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
        }
    }

    #[test]
    fn played_card_heading_uses_phrase_form() {
        let card = played_item(&fork_hit());
        assert_eq!(card.heading, "You played a fork");
        assert_eq!(card.sentiment, Sentiment::Positive);
        assert_eq!(card.score_delta_pawns, Some(3.0));
    }

    #[test]
    fn missed_card_inverts_score_and_drops_annotations_when_reveal_off() {
        let card = missed_item(&fork_hit(), /*reveal_best_moves=*/ false);
        assert_eq!(card.heading, "You missed a fork");
        assert_eq!(card.sentiment, Sentiment::Negative);
        assert_eq!(card.score_delta_pawns, Some(-3.0));
        assert!(card.annotations.is_empty(),
            "reveal-off must not paint the engine's preferred line");
    }

    #[test]
    fn missed_card_keeps_annotations_when_reveal_on() {
        let card = missed_item(&fork_hit(), /*reveal_best_moves=*/ true);
        assert!(!card.annotations.is_empty());
    }

    #[test]
    fn mate_suffix_only_for_default_surfaced_patterns() {
        let mut hit = fork_hit();
        hit.mate_pattern = Some(MatePattern::BackRank);
        let card = played_item(&hit);
        assert!(card.heading.ends_with("back-rank mate"));

        hit.mate_pattern = Some(MatePattern::Anastasia);
        let card = played_item(&hit);
        // Anastasia is engine-known but not surfaced_by_default; no
        // heading suffix.
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
        };
        let card = played_item(&hit);
        assert_eq!(card.heading, "You played checkmate — back-rank mate");
        assert_eq!(card.summary, "forced mate");
    }

    #[test]
    fn walked_into_card_is_negative_with_warning_framing() {
        let card = walked_into_item(&fork_hit());
        assert_eq!(card.sentiment, Sentiment::Negative);
        assert!(card.heading.starts_with("If they reply"));
    }

    #[test]
    fn annotations_include_arrow_per_target_plus_threat_highlights() {
        let hit = fork_hit();
        let anns = tactic_annotations(&hit);
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
