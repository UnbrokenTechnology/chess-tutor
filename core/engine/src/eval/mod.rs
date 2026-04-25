//! Main position evaluator — the orchestrator that assembles the pawn
//! score, the material imbalance, per-piece positional terms, mobility,
//! king safety, threats, passed-pawn bonuses, space, and initiative into a
//! single tapered [`Value`] from the side-to-move's perspective.
//!
//! The structure mirrors Stockfish 11's `Evaluation<T>` class:
//!
//! 1. [`Evaluator`] owns a per-evaluation scratchpad — attack tables by
//!    colour and piece type, a doubly-attacked set, mobility areas, king
//!    rings, and king-attacker tallies.
//! 2. [`Evaluator::initialize`] (per colour) builds the pawn/king attack
//!    tables, the mobility area, and the king ring.
//! 3. The per-term helpers in [`pieces`] and sibling modules populate the
//!    shared scratchpad and return their per-term [`Score`] contribution.
//! 4. [`Evaluator::value`] combines those contributions with the
//!    incrementally-maintained material/PSQT score and the phase-blended
//!    scaling factor, then returns the signed evaluation from the side to
//!    move.
//!
//! Numerical weights are the factual parameters from Stockfish 11's
//! `evaluate.cpp`, used under the idea/expression split. All code and
//! identifiers are independently authored.
//!
//! **Status:** the per-piece-type term is fully ported. King safety,
//! threats, passed-pawn scoring, space, and initiative are stubbed to
//! `Score::ZERO` in this first cut — they'll land in follow-up sessions.
//! The lazy-eval early exit, endgame-evaluator dispatch, and Chess960
//! cornered-bishop penalty are all deliberately skipped.

pub(crate) mod initiative;
pub(crate) mod king;
pub(crate) mod passed;
pub(crate) mod pieces;
pub(crate) mod space;
pub(crate) mod threats;

pub use crate::pawns::PawnsBreakdown;
pub use king::KingBreakdown;
pub use passed::PassedBreakdown;
pub use pieces::PiecesBreakdown;
pub use threats::ThreatsBreakdown;

use crate::attacks::king_attacks;
use crate::bitboard::{Bitboard, RANK_2, RANK_3, RANK_6, RANK_7};
use crate::material::{self, MaterialEval};
use crate::pawns::{self, PawnsEval};
use crate::position::Position;
use crate::types::{Color, File, PieceType, Rank, ScaleFactor, Score, Square, Value};

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
// Tuning constants
// =========================================================================
//
// Tempo is the first-mover bonus added to the final side-to-move
// evaluation. Factual parameter from the reference.
pub const TEMPO: Value = Value(28);

/// Maximum phase weight — matches `PHASE_MIDGAME` in the reference.
const PHASE_MAX: i32 = 128;

/// Normal scale factor (no scaling applied to the endgame half).
const SCALE_NORMAL: i32 = 64;

// =========================================================================
// Per-evaluation scratchpad
// =========================================================================

/// Scratchpad held across a single call to [`evaluate`]. The per-term
/// helpers (in sibling modules) mutate the attack tables and mobility /
/// king-attacker tallies as they score pieces.
pub(crate) struct Evaluator<'a> {
    pub pos: &'a Position,
    pub material: MaterialEval,
    pub pawns: PawnsEval,

    /// Per-colour mobility area — squares this side's minor/major pieces
    /// count as "mobility" for. Set by [`initialize`].
    pub mobility_area: [Bitboard; 2],

    /// Running mobility bonus accumulated across the per-piece-type
    /// passes. Granular per piece type (knight/bishop/rook/queen);
    /// call `.total()` on each side's breakdown for the aggregate
    /// score the main evaluator summed pre-split.
    pub mobility: [MobilityBreakdown; 2],

    /// Squares attacked by `[color][piece_type]`. Indexed by
    /// `PieceType::index()` which runs 1..=6; slot 0 is unused.
    pub attacked_by: [[Bitboard; 7]; 2],

    /// Union of every piece's attacks by colour.
    pub attacked_by_all: [Bitboard; 2],

    /// Squares attacked at least twice by a given colour. Includes
    /// pawn-double-attacks and king/pawn overlap seeded in
    /// [`initialize`]; per-piece passes top up the set as they go.
    pub attacked_by_2: [Bitboard; 2],

    /// Extended king-neighbourhood used by king-safety aggregation. Set
    /// by [`initialize`].
    pub king_ring: [Bitboard; 2],

    /// Count of enemy pieces attacking a square in our king ring.
    /// Indexed by the *attacker's* colour.
    pub king_attackers_count: [i32; 2],

    /// Sum of the attacker-weight for each enemy piece hitting our king
    /// ring. Indexed by the attacker's colour.
    pub king_attackers_weight: [i32; 2],

    /// Count of squares immediately adjacent to our king attacked by the
    /// enemy (double-counted for pieces that attack more than one).
    /// Indexed by the attacker's colour.
    pub king_attacks_count: [i32; 2],
}

impl<'a> Evaluator<'a> {
    pub(crate) fn new(pos: &'a Position) -> Evaluator<'a> {
        Evaluator {
            pos,
            material: material::evaluate(pos),
            pawns: pawns::evaluate(pos),
            mobility_area: [Bitboard::EMPTY; 2],
            mobility: [MobilityBreakdown::zero(); 2],
            attacked_by: [[Bitboard::EMPTY; 7]; 2],
            attacked_by_all: [Bitboard::EMPTY; 2],
            attacked_by_2: [Bitboard::EMPTY; 2],
            king_ring: [Bitboard::EMPTY; 2],
            king_attackers_count: [0; 2],
            king_attackers_weight: [0; 2],
            king_attacks_count: [0; 2],
        }
    }

    // --------------------------------------------------------------------
    // Initialization
    // --------------------------------------------------------------------

    /// Populate per-colour king and pawn attack tables, compute the
    /// mobility area, and seed the king ring and king-attacker counts.
    /// Mirrors Stockfish's `Evaluation::initialize<Us>()`.
    pub(crate) fn initialize(&mut self, us: Color) {
        let them = !us;
        let us_idx = us.index();
        let king_sq = self.pos.king_square(us);

        // Our pawn attacks and double-attacks come from the pre-computed
        // pawn eval; seed them into the attacker tables.
        let our_pawns = self.pos.pieces_of(us, PieceType::Pawn);
        let their_pawn_attacks = self.pawns.pawn_attacks[them.index()];
        let our_king_attacks = king_attacks(king_sq);

        self.attacked_by[us_idx][PieceType::King.index()] = our_king_attacks;
        self.attacked_by[us_idx][PieceType::Pawn.index()] = self.pawns.pawn_attacks[us_idx];
        self.attacked_by_all[us_idx] = our_king_attacks | self.pawns.pawn_attacks[us_idx];

        // attackedBy2 starts with: our-pawn-double-attacks plus where our
        // king and pawn coverage overlap. Later per-piece passes OR in
        // additional double-hit squares as they discover them.
        let our_double_pawn = our_pawns.pawn_double_attacks(us);
        self.attacked_by_2[us_idx] =
            our_double_pawn | (our_king_attacks & self.pawns.pawn_attacks[us_idx]);

        // Mobility area: every square except those holding our king or
        // queen, our pinned pieces, our pawns on the first two ranks
        // from our POV, our pawns blocked by any piece directly in
        // front, or squares attacked by enemy pawns. Matches the
        // reference's formula exactly.
        let low_ranks = match us {
            Color::White => RANK_2 | RANK_3,
            Color::Black => RANK_7 | RANK_6,
        };
        let down = -crate::types::Direction::pawn_push(us).0;
        let blocked_or_low_pawns =
            our_pawns & (self.pos.occupied().shift(crate::types::Direction(down)) | low_ranks);
        let king_queen =
            self.pos.pieces_of(us, PieceType::King) | self.pos.pieces_of(us, PieceType::Queen);
        let pinned = self.pos.blockers_for_king(us);

        self.mobility_area[us_idx] =
            !(blocked_or_low_pawns | king_queen | pinned | their_pawn_attacks);

        // King ring: clamp the king square into the b2..g7 interior so a
        // corner king still has an 8-square neighbourhood, then take the
        // king-attack set of the clamped square plus the square itself.
        // Finally, subtract squares the enemy double-attacks with pawns
        // — we already know those aren't worth attacking.
        let clamped_file = king_sq
            .file()
            .index()
            .clamp(File::B.index(), File::G.index()) as u8;
        let clamped_rank = king_sq
            .rank()
            .index()
            .clamp(Rank::R2.index(), Rank::R7.index()) as u8;
        let clamped = Square::new(
            File::from_index(clamped_file).unwrap(),
            Rank::from_index(clamped_rank).unwrap(),
        );
        let mut ring = king_attacks(clamped) | crate::bitboard::square_bb(clamped);

        // Count enemy pawns that immediately pressure our king ring,
        // then remove enemy double-attack squares from the ring so the
        // king-safety aggregator doesn't double-count them with safer
        // checks further out.
        self.king_attackers_count[them.index()] = (ring & their_pawn_attacks).popcount() as i32;
        let their_pawns = self.pos.pieces_of(them, PieceType::Pawn);
        ring &= !their_pawns.pawn_double_attacks(them);
        self.king_ring[us_idx] = ring;
    }
}

// =========================================================================
// Trace: per-term breakdown surfaced to callers for teaching UI
// =========================================================================

/// Per-term breakdown of a classical evaluation, captured by
/// [`evaluate_with_trace`]. The teaching layer diffs these between
/// before-move and after-move positions to show the student which
/// strategic concepts changed.
///
/// Terms are recorded as raw [`Score`] values (packed mg + eg, both
/// 16-bit). Per-colour terms store white's raw score at index 0 and
/// black's at index 1; the final net contribution is `white - black`.
/// Single-field terms (`material`, `imbalance`, `initiative`) are
/// already net.
///
/// The `pawns` and `pieces` fields are granular per-sub-term breakdowns
/// — each carries the named chess concepts (isolated pawn, knight
/// outpost, rook on open file, etc.) a teaching UI attributes score
/// changes to. Call `.total()` on either to recover the legacy aggregate.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct EvalTrace {
    /// Incrementally-maintained PSQT score — material values plus
    /// piece-square-table positional bonus, summed over every piece on
    /// the board. Already `white - black`.
    pub material: Score,
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
            material: Score::ZERO,
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

// =========================================================================
// Public entry points
// =========================================================================

/// Evaluate `pos` and return a [`Value`] from the side-to-move's point of
/// view. This is the hot path that [`search`] calls millions of times per
/// search — it does not build a trace.
pub fn evaluate(pos: &Position) -> Value {
    evaluate_inner(pos, None)
}

/// Evaluate `pos` and additionally capture a per-term [`EvalTrace`]. Use
/// for UI layers ("why is this move good?") rather than for search's
/// per-node calls — the trace-building adds local bookkeeping, though the
/// per-term scoring itself is the same cost.
pub fn evaluate_with_trace(pos: &Position) -> (Value, EvalTrace) {
    let mut trace = EvalTrace::zero();
    let v = evaluate_inner(pos, Some(&mut trace));
    (v, trace)
}

fn evaluate_inner(pos: &Position, mut trace: Option<&mut EvalTrace>) -> Value {
    let mut e = Evaluator::new(pos);

    // If material reports a specialized endgame evaluator, trust it.
    // (Currently never fires — endgame.rs isn't ported yet.)
    if let Some(v) = e.material.endgame_value {
        let signed = if pos.side_to_move() == Color::White {
            v
        } else {
            -v
        };
        if let Some(t) = trace.as_mut() {
            t.final_value = signed;
        }
        return signed;
    }

    // Seed the running score with the incrementally-maintained PSQ score
    // (material + positional), the material imbalance polynomial, and
    // the pawn-structure score — exactly the same three "free" terms the
    // reference picks up before any work happens.
    let material = pos.psq_score();
    let imbalance = e.material.imbalance;
    let mut score = material + imbalance + e.pawns.score();

    e.initialize(Color::White);
    e.initialize(Color::Black);

    // Per-piece-type positional terms, interleaved with mobility
    // accumulation. Populate attack tables as a side effect.
    let white_pieces = pieces::evaluate(&mut e, Color::White);
    let black_pieces = pieces::evaluate(&mut e, Color::Black);
    score += white_pieces.total() - black_pieces.total();
    score += e.mobility[Color::White.index()].total() - e.mobility[Color::Black.index()].total();

    let white_king = king::evaluate(&e, Color::White);
    let black_king = king::evaluate(&e, Color::Black);
    score += white_king.total() - black_king.total();

    let white_threats = threats::evaluate(&e, Color::White);
    let black_threats = threats::evaluate(&e, Color::Black);
    score += white_threats.total() - black_threats.total();

    let white_passed = passed::evaluate(&e, Color::White);
    let black_passed = passed::evaluate(&e, Color::Black);
    score += white_passed.total() - black_passed.total();

    let white_space = space::evaluate(&e, Color::White);
    let black_space = space::evaluate(&e, Color::Black);
    score += white_space - black_space;

    let initiative_score = initiative::evaluate(&e, score);
    score += initiative_score;

    // Tapered interpolation between mg and eg scores. The eg half is
    // additionally scaled by the side-specific ScaleFactor.
    let phase = e.material.game_phase.0;
    let eg_val = score.eg().0;
    let winning_side = if eg_val > 0 {
        Color::White
    } else {
        Color::Black
    };
    let sf = scale_factor(&e, eg_val, winning_side).0;

    let mg_part = score.mg().0 * phase;
    let eg_part = score.eg().0 * (PHASE_MAX - phase) * sf / SCALE_NORMAL;
    let v = (mg_part + eg_part) / PHASE_MAX;

    let stm_signed = if pos.side_to_move() == Color::White {
        v
    } else {
        -v
    };
    let final_value = Value(stm_signed) + TEMPO;

    if let Some(t) = trace.as_mut() {
        t.material = material;
        t.imbalance = imbalance;
        t.pawns = e.pawns.breakdowns;
        t.pieces = [white_pieces, black_pieces];
        t.mobility = e.mobility;
        t.king = [white_king, black_king];
        t.threats = [white_threats, black_threats];
        t.passed = [white_passed, black_passed];
        t.space = [white_space, black_space];
        t.initiative = initiative_score;
        t.total = score;
        t.phase = phase;
        t.scale_factor = sf;
        t.tempo = TEMPO;
        t.final_value = final_value;
    }

    final_value
}

// =========================================================================
// Scale factor
// =========================================================================

fn scale_factor(e: &Evaluator<'_>, eg: i32, strong_side: Color) -> ScaleFactor {
    let base = e.material.scale_factor[strong_side.index()];
    if base != ScaleFactor::NORMAL {
        return base;
    }

    // Apply general "how likely is this to be drawn" heuristics only when
    // the material-level scale is NORMAL. Opposite-coloured bishops with
    // no other non-pawn material is the classic drawish endgame.
    let npm = e.pos.non_pawn_material_total().0;
    let bishop_mg_double = Value::BISHOP_MG.0 * 2;

    let sf_opp_bishops_only = e.pos.opposite_bishops() && npm == bishop_mg_double;
    let mut sf = if sf_opp_bishops_only {
        22
    } else {
        let pawn_count = e.pos.count(strong_side, PieceType::Pawn) as i32;
        let multiplier = if e.pos.opposite_bishops() { 2 } else { 7 };
        base.0.min(36 + multiplier * pawn_count)
    };

    // Draw down further based on how long it's been since a capture or
    // pawn move — the closer to the 50-move rule, the drawishre.
    let rule50 = e.pos.halfmove_clock() as i32;
    sf = sf.max(0).saturating_sub((rule50 - 12).max(0) / 4);

    // Silence unused var lint — eg is reserved here for the future
    // lazy-eval shortcut the reference uses before reaching this
    // function. Keeping the parameter documents the intended signature.
    let _ = eg;

    ScaleFactor(sf)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Tempo is the only thing distinguishing side-to-move ----------

    #[test]
    fn startpos_evaluates_to_tempo_plus_any_asymmetry_from_white_pov() {
        // The starting position is perfectly symmetric, so the signed
        // evaluation before tempo is zero. With white to move we get
        // +tempo; with black to move we get +tempo too (side-to-move
        // flip then add).
        let p = Position::startpos();
        let v = evaluate(&p);
        assert_eq!(v, TEMPO);
    }

    #[test]
    fn startpos_with_black_to_move_also_tempo() {
        let fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR b KQkq - 0 1";
        let p = Position::from_fen(fen).unwrap();
        let v = evaluate(&p);
        assert_eq!(v, TEMPO);
    }

    // ---- Material preponderance --------------------------------------

    #[test]
    fn extra_queen_favours_owning_side() {
        // White has an extra queen on d1 — evaluation from white's POV
        // should be clearly positive.
        let p = Position::from_fen("4k3/8/8/8/8/8/8/3QK3 w - - 0 1").unwrap();
        let v = evaluate(&p);
        assert!(
            v.0 > 500,
            "extra queen should yield a big positive eval, got {}",
            v.0
        );
    }

    #[test]
    fn extra_queen_for_black_is_negative_from_whites_turn() {
        // Black has an extra queen. With white to move we should be
        // deeply negative (minus queen material plus tempo).
        let p = Position::from_fen("3qk3/8/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        let v = evaluate(&p);
        assert!(
            v.0 < -500,
            "down a queen should yield a big negative eval, got {}",
            v.0
        );
    }

    // ---- Determinism --------------------------------------------------

    #[test]
    fn evaluate_is_pure() {
        let p = Position::from_fen(
            "r1bqkb1r/ppp2ppp/2np1n2/4p3/2B1P3/2N2N2/PPPP1PPP/R1BQK2R w KQkq - 0 5",
        )
        .unwrap();
        let a = evaluate(&p);
        let b = evaluate(&p);
        assert_eq!(a, b);
    }

    // ---- Mirror symmetry ---------------------------------------------

    #[test]
    fn mirrored_positions_evaluate_to_symmetric_values() {
        // White's side to move evaluation of position A should equal
        // black's side to move evaluation of the colour-flipped mirror,
        // up to sign. Concrete test: Italian Game mirrored.
        let white_pov = Position::from_fen(
            "r1bqkb1r/pppp1ppp/2n2n2/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 2 3",
        )
        .unwrap();
        let black_pov = Position::from_fen(
            "rnbqk2r/pppp1ppp/5n2/2b1p3/4P3/2N2N2/PPPP1PPP/R1BQKB1R b KQkq - 2 3",
        )
        .unwrap();
        let v1 = evaluate(&white_pov);
        let v2 = evaluate(&black_pov);
        assert_eq!(
            v1, v2,
            "mirrored positions should give equal side-to-move evals"
        );
    }

    // ---- EvalTrace ---------------------------------------------------

    #[test]
    fn evaluate_with_trace_final_value_matches_evaluate() {
        // The trace's `final_value` must match what `evaluate` returns
        // on the same position. Covers the regular-eval path; the
        // endgame short-circuit is exercised separately below.
        let fens = [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR b KQkq - 0 1",
            "r1bqkb1r/ppp2ppp/2np1n2/4p3/2B1P3/2N2N2/PPPP1PPP/R1BQK2R w KQkq - 0 5",
            // Imbalanced material but with pawns on both sides so the
            // KXK endgame driver doesn't hijack the eval.
            "4k3/1p6/8/8/8/8/P7/3QK3 w - - 0 1",
        ];
        for fen in fens {
            let p = Position::from_fen(fen).unwrap();
            let direct = evaluate(&p);
            let (traced, trace) = evaluate_with_trace(&p);
            assert_eq!(direct, traced, "values must agree for {}", fen);
            assert_eq!(
                trace.final_value, direct,
                "trace.final_value must match for {}",
                fen
            );
            assert_eq!(trace.tempo, TEMPO);
        }
    }

    #[test]
    fn evaluate_with_trace_endgame_path_reports_final_value() {
        // KXK endgame short-circuits the classical breakdown. The
        // trace still ends up with `final_value` == `evaluate(pos)`;
        // per-term fields are left at zero by design (the eval didn't
        // come from classical terms).
        let p = Position::from_fen("4k3/8/8/8/8/8/8/3QK3 w - - 0 1").unwrap();
        let direct = evaluate(&p);
        let (traced, trace) = evaluate_with_trace(&p);
        assert_eq!(direct, traced);
        assert_eq!(trace.final_value, direct);
    }

    #[test]
    fn trace_material_captures_psq_score() {
        // `trace.material` is the PSQT score (material + positional),
        // pre-taper. For startpos this is exactly zero by symmetry.
        let p = Position::startpos();
        let (_, trace) = evaluate_with_trace(&p);
        assert_eq!(trace.material, Score::ZERO);
        assert_eq!(trace.material, p.psq_score());
    }

    #[test]
    fn trace_material_is_nonzero_when_material_is_imbalanced() {
        // Extra white queen — PSQT should skew heavily positive.
        // Include pawns so the KXK endgame driver doesn't short-circuit
        // past the classical trace.
        let p = Position::from_fen("4k3/1p6/8/8/8/8/P7/3QK3 w - - 0 1").unwrap();
        let (_, trace) = evaluate_with_trace(&p);
        assert_ne!(trace.material, Score::ZERO);
        // Mg component of an extra queen is strongly positive for white.
        assert!(trace.material.mg().0 > 500);
    }

    #[test]
    fn trace_has_phase_and_scale_factor_in_valid_ranges() {
        let p = Position::from_fen(
            "r1bqkb1r/ppp2ppp/2np1n2/4p3/2B1P3/2N2N2/PPPP1PPP/R1BQK2R w KQkq - 0 5",
        )
        .unwrap();
        let (_, trace) = evaluate_with_trace(&p);
        assert!(
            (0..=128).contains(&trace.phase),
            "phase out of range: {}",
            trace.phase
        );
        assert!(
            trace.scale_factor > 0,
            "scale factor must be positive, got {}",
            trace.scale_factor
        );
    }
}
