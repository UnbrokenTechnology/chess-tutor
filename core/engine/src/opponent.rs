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

use crate::openings::OpeningId;

/// Per-game opponent configuration. See module-level docs.
#[derive(Clone, Debug, Default)]
pub struct OpponentProfile {
    pub book: BookSelection,
    pub noise: NoiseProfile,
    pub eval_mask: EvalMask,
    /// Seed for any pseudo-randomness this profile drives — opening
    /// line selection in Phase B, move sampling later. Logged at game
    /// start so a varied game can be replayed exactly by passing the
    /// same seed back in.
    pub seed: u64,
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
            noise: NoiseProfile::default(),
            eval_mask: EvalMask::default(),
            seed,
        }
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
/// instead of the search's #1, plus the probability of a deliberate
/// worse-than-good move ("blunder").
///
/// All fields default to a no-op profile (the bot always plays
/// `lines[0]`). The CLI / desktop layers expose individual knobs so a
/// student can dial up variety ("don't see the same Italian Game line
/// every time") or exploitable mistakes ("give me practice spotting
/// blunders") without weakening one to weaken the other.
#[derive(Clone, Debug)]
pub struct NoiseProfile {
    /// How many top search lines the sampler may pick from when softmax
    /// noise fires. `1` (default) disables the softmax branch entirely
    /// — the bot just plays `lines[0]`. Larger values cost roughly K×
    /// the search time because the engine runs K iterative-deepening
    /// passes (one per PV slot).
    pub candidate_pool: usize,
    /// Softmax temperature in centipawns over the score gap from #1.
    /// `0` (default) collapses to "always pick #1" even when
    /// [`Self::candidate_pool`] > 1. Higher values flatten the
    /// distribution: at `temperature_cp = 50` a #2 line that's 50 cp
    /// behind has weight `e^-1 ≈ 0.37` relative to #1; at
    /// `temperature_cp = 200` the same line weights `e^-0.25 ≈ 0.78`.
    pub temperature_cp: i32,
    /// Probability per move of deliberately dropping a "blunder" —
    /// picking uniformly from lines whose score loss vs #1 falls in
    /// the band `[blunder_min_loss_cp, blunder_max_loss_cp]`. `0.0`
    /// (default) disables the branch. Setting this > 0 widens the
    /// requested MultiPV to [`BLUNDER_POOL_MIN`] so the engine
    /// surfaces enough worse-than-best alternatives to sample from.
    pub blunder_chance: f32,
    /// Minimum loss (centipawns vs #1) for a line to count as "in
    /// band" for the blunder picker. Default `100` cp — a clear
    /// pawn-down move the student can plausibly recognise and punish.
    pub blunder_min_loss_cp: i32,
    /// Maximum loss (centipawns vs #1) for a line to count as "in
    /// band". Default `400` cp — caps blunders at roughly an exchange
    /// sacrifice, which prevents the cheesy "bot hangs its queen for
    /// no reason" outcome that broke immersion at higher blunder
    /// rates. Raise to allow more catastrophic blunders (~900 for
    /// queen hangs, [`i32::MAX`] for unbounded).
    ///
    /// The two thresholds define a **preference band**, not a hard
    /// filter. If a roll fires but no line falls inside the band
    /// (every alternative is either too good or too bad), the picker
    /// pools the line(s) closest to the band from below (loss <
    /// min, i.e. moves that aren't blundery enough) with the line(s)
    /// closest from above (loss > max, i.e. moves that are too
    /// catastrophic) and picks uniformly from that pool. Lines
    /// further from the band on either side are excluded — that's
    /// the load-bearing property that lets a bot do "small blunders
    /// only" without throwing away a piece when the only sub-band
    /// alternative is a piece sacrifice.
    pub blunder_max_loss_cp: i32,
    /// Smallest mate the bot is **guaranteed** to play through —
    /// blunders are suppressed when `lines[0]` is a mate-in-N for
    /// `N <= guaranteed_mate_in`. Default `1`: mate-in-1 is never
    /// blundered (would look like a bug, not a deliberate weakness).
    /// Set to `0` to allow blunders against any mate; raise it to
    /// guarantee deeper forced sequences are converted.
    pub guaranteed_mate_in: u32,
    /// Probability per move the bot picks **uniformly from all legal
    /// moves**, bypassing the search ranking entirely. Distinct from
    /// [`Self::blunder_chance`]: that branch picks from the engine's
    /// top-K worse-but-still-considered alternatives; this branch can
    /// pick a move the engine didn't even surface, including
    /// genuinely beginner-level mistakes like leaving a piece in a
    /// pawn's path. `0.0` (default) disables the branch.
    ///
    /// Mate-guarded the same as [`Self::blunder_chance`] — bot's
    /// `guaranteed_mate_in` shorter mates are never bypassed.
    pub wild_chance: f32,
}

/// Minimum MultiPV the play search needs when blunders are enabled.
/// Blunder candidates are filtered by score severity; the top 2–3
/// lines are usually too close in score to qualify, so we widen the
/// request to surface deliberately worse alternatives.
pub const BLUNDER_POOL_MIN: usize = 6;

impl Default for NoiseProfile {
    fn default() -> Self {
        Self {
            candidate_pool: 1,
            temperature_cp: 0,
            blunder_chance: 0.0,
            blunder_min_loss_cp: 100,
            blunder_max_loss_cp: 400,
            guaranteed_mate_in: 1,
            wild_chance: 0.0,
        }
    }
}

impl NoiseProfile {
    /// True when the profile cannot pick anything but `lines[0]` —
    /// the play loop uses this to skip the picker entirely.
    ///
    /// Temperature alone has no effect when `candidate_pool == 1` (the
    /// softmax has a one-element pool), so we treat a single-slot pool
    /// with no blunder *and* no wild as off regardless of temperature.
    pub fn is_off(&self) -> bool {
        self.blunder_chance <= 0.0 && self.wild_chance <= 0.0 && self.candidate_pool <= 1
    }

    /// MultiPV the play search should request given this profile. The
    /// softmax branch needs `candidate_pool` slots; the blunder branch
    /// needs at least [`BLUNDER_POOL_MIN`]. Off-profile collapses to
    /// `1` so the engine keeps its single-PV fast path.
    pub fn effective_multi_pv(&self) -> usize {
        let pool = self.candidate_pool.max(1);
        if self.blunder_chance > 0.0 {
            pool.max(BLUNDER_POOL_MIN)
        } else {
            pool
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
        EvalCategory::ALL.into_iter().filter(move |c| self.is_disabled(*c))
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
        assert_eq!(n.effective_multi_pv(), 1, "off-profile keeps the single-PV fast path");
    }

    #[test]
    fn noise_profile_off_when_pool_one_and_no_blunder() {
        let n = NoiseProfile {
            candidate_pool: 1,
            temperature_cp: 500, // temperature alone with pool=1 is still off
            blunder_chance: 0.0,
            ..Default::default()
        };
        assert!(n.is_off());
    }

    #[test]
    fn noise_profile_on_when_pool_above_one() {
        let n = NoiseProfile {
            candidate_pool: 3,
            temperature_cp: 50,
            ..Default::default()
        };
        assert!(!n.is_off());
        assert_eq!(n.effective_multi_pv(), 3);
    }

    #[test]
    fn noise_profile_blunder_widens_to_minimum_pool() {
        let n = NoiseProfile {
            candidate_pool: 1,
            blunder_chance: 0.1,
            ..Default::default()
        };
        assert!(!n.is_off());
        assert_eq!(n.effective_multi_pv(), BLUNDER_POOL_MIN);
    }

    #[test]
    fn noise_profile_blunder_respects_user_pool_when_larger() {
        let n = NoiseProfile {
            candidate_pool: 10,
            blunder_chance: 0.1,
            ..Default::default()
        };
        // candidate_pool > BLUNDER_POOL_MIN — keep the user's choice.
        assert_eq!(n.effective_multi_pv(), 10);
    }
}
