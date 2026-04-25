//! Retrospective teaching output: after a human plays a move,
//! analyze the *pre-move* position and report a per-move verdict.
//!
//! The caller supplies a pre-move `Position` and the move just
//! played; we call `analyze_position` with `force_include=[user_mv]`
//! so the user's move is guaranteed to appear in the output (even
//! if it was bad enough to fall outside the natural MultiPV top-k).
//! Then we classify and render a short teaching paragraph:
//!
//! ```text
//!   [retrospective] You played Qxf7?? — Blunder (Δ -8.60).
//!                   Engine preferred Nf3 (+0.15).
//!                   Best line: Qxf7+ Kxf7 — you lose 8 points (queen for pawn).
//!                   Also: king -1.20, mobility -0.40.
//! ```
//!
//! For best / best-available moves we print a single terse line.
//! The output deliberately stays under ~4 lines per move to avoid
//! burying the board.
//!
//! ## Module layout
//!
//! - [`util`] — low-level helpers (piece names, attackers list,
//!   SAN PV walking, score formatters, verdict labels + SAN
//!   annotations).
//! - [`surprise_tag`] — the shallow-vs-deep surprise-phrase
//!   selector (what to say, when to stay silent).
//! - `*_narration` modules — one per outcome category: material,
//!   threats, king safety, pawn structure, mobility, passed pawns,
//!   piece placement. Each owns its threshold constants, per-side
//!   line generators, and render function.
//! - [`secondary_terms`] — the fallback "Shifts" / "Also" line.

mod blocked_center_narration;
mod castling_narration;
mod king_safety_narration;
mod material_narration;
mod mobility_narration;
mod passed_pawns_narration;
mod pawn_structure_narration;
mod pieces_positional_narration;
mod secondary_terms;
mod space_narration;
mod surprise_tag;
mod threats_narration;
mod util;

use std::io::{self, Write};
use std::time::Duration;

use chess_tutor_engine::analysis::{
    analyze_position, compute_blocked_center_outcome, compute_castling_outcome,
    compute_king_safety_outcome, compute_material_outcome, compute_mobility_outcome,
    compute_passed_pawns_outcome, compute_pawn_structure_outcome,
    compute_pieces_positional_outcome, compute_space_outcome, compute_threats_outcome,
    MoveAnalysis, MoveVerdict, SurpriseKind, TermId,
};
use chess_tutor_engine::engine::{Engine, SearchParams};
use chess_tutor_engine::position::Position;
use chess_tutor_engine::san;
use chess_tutor_engine::types::{Color, Move};

use blocked_center_narration::render_blocked_center;
use castling_narration::render_castling;
use king_safety_narration::render_king_safety;
use material_narration::render_material_sequence;
use mobility_narration::render_mobility;
use passed_pawns_narration::render_passed_pawns;
use pawn_structure_narration::render_pawn_structure;
use pieces_positional_narration::render_pieces_positional;
use secondary_terms::render_secondary_terms;
use space_narration::render_space;
use surprise_tag::select_surprise_phrase;
use threats_narration::render_threats;
use util::{
    format_delta_pawns, format_engine_preferred_line, format_score_pawns,
    sharp_or_verdict_annotation, verdict_label,
};

/// How many alternatives to pull from the search when running
/// retrospective. Kept small (top 2 alternatives + the forced user
/// move) so the pause is tolerable on every human move.
const RETROSPECTIVE_MULTI_PV: usize = 3;

/// Configuration for a single retrospective pass.
pub struct RetrospectiveConfig {
    pub max_depth: u32,
    pub max_time_ms: Option<u64>,
    /// When true, a `Best` verdict still runs the full term-level
    /// narration instead of short-circuiting after the one-line
    /// headline. Useful when the student wants to understand *why*
    /// their move was the best, not just *that* it was.
    pub explain_best: bool,
}

/// Analyze `pre_move_pos` with the user's move forced into the
/// output, classify the move, and write a short teaching paragraph
/// to `out`.
///
/// `game_history` is the zobrist-key history up to and including
/// the pre-move position (the search treats the last element as
/// the root and the preceding ones as prior positions for
/// repetition detection).
pub fn run_and_render(
    out: &mut io::StdoutLock<'_>,
    pre_move_pos: &mut Position,
    engine: &mut Engine,
    cfg: &RetrospectiveConfig,
    game_history: Vec<u64>,
    user_mv: Move,
) -> io::Result<()> {
    let root_stm = pre_move_pos.side_to_move();
    let params = SearchParams {
        max_depth: cfg.max_depth,
        max_nodes: None,
        max_time: cfg.max_time_ms.map(Duration::from_millis),
        multi_pv: RETROSPECTIVE_MULTI_PV,
        game_history,
        force_include: vec![user_mv],
        verbose_progress: false,
    };
    let analyses = analyze_position(engine, pre_move_pos, params);
    if analyses.is_empty() {
        return Ok(());
    }

    let best = &analyses[0];
    let user = analyses.iter().find(|a| a.mv == user_mv);
    let Some(user) = user else {
        writeln!(out, "[retrospective unavailable]")?;
        return Ok(());
    };

    let verdict = user.classify(best.score);
    render_report(
        out,
        pre_move_pos,
        root_stm,
        best,
        user,
        verdict,
        cfg.explain_best,
    )
}

fn render_report(
    out: &mut io::StdoutLock<'_>,
    pre_move_pos: &Position,
    root_stm: Color,
    best: &MoveAnalysis,
    user: &MoveAnalysis,
    verdict: MoveVerdict,
    explain_best: bool,
) -> io::Result<()> {
    let user_san = san::format(pre_move_pos, user.mv);
    let delta = user.score.0 - best.score.0;
    let delta_str = format_delta_pawns(delta);

    let verdict_label_str = verdict_label(verdict);

    // "Sharp move" detection — a move the shallow static-eval
    // underrated, but the deep search sees through. Treat it as a
    // brilliancy when it's also the user's actual best choice.
    let user_is_sharp = matches!(
        (verdict, user.surprise(root_stm)),
        (
            MoveVerdict::Best | MoveVerdict::Good,
            Some(SurpriseKind::LooksBadButGood)
        )
    );
    let annotation = sharp_or_verdict_annotation(verdict, user_is_sharp);

    // Line 1: headline.
    match verdict {
        MoveVerdict::Best => {
            writeln!(
                out,
                "[retrospective] You played {user_san}{annotation} — {verdict_label_str}."
            )?;
            if user_is_sharp {
                writeln!(
                    out,
                    "                Well spotted — this looks risky at first glance, \
                     but the longer line pays off."
                )?;
            }
            // Without `explain_best`, stop here — a single
            // congratulatory line. With the flag, fall through to the
            // same per-term narration non-Best verdicts get, so the
            // student sees *why* the move was best.
            if !explain_best {
                return Ok(());
            }
        }
        MoveVerdict::BestAvailable => {
            writeln!(
                out,
                "[retrospective] You played {user_san} — {verdict_label_str}. \
                 Position was already lost ({}).",
                format_score_pawns(best.score),
            )?;
            return Ok(());
        }
        MoveVerdict::Good
        | MoveVerdict::Inaccuracy
        | MoveVerdict::Mistake
        | MoveVerdict::Blunder => {
            writeln!(
                out,
                "[retrospective] You played {user_san}{annotation} — {verdict_label_str} (Δ {delta_str}).",
            )?;
        }
    }

    // Line 2: engine's preferred move.
    if best.mv != user.mv {
        let best_san = san::format(pre_move_pos, best.mv);
        let best_score_str = format_score_pawns(best.score);
        let best_is_sharp = matches!(best.surprise(root_stm), Some(SurpriseKind::LooksBadButGood));
        writeln!(
            out,
            "                {}",
            format_engine_preferred_line(&best_san, &best_score_str, best_is_sharp),
        )?;
    }

    // Line(s) 3+: structured narration per term, followed by a
    // compact list of the remaining terms that made the
    // cumulative-75% prefix. Each term with specialized narration
    // is rendered via its own function and then *excluded* from
    // the generic shifts list.
    let mut consumed_terms: Vec<TermId> = Vec::new();

    // Material narration (capture sequence or silent if no
    // captures).
    let material_outcome = compute_material_outcome(user, pre_move_pos, root_stm);
    if !material_outcome.events.is_empty() {
        render_material_sequence(out, pre_move_pos, user, &material_outcome, root_stm)?;
        consumed_terms.push(TermId::Material);
    }

    // Threats narration (hanging / SEE-losing / pressured).
    let threats_outcome = compute_threats_outcome(user, pre_move_pos, root_stm);
    if render_threats(out, &threats_outcome)? {
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

    // King-safety narration.
    let king_safety_outcome = compute_king_safety_outcome(user, pre_move_pos, root_stm);
    if render_king_safety(out, &king_safety_outcome)? {
        consumed_terms.extend_from_slice(&[
            TermId::KingShelter,
            TermId::KingDanger,
            TermId::KingPawnlessFlank,
            TermId::KingFlankAttacks,
        ]);
    }

    // Pawn-structure narration.
    let pawn_structure_outcome = compute_pawn_structure_outcome(user, pre_move_pos, root_stm);
    if render_pawn_structure(out, &pawn_structure_outcome)? {
        consumed_terms.extend_from_slice(&[
            TermId::PawnsConnected,
            TermId::PawnsIsolated,
            TermId::PawnsBackward,
            TermId::PawnsDoubled,
            TermId::PawnsWeakUnopposed,
            TermId::PawnsWeakLever,
        ]);
    }

    // Mobility narration.
    let mobility_outcome = compute_mobility_outcome(user, pre_move_pos, root_stm);
    if render_mobility(out, &mobility_outcome)? {
        consumed_terms.extend_from_slice(&[
            TermId::MobilityKnight,
            TermId::MobilityBishop,
            TermId::MobilityRook,
            TermId::MobilityQueen,
        ]);
    }

    // Passed-pawns narration.
    let passed_outcome = compute_passed_pawns_outcome(user, pre_move_pos, root_stm);
    if render_passed_pawns(out, &passed_outcome)? {
        consumed_terms.extend_from_slice(&[
            TermId::PassedRankBonus,
            TermId::PassedKingProximity,
            TermId::PassedFreeAdvance,
            TermId::PassedStopperPenalty,
        ]);
    }

    // Piece-placement narration.
    let pieces_outcome = compute_pieces_positional_outcome(user, pre_move_pos, root_stm);
    if render_pieces_positional(out, &pieces_outcome)? {
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
    }

    // Cross-term multiplier narrators. These describe positional
    // signals that *scale* another eval term rather than contributing
    // additively, so they don't appear as their own TermId in the
    // term-deltas list — but the underlying chess concept (closed
    // centre, lost castling rights amplifying a trapped rook,
    // exchanges diluting a space advantage) is exactly the kind of
    // strategic teaching this product exists to surface.

    // Blocked-centre narration. Consumes `PiecesBishopPawns` from the
    // fallback line because that's the term the multiplier amplifies.
    let blocked_center_outcome =
        compute_blocked_center_outcome(user, pre_move_pos, root_stm);
    if render_blocked_center(out, &blocked_center_outcome)? {
        consumed_terms.push(TermId::PiecesBishopPawns);
    }

    // Castling-loss × trapped-rook narration. Consumes
    // `PiecesTrappedRook` because that's the penalty the lost
    // castling rights doubled.
    let castling_outcome = compute_castling_outcome(user, pre_move_pos, root_stm);
    if render_castling(out, &castling_outcome)? {
        consumed_terms.push(TermId::PiecesTrappedRook);
    }

    // Space dilution narration. Consumes `Space` because that's the
    // term whose quadratic piece-count weight just shrank.
    let space_outcome = compute_space_outcome(user, pre_move_pos, root_stm);
    if render_space(out, &space_outcome)? {
        consumed_terms.push(TermId::Space);
    }

    // Whatever's left from the cumulative-prefix of term deltas,
    // grouped by sign (helped / hurt) from root-STM's POV.
    render_secondary_terms(out, user, root_stm, &consumed_terms)?;

    // Optional final line: shallow-vs-deep surprise tag.
    if let Some(phrase) = select_surprise_phrase(verdict, user.surprise(root_stm)) {
        writeln!(out, "                ({phrase}.)")?;
    }

    Ok(())
}
