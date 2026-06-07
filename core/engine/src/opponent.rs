//! Per-game opponent configuration.
//!
//! Bundles the toggles that make a bot weaker, varied, or themed —
//! distinct from [`crate::engine::SearchParams`], which controls *how*
//! the engine searches a position. The profile controls *which*
//! engine result actually becomes the bot's move (or whether the bot
//! consults the search at all, in the case of an opening book).
//!
//! Strict invariant: the analytical engine that powers the
//! retrospective, hint panel, and `analyze` REPL command must **never**
//! consult an [`OpponentProfile`]. The student depends on retrospective
//! feedback judging their move against true best play; if the
//! retrospective inherited the bot's opening-book or noise settings,
//! its verdicts would be wrong.
//!
//! Phase A (this commit): the profile exists as an empty default
//! everywhere a play loop runs, but no code path reads it yet —
//! behaviour is identical to before the type was introduced. Phases
//! that follow populate one pillar at a time:
//!
//! - Opening book: bot plays a randomly chosen line from a curated
//!   set until the human deviates, then falls through to search.
//! - Evaluation signal mask: turn off named eval terms so the bot
//!   plays "as if" it didn't know about, say, king safety.
//! - Move noise / blunder injection: occasionally pick a top-K
//!   alternative or a deliberately worse move.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::endgame::EndgameSkill;
use crate::openings::OpeningId;

/// Per-game opponent configuration. See module-level docs.
#[derive(Clone, Debug)]
pub struct OpponentProfile {
    pub book: BookSelection,
    pub noise: NoiseProfile,
    pub eval_mask: EvalMask,
    /// Quiescence-search horizon cap (plies of capture resolution) the
    /// bot searches with — the "tactical vision" lever. `None` = full
    /// tactical sight (normal qsearch); `Some(0)` = tactically blind
    /// (hangs pieces like a sub-600 human, the natural way to make a
    /// believable weak bot instead of forcing statistically-bad moves).
    /// Flows to [`crate::engine::SearchParams::qsearch_max_plies`].
    /// Analytical engines never read it (full vision for true-best-play
    /// feedback), exactly like [`Self::eval_mask`].
    pub qsearch_max_plies: Option<u32>,
    /// How much closed-form endgame knowledge the bot may use — a
    /// difficulty-ordered skill ladder. [`EndgameSkill::Full`] (the
    /// default) knows every technique; lower tiers withhold the harder
    /// specialists so the bot misplays endgames like a human of that
    /// level (shuffles a won KQ, botches KBNK, stalemates). Flows to
    /// [`crate::engine::SearchParams::endgame_skill`]. Analytical engines
    /// never read it (full books for true-best-play feedback), exactly
    /// like [`Self::eval_mask`] / [`Self::qsearch_max_plies`].
    pub endgame_skill: EndgameSkill,
    /// Move-visibility ("perception") dial: `1.0` (default) sees every
    /// move; lower values make geometrically subtle moves — backward
    /// moves, knight punishes, screened rays, moves far from the
    /// action — invisible to the bot's search, deterministically per
    /// game (see [`crate::visibility`]). Flows to
    /// [`crate::engine::SearchParams::perception`] bundled with
    /// [`Self::seed`] and the root attention locus. Analytical engines
    /// never read it, exactly like [`Self::eval_mask`] /
    /// [`Self::qsearch_max_plies`] / [`Self::endgame_skill`].
    pub perception: f32,
    /// Seed for any pseudo-randomness this profile drives — opening
    /// line selection in Phase B, move sampling later. Logged at game
    /// start so a varied game can be replayed exactly by passing the
    /// same seed back in.
    pub seed: u64,
}

impl Default for OpponentProfile {
    /// Behaviour-free profile: no book, no noise, no mask, full
    /// perception. Manual impl (not derived) because `perception`'s
    /// neutral value is `1.0`, not f32's derived `0.0` (which would
    /// mean maximally blind).
    fn default() -> Self {
        Self {
            book: BookSelection::default(),
            noise: NoiseProfile::default(),
            eval_mask: EvalMask::default(),
            qsearch_max_plies: None,
            endgame_skill: EndgameSkill::Full,
            perception: 1.0,
            seed: 0,
        }
    }
}

impl OpponentProfile {
    /// Profile for a new interactive game: random seed + curated
    /// default opening book on. Use [`Self::with_seed`] to reproduce
    /// a previous game by passing back its logged seed.
    pub fn new_random() -> Self {
        Self::with_seed(random_seed())
    }

    /// Profile with a fixed seed and the curated default opening
    /// book. Used both for replaying a logged game and for tests
    /// that want repeatable opening picks. To get a behaviour-free
    /// profile (no book, no noise, no mask) use
    /// [`OpponentProfile::default`] instead — that path is for tests
    /// of code that must not touch the book.
    pub fn with_seed(seed: u64) -> Self {
        Self {
            book: BookSelection::Allowed(crate::book::all_ids()),
            seed,
            ..Self::default()
        }
    }

    /// The [`crate::engine::SearchParams::perception`] bundle for a
    /// play-engine search from this profile, or `None` when the dial
    /// is at full strength. `last_move_to` is the destination of the
    /// opponent's actual last move (the root attention locus).
    pub fn perception_params(
        &self,
        last_move_to: Option<crate::types::Square>,
    ) -> Option<crate::visibility::PerceptionParams> {
        if self.perception >= 1.0 {
            return None;
        }
        Some(crate::visibility::PerceptionParams {
            level: self.perception,
            seed: self.seed,
            last_move_to,
            exempt_root_checks: self.noise.guaranteed_mate_in >= 1,
        })
    }
}

/// Which opening lines, if any, the bot may play out of book.
#[derive(Clone, Debug, Default)]
pub enum BookSelection {
    /// Engine plays from move 1 — no opening book consulted.
    #[default]
    None,
    /// Pick uniformly at game start (seeded by [`OpponentProfile::seed`])
    /// from this set of [`OpeningId`]s. An empty vec is treated the
    /// same as [`BookSelection::None`].
    Allowed(Vec<OpeningId>),
}

/// Move-sampling noise: how often the bot picks a top-K alternative
/// instead of the search's #1 (the variety dial), plus the mate-floor
/// guarantee.
///
/// The explicit `miss_chance` / `blunder_chance` dials were REMOVED
/// 2026-06-07: real misses and blunders are pattern-shaped, not coin
/// flips, and both now emerge organically from the perception lever
/// ([`crate::visibility`]) — a miss is a winning move the bot didn't
/// see; a blunder is an opponent refutation it didn't see (Hope
/// Chess). Residual "tilt" randomness is `avg_move_rank`'s job.
///
/// All fields default to a no-op profile (the bot always plays
/// `lines[0]`).
#[derive(Clone, Debug)]
pub struct NoiseProfile {
    /// Variety dial: the **average rank of the move the bot plays**,
    /// from `1.0` (default — always the engine's #1) up to ~`10.0`. The
    /// picker samples a rank from a normal distribution centred here
    /// with spread `σ = (avg_move_rank − 1.0) × 0.5`, rounds, and clamps
    /// to the available lines. At `1.0` the spread is zero (always #1);
    /// at `3.0` it mostly plays the 2nd–4th best; at `10.0` it ranges
    /// widely. Because only [`crate::noise::NOISE_MULTI_PV`] lines are
    /// surfaced, very high centres skew a little lower than the dial
    /// suggests (the distribution is clamped at the last line).
    pub avg_move_rank: f32,
    /// Smallest mate the bot is **guaranteed** to play through —
    /// blunders are suppressed when `lines[0]` is a mate-in-N for
    /// `N <= guaranteed_mate_in`. Default `1`: mate-in-1 is never
    /// blundered (would look like a bug, not a deliberate weakness).
    /// Set to `0` to allow blunders against any mate; raise it to
    /// guarantee deeper forced sequences are converted.
    pub guaranteed_mate_in: u32,
}

/// MultiPV the play search surfaces when the variety dial is active.
/// A single fixed width because the rank sampler wants "enough of the
/// move list to work with": 10 fits the variety distribution's tail
/// while keeping the per-bot-move cost modest.
pub const NOISE_MULTI_PV: usize = 10;

impl Default for NoiseProfile {
    fn default() -> Self {
        Self {
            avg_move_rank: 1.0,
            guaranteed_mate_in: 1,
        }
    }
}

impl NoiseProfile {
    /// True when the profile cannot pick anything but `lines[0]` —
    /// the play loop uses this to skip the picker entirely. The variety
    /// dial is off at its `1.0` floor (zero spread → always #1).
    pub fn is_off(&self) -> bool {
        self.avg_move_rank <= 1.0
    }

    /// MultiPV the play search should request given this profile.
    /// The variety dial widens to the fixed [`NOISE_MULTI_PV`]; an
    /// off profile keeps the single-PV fast path.
    pub fn effective_multi_pv(&self) -> usize {
        if self.avg_move_rank > 1.0 {
            NOISE_MULTI_PV
        } else {
            1
        }
    }
}

/// Top-level evaluation categories the bot can be made "blind" to.
/// Each category corresponds to one `score +=` line in
/// [`crate::eval::evaluate`]; masking a category zeros its
/// contribution to the running sum. Material and imbalance aren't
/// exposed — disabling them would make the bot play essentially
/// random moves and isn't a meaningful teaching mode.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum EvalCategory {
    PawnStructure = 0,
    Pieces = 1,
    Mobility = 2,
    KingSafety = 3,
    Threats = 4,
    PassedPawns = 5,
    Space = 6,
    Initiative = 7,
}

impl EvalCategory {
    pub const ALL: [EvalCategory; 8] = [
        EvalCategory::PawnStructure,
        EvalCategory::Pieces,
        EvalCategory::Mobility,
        EvalCategory::KingSafety,
        EvalCategory::Threats,
        EvalCategory::PassedPawns,
        EvalCategory::Space,
        EvalCategory::Initiative,
    ];

    /// Stable, lowercase, kebab-case identifier for CLI input /
    /// settings persistence (`king-safety`, `pawn-structure`, ...).
    pub fn slug(self) -> &'static str {
        match self {
            EvalCategory::PawnStructure => "pawn-structure",
            EvalCategory::Pieces => "pieces",
            EvalCategory::Mobility => "mobility",
            EvalCategory::KingSafety => "king-safety",
            EvalCategory::Threats => "threats",
            EvalCategory::PassedPawns => "passed-pawns",
            EvalCategory::Space => "space",
            EvalCategory::Initiative => "initiative",
        }
    }

    /// Reverse of [`Self::slug`]. Returns `None` for unrecognised
    /// input.
    pub fn from_slug(s: &str) -> Option<EvalCategory> {
        EvalCategory::ALL.iter().copied().find(|c| c.slug() == s)
    }
}

/// Bitset over [`EvalCategory`]. Disabled categories contribute zero
/// to the bot's evaluation, simulating an opponent who doesn't
/// understand that concept (e.g. mask off pawn structure to spar
/// against a sub-1200 positional player).
///
/// Default-empty mask is the unbiased eval — every category
/// contributes. Empty masks are the hot path; the gating branches in
/// [`crate::eval::evaluate`] should fold to near-zero overhead under
/// branch prediction.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash)]
pub struct EvalMask(u8);

impl EvalMask {
    pub const EMPTY: EvalMask = EvalMask(0);

    /// True when no categories are disabled — the eval runs unbiased.
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// True when `c` is masked off and should contribute zero.
    pub fn is_disabled(self, c: EvalCategory) -> bool {
        (self.0 >> c as u8) & 1 == 1
    }

    pub fn disable(&mut self, c: EvalCategory) {
        self.0 |= 1 << c as u8;
    }

    pub fn enable(&mut self, c: EvalCategory) {
        self.0 &= !(1 << c as u8);
    }

    /// Iterate the categories that are currently disabled, in
    /// [`EvalCategory::ALL`] order.
    pub fn disabled_iter(self) -> impl Iterator<Item = EvalCategory> {
        EvalCategory::ALL
            .into_iter()
            .filter(move |c| self.is_disabled(*c))
    }
}

/// Mix the wall clock with a process-wide monotonic counter through
/// SplitMix64. The counter guarantees uniqueness across same-nanosecond
/// calls (Windows clock resolution can be coarse enough to collide
/// otherwise); the clock contributes per-process entropy so two CLI
/// runs don't share the same seed sequence.
fn random_seed() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0xC0FF_EE15_BEEF_F00D);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut x = nanos ^ counter.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_empty_noop() {
        // `Default` is the no-behaviour constructor — used by tests
        // of code that must not touch the book.
        let p = OpponentProfile::default();
        assert!(matches!(p.book, BookSelection::None));
        assert_eq!(p.seed, 0);
    }

    #[test]
    fn with_seed_preserves_seed_and_enables_curated_book() {
        let p = OpponentProfile::with_seed(0xDEAD_BEEF);
        assert_eq!(p.seed, 0xDEAD_BEEF);
        match &p.book {
            BookSelection::Allowed(ids) => assert!(!ids.is_empty()),
            BookSelection::None => panic!("with_seed should enable the curated book"),
        }
    }

    #[test]
    fn new_random_yields_distinct_seeds_across_calls() {
        // Two back-to-back calls should mix the clock differently
        // enough to avoid collision. If this ever flakes, the mix
        // function is broken.
        let a = OpponentProfile::new_random().seed;
        let b = OpponentProfile::new_random().seed;
        assert_ne!(a, b);
    }

    #[test]
    fn eval_mask_starts_empty_and_round_trips() {
        let mut m = EvalMask::EMPTY;
        assert!(m.is_empty());
        for c in EvalCategory::ALL {
            assert!(!m.is_disabled(c));
        }
        m.disable(EvalCategory::KingSafety);
        m.disable(EvalCategory::PawnStructure);
        assert!(!m.is_empty());
        assert!(m.is_disabled(EvalCategory::KingSafety));
        assert!(m.is_disabled(EvalCategory::PawnStructure));
        assert!(!m.is_disabled(EvalCategory::Mobility));
        m.enable(EvalCategory::KingSafety);
        assert!(!m.is_disabled(EvalCategory::KingSafety));
        let disabled: Vec<_> = m.disabled_iter().collect();
        assert_eq!(disabled, vec![EvalCategory::PawnStructure]);
    }

    #[test]
    fn eval_category_slug_round_trips() {
        for c in EvalCategory::ALL {
            assert_eq!(EvalCategory::from_slug(c.slug()), Some(c));
        }
        assert_eq!(EvalCategory::from_slug("nope"), None);
    }

    #[test]
    fn default_noise_profile_is_off() {
        let n = NoiseProfile::default();
        assert!(n.is_off(), "default noise must be a no-op (always #1)");
        assert_eq!(
            n.effective_multi_pv(),
            1,
            "off-profile keeps the single-PV fast path"
        );
    }

    #[test]
    fn noise_profile_off_at_variety_floor() {
        // avg_move_rank at its 1.0 floor with no mistake knobs is off.
        let n = NoiseProfile {
            avg_move_rank: 1.0,
            ..Default::default()
        };
        assert!(n.is_off());
        assert_eq!(n.effective_multi_pv(), 1);
    }

    #[test]
    fn noise_profile_on_when_variety_above_floor() {
        let n = NoiseProfile {
            avg_move_rank: 3.0,
            ..Default::default()
        };
        assert!(!n.is_off());
        assert_eq!(n.effective_multi_pv(), NOISE_MULTI_PV);
    }

}
