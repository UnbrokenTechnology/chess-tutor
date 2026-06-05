//! Specialised endgame evaluators and scaling functions.
//!
//! The main evaluator's classical terms — mobility, king safety, threats,
//! etc. — are tuned for middlegame play. In the endgame these signals
//! drop off (material is sparse, no threats against the king, nobody
//! cares about pawn structure in K+Q vs K). What matters instead is
//! *technique*: driving the enemy king to the edge, centralising your
//! own pieces, shepherding a pawn to promotion, recognising fortress
//! patterns. Classical search at depth 14 wanders these flat-eval
//! endgames for hundreds of millions of nodes because every move looks
//! identical to the static eval.
//!
//! This module plugs two kinds of specialised knowledge into the
//! evaluator:
//!
//! 1. **Value-returning evaluators** ([`ProbeResult::Override`]) — the
//!    specialist owns the full eval. The main evaluator skips its
//!    classical terms entirely and trusts the specialist's number. Used
//!    for bare-king mates (KXK / KBNK), bitbase-precise pawn endings
//!    (KPK), and piece-vs-piece mates / drawish patterns (KQKR / KRKB /
//!    KRKN / KQKP / KRKP / KNNK / KNNKP).
//!
//! 2. **Scaling functions** ([`ProbeResult::Scale`]) — the specialist
//!    returns a `ScaleFactor` for the strong side, which the main
//!    evaluator applies to the endgame half of the score *after*
//!    computing it with the usual classical terms. Used for fortress
//!    patterns where the eval already roughly gets the picture but
//!    needs to be capped: KBPsK rook-pawn wrong-bishop draws, KQKRPs
//!    third-rank-rook fortresses, KRPKR third-rank defence, etc. The
//!    [`ProbeResult::ScaleBoth`] variant handles KPKP, where either
//!    side could end up eg-winning and both need to be scaled.
//!
//! Override and Scale are mutually exclusive for any one position.
//! Scaling functions exist precisely *because* the position is too
//! close to drawish for a full Override to be safe but the classical
//! eval, left alone, would happily report several pawns of advantage
//! and the engine would chase the phantom win for minutes.
//!
//! **Adding a new specialisation.** Write a signature detector (e.g.
//! `strong_side(pos) -> Option<Color>` in a per-function module),
//! write the evaluator function (with a gradient for any technique
//! it's meant to drive, see the `endgame_evaluator_gradients` memory),
//! and route from `probe()`'s dispatcher.

use crate::bitboard::{DARK_SQUARES, LIGHT_SQUARES};
use crate::position::Position;
use crate::types::{Color, PieceType, ScaleFactor, Value};

// =========================================================================
// Per-function modules
// =========================================================================

// Value-returning evaluators (Override).
mod kbnk;
mod knnk;
mod knnkp;
mod kpk;
mod kqkp;
mod kqkr;
mod krkb;
mod krkn;
mod krkp;
mod kxk;

// ScaleFactor-returning scaling functions (Scale).
mod kbpkb;
mod kbpkn;
mod kbppkb;
mod kbpsk;
mod knpk;
mod knpkb;
mod kpkp;
mod kpsk;
mod kqkrps;
mod krpkb;
mod krpkr;
mod krppkrp;

// =========================================================================
// Result type
// =========================================================================

/// The outcome of probing the endgame database for a position.
///
/// - `Override` — the specialised evaluator owns the full eval; the
///   caller should use this number and skip the classical terms.
/// - `Scale` — the specialist returned a scale factor for `strong_side`
///   that should be applied to the endgame half of the classical eval.
/// - `ScaleBoth` — apply the same factor to both colors. Used by KPKP
///   where either side could be the eg-winning side depending on the
///   full eval, and both need to be scaled identically.
/// - `None` — no specialist matched; use the classical eval unmodified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeResult {
    Override(Value),
    Scale {
        strong_side: Color,
        factor: ScaleFactor,
    },
    ScaleBoth(ScaleFactor),
    None,
}

/// How much closed-form endgame knowledge the **play** engine may use — a
/// difficulty-ordered skill ladder. A weaker bot is denied the harder
/// specialists and falls back to classical eval + search, so it misplays
/// endgames the way a human of that level does: a tier-`None` bot has no
/// king-driving gradient and shuffles / stalemates a won KQ; a tier-below-
/// `Full` bot can't deliver the KBN mate (the `kbnk` corner override is
/// withheld) and herds the king to the wrong square.
///
/// **Play-engine-only**, exactly like [`crate::opponent::EvalMask`] and
/// the qsearch cap: the analytical / hint / retrospective engines always
/// use [`EndgameSkill::Full`] so teaching judges true best play and can
/// say "you *could* have won this — here's the technique."
///
/// Variants are declared weakest-first so the derived `Ord` gives the
/// ladder ordering used by [`probe_with_skill`] (`bot >= required`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum EndgameSkill {
    /// No endgame books at all — pure classical eval + search. Believable
    /// sub-~1000 play: misplaced kings, accidental stalemates, won
    /// endgames let slip. Also sidesteps the SF11-inherited Q→B
    /// underpromotion quirk (no `kbnk` override to out-rank a queen).
    None,
    /// Trivial major-piece mates only — KQK / KRK via the generic `kxk`.
    Basic,
    /// + fundamental technique: KPK opposition (bitbase), piece-vs-pawn
    /// and piece-vs-piece endings, the KNN-vs-K dead draw.
    Intermediate,
    /// Everything, including the hard mates (KBNK, KNNKP/Troitsky) and the
    /// fortress scaling functions. The analytical default.
    #[default]
    Full,
}

impl EndgameSkill {
    /// Map a 0-based tier level (CLI / UCI dial) to a skill. Levels at or
    /// above the top tier saturate to [`EndgameSkill::Full`].
    pub fn from_tier(level: u8) -> EndgameSkill {
        match level {
            0 => EndgameSkill::None,
            1 => EndgameSkill::Basic,
            2 => EndgameSkill::Intermediate,
            _ => EndgameSkill::Full,
        }
    }
}

// =========================================================================
// Tuning tables
// =========================================================================
//
// `pub(super)` so each per-function file under this directory can use
// them without exporting them from the crate.

/// Per-square bonus for having the *weak* king stand there. Centre
/// squares score lowest, edges and corners highest — drives the losing
/// king toward the edge, which is what's needed to mate it.
pub(super) const PUSH_TO_EDGES: [i32; 64] = [
    100, 90, 80, 70, 70, 80, 90, 100, //
    90, 70, 60, 50, 50, 60, 70, 90, //
    80, 60, 40, 30, 30, 40, 60, 80, //
    70, 50, 30, 20, 20, 30, 50, 70, //
    70, 50, 30, 20, 20, 30, 50, 70, //
    80, 60, 40, 30, 30, 40, 60, 80, //
    90, 70, 60, 50, 50, 60, 70, 90, //
    100, 90, 80, 70, 70, 80, 90, 100, //
];

/// Per-distance bonus for "our king is close to their king". Indexed
/// by Chebyshev distance.
pub(super) const PUSH_CLOSE: [i32; 8] = [0, 0, 100, 80, 60, 40, 20, 10];

/// Per-distance bonus for "two enemy pieces are far apart". Indexed by
/// Chebyshev distance.
pub(super) const PUSH_AWAY: [i32; 8] = [0, 5, 20, 40, 60, 80, 90, 100];

/// Per-square bonus for the weak king's position in `KBNK`.
pub(super) const PUSH_TO_CORNERS: [i32; 64] = [
    6400, 6080, 5760, 5440, 5120, 4800, 4480, 4160, //
    6080, 5760, 5440, 5120, 4800, 4480, 4160, 4480, //
    5760, 5440, 4960, 4480, 4480, 4000, 4480, 4800, //
    5440, 5120, 4480, 3840, 3520, 4480, 4800, 5120, //
    5120, 4800, 4480, 3520, 3840, 4480, 5120, 5440, //
    4800, 4480, 4000, 4480, 4480, 4960, 5440, 5760, //
    4480, 4160, 4480, 4800, 5120, 5440, 5760, 6080, //
    4160, 4480, 4800, 5120, 5440, 5760, 6080, 6400, //
];

// =========================================================================
// Dispatcher
// =========================================================================

/// If `pos` matches a recognised endgame pattern, return the specialist
/// result. The caller is responsible for any side-to-move flipping of
/// the Override value (which is in white's POV).
///
/// Full endgame knowledge — equivalent to `probe_with_skill(pos,
/// EndgameSkill::Full)`. The analytical / UI engines use this.
pub fn probe(pos: &Position) -> ProbeResult {
    probe_with_skill(pos, EndgameSkill::Full)
}

/// As [`probe`], but only consult specialists at or below the bot's
/// [`EndgameSkill`] tier; harder ones are withheld so the position falls
/// through to a coarser specialist (or to classical eval at
/// [`EndgameSkill::None`]). See [`EndgameSkill`] for the per-tier rationale.
///
/// Withholding a specialist makes the position fall *through* the
/// dispatcher: e.g. a KBN-vs-K position below `Full` skips `kbnk` and
/// lands on the generic `kxk` (which drives to the wrong — merely
/// edge — square, the human failure mode), and at `None` skips `kxk`
/// too and gets plain classical eval.
pub fn probe_with_skill(pos: &Position, skill: EndgameSkill) -> ProbeResult {
    use EndgameSkill::{Basic, Full, Intermediate};

    // ---- Value-returning evaluators (Override) ----------------------
    //
    // KBNK before KXK: same lone-king structure but a tighter
    // corner-driving score. The hardest elementary mate — `Full` only;
    // below that it falls through to the generic KXK.
    if skill >= Full {
        if let Some(strong) = kbnk::strong_side(pos) {
            return ProbeResult::Override(kbnk::evaluate(pos, strong));
        }
    }

    // KPK before KXK. The bitbase distinguishes wrong-rook-pawn /
    // opposition / stalemate; KXK would paper over those nuances.
    if skill >= Intermediate {
        if let Some(strong) = kpk::strong_side(pos) {
            return ProbeResult::Override(kpk::evaluate(pos, strong));
        }

        // KNN vs bare K — unconditional draw.
        if knnk::matches(pos) {
            return ProbeResult::Override(Value::DRAW);
        }
    }

    // KNN vs K+P — theoretical Troitsky-line win (advanced technique).
    if skill >= Full {
        if let Some(strong) = knnkp::strong_side(pos) {
            return ProbeResult::Override(knnkp::evaluate(pos, strong));
        }
    }

    // Piece-vs-piece / piece-vs-pawn endings.
    if skill >= Intermediate {
        if let Some(strong) = piece_vs_piece_signature(pos, PieceType::Queen, PieceType::Rook) {
            return ProbeResult::Override(kqkr::evaluate(pos, strong));
        }
        if let Some(strong) = piece_vs_pawn_signature(pos, PieceType::Queen) {
            return ProbeResult::Override(kqkp::evaluate(pos, strong));
        }
        if let Some(strong) = piece_vs_pawn_signature(pos, PieceType::Rook) {
            return ProbeResult::Override(krkp::evaluate(pos, strong));
        }
        if let Some(strong) = piece_vs_piece_signature(pos, PieceType::Rook, PieceType::Bishop) {
            return ProbeResult::Override(krkb::evaluate(pos, strong));
        }
        if let Some(strong) = piece_vs_piece_signature(pos, PieceType::Rook, PieceType::Knight) {
            return ProbeResult::Override(krkn::evaluate(pos, strong));
        }
    }

    // ---- ScaleFactor-returning scaling functions (Scale) ------------
    //
    // All twelve scaling-function ports are gated off by the `if
    // SCALING_ENABLED` block below. The implementations and unit
    // tests are kept in tree (see `kbpsk.rs`, `krpkr.rs`, etc.) so
    // re-enabling is a one-line flip when the search-side
    // prerequisites land.
    //
    // **Why disabled.** Each scaling function, when its signature
    // matches a subtree leaf and it returns SCALE_FACTOR_DRAW, drops
    // the endgame component of that leaf's score from "winning
    // material" to ~0. Alpha-beta cutoffs that previously fired
    // from the material score no longer do, and the search has to
    // prove there's no winning move *through* the recognised
    // fortress. On positions with many such transient subtrees the
    // node count balloons:
    //
    // - **FEN 41** (Q vs 2R+P): KQKRPs fires 917× at depth 10. With
    //   scaling on, depth-14 doesn't finish in 5 minutes; with
    //   scaling off, 8.9 s.
    // - **Bench position 39** (K+R+P vs K+R+P): KRPKR fires 7200×
    //   across the full bench. Position 39 alone becomes 813 M
    //   nodes vs ~250 k baseline.
    // - **Aggregate depth-13 bench**: 27 M → 876 M nodes (32×
    //   regression) with all twelve scaling functions live.
    //
    // SF11 ships these same functions and absorbs the cutoff loss
    // via its LMR-relaxer family — `ttPv → r-=2`, `ttHitAverage`
    // gating, `opp moveCount > 14 → r--`, `singularLMR`, and
    // escape-capture detection. We have the sticky `ttPv` save in
    // tree (2026-05-12) but none of the LMR consumers. Once those
    // land, re-enable this block.
    //
    // Order within the block (preserved for the future re-enable):
    // exact-material specialists (KRPKR, KBPKB, etc.) before the
    // generic KBPsK / KQKRPs / KPsK / KPKP, mirroring SF11
    // material.cpp's exact-key-then-generic dispatch.
    const SCALING_ENABLED: bool = false;
    if SCALING_ENABLED && skill >= Full {
        if let Some(strong) = krpkr::strong_side(pos) {
            if let Some(r) = scale_if_applies(strong, krpkr::evaluate(pos, strong)) {
                return r;
            }
        }
        if let Some(strong) = krpkb::strong_side(pos) {
            if let Some(r) = scale_if_applies(strong, krpkb::evaluate(pos, strong)) {
                return r;
            }
        }
        if let Some(strong) = krppkrp::strong_side(pos) {
            if let Some(r) = scale_if_applies(strong, krppkrp::evaluate(pos, strong)) {
                return r;
            }
        }
        if let Some(strong) = kbpkb::strong_side(pos) {
            if let Some(r) = scale_if_applies(strong, kbpkb::evaluate(pos, strong)) {
                return r;
            }
        }
        if let Some(strong) = kbppkb::strong_side(pos) {
            if let Some(r) = scale_if_applies(strong, kbppkb::evaluate(pos, strong)) {
                return r;
            }
        }
        if let Some(strong) = kbpkn::strong_side(pos) {
            if let Some(r) = scale_if_applies(strong, kbpkn::evaluate(pos, strong)) {
                return r;
            }
        }
        if let Some(strong) = knpk::strong_side(pos) {
            if let Some(r) = scale_if_applies(strong, knpk::evaluate(pos, strong)) {
                return r;
            }
        }
        if let Some(strong) = knpkb::strong_side(pos) {
            if let Some(r) = scale_if_applies(strong, knpkb::evaluate(pos, strong)) {
                return r;
            }
        }
        if let Some(strong) = kbpsk::strong_side(pos) {
            if let Some(r) = scale_if_applies(strong, kbpsk::evaluate(pos, strong)) {
                return r;
            }
        }
        if let Some(strong) = kqkrps::strong_side(pos) {
            if let Some(r) = scale_if_applies(strong, kqkrps::evaluate(pos, strong)) {
                return r;
            }
        }
        if let Some(strong) = kpsk::strong_side(pos) {
            if let Some(r) = scale_if_applies(strong, kpsk::evaluate(pos, strong)) {
                return r;
            }
        }
        if kpkp::matches(pos) {
            let factor = kpkp::evaluate(pos);
            if factor != ScaleFactor::NONE {
                return ProbeResult::ScaleBoth(factor);
            }
        }
    }

    // KXK: catch-all fallback for "one side has a lone king and the
    // other has enough material to mate." Runs after the scaling
    // functions so fortress patterns get the first chance. The lowest
    // tier of book knowledge — `Basic` and up; at `None` even the
    // trivial mates get no king-driving gradient (classical eval only).
    if skill >= Basic {
        if let Some(strong) = lone_king_opponent(pos) {
            return ProbeResult::Override(kxk::evaluate(pos, strong));
        }
    }

    ProbeResult::None
}

/// Converts a scaling-function result into `Some(ProbeResult::Scale)`
/// when the function applies (non-NONE factor), `None` when it
/// doesn't (so the dispatcher falls through to the next candidate
/// or to the KXK fallback).
fn scale_if_applies(strong_side: Color, factor: ScaleFactor) -> Option<ProbeResult> {
    if factor == ScaleFactor::NONE {
        None
    } else {
        Some(ProbeResult::Scale {
            strong_side,
            factor,
        })
    }
}

// =========================================================================
// Shared helpers
// =========================================================================

/// True iff `color` has only their king — no pawns, no pieces.
pub(super) fn is_lone_king(pos: &Position, color: Color) -> bool {
    pos.non_pawn_material(color) == Value::ZERO && pos.count(color, PieceType::Pawn) == 0
}

/// Returns `Some(strong_side)` if exactly one side has a lone king and
/// the other has enough material to force mate.
fn lone_king_opponent(pos: &Position) -> Option<Color> {
    let white_lone = is_lone_king(pos, Color::White);
    let black_lone = is_lone_king(pos, Color::Black);
    match (white_lone, black_lone) {
        (false, true) if has_mating_material(pos, Color::White) => Some(Color::White),
        (true, false) if has_mating_material(pos, Color::Black) => Some(Color::Black),
        _ => None,
    }
}

/// Does this side have enough material to force mate against a lone
/// king via the KXK fallback?
///
/// KXK is the catch-all *after* the scaling-fortress dispatch — the
/// dispatcher tries KBPsK / KPsK / KNPK / etc. first, so KXK only
/// sees positions those didn't recognise. We use the loose gate
/// (mating material is anything with pawn ≥ 1 OR Q/R OR B+N OR
/// opposite-colour bishops). K+B / K+N / K+2B-same-colour return
/// false (insufficient material — no theoretical mate).
///
/// **Why the loose gate.** SF11's `is_KXK` requires npm ≥ RookMg,
/// which would exclude K+pawns vs K (npm = 0). But our dispatcher
/// then has no Override for those positions, so the K+pawns vs K
/// leaf in any deeper search returns classical eval. That changes
/// the TT entries those leaves populate, which can cascade into
/// 100×+ regressions on unrelated bench positions later in the
/// same TT-shared run. Keeping the loose gate preserves the
/// "K+pawns vs K is winning, no fortress" Override that pre-2026-
/// 05-13 builds relied on.
fn has_mating_material(pos: &Position, strong: Color) -> bool {
    if pos.count(strong, PieceType::Pawn) > 0 {
        return true;
    }
    let q = pos.count(strong, PieceType::Queen);
    let r = pos.count(strong, PieceType::Rook);
    let b = pos.count(strong, PieceType::Bishop);
    let n = pos.count(strong, PieceType::Knight);

    if q > 0 || r > 0 || (b > 0 && n > 0) {
        return true;
    }
    if b >= 2 {
        let bishops = pos.pieces_of(strong, PieceType::Bishop);
        if (bishops & DARK_SQUARES).any() && (bishops & LIGHT_SQUARES).any() {
            return true;
        }
    }
    false
}

/// Middlegame piece value for signature checks.
pub(super) const fn mg_value(pt: PieceType) -> Value {
    match pt {
        PieceType::Pawn => Value::PAWN_MG,
        PieceType::Knight => Value::KNIGHT_MG,
        PieceType::Bishop => Value::BISHOP_MG,
        PieceType::Rook => Value::ROOK_MG,
        PieceType::Queen => Value::QUEEN_MG,
        PieceType::King => Value::ZERO,
    }
}

/// Returns `Some(strong)` when the material is exactly K + `strong_piece`
/// vs K + `weak_piece` (one of each, no pawns).
fn piece_vs_piece_signature(
    pos: &Position,
    strong_piece: PieceType,
    weak_piece: PieceType,
) -> Option<Color> {
    let strong_npm = mg_value(strong_piece);
    let weak_npm = mg_value(weak_piece);
    for &c in Color::both().iter() {
        let opp = !c;
        if pos.count(c, PieceType::Pawn) != 0 || pos.count(opp, PieceType::Pawn) != 0 {
            continue;
        }
        if pos.non_pawn_material(c) != strong_npm || pos.non_pawn_material(opp) != weak_npm {
            continue;
        }
        if pos.count(c, strong_piece) == 1 && pos.count(opp, weak_piece) == 1 {
            return Some(c);
        }
    }
    None
}

/// Returns `Some(strong)` when the material is exactly K + `strong_piece`
/// vs K + one pawn (no other weak material).
fn piece_vs_pawn_signature(pos: &Position, strong_piece: PieceType) -> Option<Color> {
    let strong_npm = mg_value(strong_piece);
    for &c in Color::both().iter() {
        let opp = !c;
        if pos.count(c, PieceType::Pawn) != 0 {
            continue;
        }
        if pos.non_pawn_material(c) != strong_npm || pos.count(c, strong_piece) != 1 {
            continue;
        }
        if pos.non_pawn_material(opp) != Value::ZERO || pos.count(opp, PieceType::Pawn) != 1 {
            continue;
        }
        return Some(c);
    }
    None
}

// =========================================================================
// Dispatcher tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startpos_returns_none() {
        let p = Position::startpos();
        assert_eq!(probe(&p), ProbeResult::None);
    }

    #[test]
    fn lone_king_with_insufficient_material_returns_none() {
        let p = Position::from_fen("7k/8/8/8/8/8/8/N3K3 w - - 0 1").unwrap();
        assert_eq!(probe(&p), ProbeResult::None);
    }

    #[test]
    fn kxk_pattern_returns_override() {
        let p = Position::from_fen("7k/8/5KQ1/8/8/8/8/8 w - - 0 1").unwrap();
        assert!(matches!(probe(&p), ProbeResult::Override(_)));
    }

    #[test]
    fn skill_tiers_withhold_harder_specialists() {
        use EndgameSkill::{Basic, Full, Intermediate, None as NoBooks};

        // KQK (trivial mate) — fires at Basic+, classical (None) at tier 0.
        let kqk = Position::from_fen("7k/8/5KQ1/8/8/8/8/8 w - - 0 1").unwrap();
        assert_eq!(probe_with_skill(&kqk, NoBooks), ProbeResult::None);
        assert!(matches!(
            probe_with_skill(&kqk, Basic),
            ProbeResult::Override(_)
        ));

        // KPK — the precise opposition/wrong-pawn bitbase is withheld
        // below Intermediate. (At Basic, K+pawns-vs-K still gets the
        // generic KXK gradient — "pawns usually win" — just not the
        // bitbase's drawn-position nuance; at None it's classical only.)
        let kpk = Position::from_fen("8/8/8/3k4/8/3K4/3P4/8 w - - 0 1").unwrap();
        assert_eq!(probe_with_skill(&kpk, NoBooks), ProbeResult::None);
        assert!(matches!(
            probe_with_skill(&kpk, Intermediate),
            ProbeResult::Override(_)
        ));

        // KBNK — the hard mate. Below Full it falls THROUGH to the generic
        // KXK (still an Override, but the wrong — merely edge-driving —
        // gradient), and at None drops to classical eval.
        let kbnk = Position::from_fen("7k/8/8/8/8/8/8/2B1K1N1 w - - 0 1").unwrap();
        assert_eq!(probe_with_skill(&kbnk, NoBooks), ProbeResult::None);
        // Generic KXK fires at Basic/Intermediate (lone king + B+N).
        assert!(matches!(
            probe_with_skill(&kbnk, Intermediate),
            ProbeResult::Override(_)
        ));
        // The dedicated KBNK override only at Full — and it scores ABOVE
        // the generic KXK (its corner gradient), so the two differ.
        let generic = probe_with_skill(&kbnk, Intermediate);
        let dedicated = probe_with_skill(&kbnk, Full);
        assert!(matches!(dedicated, ProbeResult::Override(_)));
        assert_ne!(generic, dedicated);
    }

    #[test]
    fn low_skill_prefers_queen_over_bishop_promotion() {
        // The SF11-inherited Q->B underpromotion only happens when the
        // KBNK override fires (its KNOWN_WIN+corner out-ranks a queen).
        // With books gated off, both promotions get classical eval where
        // material rules, so a queen out-scores bishop+knight. White to
        // move is the lone king; more-negative = better for the strong
        // (Black) side choosing the promotion.
        let after_bishop = "8/8/6K1/8/4k3/8/1n6/b7 w - - 0 1"; // K+B+N vs K
        let after_queen = "8/8/6K1/8/4k3/8/1n6/q7 w - - 0 1"; // K+Q+N vs K
        let pb = Position::from_fen(after_bishop).unwrap();
        let pq = Position::from_fen(after_queen).unwrap();

        // Tier 0 (no books): queen strictly better (classical material).
        let b0 = crate::eval::evaluate_with_pawn_cache(
            &pb,
            &mut crate::pawns::Table::new(),
            crate::opponent::EvalMask::EMPTY,
            EndgameSkill::None,
        );
        let q0 = crate::eval::evaluate_with_pawn_cache(
            &pq,
            &mut crate::pawns::Table::new(),
            crate::opponent::EvalMask::EMPTY,
            EndgameSkill::None,
        );
        assert!(
            q0.0 < b0.0,
            "tier 0: queen ({}) should beat bishop ({})",
            q0.0,
            b0.0
        );

        // At Full the SF11 quirk re-appears (bishop out-ranks queen) —
        // documented as deferred, asserted here so a future fix flips it.
        let bf = crate::eval::evaluate_with_pawn_cache(
            &pb,
            &mut crate::pawns::Table::new(),
            crate::opponent::EvalMask::EMPTY,
            EndgameSkill::Full,
        );
        let qf = crate::eval::evaluate_with_pawn_cache(
            &pq,
            &mut crate::pawns::Table::new(),
            crate::opponent::EvalMask::EMPTY,
            EndgameSkill::Full,
        );
        assert!(
            bf.0 < qf.0,
            "Full: SF11 quirk — bishop ({}) out-ranks queen ({})",
            bf.0,
            qf.0
        );
    }
}
