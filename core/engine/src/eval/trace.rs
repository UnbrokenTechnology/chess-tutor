//! Decomposed evaluation breakdowns surfaced to teaching UIs.
//!
//! [`EvalTrace`] is the structured per-term snapshot [`evaluate_with_trace`]
//! captures; [`MaterialBreakdown`] and [`MobilityBreakdown`] are the two
//! breakdown sub-types defined alongside it (the other per-term breakdowns —
//! pawns, pieces, king, threats, passed — live in their own term modules and
//! are re-exported from [`super`]).
//!
//! [`evaluate_with_trace`]: super::evaluate_with_trace

use super::king::KingBreakdown;
use super::passed::PassedBreakdown;
use super::pieces::PiecesBreakdown;
use super::threats::ThreatsBreakdown;
use super::SCALE_NORMAL;
use crate::pawns::PawnsBreakdown;
use crate::types::{Color, PieceType, Score, Value};

// =========================================================================
// Mobility breakdown
// =========================================================================

/// Per-piece-type mobility bonus. Mobility fires for knight, bishop,
/// rook, and queen only (pawns and kings are scored via other terms),
/// so those are the four fields tracked here. The sum equals the
/// aggregate mobility score this colour contributes — see
/// [`total`](MobilityBreakdown::total).
///
/// Mirrors the Phase 0 [`PawnsBreakdown`] / [`PiecesBreakdown`]
/// pattern: the sub-terms live here, the top-level evaluator reads
/// `.total()`, and the teaching pipeline ([`crate::analysis`])
/// surfaces the individual fields as named [`crate::analysis::TermId`]
/// variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MobilityBreakdown {
    pub knight: Score,
    pub bishop: Score,
    pub rook: Score,
    pub queen: Score,
}

impl MobilityBreakdown {
    /// An all-zero breakdown.
    pub const fn zero() -> MobilityBreakdown {
        MobilityBreakdown {
            knight: Score::ZERO,
            bishop: Score::ZERO,
            rook: Score::ZERO,
            queen: Score::ZERO,
        }
    }

    /// Sum of every sub-term. Equal to the aggregate mobility score
    /// this colour contributes (what the pre-split `mobility: [Score; 2]`
    /// field held).
    pub fn total(&self) -> Score {
        self.knight + self.bishop + self.rook + self.queen
    }

    /// Accumulate `bonus` into the slot corresponding to `pt`. No-op
    /// for piece types outside `{Knight, Bishop, Rook, Queen}` — the
    /// mobility evaluator never calls this with a pawn or king, but
    /// the match is total so future callers can pass through safely.
    pub(crate) fn add_for(&mut self, pt: PieceType, bonus: Score) {
        match pt {
            PieceType::Knight => self.knight += bonus,
            PieceType::Bishop => self.bishop += bonus,
            PieceType::Rook => self.rook += bonus,
            PieceType::Queen => self.queen += bonus,
            _ => {}
        }
    }
}

// =========================================================================
// Trace: per-term breakdown surfaced to callers for teaching UI
// =========================================================================

/// Decomposed material score: raw piece values (the part that changes
/// only on captures / promotions) and the piece-square-table
/// positional bonus (the part that changes on every piece move).
/// Stockfish's PSQT tables bake both into one number per
/// piece+square; the teaching layer wants them as separate signals so
/// "you lost a pawn" doesn't get attributed to a non-capture move
/// that happened to land on a slightly worse PSQ square.
///
/// Both fields are already `white - black` net. `total()` returns
/// the aggregate, equal to `pos.psq_score()`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct MaterialBreakdown {
    /// Sum of raw piece values: `Σ count(pt) × piece_value(pt)` over
    /// `pt ∈ {Pawn, Knight, Bishop, Rook, Queen}`, white minus
    /// black. Kings have no piece value. Changes only on captures
    /// and promotions.
    pub piece_value: Score,
    /// PSQT positional bonus: `psq_score - piece_value`. Captures the
    /// piece-square table's positional contribution (knight on c3 vs
    /// knight on a1 etc.), independent of piece counts. Changes on
    /// every piece move.
    pub psq_positional: Score,
}

impl MaterialBreakdown {
    pub const fn zero() -> MaterialBreakdown {
        MaterialBreakdown {
            piece_value: Score::ZERO,
            psq_positional: Score::ZERO,
        }
    }

    /// Aggregate matching `pos.psq_score()` — the pre-split
    /// `EvalTrace::material` value.
    pub fn total(&self) -> Score {
        self.piece_value + self.psq_positional
    }
}

/// Per-term breakdown of a classical evaluation, captured by
/// [`evaluate_with_trace`]. The teaching layer diffs these between
/// before-move and after-move positions to show the student which
/// strategic concepts changed.
///
/// Terms are recorded as raw [`Score`] values (packed mg + eg, both
/// 16-bit). Per-colour terms store white's raw score at index 0 and
/// black's at index 1; the final net contribution is `white - black`.
/// Single-field terms (`imbalance`, `initiative`) are already net.
///
/// The `pawns` and `pieces` fields are granular per-sub-term breakdowns
/// — each carries the named chess concepts (isolated pawn, knight
/// outpost, rook on open file, etc.) a teaching UI attributes score
/// changes to. Call `.total()` on either to recover the legacy aggregate.
///
/// [`evaluate_with_trace`]: super::evaluate_with_trace
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct EvalTrace {
    /// Material score split into the two distinct chess concepts the
    /// PSQT tables conflate: raw piece values (changes only on
    /// captures and promotions — colloquial "material") and the
    /// positional piece-square bonus (changes on every piece move).
    /// Stockfish lumps these into one PSQ table per piece+square, but
    /// for teaching narration the split lets a "Material" narrator
    /// fire on captures while quiet PSQ shifts surface separately.
    /// Use [`MaterialBreakdown::total`] for the aggregate matching
    /// `pos.psq_score()`.
    pub material: MaterialBreakdown,
    /// Non-linear material-imbalance polynomial. Already `white - black`.
    pub imbalance: Score,
    /// Granular per-sub-term pawn-structure breakdown per colour:
    /// connected, isolated, backward, doubled, weak-unopposed, weak-lever.
    /// Passed-pawn scoring lives in its own top-level `passed` field.
    pub pawns: [PawnsBreakdown; 2],
    /// Granular per-sub-term per-piece positional breakdown per colour:
    /// outposts, reachable outposts, minor-behind-pawn, king-protector,
    /// bishop-pawns, long-diagonal bishop, rook-on-queen-file,
    /// rook-on-(open|semiopen)-file, trapped-rook, weak-queen. Mobility
    /// lives in its own top-level `mobility` field.
    pub pieces: [PiecesBreakdown; 2],
    /// Mobility bonus accumulated across every minor/major piece of
    /// each colour, split by piece type (knight / bishop / rook /
    /// queen). Use [`MobilityBreakdown::total`] for the aggregate.
    pub mobility: [MobilityBreakdown; 2],
    /// King safety per colour, split into four sub-terms: pawn
    /// shelter, the kingDanger quadratic aggregator, the pawnless-flank
    /// penalty, and linear flank-attack pressure. Use
    /// [`KingBreakdown::total`] for the aggregate.
    pub king: [KingBreakdown; 2],
    /// Threats on enemy pieces, granular per sub-term (hanging,
    /// restricted, by-minor, by-rook, by-king, by-safe-pawn,
    /// by-pawn-push, knight-on-queen, slider-on-queen). Use
    /// [`ThreatsBreakdown::total`] for the aggregate.
    pub threats: [ThreatsBreakdown; 2],
    /// Passed-pawn scoring per colour, split into four sub-terms:
    /// rank bonus, king proximity, free advance, and file-fold
    /// stopper penalty. Use [`PassedBreakdown::total`] for the
    /// aggregate.
    pub passed: [PassedBreakdown; 2],
    /// Middlegame space score (safe squares behind our pawns).
    pub space: [Score; 2],
    /// Complexity / initiative correction applied to the running total.
    /// Already net.
    pub initiative: Score,

    /// Net pre-taper [`Score`]: the sum of every term with per-colour
    /// contributions resolved via `white - black`.
    pub total: Score,
    /// Game phase on the `[0, 128]` scale — 128 = pure midgame, 0 =
    /// pure endgame.
    pub phase: i32,
    /// Endgame scale factor (64 = normal). Multiplies the endgame
    /// component when tapering.
    pub scale_factor: i32,
    /// Tempo bonus added to the final side-to-move value.
    pub tempo: Value,
    /// Final, tapered, scaled, side-to-move-signed [`Value`] after
    /// adding tempo. Matches `evaluate(pos)`'s return value exactly.
    pub final_value: Value,
}

impl EvalTrace {
    /// An all-zero trace, suitable as a before-build scratchpad.
    pub const fn zero() -> EvalTrace {
        EvalTrace {
            material: MaterialBreakdown::zero(),
            imbalance: Score::ZERO,
            pawns: [PawnsBreakdown::zero(); 2],
            pieces: [PiecesBreakdown::zero(); 2],
            mobility: [MobilityBreakdown::zero(); 2],
            king: [KingBreakdown::zero(); 2],
            threats: [ThreatsBreakdown::zero(); 2],
            passed: [PassedBreakdown::zero(); 2],
            space: [Score::ZERO; 2],
            initiative: Score::ZERO,
            total: Score::ZERO,
            phase: 0,
            scale_factor: SCALE_NORMAL,
            tempo: Value::ZERO,
            final_value: Value::ZERO,
        }
    }

    /// Aggregate pawn-structure score per colour — sum of every
    /// sub-term on the granular [`PawnsBreakdown`]. Matches what the
    /// pre-Phase-0 `pawns: [Score; 2]` field held.
    pub fn pawns_total(&self, color: Color) -> Score {
        self.pawns[color.index()].total()
    }

    /// Aggregate per-piece positional score per colour — sum of every
    /// sub-term on the granular [`PiecesBreakdown`]. Matches what the
    /// pre-Phase-0 `pieces: [Score; 2]` field held.
    pub fn pieces_total(&self, color: Color) -> Score {
        self.pieces[color.index()].total()
    }

    /// Aggregate mobility score per colour — sum of every piece-type
    /// slot on the granular [`MobilityBreakdown`]. Matches what the
    /// pre-split `mobility: [Score; 2]` field held.
    pub fn mobility_total(&self, color: Color) -> Score {
        self.mobility[color.index()].total()
    }

    /// Aggregate king-safety score per colour — sum of every sub-term
    /// on the granular [`KingBreakdown`]. Matches what the pre-split
    /// `king: [Score; 2]` field held.
    pub fn king_total(&self, color: Color) -> Score {
        self.king[color.index()].total()
    }

    /// Aggregate passed-pawn score per colour — sum of every sub-term
    /// on the granular [`PassedBreakdown`]. Matches what the pre-split
    /// `passed: [Score; 2]` field held.
    pub fn passed_total(&self, color: Color) -> Score {
        self.passed[color.index()].total()
    }

    /// Aggregate threats score per colour — sum of every sub-term
    /// on the granular [`ThreatsBreakdown`]. Matches what the
    /// pre-split `threats: [Score; 2]` field held.
    pub fn threats_total(&self, color: Color) -> Score {
        self.threats[color.index()].total()
    }

    /// Return `final_value` normalised to white's POV with the tempo
    /// bonus stripped off. Useful for comparing scores across plies of
    /// a principal variation — [`final_value`] is side-to-move-signed
    /// and includes the `+TEMPO` bonus, both of which flip every ply,
    /// introducing a ~2×TEMPO sawtooth even when the evaluation is
    /// otherwise steady.
    ///
    /// `stm_at_eval` is the side to move at the position that produced
    /// this trace (so after playing a move, pass the *opponent's* color).
    pub fn white_pov_value(&self, stm_at_eval: Color) -> Value {
        let stm_unsigned = self.final_value.0 - self.tempo.0;
        let signed = if stm_at_eval == Color::White {
            stm_unsigned
        } else {
            -stm_unsigned
        };
        Value(signed)
    }
}
