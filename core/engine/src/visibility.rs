//! Move visibility: how hard is a move for a human to SEE — the
//! "perception" weak-bot lever and the retrospective findability
//! signal. Design + evidence: `PLAN-perception.md` (research synthesis
//! from coaching curricula, self-report corpora, eye-tracking /
//! puzzle-difficulty studies, low-ELO game commentary).
//!
//! The model: a per-move **visibility score** `V ∈ (0, 1]` — a product
//! of independent difficulty factors — and a **margin curve** turning
//! `V` plus the bot's `perception` dial into `P(see)`. The search
//! prunes unseen moves before searching them ("a move you didn't see
//! is never in your tree"), which makes deep quiet combinations
//! invisible by *compounding* (every link must be seen) while forcing
//! chains survive (their links are high-V) — no explicit payoff-depth
//! term needed.
//!
//! Factor families (each defaults to 1.0 when not applicable):
//!
//! - **S — salience = rule-familiarity only.** Normal moves — quiet,
//!   captures, checks, queen promotions — are all base 1.0 ("marching
//!   a pawn is never hard to see"). Penalties only for special-rule /
//!   abnormal movement: castling, en passant (the only capture that
//!   doesn't land on the victim's square), underpromotion. There is
//!   deliberately NO capture>quiet gradient: a recapture's easiness
//!   emerges from the attention factor (the capture square IS the
//!   last-move locus), and quiet *key* moves are hard via geometry or
//!   a beyond-horizon payoff (the depth/qsearch levers' job), never
//!   via quietness itself.
//! - **D — direction** (mover-relative): forward < sideways < backward
//!   ("we are almost programmed not to look for them" — the
//!   Qe7-found/Qe1-never classroom test).
//! - **K — piece**: knight moves are non-collinear ("circles you have
//!   to visualise"); backward knight stacks with D.
//! - **O — ray occlusion**: the strongest geometric signal in the
//!   evidence (discovered/pin puzzle motifs median ~2200 vs fork
//!   ~1450): discovered-attack vehicles and sliders threading crowded
//!   paths.
//! - **A — attention**: two-endpoint distance from the opponent's
//!   last move (a move is a *relation*; you must attend BOTH squares —
//!   seeing your bishop doesn't mean seeing its far target, and
//!   vice versa). Subsumes any standalone move-length penalty: a long
//!   move can't have both endpoints near one locus.
//!
//! `P(see)` is a margin model on `m = p − (1 − V)` (how far perception
//! clears the move's difficulty): cleared → a **perception-scaled**
//! plateau ramping to a deterministic 1.0 ("reliably sees" classes —
//! the believability critique of existing weak bots is
//! *inconsistency*; and a sharper scan is also a more reliable scan,
//! so the fumble rate on cleared moves shrinks as p rises, converging
//! smoothly into the p = 1.0 bypass); missed → a quadratic cliff to
//! literal 0 (at `p = 0`, everything `V < 0.55` is never seen, while
//! every V = 1.0 normal move always is: maximally geometry-blind, not
//! move-blind).
//!
//! Because the per-game roll is deterministic, a per-move miss
//! probability is really a **permanent blind-spot rate**: P = 0.9
//! means ~10% of such moves are invisible for the whole game, every
//! re-search agreeing. Per-move values therefore sit deliberately
//! high (hardest common stacks ≈ 0.5); the weakness comes from
//! compounding — across the plies of a line and across the game's
//! many decisions — not from single moves being coin-flips.
//!
//! **Opponent-ply asymmetry**: on plies where the side-not-being-
//! modeled moves, `V^OPP_EXPONENT` is applied before the curve — a
//! power barely moves V≈1 (the adjacent recapture stays feared — the
//! Einstellung boundary condition: even novices SEE outright-losing
//! moves) but pushes subtle refutations deep into the cliff (Hope
//! Chess: blunders are caused by the unseen opponent reply). This is
//! what lets perception generate organic, believable blunders.
//!
//! Determinism: the see/miss roll is a pure function of
//! `(game_seed, zobrist key, move)` — **no ply mixing** — so the same
//! position+move always resolves the same way within a game: TT
//! entries stay coherent across re-visits and across the game's
//! successive searches, and the bot has stable per-game blind spots
//! (misses the same long diagonal all game, like a human having a bad
//! day). Replaying a seed reproduces the game exactly.
//!
//! Strict invariant: only the **play** engine sets a perception level
//! below 1.0. Analytical paths (retrospective, hint, `analyze`) run
//! with full perception, exactly like `eval_mask` / `qsearch_cap` /
//! `endgame_skill`. The retrospective additionally consumes
//! [`visibility`] *read-only* (line findability at a fixed
//! strong-human reference level) — that path never rolls dice.

use crate::attacks::{between_bb, line_bb, pseudo_attacks};
use crate::bitboard::{king_distance, Bitboard};
use crate::position::Position;
use crate::types::{Color, Move, MoveKind, PieceType, Square};

// =========================================================================
// Tunable constants (feel-tuned 2026-06-07; FROZEN once the calibration
// grid measures them — re-tuning afterwards invalidates the grid)
// =========================================================================

/// Salience penalties — special-rule / abnormal-movement moves only.
pub const S_CASTLING: f64 = 0.80;
pub const S_EN_PASSANT: f64 = 0.55;
pub const S_UNDERPROMOTION: f64 = 0.25;

/// Direction factors, mover-relative. Applied in FULL to quiet moves;
/// square-rooted for captures (the target piece pulls the eye,
/// overriding path-direction bias — the forward-attention evidence is
/// about quiet moves; an open-board sideways rook *take* is nothing
/// like a quiet rank slide). Checks deliberately do NOT get the
/// attenuation: missed checks (including mate-in-1) are documented
/// low-ELO behavior the lever must stay able to produce.
pub const D_SIDEWAYS: f64 = 0.85;
pub const D_BACKWARD: f64 = 0.75;

/// Knight moves are non-collinear — the hardest piece to visualize.
pub const K_KNIGHT: f64 = 0.85;

/// Discovered-attack vehicle: the mover's departure unveils a friendly
/// slider's attack on the enemy king. (v1 scope: king targets only —
/// the cached `blockers_for_king` makes it free; queen/rook targets
/// are a follow-up.)
pub const O_VEHICLE: f64 = 0.75;

/// Slider path threading traffic: occupied squares adjacent to the
/// path interior.
pub const O_THREAD_HEAVY: f64 = 0.85; // >= 4 neighbours
pub const O_THREAD_LIGHT: f64 = 0.92; // 2..=3 neighbours

/// Per-endpoint attention factor by Chebyshev distance from the
/// opponent's last-move square: `<=2 -> 1.0`, `3..=4 ->`
/// [`A_NEAR`], `>=5 ->` [`A_FAR`]. Applied to BOTH endpoints.
pub const A_NEAR: f64 = 0.95;
pub const A_FAR: f64 = 0.90;

/// Endpoint clutter (visual crowding): occupied squares in the union
/// of the from/to king-rings (the two endpoints excluded). Dense
/// middlegame tangles degrade perception ("misses spike when pieces
/// start staring at each other"); thresholds are set high enough that
/// ordinary opening formations (home-rank neighbours of a pawn push)
/// stay neutral.
pub const A_CLUTTER_LIGHT: f64 = 0.94; // 7..=9 occupied ring squares
pub const A_CLUTTER_HEAVY: f64 = 0.88; // >= 10

/// Margin-curve shape: perception clears difficulty → P starts at the
/// perception-scaled plateau `1 − (1 − PLATEAU_FLOOR)·(1 − p)` and
/// ramps to 1.0 over [`RAMP`] of margin; perception falls short →
/// quadratic cliff (from the same plateau) hitting literal 0 at
/// −[`CLIFF`]. Scaling the plateau by `p` means "how often you fumble
/// a move you're capable of seeing" shrinks as perception rises, and
/// the curve converges smoothly into the `p = 1.0` bypass instead of
/// jumping (a p = 0.95 bot fumbles ~1%, not a flat 20%).
pub const PLATEAU_FLOOR: f64 = 0.80;
pub const RAMP: f64 = 0.30;
pub const CLIFF: f64 = 0.45;

/// Exponent applied to `V` on opponent plies before the curve
/// ("I saw it for me but not for them").
pub const OPP_EXPONENT: f64 = 1.5;

// =========================================================================
// Visibility score
// =========================================================================

/// Per-node inputs the score needs beyond the position itself.
/// Build once per node with [`VisibilityContext::at_node`].
#[derive(Copy, Clone, Debug)]
pub struct VisibilityContext {
    /// Destination square of the opponent's previous move — the
    /// attention locus. `None` (no context / null-move parent) leaves
    /// the attention factor neutral.
    pub last_move_to: Option<Square>,
    /// Squares whose occupant (a side-to-move piece) blocks a friendly
    /// slider's line to the enemy king — discovered-attack vehicles.
    pub vehicles: Bitboard,
}

impl VisibilityContext {
    /// Context for the side to move in `pos`. `blockers_for_king` is
    /// cached in the position's check info, so this is a couple of
    /// bitboard ANDs.
    pub fn at_node(pos: &Position, last_move_to: Option<Square>) -> Self {
        let us = pos.side_to_move();
        VisibilityContext {
            last_move_to,
            vehicles: pos.blockers_for_king(!us) & pos.pieces_by_color(us),
        }
    }
}

/// The visibility score `V ∈ (0, 1]` for a legal move of the side to
/// move in `pos`. `1.0` means "no difficulty features — nothing to
/// miss"; lower means harder to see. Pure function; no randomness.
pub fn visibility(pos: &Position, mv: Move, ctx: &VisibilityContext) -> f64 {
    let us = pos.side_to_move();
    let from = mv.from();
    let to = mv.to();

    // S — salience: rule-familiarity only.
    let s = match mv.kind() {
        MoveKind::Castling => S_CASTLING,
        MoveKind::EnPassant => S_EN_PASSANT,
        MoveKind::Promotion if mv.promoted_to() != PieceType::Queen => S_UNDERPROMOTION,
        _ => 1.0,
    };

    // D — direction, mover-relative (perspective flip makes "forward"
    // = increasing rank for both colors). Captures get the attenuated
    // (square-rooted) penalty: the target pulls the eye.
    let is_capture = pos.is_capture(mv);
    let from_rank = from.from_perspective(us).rank() as i8;
    let to_rank = to.from_perspective(us).rank() as i8;
    let d = match to_rank - from_rank {
        delta if delta > 0 => 1.0,
        0 => D_SIDEWAYS,
        _ => D_BACKWARD,
    };
    let d = if is_capture { d.sqrt() } else { d };

    // K — piece type. `piece_on(from)` exists for any legal move.
    let mover = pos
        .piece_on(from)
        .map(|p| p.kind())
        .unwrap_or(PieceType::Pawn);
    let k = if mover == PieceType::Knight { K_KNIGHT } else { 1.0 };

    // O — ray occlusion.
    let mut o = 1.0;
    // Discovered-attack vehicle: the mover sits on a friendly slider's
    // line to the enemy king AND leaves that line (moving along the
    // ray unveils nothing).
    if (ctx.vehicles & from).any() {
        let enemy_king = pos.king_square(!us);
        if !(line_bb(from, enemy_king) & to).any() {
            o *= O_VEHICLE;
        }
    }
    // Threading: slider path squeezing past traffic. Neighbours of the
    // path interior (excluding the endpoints themselves) that are
    // occupied.
    if matches!(
        mover,
        PieceType::Bishop | PieceType::Rook | PieceType::Queen
    ) && mv.kind() == MoveKind::Normal
    {
        let interior = between_bb(from, to);
        if interior.any() {
            let mut neighbours = Bitboard::EMPTY;
            let mut walk = interior;
            while walk.any() {
                let sq = walk.pop_lsb();
                neighbours |= pseudo_attacks(PieceType::King, sq);
            }
            neighbours = neighbours.without(from).without(to) & !interior;
            let traffic = (neighbours & pos.occupied()).popcount();
            o *= match traffic {
                0..=1 => 1.0,
                2..=3 => O_THREAD_LIGHT,
                _ => O_THREAD_HEAVY,
            };
        }
    }

    // A — two-endpoint attention.
    let mut a = 1.0;
    if let Some(locus) = ctx.last_move_to {
        a *= endpoint_attention(from, locus);
        a *= endpoint_attention(to, locus);
    }
    // Endpoint clutter: visual crowding around the move's two squares.
    let rings = (pseudo_attacks(PieceType::King, from) | pseudo_attacks(PieceType::King, to))
        .without(from)
        .without(to);
    a *= match (rings & pos.occupied()).popcount() {
        0..=6 => 1.0,
        7..=9 => A_CLUTTER_LIGHT,
        _ => A_CLUTTER_HEAVY,
    };

    (s * d * k * o * a).clamp(f64::MIN_POSITIVE, 1.0)
}

/// Attention factor for one endpoint of the move-relation.
fn endpoint_attention(sq: Square, locus: Square) -> f64 {
    match king_distance(sq, locus) {
        0..=2 => 1.0,
        3..=4 => A_NEAR,
        _ => A_FAR,
    }
}

// =========================================================================
// P(see): the margin curve
// =========================================================================

/// Probability the move is seen, given its visibility score and the
/// bot's perception level (`0.0..=1.0`; `>= 1.0` bypasses — always
/// seen). Margin model: `m = p − (1 − V)`, with a perception-scaled
/// plateau (a sharper scan is also a more *reliable* scan).
///
/// `V == 1.0` is an exact special case (factors are discrete; a move
/// with zero difficulty flags has nothing to miss) — so even at
/// `p = 0` the bot still sees every normal move: maximally
/// geometry-blind, never move-blind.
pub fn p_see(v: f64, perception: f64) -> f64 {
    if perception >= 1.0 || v >= 1.0 {
        return 1.0;
    }
    let plateau = 1.0 - (1.0 - PLATEAU_FLOOR) * (1.0 - perception);
    let m = perception + v - 1.0;
    if m >= 0.0 {
        plateau + (1.0 - plateau) * (m / RAMP).min(1.0)
    } else {
        let t = 1.0 + m / CLIFF;
        if t <= 0.0 {
            0.0
        } else {
            plateau * t * t
        }
    }
}

// =========================================================================
// The deterministic roll
// =========================================================================

/// Decide whether the bot sees `mv` at this position. Pure function of
/// `(seed, key, mv, p)`: no ply, no clock — the same position + move
/// resolves identically all game (TT-coherent, stable blind spots).
pub fn sees(seed: u64, key: u64, mv: Move, p: f64) -> bool {
    if p >= 1.0 {
        return true;
    }
    if p <= 0.0 {
        return false;
    }
    roll_unit(seed ^ key.wrapping_mul(VIS_KEY_MUL) ^ ((mv.raw() as u64) << 1)) < p
}

/// Odd multiplier decorrelating the zobrist key from the seed before
/// the SplitMix finalizer (distinct stream from `noise.rs`'s mixes).
const VIS_KEY_MUL: u64 = 0x9E37_79B9_7F4A_7C55;

/// SplitMix64 finalizer → uniform `f64` in `[0, 1)`.
fn roll_unit(x: u64) -> f64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64
}

// =========================================================================
// Search-facing parameter bundle
// =========================================================================

/// Everything the search needs to run the perception filter. Carried
/// on [`crate::engine::SearchParams::perception`]; `None` there is the
/// full-strength bypass (zero cost beyond one branch per move).
#[derive(Copy, Clone, Debug)]
pub struct PerceptionParams {
    /// The dial: `0.0` = maximally geometry-blind, `1.0` = sees
    /// everything (values `>= 1.0` bypass the filter entirely).
    pub level: f32,
    /// Per-game seed for the deterministic rolls — the same seed the
    /// noise profile uses, so one logged seed replays the whole game.
    pub seed: u64,
    /// Destination of the opponent's actual last move at the root
    /// (the attention locus for ply 0; inner plies read it off the
    /// search stack).
    pub last_move_to: Option<Square>,
    /// `guaranteed_mate_in >= 1` contract patch: exempt *root*
    /// checking moves from the filter so a mate-in-1 is always
    /// resolved and the guard can fire. A training feature, not a
    /// realism feature — realism rungs set the guarantee to 0.
    pub exempt_root_checks: bool,
}

/// Line-level findability for retrospective use: the probability a
/// human at `perception` sees every move the line's mover plays
/// through ply `last_ply` (inclusive) — `∏ P(see)` over the mover's
/// plies, with opponent plies contributing nothing (the question is
/// "could the student have found this line," not "would the opponent
/// cooperate"). `mover` is the side to move at `root`.
///
/// Deep quiet combinations compound toward 0 while forcing chains
/// stay findable — the forcing-chain discount, by construction.
pub fn line_findability(
    root: &Position,
    line: &[Move],
    mover: Color,
    last_ply: usize,
    perception: f64,
) -> f64 {
    let mut scratch = root.clone();
    let mut product = 1.0;
    let mut last_to: Option<Square> = None;
    for (ply, &mv) in line.iter().enumerate() {
        if ply > last_ply {
            break;
        }
        if scratch.side_to_move() == mover {
            let ctx = VisibilityContext::at_node(&scratch, last_to);
            product *= p_see(visibility(&scratch, mv, &ctx), perception);
        }
        last_to = Some(mv.to());
        scratch.do_move(mv);
    }
    product
}

#[cfg(test)]
#[path = "visibility_tests.rs"]
mod tests;
