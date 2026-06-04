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

mod depth_honesty;
mod desperado;
mod forced_consequences;
mod headline;
mod helpers;
mod initiative;
mod king_safety;
mod material;
mod missed_prophylaxis;
mod mobility;
mod override_note;
mod passed_pawns;
mod pawn_structure;
mod pieces;
mod positional_win;
mod secondary;
mod space;
mod tactic;
mod threats;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use chess_tutor_engine::analysis::{
    compute_king_safety_outcome, compute_material_outcome, compute_mobility_outcome,
    compute_passed_pawns_outcome, compute_pawn_structure_outcome,
    compute_pieces_positional_outcome, compute_space_outcome, compute_threats_outcome,
    MoveAnalysis, PriorMove, TermId,
};
use chess_tutor_engine::movegen::legal_moves_vec;
use chess_tutor_engine::position::Position;
use chess_tutor_engine::types::{Move, Square};

use chess_tutor_teaching::phrasing::Perspective;

use crate::view::{
    RetrospectiveItem, RetrospectiveViewModel,
};

use depth_honesty::*;
use desperado::*;
use forced_consequences::*;
use headline::*;
use initiative::*;
use override_note::*;
use king_safety::*;
use material::*;
use missed_prophylaxis::*;
use mobility::*;
use passed_pawns::*;
use pawn_structure::*;
use pieces::*;
use positional_win::*;
use secondary::*;
use space::*;
use tactic::*;
use threats::*;
use helpers::*;

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
///
/// `prior_move` is the opponent's move that produced `pre_move_pos`,
/// used by the tactic detector's hanging-capture recapture guard so a
/// trade isn't mis-labelled "free piece." Pass `None` at game start /
/// for ad-hoc analyses without history; the guard simply isn't applied.
///
/// `perspective` selects "you" vs "they": `Player` when the user made
/// the move (`moved_by == user_color`), `Opponent` when the engine /
/// other side did. It threads into every per-card builder's
/// [`PhrasingContext`] so opponent-move retrospectives reframe to the
/// student's benefit (an opponent's blunder is *your* chance). The
/// builders' sentiment colouring derives from the same perspective, so
/// a card stays green/red from the *student's* point of view regardless
/// of who moved. The translator (`core/teaching`) is the single home for
/// the reframe; this layer only carries the perspective through.
pub fn build_retrospective_view(
    pre_move_pos: &Position,
    analyses: &[MoveAnalysis],
    user_move: Move,
    show_all: bool,
    reveal_best_moves: bool,
    prior_move: Option<PriorMove>,
    perspective: Perspective,
) -> RetrospectiveViewModel {
    if analyses.is_empty() {
        return RetrospectiveViewModel::default();
    }
    let best = &analyses[0];
    let Some(user) = analyses.iter().find(|a| a.mv == user_move) else {
        return RetrospectiveViewModel::default();
    };
    let root_stm = pre_move_pos.side_to_move();
    // Material outcomes of both lines, mover-POV engine-mg-cp. The
    // user's is reused below for the material / threats cards; the
    // best line's is needed only for the material-aware verdict
    // (Miss = best wins material, user declined without hanging).
    let material_outcome = compute_material_outcome(user, pre_move_pos, root_stm);
    let best_material = compute_material_outcome(best, pre_move_pos, root_stm);
    let verdict =
        user.classify_with_material(best.score, material_outcome.net_mg_cp, best_material.net_mg_cp);

    // `perspective` is threaded from the caller: `Player` for the user's
    // own moves, `Opponent` for engine moves. The translator owns the
    // "you" / "they" reframe; here we just carry the perspective through.
    let headline = build_headline(
        pre_move_pos,
        analyses,
        best,
        user,
        verdict,
        root_stm,
        perspective,
        reveal_best_moves,
    );

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

    if let Some(it) = build_material_item(
        pre_move_pos,
        &material_outcome,
        root_stm,
        perspective,
    ) {
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
    for it in build_threat_items(&threats_outcome, &user_captures_by_square, perspective) {
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
    for it in build_king_safety_items(&king_safety_outcome, perspective) {
        items.push(it);
        consumed_terms.extend_from_slice(&[
            TermId::KingPawnShield,
            TermId::KingDanger,
            TermId::KingPawnlessFlank,
            TermId::KingFlankAttacks,
        ]);
    }

    let pawn_structure_outcome = compute_pawn_structure_outcome(user, pre_move_pos, root_stm);
    if let Some(it) = build_pawn_structure_item(&pawn_structure_outcome, perspective) {
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
    for it in build_forced_consequences_items(user, pre_move_pos, root_stm, perspective) {
        items.push(it);
    }

    // Missed-prophylaxis card (Feature 2): when the user's move allowed a
    // deep punishing line the engine's best move would have *prevented*,
    // name what they needed to stop and why — "you needed Ra8 to stop
    // Rxe7+; otherwise king safety collapses." Built before the tactic
    // cards so it can *supersede* a bare ALLOWED reframe (the two would
    // otherwise lead with the identical swing). A tactic-grade card; it
    // sits in the top group right before the tactic cards. Consume the
    // exploded term so the Secondary builder doesn't double-render it.
    let prophylaxis = build_missed_prophylaxis_item(
        pre_move_pos,
        best,
        user,
        root_stm,
        reveal_best_moves,
        perspective,
    );
    let suppress_allowed_reframe = prophylaxis.is_some();
    if let Some(it) = prophylaxis {
        items.push(it);
        // The exploded term is mover-signed king danger / flank attacks in
        // the case study; consume the king terms so the "Other shifts" row
        // doesn't repeat the collapse the card already explained.
        consumed_terms.extend_from_slice(&[TermId::KingDanger, TermId::KingFlankAttacks]);
    }

    // Tactic cards: played / missed / walked-into named patterns
    // (fork, pin, free piece, …). Compute-tactic-outcome handles
    // the three-slot dispatch internally; we forward `prior_move`
    // so the hanging-capture recapture guard fires when history
    // exists. `suppress_allowed_reframe` drops the duplicate ALLOWED
    // lead when the missed-prophylaxis card already owns the swing.
    for it in build_tactic_items(
        pre_move_pos,
        best,
        user,
        root_stm,
        prior_move,
        reveal_best_moves,
        perspective,
        suppress_allowed_reframe,
    ) {
        items.push(it);
    }

    // Positional-win card (Feature 1): when the engine's best move is a
    // *sound sacrifice* (down material, search-positive), explain the
    // positional compensation off the STATIC term diff — "down a point,
    // but king safety swings hard in your favour." A tactic-grade card,
    // so it sits in the top group with the tactic cards. Consume its
    // dominant term so the Secondary "Other shifts" builder doesn't
    // double-render the same swing.
    if let Some((it, dominant_term)) =
        build_positional_win_item(best, pre_move_pos, root_stm, perspective)
    {
        items.push(it);
        consumed_terms.push(dominant_term);
    }

    // Desperado-aware material narration (PLAN §4): when a doomed piece
    // cashes a pawn with check before it falls, narrate "−X becomes
    // −X+pawn", not "you're fine".
    if let Some(it) = build_desperado_item(pre_move_pos, user, root_stm, perspective) {
        items.push(it);
    }

    // Static-vs-search override note (PLAN §4.2): when the term ledger and
    // the search rank the user's move and the engine's pick in opposite
    // directions, say so — never invent a positional justification.
    if let Some(it) = build_override_note_item(best, user, root_stm, perspective) {
        items.push(it);
    }

    // Loss-of-initiative note: a static-vs-search surprise-mistake whose
    // mechanism is a forcing run (no named tactic). This is the *why* the
    // static eval and the search disagree — when it fires we have the
    // human-findable lesson, so the depth-honesty fallback below stays
    // quiet (the two are mutually exclusive: one says "here's the
    // mechanism", the other says "there's no shorter lesson").
    let had_initiative_note = if let Some(it) =
        build_initiative_item(pre_move_pos, best, user, root_stm, prior_move, perspective)
    {
        items.push(it);
        true
    } else {
        false
    };

    // Silent-sequencing depth-honesty note (PLAN §4.3): when the move is
    // worse only beyond practical calculation depth and no detector fires,
    // be honest that there's no shorter lesson. Bounded two-depth search
    // inside the detector; only runs on a non-best move with a real gap.
    // Suppressed when the initiative note already supplied the mechanism.
    if !had_initiative_note {
        if let Some(it) =
            build_depth_honesty_item(pre_move_pos, best, user, root_stm, prior_move, perspective)
        {
            items.push(it);
        }
    }

    let mobility_outcome = compute_mobility_outcome(user, pre_move_pos, root_stm);
    for it in build_mobility_items(&mobility_outcome, &post_pos, root_stm, show_all, perspective) {
        items.push(it);
        consumed_terms.extend_from_slice(&[
            TermId::MobilityKnight,
            TermId::MobilityBishop,
            TermId::MobilityRook,
            TermId::MobilityQueen,
        ]);
    }

    let passed_outcome = compute_passed_pawns_outcome(user, pre_move_pos, root_stm);
    if let Some(it) = build_passed_pawns_item(&passed_outcome, perspective) {
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
    let kp_supp = capture_suppression(&material_outcome, root_stm);
    for it in build_pieces_positional_items(&pieces_outcome, root_stm, kp_supp, perspective) {
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
    let space_items = build_space_items(&space_outcome, show_all, perspective);
    if !space_items.is_empty() {
        consumed_terms.push(TermId::Space);
    }
    for it in space_items {
        items.push(it);
    }

    if let Some(it) = build_secondary_item(user, root_stm, &consumed_terms, show_all, perspective) {
        items.push(it);
    }

    RetrospectiveViewModel { headline, items }
}

