//! Pawn-structure evaluation: scores and bitboard artifacts that `evaluate.rs`
//! consumes when piecing together the full position evaluation.
//!
//! What this module produces (per colour) via [`evaluate`]:
//!
//! - A `Score` that captures everything pawns-only can see: bonuses for
//!   connected/phalanx pawns, and penalties for isolated, backward, doubled,
//!   and weak-lever pawns.
//! - A `pawn_attacks` bitboard — every square attacked by one of this colour's
//!   pawns.
//! - A `pawn_attacks_span` bitboard — pawn_attacks extended with the attack
//!   span of every non-backward, non-blocked pawn, projected all the way to
//!   promotion. Used by the main evaluator to identify stable outposts and
//!   to score space.
//! - A `passed_pawns` bitboard — pawns whose promotion is no longer
//!   preventable by opposing pawns alone. The per-passer score lives in the
//!   main evaluator because it needs king and piece-attack information we
//!   don't have here.
//!
//! King safety — pawn-shelter strength in front of the king, and the pawn
//! storm coming from the opposite side — is a separate [`king_safety`]
//! function in this module. It's kept alongside pawn evaluation because
//! both are pure pawn/king geometry; in the reference the two are members of
//! the same `Pawns::Entry`.
//!
//! Numerical weight tables (`CONNECTED`, `SHELTER_STRENGTH`,
//! `UNBLOCKED_STORM`, etc.) are the factual parameters from Stockfish 11's
//! `pawns.cpp`, used under the idea/expression split. All code and
//! identifiers are independently authored.
//!
//! Caching: structure-only [`PawnsEval`] results are cached by
//! `pos.pawn_key()` in a [`Table`]. Measured hit rate is 87-89 % in
//! deep middlegame search and ~100 % in pawn-only endgames — pawn
//! structure changes only on pawn moves and captures, while every
//! search node calls into pawn evaluation. The cached form excludes
//! `breakdowns` (only the teaching layer reads those, and it goes
//! through [`evaluate`] directly without the cache), which keeps the
//! per-entry footprint at 64 bytes — one cache line per probe.
//!
//! Shelter (`king_safety` below) is not cached here because its key
//! also needs the king square and castling rights. A separate
//! king-safety cache is on the perf docket.
//!
//! **>500 LOC by design (W2 exception).** This file deliberately exceeds
//! the workspace's ≤500-LOC-per-source-file guideline: pawn structure is
//! one cohesive evaluation term, and splitting it across files would
//! fragment a single algorithm whose sub-terms (connected / isolated /
//! backward / doubled / weak-lever / shelter / storm) all share the same
//! per-colour bitboard scaffolding. Tests live in the sibling
//! `pawns_tests.rs`. (User decision, 2026-05-26.)

use crate::attacks::{king_attacks, pawn_attacks_from, square_distance};
use crate::bitboard::{
    adjacent_files_bb, file_bb, forward_file_bb, forward_ranks_bb, passed_pawn_span,
    pawn_attack_span, rank_bb, square_bb, Bitboard,
};
use crate::position::Position;
use crate::types::{CastlingRights, Color, Direction, File, PieceType, Rank, Score, Square};

// =========================================================================
// Weight tables
// =========================================================================
//
// Factual numerical parameters from Stockfish 11's `pawns.cpp`. Names and
// layout are independently chosen; values are carried over verbatim.

/// Penalty for a backward pawn.
const BACKWARD: Score = Score::new(9, 24);

/// Penalty when an enemy storm pawn is blocked directly by our shelter pawn
/// on the third rank — applied in place of the normal storm term.
const BLOCKED_STORM: Score = Score::new(82, 82);

/// Penalty for a doubled pawn that has no support from its own side.
const DOUBLED: Score = Score::new(11, 56);

/// Penalty for an isolated pawn.
const ISOLATED: Score = Score::new(5, 15);

/// Additional penalty for a pawn attacked by more than one enemy pawn with
/// no friendly support to defend it.
const WEAK_LEVER: Score = Score::new(0, 56);

/// Additional penalty for isolated / backward pawns that are also
/// unopposed (no enemy pawn on the same file ahead) — those are prime
/// attack targets.
const WEAK_UNOPPOSED: Score = Score::new(13, 27);

/// Per-relative-rank bonus for a connected pawn (one with a same-colour
/// neighbour on an adjacent file, either in phalanx on the same rank or
/// supporting it from directly behind). Indexed by [`Rank::index()`] from
/// the pawn's own perspective. Ranks 1 and 8 never host pawns, so those
/// slots are zero placeholders.
const CONNECTED: [i32; 8] = [0, 7, 8, 12, 29, 48, 86, 0];

/// Shelter strength for the king's own pawns, indexed
/// `[distance_to_queenside_half][our_frontmost_pawn_rank_from_our_pov]`.
///
/// The file index runs 0..=3: 0 = a/h, 1 = b/g, 2 = c/f, 3 = d/e (the file
/// is always folded to the queenside half for symmetry). The rank index is
/// the relative rank of our frontmost pawn on that file, or 0 when we have
/// no shelter pawn on that file.
const SHELTER_STRENGTH: [[i32; 8]; 4] = [
    [-6, 81, 93, 58, 39, 18, 25, 0],
    [-43, 61, 35, -49, -29, -11, -63, 0],
    [-10, 75, 23, -2, 32, 3, -45, 0],
    [-39, -13, -29, -52, -48, -67, -166, 0],
];

/// Storm danger from an unblocked enemy pawn advancing toward our king,
/// indexed `[distance_to_queenside_half][their_frontmost_pawn_rank_from_our_pov]`.
/// The rank index is zero when the enemy has no pawn on that file within the
/// half of the board closer to our king.
const UNBLOCKED_STORM: [[i32; 8]; 4] = [
    [85, -289, -166, 97, 50, 45, 50, 0],
    [46, -25, 122, 45, 37, -10, 20, 0],
    [-6, 51, 168, 34, -2, -22, -14, 0],
    [-15, -11, 101, 4, 11, -15, -29, 0],
];

/// Base shelter/storm bonus added before any per-file contributions.
const SHELTER_BASE: Score = Score::new(5, 5);

/// Per-square mg penalty accrued for each king-to-nearest-own-pawn step in
/// the endgame. Reference: `- make_score(0, 16 * minPawnDist)`.
const KING_TO_NEAREST_PAWN_PENALTY_EG: i32 = 16;

// =========================================================================
// Public output shape
// =========================================================================

/// Per-colour breakdown of the pawn-structure score into its named
/// sub-terms. The teaching layer consumes this directly — each field maps
/// to a chess concept a student can read about (isolated pawn, connected
/// pawn, etc.). Values are cumulative across all of this colour's pawns;
/// bonuses are positive, penalties are negative.
///
/// The sum of all fields equals the aggregate pawn-structure score this
/// colour contributes — see [`total`](PawnsBreakdown::total).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PawnsBreakdown {
    /// Bonus for pawns that have a same-colour neighbour on an adjacent
    /// file — either in phalanx (same rank) or supporting from directly
    /// behind. Scaled by rank and by whether the pawn is opposed.
    pub connected: Score,
    /// Penalty for a pawn with no same-colour neighbour on either
    /// adjacent file.
    pub isolated: Score,
    /// Penalty for a pawn that cannot advance safely: no same-colour
    /// neighbour that could defend the push square, and the push square
    /// is either blocked or under enemy-pawn lever attack.
    pub backward: Score,
    /// Penalty for a pawn stacked behind a same-colour pawn on the same
    /// file, with no friendly support from an adjacent file.
    pub doubled: Score,
    /// Extra penalty applied to isolated or backward pawns that are also
    /// unopposed (no enemy pawn blocks the file ahead of them) — a prime
    /// attack target.
    pub weak_unopposed: Score,
    /// Penalty for a pawn attacked by more than one enemy pawn with no
    /// same-colour pawn supporting it from behind.
    pub weak_lever: Score,
}

impl PawnsBreakdown {
    /// An all-zero breakdown.
    pub const fn zero() -> PawnsBreakdown {
        PawnsBreakdown {
            connected: Score::ZERO,
            isolated: Score::ZERO,
            backward: Score::ZERO,
            doubled: Score::ZERO,
            weak_unopposed: Score::ZERO,
            weak_lever: Score::ZERO,
        }
    }

    /// Sum of every sub-term. Equal to the aggregate pawn-structure score
    /// this colour contributes before king safety.
    pub fn total(&self) -> Score {
        self.connected
            + self.isolated
            + self.backward
            + self.doubled
            + self.weak_unopposed
            + self.weak_lever
    }
}

/// The artifacts of pawn-structure evaluation that other evaluators need.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PawnsEval {
    /// Pawn-only score per colour. White is index 0, black index 1.
    /// Always equal to `breakdowns[c].total()`; kept as a cached field so
    /// hot callers don't re-sum the sub-terms on every access.
    pub scores: [Score; 2],
    /// Granular per-sub-term pawn-structure scores per colour. The
    /// teaching layer reads these to attribute score changes to specific
    /// chess concepts.
    pub breakdowns: [PawnsBreakdown; 2],
    /// Bitboard of passed pawns per colour. These are detected here and
    /// scored later when the full attack picture is available.
    pub passed_pawns: [Bitboard; 2],
    /// Every square attacked by any pawn of the given colour.
    pub pawn_attacks: [Bitboard; 2],
    /// Pawn attacks extended with the `pawn_attack_span` of every
    /// non-backward, non-blocked pawn — the set of squares this colour's
    /// pawns might plausibly attack if we keep pushing.
    pub pawn_attacks_span: [Bitboard; 2],
}

impl PawnsEval {
    /// Signed pawn score from white's perspective: `scores[white] − scores[black]`.
    pub fn score(&self) -> Score {
        self.scores[Color::White.index()] - self.scores[Color::Black.index()]
    }
}

// =========================================================================
// Cache
// =========================================================================

/// log2 of the [`Table`] slot count. 14 → 16384 slots × 64 B/entry =
/// 1 MB total, which fits L2 on most CPUs. Measured 87-89 % hit rate
/// in deep middlegame search; an earlier experiment at 16 bits (4 MB,
/// L3-resident) bumped hit rate to ~92 % but produced no measurable
/// NPS improvement — the colder cache offset the fewer misses. Stay
/// at 14 bits unless we revisit the eval pipeline to reduce the
/// per-miss cost (which is what would actually be paying for the
/// extra slots if it were a real bottleneck).
const TABLE_BITS: u32 = 14;
const TABLE_SLOTS: usize = 1 << TABLE_BITS;
const TABLE_MASK: u64 = (TABLE_SLOTS - 1) as u64;

/// The subset of [`PawnsEval`] that the search hot path actually reads:
/// `scores` (used directly in eval), and the three bitboards that other
/// eval terms (mobility, threats, passed, space, king safety) consume.
/// Notably **excludes** `breakdowns` — those are only read by the
/// teaching layer via [`evaluate_with_trace`], which does not go through
/// the cache. By dropping breakdowns from the cached form we shrink the
/// per-entry footprint from 104 → 56 bytes, and combined with the
/// 8-byte key the entry lands at exactly 64 bytes (one cache line).
#[derive(Clone, Copy)]
struct CachedPawnsEval {
    scores: [Score; 2],
    passed_pawns: [Bitboard; 2],
    pawn_attacks: [Bitboard; 2],
    pawn_attacks_span: [Bitboard; 2],
}

impl CachedPawnsEval {
    const EMPTY: Self = Self {
        scores: [Score::ZERO; 2],
        passed_pawns: [Bitboard::EMPTY; 2],
        pawn_attacks: [Bitboard::EMPTY; 2],
        pawn_attacks_span: [Bitboard::EMPTY; 2],
    };

    /// Compute a cached form by dropping the breakdowns from a full
    /// `PawnsEval`. The breakdowns are still computed during the miss
    /// path (because they fall out of `evaluate_color`'s work for free)
    /// but they're not retained in the cache.
    #[inline(always)]
    fn from_full(eval: &PawnsEval) -> Self {
        Self {
            scores: eval.scores,
            passed_pawns: eval.passed_pawns,
            pawn_attacks: eval.pawn_attacks,
            pawn_attacks_span: eval.pawn_attacks_span,
        }
    }

    /// Re-expand to a full `PawnsEval` for caller compatibility. The
    /// breakdowns slot is filled with zeros — search-path callers never
    /// read it (the teaching layer goes through a different code path
    /// that computes the real breakdowns fresh).
    #[inline(always)]
    fn to_full(self) -> PawnsEval {
        PawnsEval {
            scores: self.scores,
            breakdowns: [PawnsBreakdown::zero(); 2],
            passed_pawns: self.passed_pawns,
            pawn_attacks: self.pawn_attacks,
            pawn_attacks_span: self.pawn_attacks_span,
        }
    }
}

/// 64-byte cache-line-aligned table entry. Holds the discriminating
/// Zobrist key (`0` means "empty slot" — real pawn keys are never zero)
/// and the cached evaluation. Sized so a probe touches exactly one
/// cache line and never straddles two.
#[derive(Clone, Copy)]
#[repr(C, align(64))]
struct TableEntry {
    key: u64,
    eval: CachedPawnsEval,
}

impl TableEntry {
    const EMPTY: Self = Self {
        key: 0,
        eval: CachedPawnsEval::EMPTY,
    };
}

// The whole point of the layout above is that a TableEntry occupies
// exactly one cache line. If a field is added, removed, or its size
// changes such that the entry grows to two lines (or pads out to more
// than 64 bytes), this assert catches it at compile time so we can
// re-decide the cache geometry rather than silently regress.
const _: () = assert!(std::mem::size_of::<TableEntry>() == 64);

/// Direct-mapped pawn-structure cache. Replaces a per-node call to
/// [`evaluate`] with a key-compare and (on hit) a copy of the cached
/// [`PawnsEval`]. On miss the full computation runs and the slot is
/// overwritten — no aging or replacement logic; pawn keys collide rarely
/// enough that simple replacement is fine.
#[derive(Clone)]
pub struct Table {
    entries: Box<[TableEntry]>,
    /// TEMPORARY: hit/miss counters for perf investigation. Read via
    /// [`stats`](Self::stats); reset via [`reset_stats`](Self::reset_stats).
    /// Remove once we've used them to decide on cache-sizing changes.
    hits: u64,
    misses: u64,
}

impl Table {
    pub fn new() -> Self {
        Self {
            entries: vec![TableEntry::EMPTY; TABLE_SLOTS].into_boxed_slice(),
            hits: 0,
            misses: 0,
        }
    }

    pub fn clear(&mut self) {
        for e in self.entries.iter_mut() {
            *e = TableEntry::EMPTY;
        }
    }

    /// TEMPORARY (perf investigation): return `(hits, misses)` since
    /// construction or the last [`reset_stats`](Self::reset_stats).
    pub fn stats(&self) -> (u64, u64) {
        (self.hits, self.misses)
    }

    /// TEMPORARY (perf investigation): zero the hit/miss counters.
    pub fn reset_stats(&mut self) {
        self.hits = 0;
        self.misses = 0;
    }

    /// Probe-or-compute: returns the cached evaluation for `pos`'s pawn
    /// structure, computing and caching it on miss.
    pub fn evaluate(&mut self, pos: &Position) -> PawnsEval {
        let key = pos.pawn_key();
        let slot = (key & TABLE_MASK) as usize;
        let entry = self.entries[slot];
        if entry.key == key {
            self.hits += 1;
            return entry.eval.to_full();
        }
        self.misses += 1;
        let eval = evaluate(pos);
        self.entries[slot] = TableEntry {
            key,
            eval: CachedPawnsEval::from_full(&eval),
        };
        eval
    }
}

impl Default for Table {
    fn default() -> Self {
        Self::new()
    }
}

// =========================================================================
// Public entry point
// =========================================================================

/// Evaluate the pawn structure of both colours on this position.
pub fn evaluate(pos: &Position) -> PawnsEval {
    let mut eval = PawnsEval {
        scores: [Score::ZERO; 2],
        breakdowns: [PawnsBreakdown::zero(); 2],
        passed_pawns: [Bitboard::EMPTY; 2],
        pawn_attacks: [Bitboard::EMPTY; 2],
        pawn_attacks_span: [Bitboard::EMPTY; 2],
    };
    for &us in &Color::both() {
        evaluate_color(pos, us, &mut eval);
    }
    eval
}

// =========================================================================
// Per-colour evaluation
// =========================================================================

fn evaluate_color(pos: &Position, us: Color, eval: &mut PawnsEval) {
    let them = !us;
    let up = Direction::pawn_push(us);

    let our_pawns = pos.pieces_of(us, PieceType::Pawn);
    let their_pawns = pos.pieces_of(them, PieceType::Pawn);
    let their_double_attacks = their_pawns.pawn_double_attacks(them);

    let our_attacks = our_pawns.pawn_attacks(us);
    eval.pawn_attacks[us.index()] = our_attacks;

    // Attack-span starts equal to the raw attack set and accumulates
    // contributions from every non-backward, non-blocked pawn below.
    let mut attacks_span = our_attacks;
    let mut breakdown = PawnsBreakdown::zero();
    let mut passed = Bitboard::EMPTY;

    for s in our_pawns {
        let r_rel = s.rank().from_perspective(us).index() as i32;

        let opposed = their_pawns & forward_file_bb(us, s);
        let blocked = their_pawns & (s + up);
        let stoppers = their_pawns & passed_pawn_span(us, s);
        let lever = their_pawns & pawn_attacks_from(us, s);
        let lever_push = their_pawns & pawn_attacks_from(us, s + up);
        let doubled = our_pawns & (s - up);
        let neighbours = our_pawns & adjacent_files_bb(s);
        let phalanx = neighbours & rank_bb(s.rank());
        let support = neighbours & rank_bb((s - up).rank());

        // Backward: no same-colour neighbour that could defend the push
        // target or any square beyond it, *and* the pawn is either blocked
        // or the push square is under enemy pawn lever attack.
        let has_advancing_neighbour = (neighbours & forward_ranks_bb(them, s + up)).any();
        let is_backward = !has_advancing_neighbour && (lever_push | blocked).any();

        if !is_backward && blocked.is_empty() {
            attacks_span |= pawn_attack_span(us, s);
        }

        // Passed: no stoppers other than levers / leverPush we outnumber,
        // or a single blocker we could lever past from rank 5+.
        let is_passed = {
            let no_stoppers_beyond_lever = (stoppers ^ lever).is_empty();
            let only_lever_push_stoppers =
                (stoppers ^ lever_push).is_empty() && phalanx.popcount() >= lever_push.popcount();
            let lone_blocker_can_be_levered = stoppers == blocked
                && r_rel >= Rank::R5.index() as i32
                && (support.shift(up) & !(their_pawns | their_double_attacks)).any();
            no_stoppers_beyond_lever || only_lever_push_stoppers || lone_blocker_can_be_levered
        };

        if is_passed {
            passed = passed | s;
        }

        // Connected / phalanx bonus.
        if (support | phalanx).any() {
            let phalanx_bonus = if phalanx.any() { 1 } else { 0 };
            let opposed_penalty = if opposed.any() { 1 } else { 0 };
            let v = CONNECTED[r_rel as usize] * (2 + phalanx_bonus - opposed_penalty)
                + 21 * support.popcount() as i32;
            let mg = v;
            let eg = v * (r_rel - 2) / 4;
            breakdown.connected += Score::new(mg, eg);
        } else if neighbours.is_empty() {
            breakdown.isolated -= ISOLATED;
            if opposed.is_empty() {
                breakdown.weak_unopposed -= WEAK_UNOPPOSED;
            }
        } else if is_backward {
            breakdown.backward -= BACKWARD;
            if opposed.is_empty() {
                breakdown.weak_unopposed -= WEAK_UNOPPOSED;
            }
        }

        if support.is_empty() {
            if doubled.any() {
                breakdown.doubled -= DOUBLED;
            }
            if lever.more_than_one() {
                breakdown.weak_lever -= WEAK_LEVER;
            }
        }
    }

    eval.pawn_attacks_span[us.index()] = attacks_span;
    eval.passed_pawns[us.index()] = passed;
    eval.scores[us.index()] = breakdown.total();
    eval.breakdowns[us.index()] = breakdown;
}

// =========================================================================
// King safety
// =========================================================================

/// Decomposed shelter / storm result returned by [`king_safety`].
/// The teaching layer surfaces each component as its own
/// [`crate::analysis::TermId`] so a "you nudged the storm tables"
/// move doesn't get narrated as if your pawn shield improved.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ShelterComponents {
    /// Friendly pawn-shield bonus: `SHELTER_BASE +
    /// Σ SHELTER_STRENGTH[file][rank]`. Mg-only in the reference
    /// tables; eg part comes from `SHELTER_BASE` (which carries
    /// `Score::new(5, 5)`) so this can be slightly non-zero in eg.
    pub pawn_shield: Score,
    /// Enemy pawn-storm penalty: `−Σ UNBLOCKED_STORM[file][rank]` (or
    /// `−BLOCKED_STORM` on the special blocked-pair case). Stored
    /// negated so `total()` adds.
    pub pawn_storm: Score,
    /// Endgame king-to-nearest-own-pawn penalty (mg = 0).
    pub king_pawn_distance: Score,
}

impl ShelterComponents {
    pub const fn zero() -> ShelterComponents {
        ShelterComponents {
            pawn_shield: Score::ZERO,
            pawn_storm: Score::ZERO,
            king_pawn_distance: Score::ZERO,
        }
    }

    /// Sum of all three components — the value the pre-split
    /// `king_safety` returned.
    pub fn total(&self) -> Score {
        self.pawn_shield + self.pawn_storm + self.king_pawn_distance
    }
}

/// Pawn-shelter and pawn-storm evaluation for one colour's king, plus an
/// endgame penalty proportional to king-to-nearest-own-pawn distance.
/// Returns a [`ShelterComponents`] split into pawn-shield bonus,
/// pawn-storm penalty, and king-pawn-distance penalty so the teaching
/// layer can attribute eval shifts to the right named concept.
///
/// When the king still has castling rights, the returned (shield,
/// storm) pair is taken from the candidate king square (current or
/// castling target) with the highest combined mg shelter — the
/// evaluator assumes the side will pick the best shelter available.
pub fn king_safety(pos: &Position, us: Color) -> ShelterComponents {
    let king_sq = pos.king_square(us);
    let (mut best_shield, mut best_storm) = evaluate_shelter(pos, us, king_sq);
    let mut best_mg = best_shield.mg().0 + best_storm.mg().0;

    let our_rights = pos.castling_rights() & CastlingRights::for_color(us);
    let mut consider = |shield: Score, storm: Score| {
        let mg = shield.mg().0 + storm.mg().0;
        if mg > best_mg {
            best_shield = shield;
            best_storm = storm;
            best_mg = mg;
        }
    };
    if our_rights.intersects(CastlingRights::KING_SIDE) {
        let target = Square::G1.from_perspective(us);
        let (shield, storm) = evaluate_shelter(pos, us, target);
        consider(shield, storm);
    }
    if our_rights.intersects(CastlingRights::QUEEN_SIDE) {
        let target = Square::C1.from_perspective(us);
        let (shield, storm) = evaluate_shelter(pos, us, target);
        consider(shield, storm);
    }

    // Endgame: bring the king close to our nearest pawn. The reference
    // uses 8 as the "no pawns" fallback (larger than any chebyshev
    // distance); we mirror that by returning 0 in that case so the
    // endgame penalty vanishes when there's no pawn to approach.
    let our_pawns = pos.pieces_of(us, PieceType::Pawn);
    let min_dist = nearest_own_pawn_distance(king_sq, our_pawns);
    let king_pawn_distance = Score::new(0, -KING_TO_NEAREST_PAWN_PENALTY_EG * min_dist);

    ShelterComponents {
        pawn_shield: best_shield,
        pawn_storm: best_storm,
        king_pawn_distance,
    }
}

/// Evaluate pawn shelter + storm as if our king stood on `king_sq`,
/// returning `(pawn_shield, pawn_storm)` as a pair. The shield is the
/// `SHELTER_BASE + SHELTER_STRENGTH` accumulation; the storm is the
/// `−UNBLOCKED_STORM` / `−BLOCKED_STORM` accumulation, stored negated
/// so the caller can sum the two for the legacy aggregate.
///
/// The square may differ from the actual king square when we're
/// speculatively comparing castling options.
fn evaluate_shelter(pos: &Position, us: Color, king_sq: Square) -> (Score, Score) {
    let them = !us;

    // Only pawns on our side of the king contribute to shelter / storm.
    // "Our side" = squares not strictly in front of our king from our POV.
    let relevant = pos.pieces(PieceType::Pawn) & !forward_ranks_bb(them, king_sq);
    let our_pawns = relevant & pos.pieces_by_color(us);
    let their_pawns = relevant & pos.pieces_by_color(them);

    // Two accumulators: shield gathers SHELTER_BASE + SHELTER_STRENGTH;
    // storm gathers the negated UNBLOCKED_STORM / BLOCKED_STORM
    // contributions. Their sum reproduces the pre-split aggregate.
    let mut shield = SHELTER_BASE;
    let mut storm = Score::ZERO;

    // Evaluate across the king's file and its two neighbours. Clamp the
    // center file to the b..g range so the three-file sweep never falls
    // off the edge of the board.
    let center_idx = (king_sq.file().index()).clamp(File::B.index(), File::G.index());
    for offset in -1i32..=1 {
        let f_idx = (center_idx as i32 + offset) as usize;
        let file = File::from_index(f_idx as u8).expect("clamped in b..g; ±1 stays in a..h");

        let our_on_file = our_pawns & file_bb(file);
        let our_rank = if our_on_file.any() {
            our_on_file
                .frontmost(them)
                .rank()
                .from_perspective(us)
                .index() as i32
        } else {
            0
        };

        let their_on_file = their_pawns & file_bb(file);
        let their_rank = if their_on_file.any() {
            their_on_file
                .frontmost(them)
                .rank()
                .from_perspective(us)
                .index() as i32
        } else {
            0
        };

        let folded = file.fold_to_queenside().index();
        shield += Score::new(SHELTER_STRENGTH[folded][our_rank as usize], 0);

        if our_rank > 0 && our_rank == their_rank - 1 {
            // Their pawn is immediately in front of ours — blocked storm.
            // The very-early case (their pawn on relative rank 3) draws
            // the heaviest penalty. `Rank::R3.index()` is 2 in 0-indexed
            // terms, which matches the reference's `RANK_3` enum value.
            if their_rank == Rank::R3.index() as i32 {
                storm -= BLOCKED_STORM;
            }
        } else {
            storm -= Score::new(UNBLOCKED_STORM[folded][their_rank as usize], 0);
        }
    }

    (shield, storm)
}

/// The friendly pawns that form the king's shelter — the frontmost (most
/// advanced toward the enemy) own pawn on the king's file and each of its
/// two neighbours, restricted to the king's own side of the board. These
/// are exactly the pawns whose advancement [`SHELTER_STRENGTH`] scores in
/// [`evaluate_shelter`], so a "your king is safer (pawn shield)" card can
/// highlight the cover the move strengthened. A file with no own pawn on
/// the king's side (an open shelter file) contributes nothing. The centre
/// file is clamped to `b..g` so the three-file sweep never falls off the
/// board, matching [`evaluate_shelter`].
pub fn king_shield_pawns(pos: &Position, us: Color) -> Bitboard {
    let them = !us;
    let king_sq = pos.king_square(us);
    let our_pawns = pos.pieces(PieceType::Pawn)
        & pos.pieces_by_color(us)
        & !forward_ranks_bb(them, king_sq);
    let center_idx = king_sq.file().index().clamp(File::B.index(), File::G.index());
    let mut shield = Bitboard::EMPTY;
    for offset in -1i32..=1 {
        let file = File::from_index((center_idx as i32 + offset) as u8)
            .expect("clamped to b..g; ±1 stays in a..h");
        let on_file = our_pawns & file_bb(file);
        if on_file.any() {
            shield |= square_bb(on_file.frontmost(them));
        }
    }
    shield
}

/// Chebyshev distance from `king_sq` to the nearest own pawn, for use as
/// the endgame king-activity term. Returns 0 when the side has no pawns
/// (making the term vanish), and short-circuits to 1 when any pawn is
/// inside the king's attack radius.
fn nearest_own_pawn_distance(king_sq: Square, our_pawns: Bitboard) -> i32 {
    if our_pawns.is_empty() {
        return 0;
    }
    if (our_pawns & king_attacks(king_sq)).any() {
        return 1;
    }
    let mut min_dist = 8i32;
    for pawn_sq in our_pawns {
        let d = square_distance(king_sq, pawn_sq) as i32;
        if d < min_dist {
            min_dist = d;
        }
    }
    min_dist
}

// =========================================================================
// Tests
// =========================================================================


#[cfg(test)]
#[path = "pawns_tests.rs"]
mod tests;
