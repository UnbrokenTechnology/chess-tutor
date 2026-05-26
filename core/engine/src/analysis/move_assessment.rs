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
//! The two gates are independent: a move can be Fine, just-Blunder,
//! just-TeachingMoment, or both. The UI layer decides priority
//! (blunder safety typically wins on the live prompt; both surface
//! independently in the game review).
//!
//! ## Why not reuse [`super::MoveVerdict`]?
//!
//! Verdict cares about *magnitude*; this classifier cares about
//! *teachability*. A 200 cp drop that's pure tactical depth ("you
//! missed a deep combination") is Mistake by verdict but Fine here
//! — there's no chess concept a 1200 player could have spotted.
//! A 60 cp drop that's all king safety is Good by verdict but a
//! Teaching Moment here — concrete, teachable, in the student's ZPD.

use super::{compute_material_outcome, MoveAnalysis, TermId};
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
}

impl MoveAssessment {
    /// `true` when neither gate fired — the game should continue
    /// without any live intervention. The retrospective still renders
    /// (it always does); this only controls the in-game pause.
    pub fn is_fine(&self) -> bool {
        self.blunder.is_none() && self.teaching.is_none()
    }

    /// Convenience constructor for tests / no-op assessments.
    pub const fn fine() -> Self {
        Self {
            blunder: None,
            teaching: None,
        }
    }
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
) -> MoveAssessment {
    let Some(best) = analyses.first() else {
        return MoveAssessment::fine();
    };
    let Some(user) = analyses.iter().find(|a| a.mv == user_move) else {
        return MoveAssessment::fine();
    };
    let root_stm = pre_pos.side_to_move();

    let blunder = assess_blunder(pre_pos, user, root_stm, config);
    let teaching = assess_teaching(best, user, root_stm, config);

    MoveAssessment { blunder, teaching }
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
        if top1.map_or(true, |(_, m)| magnitude > m) {
            top2 = top1;
            top1 = Some((td.term, magnitude));
        } else if top2.map_or(true, |(_, m)| magnitude > m) {
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
mod tests {
    use super::*;
    use crate::eval::EvalTrace;
    use crate::types::{Square, Value};

    use super::super::TermDelta;

    fn make_delta(term: TermId, white_pov_tapered: i32) -> TermDelta {
        TermDelta {
            term,
            delta_mg: white_pov_tapered,
            delta_eg: white_pov_tapered,
            delta_tapered: white_pov_tapered,
            piece_involved: None,
        }
    }

    /// Build a minimal `MoveAnalysis` suitable for assess_teaching
    /// tests. PV is just the single user move so `compute_material_*`
    /// paths (when called) see no captures.
    fn make_analysis(mv: Move, score_cp: i32, term_deltas: Vec<TermDelta>) -> MoveAnalysis {
        MoveAnalysis {
            mv,
            score: Value(score_cp),
            depth: 8,
            pv: vec![mv],
            ply_traces: vec![EvalTrace::zero()],
            settled_ply: Some(0),
            pre_move_trace: EvalTrace::zero(),
            pre_score: Value::ZERO,
            term_deltas,
        }
    }

    fn quiet_move() -> Move {
        // a2-a3 — legal from startpos, never a capture.
        Move::normal(Square::A2, Square::A3)
    }

    fn other_quiet_move() -> Move {
        // h2-h3 — legal from startpos, distinct from a2-a3.
        Move::normal(Square::H2, Square::H3)
    }

    // ---- assess_teaching: noise floor + dominance gate -------------

    #[test]
    fn teaching_fires_on_single_term_dominance() {
        // User move drops 80 cp on the user-side; one TermId carries
        // 70/80 = 87.5%. White-to-move so root_stm is White; negative
        // white-POV tapered deltas are user-side drops.
        let best = make_analysis(other_quiet_move(), 60, vec![]);
        let user = make_analysis(
            quiet_move(),
            -20,
            vec![
                make_delta(TermId::KingDanger, -70),
                make_delta(TermId::KingPawnShield, -10),
            ],
        );
        let info = assess_teaching(&best, &user, Color::White, &GatingConfig::default())
            .expect("dominant king-safety drop should fire");
        assert_eq!(info.dominant.term, TermId::KingDanger);
        assert_eq!(info.dominant.severity_cp, 70);
        assert!((info.dominant.share_of_drop - 70.0 / 80.0).abs() < 1e-6);
        // 70 cp is above the absolute-severity escape (50 cp), so this
        // would have fired via that path even without a 60% share.
        // The dominance-share path takes precedence when both pass.
        // Either way, single-signal → no secondary.
        assert!(info.secondary.is_none());
    }

    #[test]
    fn teaching_skipped_when_drop_spread_within_a_family() {
        // 40 cp total drop spread 15/13/12 across three piece-placement
        // sub-terms. Per-family gating would have fired ("40 cp of
        // piece placement!"); per-term gating doesn't, because no
        // single TermId carries 60% AND none crosses the absolute-
        // severity escape (50 cp). The Nc3-in-Four-Knights case.
        let best = make_analysis(other_quiet_move(), 60, vec![]);
        let user = make_analysis(
            quiet_move(),
            20,
            vec![
                make_delta(TermId::PiecesKingProtector, -15),
                make_delta(TermId::PiecesBishopPawns, -13),
                make_delta(TermId::PiecesMinorBehindPawn, -12),
            ],
        );
        assert_eq!(
            assess_teaching(&best, &user, Color::White, &GatingConfig::default()),
            None
        );
    }

    #[test]
    fn teaching_fires_via_absolute_escape_when_no_single_term_dominates() {
        // 100 cp total drop split 55/30/15. Top term doesn't hit the
        // 60% share gate (55/100), but it does clear the 50 cp
        // absolute-severity escape. Fire on the single dominant term.
        let best = make_analysis(other_quiet_move(), 80, vec![]);
        let user = make_analysis(
            quiet_move(),
            -20,
            vec![
                make_delta(TermId::KingDanger, -55),
                make_delta(TermId::ThreatsHanging, -30),
                make_delta(TermId::PiecesBishopPawns, -15),
            ],
        );
        let info = assess_teaching(&best, &user, Color::White, &GatingConfig::default())
            .expect("absolute-severity escape should fire on 55 cp signal");
        assert_eq!(info.dominant.term, TermId::KingDanger);
        assert_eq!(info.dominant.severity_cp, 55);
        // 55+30 = 85 ≥ 75 — multi-term wins over the escape path.
        // This codifies the priority order.
        assert_eq!(
            info.secondary.map(|s| s.term),
            Some(TermId::ThreatsHanging)
        );
    }

    #[test]
    fn teaching_fires_multi_term_on_two_real_signals() {
        // 100 cp total drop split 40/40/20. Neither term dominates
        // (each is 40% of the drop), but both individually clear the
        // 25 cp severity floor and together cover 80% — two real,
        // teachable signals. Surface both.
        let best = make_analysis(other_quiet_move(), 80, vec![]);
        let user = make_analysis(
            quiet_move(),
            -20,
            vec![
                make_delta(TermId::PiecesRookOnOpenFile, -40),
                make_delta(TermId::KingPawnShield, -40),
                make_delta(TermId::MobilityBishop, -20),
            ],
        );
        let info = assess_teaching(&best, &user, Color::White, &GatingConfig::default())
            .expect("multi-term gate should fire on 40/40 case");
        assert_eq!(info.dominant.term, TermId::PiecesRookOnOpenFile);
        assert_eq!(info.dominant.severity_cp, 40);
        let secondary = info.secondary.expect("secondary present");
        assert_eq!(secondary.term, TermId::KingPawnShield);
        assert_eq!(secondary.severity_cp, 40);
    }

    #[test]
    fn teaching_skipped_when_drop_distributed_across_families() {
        // 80 cp total drop split 30/25/25 across three terms in
        // different families. No single term hits the 60% share gate
        // (30/80 = 37.5%), none crosses the 50 cp escape, and the
        // top-two coverage is only 55/80 = 69% — below the 75%
        // multi-term threshold. Genuine noise — skip.
        let best = make_analysis(other_quiet_move(), 60, vec![]);
        let user = make_analysis(
            quiet_move(),
            -20,
            vec![
                make_delta(TermId::KingDanger, -30),
                make_delta(TermId::PiecesOutposts, -25),
                make_delta(TermId::MobilityKnight, -25),
            ],
        );
        assert_eq!(
            assess_teaching(&best, &user, Color::White, &GatingConfig::default()),
            None
        );
    }

    #[test]
    fn teaching_skipped_when_multi_term_secondary_below_severity_floor() {
        // 100 cp drop split 75/15/5/5. The top hits the absolute
        // escape and would fire single-term. The second is below the
        // 25 cp severity floor, so the multi-term branch doesn't
        // surface it. Result: single-term intervention.
        let best = make_analysis(other_quiet_move(), 80, vec![]);
        let user = make_analysis(
            quiet_move(),
            -20,
            vec![
                make_delta(TermId::KingDanger, -75),
                make_delta(TermId::MobilityKnight, -15),
                make_delta(TermId::ThreatsHanging, -5),
                make_delta(TermId::PiecesBishopPawns, -5),
            ],
        );
        let info = assess_teaching(&best, &user, Color::White, &GatingConfig::default())
            .expect("fires");
        assert_eq!(info.dominant.term, TermId::KingDanger);
        assert!(info.secondary.is_none(), "secondary too small to surface");
    }

    #[test]
    fn teaching_skipped_when_drop_below_noise_floor() {
        // 20 cp drop, entirely king-safety — but below the default
        // 30 cp noise floor. No prompt.
        let best = make_analysis(other_quiet_move(), 30, vec![]);
        let user = make_analysis(
            quiet_move(),
            10,
            vec![make_delta(TermId::KingDanger, -20)],
        );
        assert_eq!(
            assess_teaching(&best, &user, Color::White, &GatingConfig::default()),
            None
        );
    }

    #[test]
    fn teaching_skipped_when_dominant_term_severity_below_min() {
        // 35 cp total drop, all in one term (100% share!) but the
        // term-severity gate (25 cp default) still passes because
        // 35 ≥ 25. Tighten the threshold to verify the gate works.
        let best = make_analysis(other_quiet_move(), 30, vec![]);
        let user = make_analysis(
            quiet_move(),
            -10,
            vec![make_delta(TermId::KingDanger, -35)],
        );
        let strict = GatingConfig {
            teaching_term_severity_min_cp: 50,
            ..GatingConfig::default()
        };
        assert_eq!(assess_teaching(&best, &user, Color::White, &strict), None);
    }

    #[test]
    fn teaching_skipped_when_position_already_hopeless() {
        // best.score is -600 — past the -500 default hopeless cap.
        // Even a real teaching dimension shouldn't fire mid-loss.
        let best = make_analysis(other_quiet_move(), -600, vec![]);
        let user = make_analysis(
            quiet_move(),
            -700,
            vec![make_delta(TermId::KingDanger, -100)],
        );
        assert_eq!(
            assess_teaching(&best, &user, Color::White, &GatingConfig::default()),
            None
        );
    }

    #[test]
    fn teaching_skipped_when_user_is_best_move() {
        // user.score == best.score → drop is zero → noise floor.
        let best = make_analysis(quiet_move(), 60, vec![]);
        let user = make_analysis(
            quiet_move(),
            60,
            vec![make_delta(TermId::KingDanger, -100)], // shouldn't matter
        );
        assert_eq!(
            assess_teaching(&best, &user, Color::White, &GatingConfig::default()),
            None
        );
    }

    #[test]
    fn teaching_skipped_when_dominant_term_is_material_piece_value() {
        // Material piece-value drops are handled by the blunder gate.
        // A pure piece-value drop here would otherwise pass the
        // share+severity gates, but we explicitly exclude it so we
        // don't double-narrate ("teaching: material" alongside
        // "blunder: lost N cp").
        let best = make_analysis(other_quiet_move(), 60, vec![]);
        let user = make_analysis(
            quiet_move(),
            -40,
            vec![make_delta(TermId::MaterialPieceValue, -100)],
        );
        assert_eq!(
            assess_teaching(&best, &user, Color::White, &GatingConfig::default()),
            None
        );
    }

    #[test]
    fn teaching_picks_largest_negative_term() {
        // Two negative deltas; the prompt's dominant.term should be
        // whichever single TermId carried more. (Both are in the same
        // family here, which is fine — the gate is per-term, but the
        // chosen term is just whichever has the largest magnitude.)
        let best = make_analysis(other_quiet_move(), 60, vec![]);
        let user = make_analysis(
            quiet_move(),
            -20,
            vec![
                make_delta(TermId::KingDanger, -30),
                make_delta(TermId::KingPawnShield, -50),
            ],
        );
        let info = assess_teaching(&best, &user, Color::White, &GatingConfig::default())
            .expect("fires");
        assert_eq!(info.dominant.term, TermId::KingPawnShield);
    }

    #[test]
    fn teaching_root_stm_black_flips_sign() {
        // root_stm is Black, so a *positive* white-POV delta is a
        // user-side drop. Same scenario as the dominance test but with
        // signs flipped.
        let best = make_analysis(other_quiet_move(), 60, vec![]);
        let user = make_analysis(
            quiet_move(),
            -20,
            vec![
                make_delta(TermId::KingDanger, 70),
                make_delta(TermId::KingPawnShield, 10),
            ],
        );
        let info = assess_teaching(&best, &user, Color::Black, &GatingConfig::default())
            .expect("black-side drop should fire");
        assert_eq!(info.dominant.term, TermId::KingDanger);
        assert_eq!(info.dominant.severity_cp, 70);
    }

    // ---- term_family mapping coverage ------------------------------

    #[test]
    fn term_family_every_term_id_has_a_mapping() {
        // Exhaustive sweep: every TermId returns a family without
        // panicking. Catches future TermId additions that forget to
        // extend `TermFamily::of`.
        for &t in &TermId::ALL {
            let _ = TermFamily::of(t);
        }
    }

    #[test]
    fn term_family_groups_king_subterms_together() {
        assert_eq!(TermFamily::of(TermId::KingDanger), TermFamily::KingSafety);
        assert_eq!(
            TermFamily::of(TermId::KingPawnShield),
            TermFamily::KingSafety
        );
        assert_eq!(
            TermFamily::of(TermId::KingFlankAttacks),
            TermFamily::KingSafety
        );
    }

    // ---- classify_user_move: end-to-end on a real position ---------

    #[test]
    fn classify_returns_fine_when_user_move_not_in_analyses() {
        let pre = Position::startpos();
        let analyses: Vec<MoveAnalysis> = Vec::new();
        let assessment = classify_user_move(
            &pre,
            &analyses,
            quiet_move(),
            &GatingConfig::default(),
        );
        assert!(assessment.is_fine());
    }

    /// Real position where Black is to move and can hang the queen
    /// to a knight pickup. We run a small search, force the hanging
    /// move into the analyses, and confirm the classifier flags it
    /// as a blunder.
    #[test]
    fn classify_flags_hung_queen_as_blunder() {
        use crate::engine::{Engine, SearchParams};

        // White: K e1, N f3. Black: K e8, Q d8. Black plays Qd4 and
        // White's Nxd4 wins the queen — a 900+ cp realized loss.
        let mut pre = Position::from_fen(
            "3qk3/8/8/8/8/5N2/8/4K3 b - - 0 1",
        )
        .expect("valid FEN");
        let hang = Move::normal(Square::D8, Square::D4);

        let mut engine = Engine::default();
        let analyses = super::super::analyze_position(
            &mut engine,
            &mut pre,
            SearchParams {
                max_depth: 4,
                multi_pv: 4,
                force_include: vec![hang],
                ..SearchParams::default()
            },
        );
        let pre = Position::from_fen("3qk3/8/8/8/8/5N2/8/4K3 b - - 0 1").unwrap();
        let assessment = classify_user_move(&pre, &analyses, hang, &GatingConfig::default());
        let blunder = assessment.blunder.expect("Qd4 should trip blunder gate");
        // Queen midgame value is well above 300 cp.
        assert!(
            blunder.material_loss_cp >= 700,
            "expected ≥ 700 cp loss, got {}",
            blunder.material_loss_cp
        );
        // The hanging piece lands on d4 after Nxd4.
        assert_eq!(blunder.lost_piece_square, Some(Square::D4));
    }
}
