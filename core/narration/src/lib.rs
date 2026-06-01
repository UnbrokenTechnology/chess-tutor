//! Per-move retrospective narration: turn the structured outcomes
//! produced by the engine's analysis layer into human-readable text
//! suitable for any UI surface (CLI stdout, egui panel, mobile app).
//!
//! The crate's stable contract is the [`MoveAnalysis`] structured
//! data the engine produces; this layer is only the prose renderer.
//! UIs that want to surface visual annotations (highlights, arrows)
//! consume the underlying `*Outcome` structs from
//! `chess_tutor_engine::analysis` directly and may additionally use
//! [`format_retrospective`] for the prose alongside.
//!
//! ```text
//!   [retrospective] You played Qxf7?? — Blunder (-8.20 vs +0.15 best, Δ -8.35).
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
mod initiative_narration;
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

use std::io;

use chess_tutor_engine::analysis::{
    compute_blocked_center_outcome, compute_castling_outcome, compute_initiative_outcome,
    compute_king_safety_outcome, compute_material_outcome, compute_mobility_outcome,
    compute_passed_pawns_outcome, compute_pawn_structure_outcome,
    compute_pieces_positional_outcome, compute_space_outcome, compute_threats_outcome,
    MoveAnalysis, MoveVerdict, SurpriseKind, TermId,
};
use chess_tutor_engine::position::Position;
use chess_tutor_engine::san;
use chess_tutor_engine::types::{Color, Move};

use blocked_center_narration::render_blocked_center;
use castling_narration::render_castling;
use initiative_narration::render_initiative;
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

/// Knobs that change the shape of the rendered narration without
/// affecting which terms are computed.
#[derive(Clone, Debug)]
pub struct NarrationOptions {
    /// When true, a `Best` verdict still runs the full term-level
    /// narration instead of short-circuiting after the one-line
    /// headline. Defaults on: a student who picks the right move
    /// for the wrong reason — or no reason at all — learns nothing
    /// from a bare congratulatory line.
    pub explain_best: bool,
}

impl Default for NarrationOptions {
    fn default() -> Self {
        Self { explain_best: true }
    }
}

/// Format the retrospective for `user_move` given a slice of
/// [`MoveAnalysis`] returned by the engine's `analyze_position`.
///
/// `analyses[0]` must be the engine's preferred move (the ranking is
/// defined by the search). `user_move` should appear somewhere in
/// the slice — typically by passing `force_include = vec![user_move]`
/// to `SearchParams` so it's guaranteed to be present even when the
/// move is bad enough to fall outside the natural top-k.
///
/// Returns the rendered text. Empty when `analyses` is empty (a
/// terminal-position search). When `user_move` isn't in `analyses`
/// the returned string is the single line `"[retrospective
/// unavailable]"`.
pub fn format_retrospective(
    pre_move_pos: &Position,
    analyses: &[MoveAnalysis],
    user_move: Move,
    opts: &NarrationOptions,
) -> String {
    if analyses.is_empty() {
        return String::new();
    }

    let best = &analyses[0];
    let Some(user) = analyses.iter().find(|a| a.mv == user_move) else {
        return String::from("[retrospective unavailable]\n");
    };

    // Material-aware verdict so the CLI retrospective distinguishes a
    // Miss (declined a forced material win) from a Blunder (hung your
    // own material) — same signal the GUI and the opponent bot use.
    let root_stm = pre_move_pos.side_to_move();
    let user_material = compute_material_outcome(user, pre_move_pos, root_stm);
    let best_material = compute_material_outcome(best, pre_move_pos, root_stm);
    let verdict =
        user.classify_with_material(best.score, user_material.net_mg_cp, best_material.net_mg_cp);
    let mut buf: Vec<u8> = Vec::with_capacity(512);
    // Writing to a Vec<u8> is infallible, so `expect` here would
    // only fire on an out-of-memory panic, which the allocator
    // would have already raised; pass it through.
    render_report(
        &mut buf,
        pre_move_pos,
        pre_move_pos.side_to_move(),
        best,
        user,
        verdict,
        opts.explain_best,
    )
    .expect("writing to Vec<u8> never fails");
    String::from_utf8(buf).expect("narration emits ASCII / UTF-8 only")
}

fn render_report(
    out: &mut dyn io::Write,
    pre_move_pos: &Position,
    root_stm: Color,
    best: &MoveAnalysis,
    user: &MoveAnalysis,
    verdict: MoveVerdict,
    explain_best: bool,
) -> io::Result<()> {
    let user_san = san::format(pre_move_pos, user.mv);

    // Headline shows the **absolute** post-move scores plus the gap
    // between them (root-STM POV). The earlier format showed the two
    // *swings* (user.score - pre_score, best.score - pre_score) which
    // read ambiguously: "Δ +0.00 vs Δ +0.68" looks like absolute
    // scores to a chess player but actually means "your move didn't
    // change the eval; best would have raised it by 0.68." Switching
    // to absolutes matches the mental model the eval bar uses.
    //
    // - `user_score_str` — the post-user-move position eval.
    // - `best_score_str` — the post-best-move position eval (same
    //   pre-move root, alternative continuation).
    // - `gap_str` — `user.score - best.score`, always ≤ 0 for non-
    //   Best verdicts (a negative number reads as "you're behind
    //   where you could be by this much").
    let user_score_str = format_score_pawns(user.score);
    let best_score_str = format_score_pawns(best.score);
    let gap_str = format_delta_pawns(user.score.0 - best.score.0);

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
    //
    // Best: single absolute score — the user *is* the best, no "vs …
    //   best" half to add. *"You played Nf3 — Best (+0.30)."*
    // BestAvailable: keep the "Position was already lost" framing —
    //   the absolute score tells the story.
    // Other verdicts: user score vs best score, plus the gap. The
    //   gap is `user - best` (always ≤ 0 for non-Best) so it reads
    //   as "you're {Δ} behind where you could be."
    match verdict {
        MoveVerdict::Best => {
            writeln!(
                out,
                "[retrospective] You played {user_san}{annotation} — {verdict_label_str} ({user_score_str})."
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
                 Position was already lost ({best_score_str})."
            )?;
            return Ok(());
        }
        MoveVerdict::Good
        | MoveVerdict::Inaccuracy
        | MoveVerdict::Mistake
        | MoveVerdict::Blunder
        | MoveVerdict::Miss => {
            writeln!(
                out,
                "[retrospective] You played {user_san}{annotation} — {verdict_label_str} ({user_score_str} vs {best_score_str} best, Δ {gap_str})."
            )?;
        }
    }

    // Line 2: engine's preferred move (drops the score, since it's
    // now on line 1).
    if best.mv != user.mv {
        let best_san = san::format(pre_move_pos, best.mv);
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
    // captures). When the capture-driven narrator fires, it
    // accounts for both halves of the split material score (pieces
    // disappearing changes both `piece_value` and the captured
    // square's `psq_positional` contribution), so consume both. When
    // it stays silent (no captures), `psq_positional` may still
    // surface in the fallback as "piece placement" — pure positional
    // PSQ shifts from quiet moves, no longer mislabelled as material.
    let material_outcome = compute_material_outcome(user, pre_move_pos, root_stm);
    if !material_outcome.events.is_empty() {
        render_material_sequence(out, pre_move_pos, user, &material_outcome, root_stm)?;
        consumed_terms.push(TermId::MaterialPieceValue);
        consumed_terms.push(TermId::MaterialPsqPositional);
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

    // King-safety narration. The narrator describes attackers and
    // *pawn-shield* shifts; pawn-storm and king-pawn-distance shifts
    // surface separately in the fallback line as their own named
    // terms (Stockfish's storm tables are non-monotonic in rank, so
    // narrating storm under the same "shelter" rubric mislabels
    // common opening pawn pushes — see the 1.e4 g6 case). When the
    // narrator fires we still consume KingPawnShield (covered by
    // the prose) and the per-king attackers terms; KingPawnStorm /
    // KingPawnDistance fall through to the fallback regardless.
    let king_safety_outcome = compute_king_safety_outcome(user, pre_move_pos, root_stm);
    if render_king_safety(out, &king_safety_outcome)? {
        consumed_terms.extend_from_slice(&[
            TermId::KingPawnShield,
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

    // Initiative / forcing-hierarchy narration. Sits between the
    // state-based positional narrators and the cross-term
    // multipliers because it's about the user's move-to-opponent's-
    // reply *relationship* rather than a board-state shift. Doesn't
    // consume any TermId — it's not summarising an eval term.
    let initiative_outcome = compute_initiative_outcome(user, pre_move_pos, root_stm);
    let _ = render_initiative(out, &initiative_outcome)?;

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
