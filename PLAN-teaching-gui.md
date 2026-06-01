# PLAN: surfacing the CLI's teaching power in the GUI (no LLM, no NN)

Design brief for porting the agent-via-CLI teaching capability into the GUI's
three learning modes, **without an AI agent, language model, or neural net.**
Follows the retired-PLAN convention (`PLAN-cli.md`, `PLAN-tactic-escape.md`):
this is a forward-looking design doc; retire it to git history once the work
lands, moving rationale into the relevant module `//!` docs.

> Read first: [`CLAUDE.md`](CLAUDE.md) (mission, ground rules),
> [`HANDOFF-ux.md`](HANDOFF-ux.md) (current teaching-layer state),
> the five [`teaching-positions/`](teaching-positions/) case studies (the
> regression targets this plan must satisfy).

---

## Why this plan exists

We have reached a good place on one axis: **an AI agent, using the CLI, can
explain a position that confused a 1200-rated player.** It works because the
agent reads the CLI top-down — the `danger:` block (the opponent's loaded
threat) is physically first, and the agent indexes a played move on the *move*
via `critique`, not on the position. The CLI's discipline lives in the agent's
reading order.

The GUI has no agent. So that reading order — *resolve the opponent's threats
first, then judge your move, and only talk positional chess when nothing
tactical is live* — has to be **baked into which cards show and in what
order.** This plan does that.

## The core principle (decided with the user)

**The "is this position tactically live?" gate is detectors-only.** A position
is tactically live for teaching purposes **iff a named, human-findable pattern
fires** (latent threat, check-followup, hanging piece, in-check, a tactic we
can play, a self-replenishing check chain). We deliberately **do not** use a
static-vs-quiescence eval delta as a gate.

Rationale (the user's, and it is load-bearing): if no named pattern fires and
the only signal is "search disagrees with static eval," then *a human could
never have seen the position was tactical* — only an engine notices. There is
nothing teachable there. A "quiet tactical position" is engine-only noise;
surfacing it would teach nothing and risks the chess.com failure mode of
narrating a mechanism the student can't act on.

This single decision unifies the whole design:

- It defines the **tactical-mode gate** (below) that every surface consults.
- It **subsumes silent-sequencing**: the `…Qc8` case fires no detector, so it
  reads as quiet, so no tactic card appears — exactly what
  [`silent-sequencing-after-qc8`](teaching-positions/silent-sequencing-after-qc8.md)
  demands. We get its humility for free instead of building a separate
  suppressor for the *coaching* surface (the *retrospective* still needs a
  small depth-honesty note — see §4).

## The gap, precisely

Three gaps, each independently confirmed by the case studies:

1. **No tactical-vs-positional gate exists in the GUI.** `build_coaching_view`
   surfaces pawn-weakness cards unconditionally, alongside tactic cards. That is
   the keystone-principle violation: positional advice while a tactic is in
   charge. (memory `project_tactical_vs_positional_modes`)
2. **Coaching consumes neither `find_latent_threats` nor `find_check_followups`.**
   These are CLI-only today. They are the detectors that catch the *opponent's
   standing threat against the user* — the exact thing every case study turns on.
3. **The retrospective lacks the `critique` logic** — no `gave_away_advantage` /
   ALLOWED-not-MISSED reframe, no static-vs-search override note, and
   `compute_tactic_outcome.user_walked_into` still requires the opponent to
   *actually play* the tactic (latent threats not wired into it).

### What's already engine-available (no new engine work to consume it)

| API | Module | Detects |
|---|---|---|
| `find_latent_threats(pos, defender_color) -> Vec<LatentThreat>` | `analysis/latent_threats.rs` | opponent's loaded DiscoveredAttack / Pin / RelativePin / Skewer / RemovingDefender |
| `find_check_followups(pos, mover, prior) -> Vec<CheckFollowup>` | `analysis/check_followups.rs` | a check whose forced reply leaves a follow-up tactic (two-step fork) |
| `find_best_tactic_in_position(pos, mover, prior) -> Option<TacticHit>` | `analysis/tactic_outcome/` | a tactic *we* can play now |
| `find_overloaded(pos, victim) -> Vec<OverloadedPiece>` | `analysis/overloading.rs` | enemy defender doing two jobs |
| `list_hanging` / `list_see_losing` | `analysis/threats_outcome.rs` | loose / SEE-losing pieces, both sides |
| `gave_away_advantage(best, forced)` (logic) | `core/cli/src/main.rs` | a forced move that handed over a winning/equal position |
| `win_chances(Value) -> f64` | `analysis/win_chances.rs` | cp → win% (gate thresholds in human terms) |

### New engine work this plan needs (small)

- **`analysis/tactical_mode.rs`** — the shared gate predicate (§1). Pure
  composition of the detectors above; no search.
- **`analysis/forcing_check_chain.rs`** — generalize `check_followups` to
  "after my reply, is *another* check available, and after that another?" Report
  the chain depth. ≥3 self-replenishing checks → a soft, mechanism-free warning
  (the [`mating-net-after-ng5`](teaching-positions/mating-net-after-ng5.md)
  case; the user's "three-checks-deep" rule is itself a *human-findable*
  detector, so it belongs under detectors-only).

---

## 1. The tactical-mode gate (shared spine)

New `analysis/tactical_mode.rs`, consumed by every UI surface so the three
renderers agree:

```rust
pub struct TacticalState {
    pub live: bool,
    /// Nameable causes, highest-priority first. Card builders render these
    /// directly; the ordering here is the card ordering.
    pub reasons: Vec<TacticalReason>,
}

pub enum TacticalReason {
    InCheck,                                  // you must respond now
    OpponentLatentThreat(LatentThreat),       // they have it loaded against you
    OpponentCheckFollowup(CheckFollowup),     // their check is the first half of a fork
    ForcingCheckChain { depth: u8 },          // ≥3 self-replenishing checks at your king
    OurTactic(TacticHit),                     // you have a combination available
    LoosePiece(/* side, square */),           // hanging / SEE-losing, either side
}

/// `user_color` is the side the student plays; pass it as both the live STM
/// (coaching is always the user's turn) and the `defender_color` for the
/// opponent-threat scans. `prior_move` feeds the recapture guard.
pub fn classify_tactical_mode(
    pos: &Position,
    user_color: Color,
    prior_move: Option<PriorMove>,
) -> TacticalState;
```

`live == !reasons.is_empty()`. Detectors-only: **no quiescence delta.** Cost is
the sum of the static scans (all sub-ms in release), so it can run every frame
like the rest of coaching.

---

## 2. Coached mode — pre-move "what to notice"

`build_coaching_view` gains the gate. When `TacticalState.live`:

1. Emit cards from `reasons`, **in `reasons` order** (priority above):
   - `InCheck` → existing `check_card`.
   - `OpponentLatentThreat` → **new** `latent_threat_card`: "Your opponent has a
     *discovered attack* loaded — a move that doesn't address it lets them fire
     it." Name the pattern; **withhold the squares** (pre-move pedagogical rule).
   - `OpponentCheckFollowup` → **new** `check_followup_card`: "Their `…Nd3+`
     isn't a stall — look one ply past the check: after your reply they have a
     fork. Defuse it before you do anything else." (No squares.)
   - `ForcingCheckChain` → **new** `king_hunt_card`: soft, **mechanism-free**:
     "Your king faces a forcing check sequence at least three deep — these tend
     to end in a mating net or a perpetual. Look for a more defensive move."
     Never names a mate or a line.
   - `OurTactic` → existing `tactic_card`.
   - `LoosePiece` → existing `opportunity_card` / `risk_card`.
2. **Positional cards collapse under a muted fold** titled e.g. *"Quiet-position
   notes — not the priority right now"* (pawn weakness, space, outposts). They
   stay available (honest about information) but visibly demoted.

When `!live`: positional cards lead (today's behavior), and the fold is gone.
This is where the bind/outpost teaching lives
([`positional-punish-after-qe6`](teaching-positions/positional-punish-after-qe6.md)
once `…Nxe4` is defused — but note in *that* position the gate is still live
until `…Nxe4` is addressed, so the positional bind narration belongs in the
retrospective, not pre-move coaching).

Pedagogical rules preserved: pre-move coaching names **no squares** on the new
opponent-threat cards (same as `tactic_card`), only the pattern + the "address
it first" instruction.

---

## 3. Supported mode — pause on mistake

Augment `classify_user_move` / `intervention_required`:

- **ALLOWED-not-MISSED pause** — port `gave_away_advantage(best, played)` logic
  into the classifier (already validated in `critique`). When the user's move
  swung a winning/equal position into the opponent's favor **and** a detector
  explains it (a `user_walked_into` tactic, or — see §4 wiring — a latent threat
  they failed to address), the intervention prompt uses the *ALLOWED* framing:
  *"your move let your opponent do something — what did you let them do?"* not
  *"you missed a better move."* (memory `project_defusal_and_allowed_banner`.)
- **Silent-sequencing suppressor (the fourth gate)** — do **not** pause when the
  bad eval needs depth and **no detector fires.** Concretely the
  [`silent-sequencing-after-qc8`](teaching-positions/silent-sequencing-after-qc8.md)
  two-depth check: gap small at shallow depth (≈6) + large at full depth + no
  tactic/latent/overload detector → suppress. Detectors-only already makes
  coaching silent here; this makes the *pause* silent too. Interrupting play
  with an unactionable verdict is worse than not interrupting.

---

## 4. Practicing mode — retrospective (and shared retrospective upgrades)

Same content, no pause. Three retrospective-only narration requirements come
straight from the case studies; all three are currently missing.

1. **ALLOWED-not-MISSED reframe + latent wiring.** Wire `find_latent_threats`
   into `compute_tactic_outcome`'s `user_walked_into` slot so the retrospective
   fires *pre-emptively* against a move that failed to disrupt a standing
   alignment — today it only fires if the opponent actually plays the tactic.
   Lead the card with the swing and "what you allowed," mirroring
   `print_allowed_banner`. (HANDOFF-ux "Latent-threat retrospective wiring";
   [`discovered-attack-after-qxe6`](teaching-positions/discovered-attack-after-qxe6.md).)
2. **Static-vs-search override note (the hard one).** When the recommended move
   is a *static downgrade but a search upgrade*, say so explicitly — never
   invent a positional justification. Compare the term-delta direction against
   the search-score direction; when they disagree, emit:
   *"the term breakdown would tell you to castle and keep the attack; the search
   overrules it because castling lets `…Nxe4` in and the attack was an
   illusion."* If the GUI ever calls `Ne3` "positionally strong," the layer is
   lying.
   ([`positional-punish-after-qe6`](teaching-positions/positional-punish-after-qe6.md),
   "The static ledger lies here.")
3. **Silent-sequencing depth-honesty note.** When the suppressor (§3) fires,
   the retrospective shows the visible static-counting facts + an honest
   *"the engine sees this getting difficult over the next several moves, but the
   reason is beyond practical calculation depth — there isn't a shorter lesson
   here."* No "blunder" stamp, no fabricated mechanism.

Also adopt where relevant:
- **Desperado-aware material narration** — when a piece is lost, account for
  whether it can cash itself for a pawn first (`Nxg7+`): narrate "−1.0 becomes
  0.0 because of the desperado," not "you're fine."
  ([`positional-punish-after-qe6`](teaching-positions/positional-punish-after-qe6.md),
  the desperado safety-net table.)
- **`win_chances` thresholds** — express the gate in win%-lost (with the
  absolute-cp backstop; `win_chances` saturates near 1.0 in winning positions).
  (memory `project_win_chances_adoption`, `feedback_winning_position_saturation`.)

---

## 5. Regression matrix (acceptance criteria)

Each row is a case study; the cells are the expected GUI behavior per mode. This
is the done-definition for the work.

| Position | Detector that fires | Coached (pre-move) | Supported (pause?) | Practicing (retrospective) |
|---|---|---|---|---|
| [discovered-attack-after-qxe6](teaching-positions/discovered-attack-after-qxe6.md) | `find_latent_threats` → DiscoveredAttack | "opponent has a discovered attack loaded — address it" card; positional cards folded | pause: ALLOWED — `Qc5+` let `…Bxh2+`/`…Qxe1` fire | walked-into card via latent wiring; names the alignment |
| [positional-punish-after-qe6](teaching-positions/positional-punish-after-qe6.md) | `find_latent_threats` → RemovingDefender (`…Nxe4`) | "opponent threatens `…Nxe4` (removes Nf5's defender) — deny it" card; positional folded | pause on `O-O` (allowed `…Nxe4`) | **static-vs-search override note** + desperado-aware material; never "Ne3 is positionally strong" |
| [double-fork-after-qd8](teaching-positions/double-fork-after-qd8.md) | `find_check_followups` → `…Nd3+`→`…Nf2` | "their check is the first half of a fork — look one ply past it" card | pause if the user fails to defuse | check-followup card naming Fork; suggests "address the c5 knight" |
| [silent-sequencing-after-qc8](teaching-positions/silent-sequencing-after-qc8.md) | **none** (correct) | **no tactic card** — reads as quiet (detectors-only) | **no pause** (silent-sequencing suppressor) | static-counting facts + depth-honesty note; **no blunder stamp** |
| [mating-net-after-ng5](teaching-positions/mating-net-after-ng5.md) | `forcing_check_chain` (≥3 deep) | soft **mechanism-free** king-hunt warning | pause: soft warning, no fabricated mechanism | "long forced sequence, fragile king; reason beyond summary" — no fake named tactic |

A future check: run the gate on `silent-sequencing-after-qc8` → empty `reasons`;
run it on `double-fork-after-qd8` → an `OpponentCheckFollowup`. Those two are the
calibration bookends (over-tuning vs under-tuning).

---

## 6. Build order (proposed)

1. **`analysis/tactical_mode.rs`** + tests (the gate; pure composition).
2. **Coached wiring** — `latent_threat_card`, `check_followup_card`, the
   positional-card fold. Highest validated value (fixes 3 of 5 positions
   pre-move). New `AnnotationKind`? No — coaching names no squares.
3. **`forcing_check_chain.rs`** + `king_hunt_card` (the ng5 soft warning).
4. **Retrospective upgrades** — latent→`user_walked_into` wiring + ALLOWED
   reframe; then the static-vs-search override note; then the silent-sequencing
   depth-honesty note + suppressor in the classifier.
5. **Supported-mode pause gates** — ALLOWED pause + silent-sequencing suppressor.

Land each as its own change, A/B against the regression matrix
(memory `feedback_pruning_bundles`: never bundle).

## 7. Open questions / deferred

- **`ForcingCheckChain` depth threshold** — the writeup says "≥3 checks deep."
  Confirm 3 in real play; it may want tuning per king-exposure.
- **Card fold UX** — desktop egui collapsing section vs a dimmed always-visible
  list. Renderer-neutral data; decide in `draw::*`, not `core/ui`.
- **Latent-threat min_gain in the retrospective** — `find_latent_threats` uses a
  permissive `min_gain` (value-of-exposed-piece, not full SEE). Watch for
  over-firing once it drives a *pause*; tighten with a second-pass search if so.
- **Static-vs-search override mechanics** — exactly which term aggregate to
  compare against the search score (the per-term tapered net total vs the search
  PV score). Needs a short spike against the `positional-punish` FEN.
