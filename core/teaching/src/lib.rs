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
//!                   Best line: Qxf7+ Kxf7 — You lost 8 points (queen for pawn).
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
//! - [`claim`] — the language-free Claim IR + the per-category salience
//!   builders. **Every** outcome category — verdict, material, threats,
//!   king safety, mobility, pawn structure, passed pawns, piece
//!   placement, space, initiative, the cross-term multipliers (centre
//!   structure, castling × trapped rook), the special UI narratives, and
//!   the secondary "other shifts" list — is produced as a [`claim::Claim`]
//!   here and rendered to prose by [`phrasing::phrase`]. There is no
//!   separate hardcoded-prose path left: `format_retrospective` is a pure
//!   `claims + phrase` join.

pub mod claim;
pub mod phrasing;

mod util;

use std::io;

use chess_tutor_engine::analysis::{
    compute_blocked_center_outcome, compute_castling_outcome, compute_initiative_outcome,
    compute_king_safety_outcome, compute_material_outcome, compute_mobility_outcome,
    compute_passed_pawns_outcome, compute_pawn_structure_outcome,
    compute_pieces_positional_outcome, compute_space_outcome, compute_threats_outcome,
    MaterialOutcome, MoveAnalysis, MoveVerdict, SurpriseKind, TermId,
};
use chess_tutor_engine::position::Position;
use chess_tutor_engine::san;
use chess_tutor_engine::types::{Color, Move};

use claim::{material_claim, verdict_claim};
use phrasing::{phrase, Locale, Perspective, PhrasingContext, Verbosity};

use util::{format_engine_preferred_line, format_score_pawns, pv_to_san_through};

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

/// Engine-cp reporting floor for the CLI's mobility lines. ~0.50 of a
/// pawn — a lower floor fired on almost every opening move (any nudge to
/// an enemy pawn shifts the mobility-area bitmap, which the term weights
/// even when our pieces haven't actually moved). 50 cp cuts the noise
/// without hiding shifts that reflect a real change in a piece's reach.
/// The GUI uses its own (lower) floor for a richer card list.
const CLI_MOBILITY_DELTA_THRESHOLD_CP: i32 = 50;

/// Format the retrospective for `user_move` given a slice of
/// [`MoveAnalysis`] returned by the engine's `analyze_position`.
///
/// `analyses[0]` must be the engine's preferred move (the ranking is
/// defined by the search). `user_move` should appear somewhere in
/// the slice — typically by passing `force_include = vec![user_move]`
/// to `SearchParams` so it's guaranteed to be present even when the
/// move is bad enough to fall outside the natural top-k.
///
/// `perspective` selects the "you" vs "they" framing: pass
/// [`Perspective::Player`] for the user's own moves, [`Perspective::Opponent`]
/// for the engine's (the directional reframe — an opponent's blunder is
/// *your* chance — lives entirely in [`phrasing::phrase`]).
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
    perspective: Perspective,
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
        analyses,
        best,
        user,
        verdict,
        opts.explain_best,
        perspective,
    )
    .expect("writing to Vec<u8> never fails");
    String::from_utf8(buf).expect("narration emits ASCII / UTF-8 only")
}

#[allow(clippy::too_many_arguments)]
fn render_report(
    out: &mut dyn io::Write,
    pre_move_pos: &Position,
    root_stm: Color,
    analyses: &[MoveAnalysis],
    best: &MoveAnalysis,
    user: &MoveAnalysis,
    verdict: MoveVerdict,
    explain_best: bool,
    perspective: Perspective,
) -> io::Result<()> {
    let best_score_str = format_score_pawns(best.score);

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

    // Line 1: headline, via the shared Claim IR + translator. The
    // translator owns the verdict word (incl. the chess.com "Great" /
    // "Brilliant" tier remap), the perspective ("You played …" /
    // "They played …"), the score formatting, and the verdict-specific
    // note (lost position / missed material). The caller-supplied
    // `perspective` flows into every `PhrasingContext` below; the
    // engine-preferred line (line 2) and the per-term body stay
    // CLI-owned for now.
    let claim = verdict_claim(pre_move_pos, analyses, best, user, verdict, false);
    let ctx = PhrasingContext {
        perspective,
        locale: Locale::En,
        verbosity: Verbosity::Normal,
        reveal_moves: false,
    };
    let phrasing = phrase(&claim, &ctx);
    writeln!(out, "[retrospective] {}", phrasing.summary)?;
    if let Some(detail) = &phrasing.detail {
        writeln!(out, "                {detail}")?;
    }

    match verdict {
        MoveVerdict::Best => {
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
        // The position was already lost — the verdict-specific note
        // (phrased above) tells the story; no per-term body.
        MoveVerdict::BestAvailable => return Ok(()),
        MoveVerdict::Good
        | MoveVerdict::Inaccuracy
        | MoveVerdict::Mistake
        | MoveVerdict::Blunder
        | MoveVerdict::Miss => {}
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
        render_material_sequence(out, pre_move_pos, user, &material_outcome, root_stm, perspective)?;
        consumed_terms.push(TermId::MaterialPieceValue);
        consumed_terms.push(TermId::MaterialPsqPositional);
    }

    // Threats narration (hanging / SEE-losing / pressured), via the
    // shared Claim IR + translator. `threats_claims` owns the salience
    // (delta-gating, guaranteed-list selection for the opponent side,
    // pressure de-dup); `phrase` owns the perspective ("Your piece is
    // hanging" / "You can win material"). The CLI today always narrates
    // the user's own move, so the perspective is `Player`.
    let threats_outcome = compute_threats_outcome(user, pre_move_pos, root_stm);
    let threat_claims = claim::threats_claims(&threats_outcome);
    if !threat_claims.is_empty() {
        let ctx = PhrasingContext {
            perspective,
            locale: Locale::En,
            verbosity: Verbosity::Normal,
            reveal_moves: false,
        };
        for tc in &threat_claims {
            let phrasing = phrase(tc, &ctx);
            writeln!(out, "                {}", phrasing.summary)?;
        }
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
    let king_safety_claims = claim::king_safety_claims(&king_safety_outcome);
    if !king_safety_claims.is_empty() {
        let ctx = PhrasingContext {
            perspective,
            locale: Locale::En,
            verbosity: Verbosity::Normal,
            reveal_moves: false,
        };
        for ksc in &king_safety_claims {
            let phrasing = phrase(ksc, &ctx);
            writeln!(out, "                {}", phrasing.summary)?;
        }
        consumed_terms.extend_from_slice(&[
            TermId::KingPawnShield,
            TermId::KingDanger,
            TermId::KingPawnlessFlank,
            TermId::KingFlankAttacks,
        ]);
    }

    // Pawn-structure narration, via the shared Claim IR + translator.
    // `pawn_structure_claims` owns the salience (per-sub-term threshold
    // gating, worsened-over-improved precedence per side); `phrase` owns
    // the perspective ("Your pawn structure weakened" / "You weakened the
    // opponent's"). The CLI today always narrates the user's own move, so
    // the perspective is `Player`.
    let pawn_structure_outcome = compute_pawn_structure_outcome(user, pre_move_pos, root_stm);
    let pawn_structure_claims = claim::pawn_structure_claims(&pawn_structure_outcome);
    if !pawn_structure_claims.is_empty() {
        let ctx = PhrasingContext {
            perspective,
            locale: Locale::En,
            verbosity: Verbosity::Normal,
            reveal_moves: false,
        };
        for psc in &pawn_structure_claims {
            let phrasing = phrase(psc, &ctx);
            writeln!(out, "                {}", phrasing.summary)?;
        }
        consumed_terms.extend_from_slice(&[
            TermId::PawnsConnected,
            TermId::PawnsIsolated,
            TermId::PawnsBackward,
            TermId::PawnsDoubled,
            TermId::PawnsWeakUnopposed,
            TermId::PawnsWeakLever,
        ]);
    }

    // Mobility narration. Prose comes from the shared teaching
    // translator; the shared salience (per-piece-type threshold gating,
    // biggest-first ordering, mover-side-first) lives in
    // [`claim::mobility_claims`]. The CLI uses a higher reporting floor
    // than the GUI — roughly one line per side — so an opening pawn push
    // nudging an enemy pawn (which shifts the mobility-area bitmap even
    // when our pieces haven't moved) doesn't fire a line every move.
    let mobility_outcome = compute_mobility_outcome(user, pre_move_pos, root_stm);
    let mobility_claims = claim::mobility_claims(&mobility_outcome, CLI_MOBILITY_DELTA_THRESHOLD_CP);
    if !mobility_claims.is_empty() {
        let ctx = PhrasingContext {
            perspective,
            locale: Locale::En,
            verbosity: Verbosity::Normal,
            reveal_moves: false,
        };
        for mc in &mobility_claims {
            let phrasing = phrase(mc, &ctx);
            match phrasing.detail {
                Some(d) => writeln!(out, "                {} — {d}", phrasing.summary)?,
                None => writeln!(out, "                {}.", phrasing.summary)?,
            }
        }
        consumed_terms.extend_from_slice(&[
            TermId::MobilityKnight,
            TermId::MobilityBishop,
            TermId::MobilityRook,
            TermId::MobilityQueen,
        ]);
    }

    // Passed-pawns narration, via the shared Claim IR + translator.
    // `passed_pawns_claims` owns the salience (aggregate threshold gating
    // per side); `phrase` owns the perspective ("Your passed pawns
    // advanced" / "You blunted the opponent's passed pawns"). The CLI
    // today always narrates the user's own move, so the perspective is
    // `Player`.
    let passed_outcome = compute_passed_pawns_outcome(user, pre_move_pos, root_stm);
    let passed_claims = claim::passed_pawns_claims(&passed_outcome);
    if !passed_claims.is_empty() {
        let ctx = PhrasingContext {
            perspective,
            locale: Locale::En,
            verbosity: Verbosity::Normal,
            reveal_moves: false,
        };
        for ppc in &passed_claims {
            let phrasing = phrase(ppc, &ctx);
            writeln!(out, "                {}", phrasing.summary)?;
        }
        consumed_terms.extend_from_slice(&[
            TermId::PassedRankBonus,
            TermId::PassedKingProximity,
            TermId::PassedFreeAdvance,
            TermId::PassedStopperPenalty,
        ]);
    }

    // Piece-placement narration, via the shared Claim IR + translator.
    // `pieces_positional_claims` owns the salience (per-sub-term threshold
    // gating, BishopPawns geometry suppression); `phrase` owns the
    // perspective ("Your knight reached an outpost" / "You denied the
    // opponent's knight an outpost"). The CLI today always narrates the
    // user's own move, so the perspective is `Player`.
    let pieces_outcome = compute_pieces_positional_outcome(user, pre_move_pos, root_stm);
    let pieces_claims = claim::pieces_positional_claims(&pieces_outcome);
    if !pieces_claims.is_empty() {
        let ctx = PhrasingContext {
            perspective,
            locale: Locale::En,
            verbosity: Verbosity::Normal,
            reveal_moves: false,
        };
        for pc in &pieces_claims {
            let phrasing = phrase(pc, &ctx);
            writeln!(out, "                {}.", phrasing.summary)?;
        }
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

    // Initiative / forcing-hierarchy narration, via the shared Claim IR +
    // translator. Sits between the state-based positional narrators and
    // the cross-term multipliers because it's about the user's
    // move-to-opponent's-reply *relationship* rather than a board-state
    // shift. `initiative_claim` owns the template selection (reinforcement
    // / refutation / held-despite, the swing gating); `phrase` owns the
    // perspective. Doesn't consume any TermId — it's not summarising an
    // eval term.
    let initiative_outcome = compute_initiative_outcome(user, pre_move_pos, root_stm);
    if let Some(ic) = claim::initiative_claim(&initiative_outcome, root_stm) {
        let ctx = PhrasingContext {
            perspective,
            locale: Locale::En,
            verbosity: Verbosity::Normal,
            reveal_moves: false,
        };
        writeln!(out, "                {}", phrase(&ic, &ctx).summary)?;
    }

    // Cross-term multiplier narrators. These describe positional
    // signals that *scale* another eval term rather than contributing
    // additively, so they don't appear as their own TermId in the
    // term-deltas list — but the underlying chess concept (closed
    // centre, lost castling rights amplifying a trapped rook,
    // exchanges diluting a space advantage) is exactly the kind of
    // strategic teaching this product exists to surface. All via the
    // shared Claim IR + translator now; the salience lives in the
    // builders, the perspective in `phrase`.

    // Blocked-centre narration. Consumes `PiecesBishopPawns` from the
    // fallback line because that's the term the multiplier amplifies.
    let blocked_center_outcome = compute_blocked_center_outcome(user, pre_move_pos, root_stm);
    let center_claims = claim::center_structure_claims(&blocked_center_outcome, root_stm);
    if !center_claims.is_empty() {
        let ctx = PhrasingContext {
            perspective,
            locale: Locale::En,
            verbosity: Verbosity::Normal,
            reveal_moves: false,
        };
        for cc in &center_claims {
            writeln!(out, "                {}", phrase(cc, &ctx).summary)?;
        }
        consumed_terms.push(TermId::PiecesBishopPawns);
    }

    // Castling-loss × trapped-rook narration. Consumes
    // `PiecesTrappedRook` because that's the penalty the lost
    // castling rights doubled.
    let castling_outcome = compute_castling_outcome(user, pre_move_pos, root_stm);
    let castling_claims = claim::castling_claims(&castling_outcome);
    if !castling_claims.is_empty() {
        let ctx = PhrasingContext {
            perspective,
            locale: Locale::En,
            verbosity: Verbosity::Normal,
            reveal_moves: false,
        };
        for cc in &castling_claims {
            writeln!(out, "                {}", phrase(cc, &ctx).summary)?;
        }
        consumed_terms.push(TermId::PiecesTrappedRook);
    }

    // Space narration, via the shared Claim IR + translator.
    // `space_claims` owns the salience (per-side threshold gating);
    // `phrase` owns the perspective ("You gained space" / "You squeezed
    // the opponent's space"). Consumes `Space` because that's the term
    // whose quadratic piece-count weight just shifted.
    let space_outcome = compute_space_outcome(user, pre_move_pos, root_stm);
    let space_claims = claim::space_claims(&space_outcome, claim::SPACE_DEFAULT_THRESHOLD_CP);
    if !space_claims.is_empty() {
        let ctx = PhrasingContext {
            perspective,
            locale: Locale::En,
            verbosity: Verbosity::Normal,
            reveal_moves: false,
        };
        for sc in &space_claims {
            writeln!(out, "                {}.", phrase(sc, &ctx).summary)?;
        }
        consumed_terms.push(TermId::Space);
    }

    // Whatever's left from the cumulative-prefix of term deltas,
    // grouped by sign (helped / hurt) from the mover's POV, via the
    // shared Claim IR + translator.
    if let Some(secondary) =
        claim::secondary_claim(user, root_stm, &consumed_terms, claim::SECONDARY_DEFAULT_TOP_PERCENT)
    {
        let ctx = PhrasingContext {
            perspective,
            locale: Locale::En,
            verbosity: Verbosity::Normal,
            reveal_moves: false,
        };
        let phrasing = phrase(&secondary, &ctx);
        if let Some(detail) = phrasing.detail {
            for line in detail.lines().filter(|l| !l.is_empty()) {
                writeln!(out, "                {line}.")?;
            }
        }
    }

    // Optional final line: shallow-vs-deep surprise tag, via the shared
    // Claim IR + translator. `surprise_claim` owns the salience (which
    // verdict/kind combinations surface); `phrase` owns the perspective.
    if let Some(claim) = claim::surprise_claim(verdict, user.surprise(root_stm), root_stm) {
        let ctx = PhrasingContext {
            perspective,
            locale: Locale::En,
            verbosity: Verbosity::Normal,
            reveal_moves: false,
        };
        writeln!(out, "                ({})", phrase(&claim, &ctx).summary)?;
    }

    Ok(())
}

/// Render the CLI material line — *"Best line: Nxd5 exd5 — You lose a
/// pawn (knight for bishop + pawn)."*
///
/// Labeled "Best line" — **not** "Forced sequence" — because the PV is
/// the engine's principal variation under optimal play from both sides,
/// not a line where every move is compelled.
///
/// The capture-ledger phrasing ("You won a pawn …") comes from the
/// shared translator ([`phrasing::phrase`]) via [`material_claim`]; this
/// only renders the CLI-specific "Best line: <SAN sequence> — <story>."
/// framing around it. The SAN-sequence prefix is the hypothetical
/// continuation, so it uses the *full* PV `events` (through the settled
/// ply), not the realized-only subset the GUI past-tense card consumes.
/// The CLI today always narrates the user's own move, so the perspective
/// is `Player`.
fn render_material_sequence(
    out: &mut dyn io::Write,
    pre_move_pos: &Position,
    user: &MoveAnalysis,
    outcome: &MaterialOutcome,
    root_stm: Color,
    perspective: Perspective,
) -> io::Result<()> {
    let sequence = pv_to_san_through(pre_move_pos, &user.pv, outcome.last_ply);
    let joined = sequence.join(" ");

    // An empty `events` (no captures) yields no story; fall back to the
    // bare SAN line, matching the prior behaviour where a balanced/empty
    // sequence let the SAN itself be the story.
    let story = if outcome.events.is_empty() {
        String::new()
    } else {
        let claim = material_claim(&outcome.events, root_stm);
        let ctx = PhrasingContext {
            perspective,
            locale: Locale::En,
            verbosity: Verbosity::Normal,
            reveal_moves: false,
        };
        phrase(&claim, &ctx).summary
    };

    writeln!(
        out,
        "                Best line: {joined}{}",
        if story.is_empty() {
            ".".to_string()
        } else {
            format!(" — {story}.")
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chess_tutor_engine::analysis::analyze_position;
    use chess_tutor_engine::engine::{Engine, SearchParams};
    use chess_tutor_engine::movegen::legal_moves_vec;

    /// Smoke: the migrated CLI headline (Claim + phrase) renders the
    /// retrospective for `1.e4` on the start position without panic and
    /// in the player's perspective. Guards against the
    /// `feedback_teaching_surface_smoke_test` regression mode.
    #[test]
    fn format_retrospective_smoke_startpos_e4() {
        let mut pos = Position::startpos();
        // e2e4 from the legal list.
        let e4 = legal_moves_vec(&mut pos)
            .into_iter()
            .find(|m| {
                m.from() == chess_tutor_engine::types::Square::E2
                    && m.to() == chess_tutor_engine::types::Square::E4
            })
            .unwrap();

        let mut engine = Engine::default();
        let analyses = analyze_position(
            &mut engine,
            &mut pos,
            SearchParams {
                max_depth: 6,
                multi_pv: 2,
                force_include: vec![e4],
                ..SearchParams::default()
            },
        );

        let text = format_retrospective(&pos, &analyses, e4, &NarrationOptions::default(), Perspective::Player);
        assert!(
            text.starts_with("[retrospective] You played e4"),
            "unexpected headline: {text}"
        );
        // Player perspective never produces the opponent reframe.
        assert!(!text.contains("Your chance"));

        // Step 12: the *same* analysis rendered from the opponent's side
        // must flip the headline to "They played …" and never say "You
        // played" — the perspective is the only thing that changed.
        let opp = format_retrospective(&pos, &analyses, e4, &NarrationOptions::default(), Perspective::Opponent);
        assert!(
            opp.starts_with("[retrospective] They played e4"),
            "opponent headline should read 'They played': {opp}"
        );
        assert!(
            !opp.contains("You played"),
            "opponent perspective must never say 'You played': {opp}"
        );
    }

    /// The migrated CLI material narration ("Best line: … — <story>")
    /// now sources its capture story from the shared Claim + phrase
    /// layer. On a clean pawn grab whose best line settles with a
    /// recapture, the story reads "You won/lost …" (player perspective)
    /// rather than the old lowercase "you win …".
    #[test]
    fn format_retrospective_material_line_uses_translator() {
        use chess_tutor_engine::types::Square;
        // White to capture on e5: 1.exd5-style open trade. The PV walks
        // a capture, so the material narrator fires.
        let mut pos =
            Position::from_fen("rnbqkbnr/ppp1pppp/8/3p4/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2")
                .unwrap();
        let exd5 = legal_moves_vec(&mut pos)
            .into_iter()
            .find(|m| m.from() == Square::E4 && m.to() == Square::D5)
            .unwrap();

        let mut engine = Engine::default();
        let analyses = analyze_position(
            &mut engine,
            &mut pos,
            SearchParams {
                max_depth: 8,
                multi_pv: 2,
                force_include: vec![exd5],
                ..SearchParams::default()
            },
        );

        let text = format_retrospective(&pos, &analyses, exd5, &NarrationOptions::default(), Perspective::Player);
        // The material line, if present, must carry the translator's
        // capitalized perspective phrasing — never the deleted lowercase
        // "you win/lose" prose.
        if let Some(line) = text.lines().find(|l| l.contains("Best line:")) {
            assert!(
                !line.contains("you win") && !line.contains("you lose"),
                "old hardcoded material prose leaked: {line}"
            );
        }
        // No panic + player perspective regardless of whether captures
        // surfaced in the settled PV.
        assert!(!text.contains("Your chance"));
    }

    /// Mobility migration smoke: a developing move that opens a piece's
    /// reach renders the translator's "activity" wording, never the
    /// deleted "activity dropped" / "activity improved" prose. We assert
    /// the absence of the old phrasing (the migration must leave no dual
    /// path) on the full retrospective path.
    #[test]
    fn format_retrospective_mobility_uses_translator() {
        use chess_tutor_engine::types::Square;
        // 1.Nf3 — the knight's reach changes; whatever mobility line
        // surfaces must come from the translator.
        let mut pos = Position::startpos();
        let nf3 = legal_moves_vec(&mut pos)
            .into_iter()
            .find(|m| m.from() == Square::G1 && m.to() == Square::F3)
            .unwrap();

        let mut engine = Engine::default();
        let analyses = analyze_position(
            &mut engine,
            &mut pos,
            SearchParams {
                max_depth: 6,
                multi_pv: 2,
                force_include: vec![nf3],
                ..SearchParams::default()
            },
        );

        let text = format_retrospective(&pos, &analyses, nf3, &NarrationOptions::default(), Perspective::Player);
        // The deleted hardcoded prose must not leak.
        assert!(
            !text.contains("activity dropped") && !text.contains("activity improved"),
            "old hardcoded mobility prose leaked: {text}"
        );
        // Any mobility line carries the new "more active" / "less active"
        // / "restrict the opponent's" wording — and the player POV never
        // emits the opponent reframe.
        assert!(!text.contains("Your chance"));
    }

    /// Step-9 smoke: a developing move on a richer middlegame position
    /// runs the migrated piece-placement / space / secondary surfaces
    /// through the shared Claim + phrase path without panic, in the
    /// player's perspective. Guards the
    /// `feedback_teaching_surface_smoke_test` regression mode.
    #[test]
    fn format_retrospective_step9_surfaces_smoke() {
        // An Italian-game middlegame where developing the rook to an
        // open/semi-open file is plausible — exercises the placement /
        // secondary lines on a real board rather than a contrived shape.
        let mut pos = Position::from_fen(
            "r1bqk2r/pppp1ppp/2n2n2/2b1p3/2B1P3/2N2N2/PPPP1PPP/R1BQ1RK1 w kq - 0 1",
        )
        .unwrap();
        let user = legal_moves_vec(&mut pos)
            .into_iter()
            .find(|m| {
                m.from() == chess_tutor_engine::types::Square::D2
                    && m.to() == chess_tutor_engine::types::Square::D3
            })
            .unwrap();

        let mut engine = Engine::default();
        let analyses = analyze_position(
            &mut engine,
            &mut pos,
            SearchParams {
                max_depth: 6,
                multi_pv: 2,
                force_include: vec![user],
                ..SearchParams::default()
            },
        );

        let text = format_retrospective(&pos, &analyses, user, &NarrationOptions::default(), Perspective::Player);
        // No panic, headline renders, and the player POV never emits the
        // opponent reframe ("Your chance" / "they …").
        assert!(text.starts_with("[retrospective] You played"), "{text}");
        assert!(!text.contains("Your chance"), "{text}");
        // None of the deleted hardcoded-prose fragments leak. (The
        // migrated translator never emits the lowercase narrator forms.)
        assert!(
            !text.contains("Your piece placement weakened")
                && !text.contains("Your piece placement improved")
                && !text.contains("diluting the opponent's space advantage"),
            "old hardcoded step-9 prose leaked: {text}"
        );
    }
}
