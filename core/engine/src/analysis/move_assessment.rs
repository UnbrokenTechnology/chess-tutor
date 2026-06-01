//! Live-intervention classifier: decide whether a user move warrants
//! pausing the game for a teaching prompt or a blunder safety net.
//!
//! This is **separate** from [`super::MoveVerdict`]. The verdict is a
//! retrospective qualitative label (Best / Inaccuracy / Mistake / …)
//! based on score drop alone; it drives the headline annotation and
//! the post-move card sentiment. The assessment here decides whether
//! to *interrupt* the player mid-game, which is a much higher bar:
//! interrupting on every non-best move trains a crutch, so the gate
//! only fires when there's a concrete, teachable concept.
//!
//! ## The two gates
//!
//! - **Blunder safety**: realized material loss ≥ a configurable
//!   threshold (default 300 cp). Catches "you just hung a piece" /
//!   "you just walked into a losing trade." UI shows a *takeback*
//!   prompt with no teaching content — the student already knows
//!   what they did; the prompt only saves time.
//!
//! - **Teaching moment**: the move's score drop is concentrated in a
//!   single granular eval [`TermId`] (KingDanger, MobilityKnight,
//!   PiecesBishopPawns, …). A drop that's distributed across many
//!   terms — even within the same family — is "engine subtlety,"
//!   not a concept, and gets filtered out. Gating at the term level
//!   (rather than at the broader `TermFamily` it belongs to) is what
//!   lets the prompt name a specific, learnable concept ("your
//!   knight covers fewer squares") instead of a vague catch-all
//!   ("piece placement"). UI shows a "Look again? / Show me what I
//!   missed / Continue" prompt that names the concept without ever
//!   naming the engine's preferred move.
//!
//! - **ALLOWED-not-MISSED** (PLAN-teaching-gui §3): the move swung a
//!   winning/equal position into the opponent's favour
//!   ([`gave_away_advantage`]) *and* a named detector explains it — a
//!   `user_walked_into` tactic from [`compute_tactic_outcome`] (which
//!   already folds in the surviving-latent-alignment fallback). The
//!   prompt reframes from "you missed a better move" to "your move *let
//!   your opponent do* something — what did you let them do?" This is
//!   the discovered-attack `Qc5+` / positional-punish `O-O` shape: the
//!   damage is something the opponent now gets to play, not a quiet
//!   downgrade. ALLOWED is a *teaching* dimension (it shares the
//!   `MistakeHandling` gate, not the blunder-safety gate); when it fires
//!   the UI leads with the reframe instead of the term-based prompt.
//!
//! The gates are independent: a move can be Fine, just-Blunder,
//! just-TeachingMoment, ALLOWED, or some combination. The UI layer
//! decides priority (blunder safety typically wins on the live prompt;
//! all surface independently in the game review).
//!
//! ## The silent-sequencing suppressor (PLAN-teaching-gui §3)
//!
//! There is a fourth, *negative* gate: when the bad eval only emerges
//! below human calculation depth **and no named detector fires**
//! ([`is_silent_sequencing`]), we suppress the pause entirely. Pausing
//! with a verdict the student could never have found — and can't act on
//! — is worse than not pausing (the `…Qc8` case). Detectors-only
//! already keeps the *coaching* surface quiet there; this makes the
//! *pause* quiet too. The suppressor runs last and, when it fires,
//! clears the teaching/allowed dimensions so [`MoveAssessment::is_fine`]
//! holds for the in-game pause.
//!
//! ## Why not reuse [`super::MoveVerdict`]?
//!
//! Verdict cares about *magnitude*; this classifier cares about
//! *teachability*. A 200 cp drop that's pure tactical depth ("you
//! missed a deep combination") is Mistake by verdict but Fine here
//! — there's no chess concept a 1200 player could have spotted.
//! A 60 cp drop that's all king safety is Good by verdict but a
//! Teaching Moment here — concrete, teachable, in the student's ZPD.

use super::{
    compute_material_outcome, compute_tactic_outcome, gave_away_advantage, is_silent_sequencing,
    MoveAnalysis, MoveVerdict, PriorMove, TacticHit, TermId,
};
use crate::position::Position;
use crate::types::{Color, Move, Square};

/// Tunable gates. Defaults are picked for "rarely interrupts, fires
/// only when the move has a concrete teachable concept."
#[derive(Clone, Copy, Debug)]
pub struct GatingConfig {
    /// Minimum score drop (best - user, root-STM POV, in engine cp)
    /// for the teaching gate to even consider a move. Drops below
    /// this are engine micro-imprecision; we never interrupt for
    /// them. Default 30 cp.
    pub noise_floor_cp: i32,
    /// Minimum fraction of the *signed* drop that must be carried by
    /// the dominant [`TermId`]. Drops that are 30/25/20/15/10 split
    /// across five terms are noise; drops where one term carries ≥60%
    /// of the swing are concept-shaped. Default 0.60.
    pub dominant_term_share_min: f32,
    /// Minimum absolute drop contributed by the dominant term for the
    /// teaching gate to fire. Even if a term carries 100% of a 35 cp
    /// drop, that's barely above the noise floor — the term-level
    /// severity needs its own minimum. Default 25 cp.
    pub teaching_term_severity_min_cp: i32,
    /// Absolute-severity escape. When a single term clears this
    /// magnitude on its own, fire the teaching moment regardless of
    /// the share gate — the signal is loud enough to teach from even
    /// if other terms also shifted. Default 50 cp.
    pub teaching_term_severity_escape_cp: i32,
    /// Minimum combined share of the drop carried by the top two
    /// terms for a *multi-term* teaching moment to fire — surfacing
    /// both concepts to the student instead of just one. The single-
    /// term share gate (`dominant_term_share_min`) takes precedence
    /// when it passes; this gate only matters when no single term
    /// dominates but the drop is still concentrated in two real
    /// signals (the 40/40/20-split case). Default 0.75.
    pub multi_term_coverage_min: f32,
    /// Minimum realized material loss (in cp, root-STM POV negative
    /// number's magnitude) to trip the blunder safety net. Default
    /// 300 cp ≈ losing a minor piece. Anything smaller (a pawn-down
    /// trade) doesn't warrant the safety prompt.
    pub blunder_material_min_cp: i32,
    /// Skip the teaching gate when the position was already this bad
    /// for the side to move (best_score ≤ this, in cp). Interrupting
    /// to teach in a hopeless position is noise — the student is
    /// past learning, they're just playing out. Default -500 cp.
    pub hopeless_score_max_cp: i32,
}

impl Default for GatingConfig {
    fn default() -> Self {
        Self {
            noise_floor_cp: 30,
            dominant_term_share_min: 0.60,
            teaching_term_severity_min_cp: 25,
            teaching_term_severity_escape_cp: 50,
            multi_term_coverage_min: 0.75,
            blunder_material_min_cp: 300,
            hopeless_score_max_cp: -500,
        }
    }
}

/// One classification of a user move's interventional shape. Blunder
/// and teaching are independent: a move can have neither, either, or
/// both. `Fine` is the convenience alias for "neither."
#[derive(Clone, Debug, PartialEq)]
pub struct MoveAssessment {
    pub blunder: Option<BlunderInfo>,
    pub teaching: Option<TeachingInfo>,
    /// `Some` when the move gave away a winning/equal position *and* a
    /// named detector explains what the opponent now gets to do — the
    /// ALLOWED-not-MISSED case. Drives the reframed prompt. Shares the
    /// `MistakeHandling` (teaching) gate, not the blunder-safety gate.
    pub allowed: Option<AllowedInfo>,
}

impl MoveAssessment {
    /// `true` when no gate fired — the game should continue without any
    /// live intervention. The retrospective still renders (it always
    /// does); this only controls the in-game pause.
    pub fn is_fine(&self) -> bool {
        self.blunder.is_none() && self.teaching.is_none() && self.allowed.is_none()
    }

    /// Convenience constructor for tests / no-op assessments.
    pub const fn fine() -> Self {
        Self {
            blunder: None,
            teaching: None,
            allowed: None,
        }
    }
}

/// The ALLOWED-not-MISSED dimension. Populated when the user's move
/// conceded a winning/equal position ([`gave_away_advantage`]) and a
/// named detector explains the swing — so the prompt can say "your move
/// let your opponent do *this*" instead of "you missed a better move."
///
/// Carries the punishing tactic (the `user_walked_into` hit) so the UI
/// can name the *pattern* (pre-move coaching names no squares; the
/// retrospective may). The cp swing is the conceded amount in root-STM
/// POV engine-cp.
#[derive(Clone, Debug, PartialEq)]
pub struct AllowedInfo {
    /// The tactic the user walked into — the opponent's punishing
    /// resource that the move failed to address. From
    /// [`compute_tactic_outcome`]'s `user_walked_into` slot, which folds
    /// in both the PV-realized tactic and the surviving-latent-alignment
    /// fallback (the `Qc5+` discovered-attack shape).
    pub walked_into: TacticHit,
    /// How much the move conceded, in root-STM POV engine-cp
    /// (`best_score − played_score`, always positive when this fires).
    pub conceded_cp: i32,
}

/// Realized material loss after the user's move. Drives the blunder
/// safety prompt — "your X is hanging — take back?"
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlunderInfo {
    /// Magnitude (positive number) of material the user is about to
    /// lose, in engine cp midgame. Threshold-passing means
    /// "≥ minor-piece worth" by default.
    pub material_loss_cp: i32,
    /// Square where the opponent's realized capture lands — the
    /// hanging piece's square. `None` when the loss is a sequence
    /// rather than a single piece (rare; UI can fall back to a
    /// generic prompt).
    pub lost_piece_square: Option<Square>,
}

/// One contributing term in a [`TeachingInfo`]. Used both for the
/// always-populated `dominant` field and the optional `secondary` —
/// when two real signals split the drop, the prompt names both.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TermContribution {
    /// The granular [`TermId`] this entry refers to.
    pub term: TermId,
    /// Absolute cp this term contributed to the user-side drop.
    pub severity_cp: i32,
    /// Fraction of total signed drop attributed to this term, in
    /// `[0.0, 1.0]`.
    pub share_of_drop: f32,
}

/// Teachable cost of the user's move. Drives the "Look again? / Show
/// me what I missed / Continue" prompt — UI names the specific
/// concept (KingDanger, MobilityKnight, …) without revealing the
/// engine's preferred move.
///
/// Callers that want the broader category can recover it cheaply via
/// [`TermFamily::of(info.dominant.term)`](TermFamily::of) — each
/// term maps 1:1 to a family.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TeachingInfo {
    /// The primary granular [`TermId`] that carried the drop. UI maps
    /// this to a specific concept-prompt ("your knight covers fewer
    /// squares", "your bishop is hemmed in by its own pawns", …).
    pub dominant: TermContribution,
    /// Optional second contributor surfaced when no single term
    /// dominates but the top two together cover ≥
    /// `multi_term_coverage_min` of the drop and both clear the
    /// severity floor. UI shows a "two things" prompt naming both.
    /// `None` when one term clearly dominates the drop.
    pub secondary: Option<TermContribution>,
}

/// Groups [`TermId`]s into the chess-concept families the UI cards
/// already organise around. Lives in the engine because the family
/// boundaries are an engine concern (which sub-terms are "really the
/// same thing" from a scoring perspective); the UI layer maps this
/// enum to its own category names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TermFamily {
    /// Piece values — what the colloquial "material" card shows.
    /// Treated separately from the other positional families so the
    /// blunder gate (which is material-specific) doesn't double-count
    /// it as a teaching dimension.
    Material,
    /// Piece-square positional contribution — the "development" card.
    Development,
    /// Imbalance term (bishop pair, knight outpost stack, etc.).
    Imbalance,
    /// Initiative — tempo-driven swings.
    Initiative,
    /// Space term.
    Space,
    /// King-safety bundle (shield, storm, danger, flank, …).
    KingSafety,
    /// Passed-pawn bundle.
    PassedPawns,
    /// Pawn-structure bundle (connected / isolated / backward / …).
    PawnStructure,
    /// Per-piece positional bundle (outpost, open file, trapped rook,
    /// long-diagonal bishop, …).
    PiecePlacement,
    /// Per-piece-type mobility bundle.
    Mobility,
    /// Threats bundle.
    Threats,
}

impl TermFamily {
    /// Map a granular [`TermId`] to its family. Single source of truth
    /// for the engine-side classifier; UI maps `TermFamily` to its own
    /// `RetrospectiveCategory`.
    pub fn of(term: TermId) -> Self {
        match term {
            TermId::MaterialPieceValue => TermFamily::Material,
            TermId::MaterialPsqPositional => TermFamily::Development,
            TermId::Imbalance => TermFamily::Imbalance,
            TermId::Initiative => TermFamily::Initiative,
            TermId::Space => TermFamily::Space,

            TermId::KingPawnShield
            | TermId::KingPawnStorm
            | TermId::KingPawnDistance
            | TermId::KingDanger
            | TermId::KingPawnlessFlank
            | TermId::KingFlankAttacks => TermFamily::KingSafety,

            TermId::PassedRankBonus
            | TermId::PassedKingProximity
            | TermId::PassedFreeAdvance
            | TermId::PassedStopperPenalty => TermFamily::PassedPawns,

            TermId::PawnsConnected
            | TermId::PawnsIsolated
            | TermId::PawnsBackward
            | TermId::PawnsDoubled
            | TermId::PawnsWeakUnopposed
            | TermId::PawnsWeakLever => TermFamily::PawnStructure,

            TermId::PiecesOutposts
            | TermId::PiecesReachableOutposts
            | TermId::PiecesMinorBehindPawn
            | TermId::PiecesKingProtector
            | TermId::PiecesBishopPawns
            | TermId::PiecesLongDiagonalBishop
            | TermId::PiecesRookOnQueenFile
            | TermId::PiecesRookOnOpenFile
            | TermId::PiecesRookOnSemiopenFile
            | TermId::PiecesTrappedRook
            | TermId::PiecesWeakQueen => TermFamily::PiecePlacement,

            TermId::MobilityKnight
            | TermId::MobilityBishop
            | TermId::MobilityRook
            | TermId::MobilityQueen => TermFamily::Mobility,

            TermId::ThreatsByMinor
            | TermId::ThreatsByRook
            | TermId::ThreatsByKing
            | TermId::ThreatsHanging
            | TermId::ThreatsRestricted
            | TermId::ThreatsBySafePawn
            | TermId::ThreatsByPawnPush
            | TermId::ThreatsKnightOnQueen
            | TermId::ThreatsSliderOnQueen => TermFamily::Threats,
        }
    }
}

/// Classify a single user move for live-intervention purposes.
///
/// - `pre_pos`: position the user faced (root of `analyses`).
/// - `analyses`: ranked search output — `[0]` is the engine's
///   preferred line, and `user_move` should appear somewhere in the
///   slice (typically via `SearchParams::force_include`).
/// - `user_move`: the move the user actually played.
/// - `config`: gating thresholds; see [`GatingConfig`].
/// - `prior_move`: the opponent's move *into* `pre_pos` (the move that
///   produced the position the user faced). Feeds the recapture guard in
///   the ALLOWED-detector's [`compute_tactic_outcome`] call and the
///   silent-sequencing suppressor's detector chain. Pass `None` at the
///   start of a game where no prior move exists.
///
/// Returns [`MoveAssessment::fine`] when the user's move can't be
/// found in `analyses` or the analyses slice is empty — the move
/// stands without intervention and the (still-running) retrospective
/// will narrate it normally.
pub fn classify_user_move(
    pre_pos: &Position,
    analyses: &[MoveAnalysis],
    user_move: Move,
    config: &GatingConfig,
    prior_move: Option<PriorMove>,
) -> MoveAssessment {
    let Some(best) = analyses.first() else {
        return MoveAssessment::fine();
    };
    let Some(user) = analyses.iter().find(|a| a.mv == user_move) else {
        return MoveAssessment::fine();
    };
    let root_stm = pre_pos.side_to_move();

    // You cannot "blunder" by playing the engine's own best move. When the
    // played move is Best (top choice) or BestAvailable (a hopeless
    // position where this is as good as anything — the Qxh8+ desperado that
    // trades an already-trapped queen for a rook), any material it sheds was
    // unavoidable: a takeback prompt is misleading because there's nothing
    // better to take it back to. This also avoids flagging a sound sacrifice
    // the engine endorses as best. A genuine blunder — a move meaningfully
    // worse than an available alternative — still has user ≠ best and fires.
    let move_is_best = matches!(
        user.classify(best.score),
        MoveVerdict::Best | MoveVerdict::BestAvailable
    );
    let blunder = if move_is_best {
        None
    } else {
        assess_blunder(pre_pos, user, root_stm, config)
    };
    let teaching = assess_teaching(best, user, root_stm, config);
    let allowed = assess_allowed(best, user, pre_pos, root_stm, prior_move);

    let mut assessment = MoveAssessment {
        blunder,
        teaching,
        allowed,
    };

    // Silent-sequencing suppressor (PLAN §3, the fourth gate). When the
    // bad eval only emerges below human calculation depth AND no *named*
    // detector fires, the verdict is unactionable — pausing on it is
    // worse than not pausing (the `…Qc8` case).
    //
    // It must override the *teaching* and *allowed* dimensions both. The
    // ALLOWED detector keys on `user_walked_into`, which surfaces even a
    // bare structural pin that "always lights" geometrically and survives
    // a defending move (`…Qc8` defends its pinned bishop 2:2). That pin
    // is exactly the pattern [`is_silent_sequencing`]'s detector chain
    // deliberately filters out (it counts only material-winning surviving
    // alignments — DiscoveredAttack / RemovingDefender / Skewer), so the
    // suppressor fires on `…Qc8` and clears the spurious ALLOWED there,
    // while leaving the genuine `Qc5+` discovered-attack ALLOWED intact
    // (that pattern IS material-winning, so the suppressor never fires).
    //
    // A realized-material blunder is always concrete and is never
    // suppressed — only the depth-hidden teaching/allowed pause is.
    let has_pause = assessment.teaching.is_some() || assessment.allowed.is_some();
    if has_pause && best.mv != user.mv {
        let deep_gap_cp = best.score.0 - user.score.0;
        if is_silent_sequencing(pre_pos, user.mv, best.mv, deep_gap_cp, prior_move) {
            assessment.teaching = None;
            assessment.allowed = None;
        }
    }

    assessment
}

/// Detect the ALLOWED-not-MISSED dimension: the move gave away a
/// winning/equal position ([`gave_away_advantage`]) AND a named
/// `user_walked_into` tactic explains what the opponent now gets to do.
///
/// `best` / `user` scores are root-STM POV engine-cp from the same root.
/// Returns `None` when the swing isn't a giveaway, or when no detector
/// explains it (in which case it's either a missed-move teaching moment
/// or — if the eval needs depth — silent sequencing, both handled
/// elsewhere).
fn assess_allowed(
    best: &MoveAnalysis,
    user: &MoveAnalysis,
    pre_pos: &Position,
    root_stm: Color,
    prior_move: Option<PriorMove>,
) -> Option<AllowedInfo> {
    if user.mv == best.mv {
        return None;
    }
    if !gave_away_advantage(best.score, user.score) {
        return None;
    }
    // A detector must explain the swing. `user_walked_into` already
    // folds in both the PV-realized opponent tactic and the
    // surviving-latent-alignment fallback (the `Qc5+` discovered-attack
    // shape PLAN §4.1 wired in), so a single call covers both the
    // "they actually play it" and "they have it loaded" cases.
    let outcome = compute_tactic_outcome(best, user, pre_pos, root_stm, prior_move);
    let walked_into = outcome.user_walked_into?;
    Some(AllowedInfo {
        walked_into,
        conceded_cp: best.score.0 - user.score.0,
    })
}

/// Compute the realized material loss from the user's move and the
/// opponent's best (ply-1) reply. Returns `Some` when the loss meets
/// the blunder threshold; `None` otherwise.
fn assess_blunder(
    pre_pos: &Position,
    user: &MoveAnalysis,
    root_stm: Color,
    config: &GatingConfig,
) -> Option<BlunderInfo> {
    let outcome = compute_material_outcome(user, pre_pos, root_stm);
    let net = outcome.realized_net_mg_cp(root_stm);
    if net > -config.blunder_material_min_cp {
        return None;
    }
    // Find the opponent's capture square in the realized window so
    // the UI can highlight the at-risk piece. If the realized window
    // has both a user capture and an opponent recapture, the
    // opponent's capture is the second event; we still want to point
    // at the piece we lost, so we pick the captor != root_stm entry.
    let lost_piece_square = outcome
        .realized_events()
        .find(|ev| ev.captor != root_stm)
        .map(|ev| ev.square);
    Some(BlunderInfo {
        material_loss_cp: -net,
        lost_piece_square,
    })
}

/// Compute the teaching-moment classification.
///
/// Returns `Some` under any of three fire scenarios, evaluated in
/// priority order — the position must always pass the hopeless gate
/// and the overall noise floor first.
///
/// 1. **Multi-term**: top two terms cover ≥ `multi_term_coverage_min`
///    of the drop with both clearing `teaching_term_severity_min_cp`
///    — surface both. Catches the 40/40/20-split case.
/// 2. **Absolute-severity escape**: a single term clears
///    `teaching_term_severity_escape_cp` on its own — surface it
///    regardless of share. Catches the loud-single-signal case where
///    the drop is split with other smaller signals.
/// 3. **Single-term dominance**: a single term carries ≥
///    `dominant_term_share_min` of the drop and clears
///    `teaching_term_severity_min_cp` — the original gate.
fn assess_teaching(
    best: &MoveAnalysis,
    user: &MoveAnalysis,
    root_stm: Color,
    config: &GatingConfig,
) -> Option<TeachingInfo> {
    if best.score.0 <= config.hopeless_score_max_cp {
        return None;
    }
    let drop = (best.score.0 - user.score.0).max(0);
    if drop < config.noise_floor_cp {
        return None;
    }
    // term_deltas are white-POV (post - pre). Flip the sign to put
    // them in root-STM POV; keep only the *negative* ones (the
    // user-side drops). We track each TermId individually rather
    // than rolling up into family buckets — gating per-term is what
    // lets the prompt name a specific concept the student can act
    // on instead of a vague catch-all.
    let sign = if root_stm == Color::White { 1 } else { -1 };
    let mut top1: Option<(TermId, i32)> = None;
    let mut top2: Option<(TermId, i32)> = None;
    let mut total_drop: i32 = 0;
    for td in &user.term_deltas {
        let signed = td.delta_tapered * sign;
        if signed >= 0 {
            continue;
        }
        let magnitude = -signed;
        total_drop += magnitude;
        // Maintain top1/top2 by magnitude. A new entry might displace
        // top1 (in which case the old top1 becomes top2), or land
        // between them (becoming top2), or be smaller than both.
        if top1.is_none_or(|(_, m)| magnitude > m) {
            top2 = top1;
            top1 = Some((td.term, magnitude));
        } else if top2.is_none_or(|(_, m)| magnitude > m) {
            top2 = Some((td.term, magnitude));
        }
    }
    let (top1_term, top1_severity) = top1?;
    if total_drop == 0 {
        return None;
    }
    // MaterialPieceValue is the blunder gate's territory — if it ever
    // tops the list, defer to the blunder pipeline.
    if top1_term == TermId::MaterialPieceValue {
        return None;
    }
    let top1_share = top1_severity as f32 / total_drop as f32;
    let dominant = TermContribution {
        term: top1_term,
        severity_cp: top1_severity,
        share_of_drop: top1_share,
    };

    // Scenario 1: multi-term. Two real signals; surface both.
    if let Some((top2_term, top2_severity)) = top2 {
        let combined_share = (top1_severity + top2_severity) as f32 / total_drop as f32;
        let both_above_severity = top1_severity >= config.teaching_term_severity_min_cp
            && top2_severity >= config.teaching_term_severity_min_cp;
        // Exclude MaterialPieceValue from the secondary slot too —
        // material-driven swings belong to the blunder gate.
        if combined_share >= config.multi_term_coverage_min
            && both_above_severity
            && top2_term != TermId::MaterialPieceValue
        {
            return Some(TeachingInfo {
                dominant,
                secondary: Some(TermContribution {
                    term: top2_term,
                    severity_cp: top2_severity,
                    share_of_drop: top2_severity as f32 / total_drop as f32,
                }),
            });
        }
    }

    // Scenario 2: absolute-severity escape. One loud signal, regardless
    // of share. Skips the dominance test entirely.
    if top1_severity >= config.teaching_term_severity_escape_cp {
        return Some(TeachingInfo {
            dominant,
            secondary: None,
        });
    }

    // Scenario 3: single-term dominance. The original gate.
    if top1_severity >= config.teaching_term_severity_min_cp
        && top1_share >= config.dominant_term_share_min
    {
        return Some(TeachingInfo {
            dominant,
            secondary: None,
        });
    }
    None
}

#[cfg(test)]
#[path = "move_assessment_tests.rs"]
mod tests;
