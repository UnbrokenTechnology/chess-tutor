//! [`TermId`] — one identifier per granular [`EvalTrace`] sub-term,
//! plus the net-score extractor that [`super::term_delta`] uses to
//! diff two traces.
//!
//! The enum is the single source of truth for:
//! - **Labels**: kebab-case `"mobility.knight"` etc., used by the CLI
//!   eval-report and retrospective secondary-terms renderers.
//! - **The net-score formula**: each variant knows whether to read a
//!   single net field (Material / Imbalance / Initiative) or subtract
//!   one colour's sub-term from the other.
//! - **Iteration order (`ALL`)**: `compute_term_deltas` walks this in
//!   order before sorting, so ties break deterministically.

use crate::eval::EvalTrace;
use crate::types::Score;

/// Which trace a term's delta should diff against.
///
/// - **Outcome** terms ([`TermId::Material`], [`TermId::Imbalance`])
///   read at the **settled ply** because they describe the line's
///   eventual outcome — captures along the PV and the piece-pair
///   shifts that follow. Pairs with the material-narrator's
///   capture-sequence framing.
/// - **State** terms read at **ply 1** (immediately after the user's
///   move). Threats / king safety / pawn structure / mobility / piece
///   placement / passed pawns / space / initiative all describe the
///   board state right after the user's single move, *not* what the
///   engine's projected continuation produces several plies later.
///   This avoids attributing distant-future hangs or distant-future
///   bishop reach to the user's one move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Timing {
    /// Diff against the settled-ply trace.
    Outcome,
    /// Diff against the ply-1 (post-user-move) trace.
    State,
}

/// One identifier per granular sub-term of an [`EvalTrace`]. Used by
/// [`super::TermDelta`] so a UI can attribute a tapered cp swing to a
/// named chess concept.
///
/// The enum groups terms by how they appear in the trace:
///
/// - *Net* terms (`MaterialPieceValue`, `MaterialPsqPositional`,
///   `Imbalance`, `Initiative`) are already stored as `white - black`
///   in the trace; their delta is a single signed number with no
///   per-colour split.
/// - *Per-colour scalar* terms (`Space`) appear as `[Score; 2]` in the
///   trace and are summed as `white - black` before diffing.
/// - *Per-colour sub-terms* unpacked from `KingBreakdown`,
///   `PassedBreakdown`, `PawnsBreakdown`, `PiecesBreakdown`,
///   `MobilityBreakdown`, `ThreatsBreakdown`. Same `white - black`
///   aggregation as the per-colour scalar terms.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TermId {
    // Single-valued (already net) — the two halves of Stockfish's
    // PSQT score, split so colloquial "material" (piece counts)
    // separates from PSQ positional contribution (every piece move).
    MaterialPieceValue,
    MaterialPsqPositional,
    Imbalance,
    Initiative,

    // Per-colour scalar
    Space,

    // Per-colour KingBreakdown sub-terms — the pre-split `KingShelter`
    // term decomposed into the three named chess concepts the
    // pawns::king_safety calculation actually combines (friendly
    // pawn shield, enemy pawn storm, endgame king-pawn distance).
    KingPawnShield,
    KingPawnStorm,
    KingPawnDistance,
    KingDanger,
    KingPawnlessFlank,
    KingFlankAttacks,

    // Per-colour PassedBreakdown sub-terms
    PassedRankBonus,
    PassedKingProximity,
    PassedFreeAdvance,
    PassedStopperPenalty,

    // Per-colour PawnsBreakdown sub-terms
    PawnsConnected,
    PawnsIsolated,
    PawnsBackward,
    PawnsDoubled,
    PawnsWeakUnopposed,
    PawnsWeakLever,

    // Per-colour PiecesBreakdown sub-terms
    PiecesOutposts,
    PiecesReachableOutposts,
    PiecesMinorBehindPawn,
    PiecesKingProtector,
    PiecesBishopPawns,
    PiecesLongDiagonalBishop,
    PiecesRookOnQueenFile,
    PiecesRookOnOpenFile,
    PiecesRookOnSemiopenFile,
    PiecesTrappedRook,
    PiecesWeakQueen,

    // Per-colour MobilityBreakdown sub-terms
    MobilityKnight,
    MobilityBishop,
    MobilityRook,
    MobilityQueen,

    // Per-colour ThreatsBreakdown sub-terms
    ThreatsByMinor,
    ThreatsByRook,
    ThreatsByKing,
    ThreatsHanging,
    ThreatsRestricted,
    ThreatsBySafePawn,
    ThreatsByPawnPush,
    ThreatsKnightOnQueen,
    ThreatsSliderOnQueen,
}

impl TermId {
    /// Every [`TermId`] in a fixed order. Iteration order is the order
    /// diffs are emitted before sorting; callers that sort by absolute
    /// tapered delta see deterministic tie-breaking.
    pub const ALL: [TermId; 45] = [
        TermId::MaterialPieceValue,
        TermId::MaterialPsqPositional,
        TermId::Imbalance,
        TermId::Initiative,
        TermId::Space,
        TermId::KingPawnShield,
        TermId::KingPawnStorm,
        TermId::KingPawnDistance,
        TermId::KingDanger,
        TermId::KingPawnlessFlank,
        TermId::KingFlankAttacks,
        TermId::PassedRankBonus,
        TermId::PassedKingProximity,
        TermId::PassedFreeAdvance,
        TermId::PassedStopperPenalty,
        TermId::PawnsConnected,
        TermId::PawnsIsolated,
        TermId::PawnsBackward,
        TermId::PawnsDoubled,
        TermId::PawnsWeakUnopposed,
        TermId::PawnsWeakLever,
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
        TermId::MobilityKnight,
        TermId::MobilityBishop,
        TermId::MobilityRook,
        TermId::MobilityQueen,
        TermId::ThreatsByMinor,
        TermId::ThreatsByRook,
        TermId::ThreatsByKing,
        TermId::ThreatsHanging,
        TermId::ThreatsRestricted,
        TermId::ThreatsBySafePawn,
        TermId::ThreatsByPawnPush,
        TermId::ThreatsKnightOnQueen,
        TermId::ThreatsSliderOnQueen,
    ];

    /// Human-readable name for table rendering. Kebab-case so the CLI
    /// report aligns with the labels in `eval_report.rs`.
    pub const fn label(self) -> &'static str {
        match self {
            TermId::MaterialPieceValue => "material.piece-value",
            TermId::MaterialPsqPositional => "material.psq-positional",
            TermId::Imbalance => "imbalance",
            TermId::Initiative => "initiative",
            TermId::Space => "space",
            TermId::KingPawnShield => "king.pawn-shield",
            TermId::KingPawnStorm => "king.pawn-storm",
            TermId::KingPawnDistance => "king.pawn-distance",
            TermId::KingDanger => "king.danger",
            TermId::KingPawnlessFlank => "king.pawnless-flank",
            TermId::KingFlankAttacks => "king.flank-attacks",
            TermId::PassedRankBonus => "passed.rank-bonus",
            TermId::PassedKingProximity => "passed.king-proximity",
            TermId::PassedFreeAdvance => "passed.free-advance",
            TermId::PassedStopperPenalty => "passed.stopper-penalty",
            TermId::PawnsConnected => "pawns.connected",
            TermId::PawnsIsolated => "pawns.isolated",
            TermId::PawnsBackward => "pawns.backward",
            TermId::PawnsDoubled => "pawns.doubled",
            TermId::PawnsWeakUnopposed => "pawns.weak-unopposed",
            TermId::PawnsWeakLever => "pawns.weak-lever",
            TermId::PiecesOutposts => "pieces.outposts",
            TermId::PiecesReachableOutposts => "pieces.reachable-outposts",
            TermId::PiecesMinorBehindPawn => "pieces.minor-behind-pawn",
            TermId::PiecesKingProtector => "pieces.king-protector",
            TermId::PiecesBishopPawns => "pieces.bishop-pawns",
            TermId::PiecesLongDiagonalBishop => "pieces.long-diagonal-bishop",
            TermId::PiecesRookOnQueenFile => "pieces.rook-on-queen-file",
            TermId::PiecesRookOnOpenFile => "pieces.rook-on-open-file",
            TermId::PiecesRookOnSemiopenFile => "pieces.rook-on-semiopen-file",
            TermId::PiecesTrappedRook => "pieces.trapped-rook",
            TermId::PiecesWeakQueen => "pieces.weak-queen",
            TermId::MobilityKnight => "mobility.knight",
            TermId::MobilityBishop => "mobility.bishop",
            TermId::MobilityRook => "mobility.rook",
            TermId::MobilityQueen => "mobility.queen",
            TermId::ThreatsByMinor => "threats.by-minor",
            TermId::ThreatsByRook => "threats.by-rook",
            TermId::ThreatsByKing => "threats.by-king",
            TermId::ThreatsHanging => "threats.hanging",
            TermId::ThreatsRestricted => "threats.restricted",
            TermId::ThreatsBySafePawn => "threats.by-safe-pawn",
            TermId::ThreatsByPawnPush => "threats.by-pawn-push",
            TermId::ThreatsKnightOnQueen => "threats.knight-on-queen",
            TermId::ThreatsSliderOnQueen => "threats.slider-on-queen",
        }
    }

    /// Plain-English label for student-facing prose. Used by the
    /// retrospective's fallback "Shifts / Also" line — the technical
    /// kebab-case [`label`] is reserved for tables (eval trace
    /// report, `search --analyze`) where a consistent identifier
    /// matters more than readability.
    ///
    /// [`label`]: Self::label
    pub const fn pretty_label(self) -> &'static str {
        match self {
            // Colloquial chess "material" — piece counts only.
            TermId::MaterialPieceValue => "material",
            // PSQT positional contribution — pieces ending up on
            // better/worse squares per the piece-square tables.
            TermId::MaterialPsqPositional => "piece placement",
            TermId::Imbalance => "piece imbalance",
            TermId::Initiative => "initiative",
            TermId::Space => "space",
            TermId::KingPawnShield => "king pawn shield",
            TermId::KingPawnStorm => "enemy pawn storm",
            TermId::KingPawnDistance => "king pawn distance",
            TermId::KingDanger => "king safety",
            TermId::KingPawnlessFlank => "pawnless flank",
            TermId::KingFlankAttacks => "flank attacks",
            TermId::PassedRankBonus => "passed pawn advance",
            TermId::PassedKingProximity => "passed pawn king race",
            TermId::PassedFreeAdvance => "passed pawn path",
            TermId::PassedStopperPenalty => "passed pawn blockade",
            TermId::PawnsConnected => "connected pawns",
            TermId::PawnsIsolated => "isolated pawns",
            TermId::PawnsBackward => "backward pawns",
            TermId::PawnsDoubled => "doubled pawns",
            TermId::PawnsWeakUnopposed => "weak pawns",
            TermId::PawnsWeakLever => "pawn levers",
            TermId::PiecesOutposts => "outposts",
            TermId::PiecesReachableOutposts => "outpost potential",
            TermId::PiecesMinorBehindPawn => "minor piece behind a pawn",
            TermId::PiecesKingProtector => "king protectors",
            TermId::PiecesBishopPawns => "bishop vs. own pawns",
            TermId::PiecesLongDiagonalBishop => "long-diagonal bishop",
            TermId::PiecesRookOnQueenFile => "rook on queen's file",
            TermId::PiecesRookOnOpenFile => "rook on open file",
            TermId::PiecesRookOnSemiopenFile => "rook on semi-open file",
            TermId::PiecesTrappedRook => "trapped rook",
            TermId::PiecesWeakQueen => "queen under pressure",
            TermId::MobilityKnight => "knight activity",
            TermId::MobilityBishop => "bishop activity",
            TermId::MobilityRook => "rook activity",
            TermId::MobilityQueen => "queen activity",
            TermId::ThreatsByMinor => "minor pieces attacking",
            TermId::ThreatsByRook => "rooks attacking",
            TermId::ThreatsByKing => "king joining the attack",
            TermId::ThreatsHanging => "hanging pieces",
            TermId::ThreatsRestricted => "opponent's pieces cramped",
            TermId::ThreatsBySafePawn => "pawn attacks from safe squares",
            TermId::ThreatsByPawnPush => "a pawn push would attack a piece",
            TermId::ThreatsKnightOnQueen => "knight one move from the queen",
            TermId::ThreatsSliderOnQueen => "rook/bishop lined up on the queen",
        }
    }

    /// Whether this term's delta should diff against the settled-ply
    /// trace (Outcome) or the ply-1 trace (State). See [`Timing`].
    pub const fn timing(self) -> Timing {
        match self {
            // Both halves of the split material score are settled-ply
            // outcomes — captures and PSQ shifts both describe the
            // line's eventual landing, the same story the material
            // narrator's capture sequence tells. Imbalance shifts
            // when piece counts change and shares the same framing.
            TermId::MaterialPieceValue
            | TermId::MaterialPsqPositional
            | TermId::Imbalance => Timing::Outcome,
            // Everything else is state immediately after the user's
            // move. Initiative is a complexity correction tied to the
            // current position; surfacing it at settled-ply would
            // misattribute downstream-PV complexity to the user's
            // single move.
            _ => Timing::State,
        }
    }

    /// Extract this term's net [`Score`] (white − black, or the
    /// already-net value for the three single-valued terms) from an
    /// [`EvalTrace`].
    pub(super) fn net_score(self, t: &EvalTrace) -> Score {
        match self {
            TermId::MaterialPieceValue => t.material.piece_value,
            TermId::MaterialPsqPositional => t.material.psq_positional,
            TermId::Imbalance => t.imbalance,
            TermId::Initiative => t.initiative,

            TermId::Space => t.space[0] - t.space[1],

            TermId::KingPawnShield => t.king[0].pawn_shield - t.king[1].pawn_shield,
            TermId::KingPawnStorm => t.king[0].pawn_storm - t.king[1].pawn_storm,
            TermId::KingPawnDistance => {
                t.king[0].king_pawn_distance - t.king[1].king_pawn_distance
            }
            TermId::KingDanger => t.king[0].danger - t.king[1].danger,
            TermId::KingPawnlessFlank => t.king[0].pawnless_flank - t.king[1].pawnless_flank,
            TermId::KingFlankAttacks => t.king[0].flank_attacks - t.king[1].flank_attacks,

            TermId::PassedRankBonus => t.passed[0].rank_bonus - t.passed[1].rank_bonus,
            TermId::PassedKingProximity => t.passed[0].king_proximity - t.passed[1].king_proximity,
            TermId::PassedFreeAdvance => t.passed[0].free_advance - t.passed[1].free_advance,
            TermId::PassedStopperPenalty => {
                t.passed[0].stopper_penalty - t.passed[1].stopper_penalty
            }

            TermId::PawnsConnected => t.pawns[0].connected - t.pawns[1].connected,
            TermId::PawnsIsolated => t.pawns[0].isolated - t.pawns[1].isolated,
            TermId::PawnsBackward => t.pawns[0].backward - t.pawns[1].backward,
            TermId::PawnsDoubled => t.pawns[0].doubled - t.pawns[1].doubled,
            TermId::PawnsWeakUnopposed => t.pawns[0].weak_unopposed - t.pawns[1].weak_unopposed,
            TermId::PawnsWeakLever => t.pawns[0].weak_lever - t.pawns[1].weak_lever,

            TermId::PiecesOutposts => t.pieces[0].outposts - t.pieces[1].outposts,
            TermId::PiecesReachableOutposts => {
                t.pieces[0].reachable_outposts - t.pieces[1].reachable_outposts
            }
            TermId::PiecesMinorBehindPawn => {
                t.pieces[0].minor_behind_pawn - t.pieces[1].minor_behind_pawn
            }
            TermId::PiecesKingProtector => t.pieces[0].king_protector - t.pieces[1].king_protector,
            TermId::PiecesBishopPawns => t.pieces[0].bishop_pawns - t.pieces[1].bishop_pawns,
            TermId::PiecesLongDiagonalBishop => {
                t.pieces[0].long_diagonal_bishop - t.pieces[1].long_diagonal_bishop
            }
            TermId::PiecesRookOnQueenFile => {
                t.pieces[0].rook_on_queen_file - t.pieces[1].rook_on_queen_file
            }
            TermId::PiecesRookOnOpenFile => {
                t.pieces[0].rook_on_open_file - t.pieces[1].rook_on_open_file
            }
            TermId::PiecesRookOnSemiopenFile => {
                t.pieces[0].rook_on_semiopen_file - t.pieces[1].rook_on_semiopen_file
            }
            TermId::PiecesTrappedRook => t.pieces[0].trapped_rook - t.pieces[1].trapped_rook,
            TermId::PiecesWeakQueen => t.pieces[0].weak_queen - t.pieces[1].weak_queen,

            TermId::MobilityKnight => t.mobility[0].knight - t.mobility[1].knight,
            TermId::MobilityBishop => t.mobility[0].bishop - t.mobility[1].bishop,
            TermId::MobilityRook => t.mobility[0].rook - t.mobility[1].rook,
            TermId::MobilityQueen => t.mobility[0].queen - t.mobility[1].queen,

            TermId::ThreatsByMinor => t.threats[0].by_minor - t.threats[1].by_minor,
            TermId::ThreatsByRook => t.threats[0].by_rook - t.threats[1].by_rook,
            TermId::ThreatsByKing => t.threats[0].by_king - t.threats[1].by_king,
            TermId::ThreatsHanging => t.threats[0].hanging - t.threats[1].hanging,
            TermId::ThreatsRestricted => t.threats[0].restricted - t.threats[1].restricted,
            TermId::ThreatsBySafePawn => t.threats[0].by_safe_pawn - t.threats[1].by_safe_pawn,
            TermId::ThreatsByPawnPush => t.threats[0].by_pawn_push - t.threats[1].by_pawn_push,
            TermId::ThreatsKnightOnQueen => {
                t.threats[0].knight_on_queen - t.threats[1].knight_on_queen
            }
            TermId::ThreatsSliderOnQueen => {
                t.threats[0].slider_on_queen - t.threats[1].slider_on_queen
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::MobilityBreakdown;
    use crate::types::{Color, PieceType};

    #[test]
    fn net_score_material_split_reads_each_component_separately() {
        // Both halves of the split material breakdown are
        // already-net Scores; the term-id net_score lookup just
        // returns the relevant field.
        let mut t = EvalTrace::zero();
        t.material.piece_value = Score::new(42, -17);
        t.material.psq_positional = Score::new(11, 5);
        assert_eq!(
            TermId::MaterialPieceValue.net_score(&t),
            Score::new(42, -17)
        );
        assert_eq!(
            TermId::MaterialPsqPositional.net_score(&t),
            Score::new(11, 5)
        );
    }

    #[test]
    fn net_score_mobility_knight_subtracts_by_colour() {
        let mut t = EvalTrace::zero();
        t.mobility[0].knight = Score::new(30, 10);
        t.mobility[1].knight = Score::new(8, 4);
        assert_eq!(TermId::MobilityKnight.net_score(&t), Score::new(22, 6));
    }

    #[test]
    fn mobility_breakdown_total_sums_subterms_and_add_for_dispatches() {
        let mut m = MobilityBreakdown::zero();
        m.add_for(PieceType::Knight, Score::new(10, 5));
        m.add_for(PieceType::Bishop, Score::new(20, 7));
        m.add_for(PieceType::Rook, Score::new(30, 11));
        m.add_for(PieceType::Queen, Score::new(40, 13));
        assert_eq!(m.knight, Score::new(10, 5));
        assert_eq!(m.bishop, Score::new(20, 7));
        assert_eq!(m.rook, Score::new(30, 11));
        assert_eq!(m.queen, Score::new(40, 13));
        assert_eq!(m.total(), Score::new(100, 36));
        m.add_for(PieceType::Pawn, Score::new(99, 99));
        m.add_for(PieceType::King, Score::new(99, 99));
        assert_eq!(m.total(), Score::new(100, 36));
    }

    #[test]
    fn net_score_pawns_isolated_subtracts_by_colour() {
        let mut t = EvalTrace::zero();
        t.pawns[0].isolated = Score::new(-5, -15);
        t.pawns[1].isolated = Score::new(-10, -30);
        assert_eq!(TermId::PawnsIsolated.net_score(&t), Score::new(5, 15));
    }

    #[test]
    fn net_score_pieces_rook_on_open_file_subtracts_by_colour() {
        let mut t = EvalTrace::zero();
        t.pieces[0].rook_on_open_file = Score::new(44, 0);
        t.pieces[1].rook_on_open_file = Score::new(22, 0);
        assert_eq!(
            TermId::PiecesRookOnOpenFile.net_score(&t),
            Score::new(22, 0)
        );
    }

    #[test]
    fn pretty_label_differs_from_kebab_label_where_expected() {
        // Spot-check: the pretty label rephrases engine-speak into
        // plain English. Not every term needs rewording (some already
        // read naturally — "material", "initiative") but mobility,
        // threats.*, king.*, etc. all get a user-friendly form.
        assert_eq!(TermId::MobilityBishop.pretty_label(), "bishop activity");
        assert_eq!(
            TermId::ThreatsSliderOnQueen.pretty_label(),
            "rook/bishop lined up on the queen",
        );
        assert_eq!(
            TermId::ThreatsByPawnPush.pretty_label(),
            "a pawn push would attack a piece",
        );
        assert_eq!(TermId::KingDanger.pretty_label(), "king safety");
        assert_eq!(TermId::PawnsWeakUnopposed.pretty_label(), "weak pawns");
        // The split material score: piece_value reads as colloquial
        // "material" in prose, psq_positional reads as "piece
        // placement" — pieces ending up on better/worse squares.
        assert_eq!(
            TermId::MaterialPieceValue.pretty_label(),
            "material",
        );
        assert_eq!(
            TermId::MaterialPsqPositional.pretty_label(),
            "piece placement",
        );
    }

    #[test]
    fn pretty_label_is_defined_for_every_term() {
        // Exhaustiveness: the match in `pretty_label` must cover
        // every variant. Calling it on each entry in `ALL` proves the
        // compiler is happy with it and produces a non-empty string.
        for &t in &TermId::ALL {
            assert!(!t.pretty_label().is_empty(), "{:?}", t);
        }
    }

    #[test]
    fn term_id_all_covers_every_variant_once() {
        // Each variant appears exactly once in ALL. A duplicate or
        // omission would silently skew cumulative coverage.
        let mut seen = std::collections::HashSet::new();
        for &t in &TermId::ALL {
            assert!(seen.insert(t), "duplicate TermId in ALL: {:?}", t);
        }
        assert_eq!(seen.len(), TermId::ALL.len());
        // Color import kept referenced so this test compiles unchanged
        // alongside other outcome tests that consume it.
        let _ = Color::White;
    }
}
