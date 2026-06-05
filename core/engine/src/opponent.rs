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
#[derive(Clone, Debug, Default)]
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
            qsearch_max_plies: None,
            endgame_skill: EndgameSkill::Full,
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
    /// Probability per move of deliberately dropping a "blunder" — a
    /// move that, by force, **loses material** (the bot ends up down
    /// material at the resolved end of the line), with the amount of
    /// material hung falling in the band
    /// `[blunder_min_material_cp, blunder_max_material_cp]`. `0.0`
    /// (default) disables the branch. Setting this > 0 widens the
    /// requested MultiPV to [`crate::noise::NOISE_MULTI_PV`] so the
    /// engine surfaces enough candidate moves to find a material-losing
    /// one.
    ///
    /// This is the chess.com sense of "blunder" (added 2023): a move
    /// that *loses your own material*, as distinct from a [`miss`](
    /// Self::miss_chance), which merely fails to capitalise on a
    /// material-winning chance. We classify by the actual material
    /// outcome of the line, not by centipawn swing — a 1.5-pawn
    /// *positional* drop is not a blunder.
    pub blunder_chance: f32,
    /// Smallest material loss (in material-centipawns, where a pawn =
    /// `100` and standard values apply — N/B = 300, R = 500, Q = 900)
    /// for a move to count as an "in band" blunder. Default `100` — a
    /// hung pawn, the lightest mistake a student can cleanly punish.
    pub blunder_min_material_cp: i32,
    /// Largest material loss (material-centipawns, pawn = `100`) for a
    /// move to count as "in band". Default `400` — caps deliberate
    /// blunders at roughly a minor-piece-and-pawn / exchange, so the
    /// bot won't cheesily gift its queen at high blunder rates. Raise
    /// toward `900` to permit hanging heavier material.
    ///
    /// The two thresholds are a **preference band**: when a roll fires
    /// but no material-losing move sits inside it, the picker takes
    /// the line(s) whose material loss is closest to the band (from
    /// below, then above), so the bot still registers a blunder
    /// without lurching to a wildly out-of-band sacrifice.
    pub blunder_max_material_cp: i32,
    /// Probability per move of deliberately playing a "miss" — when a
    /// move is available that *wins material* by force, the bot
    /// refuses it and plays the best move that does **not** win
    /// material (even if that move is itself losing). `0.0` (default)
    /// disables the branch. The roll only has an effect when a
    /// material-winning move actually exists in the searched lines;
    /// otherwise there is nothing to miss.
    ///
    /// This is the chess.com "Miss" — failing to capitalise on a
    /// tactic — kept separate from [`blunder_chance`](
    /// Self::blunder_chance) (losing your own material) because they
    /// are different mistakes a student learns to exploit differently.
    /// Mate-guarded like the other branches.
    pub miss_chance: f32,
    /// Smallest mate the bot is **guaranteed** to play through —
    /// blunders are suppressed when `lines[0]` is a mate-in-N for
    /// `N <= guaranteed_mate_in`. Default `1`: mate-in-1 is never
    /// blundered (would look like a bug, not a deliberate weakness).
    /// Set to `0` to allow blunders against any mate; raise it to
    /// guarantee deeper forced sequences are converted.
    pub guaranteed_mate_in: u32,
}

/// MultiPV the play search surfaces whenever any line-based noise
/// (variety / blunder / miss) is active. A single fixed width — rather
/// than a user-tunable pool — because all three consumers want "enough
/// of the move list to work with": the variety dial samples a rank
/// within it, and blunder/miss classify material across it. 10 is wide
/// enough that genuine punishable hangs (which rank deep in quiet
/// positions) and the variety distribution's tail both fit, while
/// keeping the per-bot-move cost modest.
pub const NOISE_MULTI_PV: usize = 10;

impl Default for NoiseProfile {
    fn default() -> Self {
        Self {
            avg_move_rank: 1.0,
            blunder_chance: 0.0,
            blunder_min_material_cp: 100,
            blunder_max_material_cp: 400,
            miss_chance: 0.0,
            guaranteed_mate_in: 1,
        }
    }
}

impl NoiseProfile {
    /// True when the profile cannot pick anything but `lines[0]` —
    /// the play loop uses this to skip the picker entirely. The variety
    /// dial is off at its `1.0` floor (zero spread → always #1).
    pub fn is_off(&self) -> bool {
        self.blunder_chance <= 0.0 && self.miss_chance <= 0.0 && self.avg_move_rank <= 1.0
    }

    /// True when a branch that reads the ranked line list is active
    /// (variety, blunder, or miss) — i.e. every active branch.
    fn needs_lines(&self) -> bool {
        self.avg_move_rank > 1.0 || self.blunder_chance > 0.0 || self.miss_chance > 0.0
    }

    /// MultiPV the play search should request given this profile.
    /// Line-based noise widens to the fixed [`NOISE_MULTI_PV`]; a
    /// wild-only or off profile keeps the single-PV fast path.
    pub fn effective_multi_pv(&self) -> usize {
        if self.needs_lines() {
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

    #[test]
    fn noise_profile_blunder_widens_to_noise_multi_pv() {
        let n = NoiseProfile {
            blunder_chance: 0.1,
            ..Default::default()
        };
        assert!(!n.is_off());
        assert_eq!(n.effective_multi_pv(), NOISE_MULTI_PV);
    }

    #[test]
    fn noise_profile_miss_widens_to_noise_multi_pv() {
        let n = NoiseProfile {
            miss_chance: 0.2,
            ..Default::default()
        };
        assert!(!n.is_off());
        assert_eq!(n.effective_multi_pv(), NOISE_MULTI_PV);
    }
}
