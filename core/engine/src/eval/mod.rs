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
//! 4. The orchestrator [`core::evaluate_inner`] combines those contributions
//!    with the incrementally-maintained material/PSQT score and the
//!    phase-blended scaling factor, then returns the signed evaluation from
//!    the side to move. The public [`evaluate`] family are thin wrappers
//!    over it; the per-term breakdown types live in [`trace`].
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

mod core;
mod scale;
mod trace;

pub(crate) mod initiative;
pub(crate) mod king;
pub(crate) mod passed;
pub(crate) mod pieces;
pub(crate) mod space;
pub(crate) mod threats;

#[cfg(test)]
mod tests;

pub use crate::pawns::PawnsBreakdown;
pub use king::KingBreakdown;
pub use passed::PassedBreakdown;
pub use pieces::PiecesBreakdown;
pub use threats::ThreatsBreakdown;
pub use trace::{EvalTrace, MaterialBreakdown, MobilityBreakdown};

use crate::attacks::king_attacks;
use crate::bitboard::{Bitboard, RANK_2, RANK_3, RANK_6, RANK_7};
use crate::endgame::EndgameSkill;
use crate::material::{self, MaterialEval};
use crate::opponent::EvalMask;
use crate::pawns::{self, PawnsEval};
use crate::position::Position;
use crate::types::{Color, File, PieceType, Rank, Score, Square, Value};

// =========================================================================
// Tuning constants
// =========================================================================
//
// Tempo is the first-mover bonus added to the final side-to-move
// evaluation. Factual parameter from the reference.
pub const TEMPO: Value = Value(28);

/// Maximum phase weight — matches `PHASE_MIDGAME` in the reference.
pub(super) const PHASE_MAX: i32 = 128;

/// Normal scale factor (no scaling applied to the endgame half).
pub(super) const SCALE_NORMAL: i32 = 64;

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

    /// Opt-in per-piece mobility tracker. `None` in the hot search
    /// path (default) — pieces::evaluate's mobility loop bypasses
    /// the bookkeeping. `Some(vec)` when callers want per-piece
    /// granularity (analysis snapshots used by the retrospective
    /// "which bishop's activity improved?" highlight). Entries are
    /// described by [`PerPieceMobilityRecord`].
    pub per_piece_mobility: Option<Vec<PerPieceMobilityRecord>>,

    /// Opt-in per-piece reachable-outpost tracker. Same opt-in
    /// discipline as [`Self::per_piece_mobility`]: `None` on the hot
    /// search path, `Some(vec)` on analysis snapshots that want to draw
    /// "this knight has a route to that outpost square." Each entry is
    /// `(knight_square, color, outpost_square)`; one entry per reachable
    /// outpost square (a knight may have several).
    pub per_piece_reachable_outpost: Option<Vec<(Square, Color, Square)>>,

    /// Opt-in per-piece minor-behind-pawn tracker. Same opt-in discipline
    /// as [`Self::per_piece_mobility`]. Each entry is `(minor_square,
    /// color, covering_pawn_square)` — the minor sitting directly behind a
    /// pawn and the pawn shielding it — so the retrospective can highlight
    /// *which* minor gained / lost pawn cover.
    pub per_piece_minor_behind_pawn: Option<Vec<(Square, Color, Square)>>,

    /// Opt-in king-ring attacker tracker. Same opt-in discipline as
    /// [`Self::per_piece_mobility`]. Each entry is `(attacker_square,
    /// threatened_king_color, attacked_ring_squares)` — a piece bearing on
    /// the *enemy* king ring, paired with the colour of the king under fire
    /// and the subset of that king's ring the piece actually attacks — so
    /// the retrospective can draw an arrow from each attacker to the ring
    /// square it bears on (the bare count doesn't say where the pressure is,
    /// and a slider rarely attacks the king square itself).
    pub per_piece_king_attacker: Option<Vec<(Square, Color, Bitboard)>>,
}

/// One entry of the [`Evaluator::per_piece_mobility`] tracker —
/// `(square, color, piece_type, score, mobility_squares)`. The
/// `mobility_squares` bitboard is `attacks & mobility_area`, the
/// precise set of squares that counted toward the mobility popcount;
/// the retrospective UI diffs it pre vs post to highlight which
/// squares a piece newly attacks (or no longer attacks) when its
/// activity changes.
pub type PerPieceMobilityRecord = (Square, Color, PieceType, Score, Bitboard);

impl<'a> Evaluator<'a> {
    /// Build an evaluator that computes pawn structure on demand. Used
    /// by analytical / UI callers that don't share a long-lived pawn
    /// cache.
    pub(crate) fn new(pos: &'a Position) -> Evaluator<'a> {
        Self::new_with_pawns(pos, pawns::evaluate(pos), EndgameSkill::Full)
    }

    /// Build an evaluator from a precomputed [`PawnsEval`]. The hot
    /// search path uses this so a per-engine [`pawns::Table`] can short-
    /// circuit pawn evaluation across sibling and child nodes.
    pub(crate) fn new_with_pawns(
        pos: &'a Position,
        pawns: PawnsEval,
        eg_skill: EndgameSkill,
    ) -> Evaluator<'a> {
        Evaluator {
            pos,
            material: material::evaluate_with_skill(pos, eg_skill),
            pawns,
            mobility_area: [Bitboard::EMPTY; 2],
            mobility: [MobilityBreakdown::zero(); 2],
            attacked_by: [[Bitboard::EMPTY; 7]; 2],
            attacked_by_all: [Bitboard::EMPTY; 2],
            attacked_by_2: [Bitboard::EMPTY; 2],
            king_ring: [Bitboard::EMPTY; 2],
            king_attackers_count: [0; 2],
            king_attackers_weight: [0; 2],
            king_attacks_count: [0; 2],
            per_piece_mobility: None,
            per_piece_reachable_outpost: None,
            per_piece_minor_behind_pawn: None,
            per_piece_king_attacker: None,
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
        // Finally, subtract the squares our own pawns doubly defend —
        // those are safe and don't belong in the king-danger zone.
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

        // Count enemy pawns that immediately pressure our king ring
        // (computed on the full ring, before the removal below — matches
        // SF11's ordering at evaluate.cpp:243 then :247), then remove
        // from the ring the squares defended by two of OUR OWN pawns:
        // those are safe, so the king-safety aggregator shouldn't treat
        // them as part of the danger zone. Mirrors SF11 evaluate.cpp:247
        // (`kingRing[Us] &= ~dblAttackByPawn`, where dblAttackByPawn is
        // `pawn_double_attacks_bb<Us>` over our own pawns — see :223).
        self.king_attackers_count[them.index()] = (ring & their_pawn_attacks).popcount() as i32;
        ring &= !our_double_pawn;
        self.king_ring[us_idx] = ring;
    }
}

// =========================================================================
// Public entry points
// =========================================================================

/// Evaluate `pos` and return a [`Value`] from the side-to-move's point of
/// view. This form does not use a pawn cache — analytical / UI callers
/// should use this; the search hot path goes through
/// [`evaluate_with_pawn_cache`]. Always runs the unbiased eval (no
/// [`EvalMask`]); the bot's mask is applied only inside `Search`.
pub fn evaluate(pos: &Position) -> Value {
    core::evaluate_inner(
        pos,
        pawns::evaluate(pos),
        EvalMask::EMPTY,
        EndgameSkill::Full,
        None,
    )
}

/// Evaluate `pos` using the supplied pawn-structure cache. The hot path
/// in [`crate::search`] calls this — pawn structure rarely changes
/// between sibling and child nodes, and probing the cache avoids
/// recomputing the most expensive single eval term (~20% of search time
/// in profiling).
///
/// `mask` lets the play engine zero out named categories (e.g.
/// [`EvalCategory::KingSafety`]) so the bot plays as if blind to that
/// concept. Pass [`EvalMask::EMPTY`] for unbiased eval — that is the
/// hot path and the gating branches fold under branch prediction.
///
/// [`EvalCategory::KingSafety`]: crate::opponent::EvalCategory::KingSafety
pub fn evaluate_with_pawn_cache(
    pos: &Position,
    pawn_cache: &mut pawns::Table,
    mask: EvalMask,
    eg_skill: EndgameSkill,
) -> Value {
    core::evaluate_inner(pos, pawn_cache.evaluate(pos), mask, eg_skill, None)
}

/// Evaluate `pos` and additionally capture a per-term [`EvalTrace`]. Use
/// for UI layers ("why is this move good?") rather than for search's
/// per-node calls — the trace-building adds local bookkeeping, though the
/// per-term scoring itself is the same cost. Always runs the unbiased
/// eval — the trace must reflect true best play so retrospective verdicts
/// can hold the student to it.
pub fn evaluate_with_trace(pos: &Position) -> (Value, EvalTrace) {
    let mut trace = EvalTrace::zero();
    let v = core::evaluate_inner(
        pos,
        pawns::evaluate(pos),
        EvalMask::EMPTY,
        EndgameSkill::Full,
        Some(&mut trace),
    );
    (v, trace)
}
