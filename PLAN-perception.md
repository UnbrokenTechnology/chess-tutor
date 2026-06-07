# PLAN — move-perception lever + settled-ply redesign

**Status:** DESIGN AGREED 2026-06-06, instrumentation starting. Companion to
[`PLAN-elo-calibration.md`](PLAN-elo-calibration.md) and
[`HANDOFF-weak-bot-tuning.md`](HANDOFF-weak-bot-tuning.md). This work is
**grid-blocking by choice**: the full calibration grid is the expensive
artifact, and this lever changes weak-bot strength, so it lands (and is
feel-frozen) before the grid runs. The current `bands.txt` rungs are
trustworthy for the *current* lever set; they will be re-derived after this
lands.

---

## The idea

A **move-visibility ("perception") lever**: weight the likelihood that the
bot considers a move by *how hard it is for a human to see*. Two signal
families:

- **Geometric / immediate:** backward and sideways moves are harder to see
  than forward; long-range moves harder than short; diagonal moves passing
  through traffic harder; knight retreats notoriously hard.
- **Tactical depth:** a payoff N plies away is harder to see than an
  immediate one (a move whose point arrives at ply 5 vs a simple recapture).

Dual use — one scorer, two consumers:

1. **Bot believability:** a low-ELO bot stops sniping you across the board
   through a pawn chain, stops fearing threats no human at that level would
   see, and misses the *human-missed* moves.
2. **Retrospective fairness:** "don't punish the student for a move that was
   not humanly visible" — refactors the existing depth-honesty / verdict
   demotion heuristics onto one principled signal.

## Decisions locked (2026-06-06, with the user)

1. **One scalar difficulty score, one continuous dial.**
   `move_difficulty(pos, mv, line_ctx) -> f64` lives in engine `analysis/`
   (shared by bot + retrospective). Internal weights (direction, distance,
   mover type, ray traffic, payoff depth) are **fixed constants** — like
   `CAPTURE_RESCUE_C` / `SELF_HANG_C` — not user knobs. The user-facing dial
   is a single `perception` (0..=1) on `OpponentProfile`.
   `P(see) = f(perception, difficulty)`, probabilistic not hard-threshold
   (no cliff, no identical blind spot every game). GUI = one slider in the
   advanced dropdown (can present labeled stops; internal stays continuous
   so the ELO solver interpolates).
   - REJECTED: per-category checkboxes (GUI clutter, 2^k grid blowup, no
     strength ordering for the solver, incoherent configs).
   - REJECTED: folding into `avg_move_rank` (would recreate the 700→800
     rung artifact — rank goes UP as qsearch goes up, so conflated
     visibility would *flip* across rungs; would also invalidate every
     measured rank curve and undo the just-finished lever orthogonality:
     hangs come only from qsearch-blindness or the blunder dial, never
     incidentally).
   - Enum levels OK as GUI presentation only, never internal representation.

2. **In-search filter, not a post-search noise-layer filter.** "Not seeing
   a move" = never considering it or its descendants — it is *pruning*, and
   may make the (already tiny d1–d4) weak-bot searches faster. Decisive
   advantages over filtering MultiPV lines after the fact:
   - The engine genuinely never returns the snipe → every downstream
     consumer (noise branches, MultiPV, played move) coherent for free; no
     "#0 droppable" special case, no fallback logic.
   - Pruning applies at **all plies**, both sides: fixes the defense
     asymmetry (a bot that can't see a threat *walks into it* instead of
     mysteriously defending), and models human projection ("I can't see
     that move, so my opponent won't play it") — both believability wins.
   - Perf constraints: full-strength/analytical paths take a
     `perception >= 1.0` **bypass** (byte-identical path, same trick as
     `QSEARCH_UNBOUNDED`); per-move test must be cheap bitboard arithmetic
     on data the move loop already has; minimize splitmix evaluations.

3. **Determinism / TT coherence — key the roll on `(game_seed, zobrist,
   move)`, NO ply mixing** (unlike `noise::pick`'s per-ply salts). Then
   visibility is a pure function of position+move for the whole game:
   - TT entries stay consistent across re-visits AND across the game's
     successive searches (the play TT persists between moves).
   - Side effect that is a feature: **stable per-game blind spots** (the
     bot misses the same long diagonal all game — human "bad day").
   - Replays with the same seed reproduce exactly. Analytical engines
     bypass (invariant: analytical paths never read the profile).

4. **Salience floors** — always visible regardless of geometry: checks,
   recaptures on the just-captured square, captures of the piece that just
   moved. Human truth (checks are loud) AND defuses the
   `guaranteed_mate_in` conflict (can't mate-guard a move the search never
   generated; mate-in-1s are nearly always checks). Residual case (quiet
   mating moves under a high guarantee) gets a test, not a redesign.

5. **Never-empty guarantee** — a node where the filter would prune every
   legal move keeps something (skip filtering when the survivor set would
   be empty).

6. **Scorer weights are FROZEN BY THE GRID.** Once the grid measures Elo
   with weights v1, any reweighting silently invalidates it. So
   feel-validation (manual chess.com/GUI games) happens **pre-grid** and is
   critical path, not polish. The retrospective consumes the same frozen
   scorer (product coherence: bot blind spots and verdict fairness agree on
   "hard to see").

7. **`miss_chance` is a deletion candidate — and `blunder_chance` may
   be too.** Real misses are pattern-shaped (a tactic you aren't
   trained to see), not coin flips; residual randomness (tilt, fatigue)
   is avg_rank's job. The blunder hypothesis (raised 2026-06-06): most
   human blunders are caused by not seeing the OPPONENT's refutation —
   Heisman's "Hope Chess" (moving without checking whether the
   opponent's forcing replies can be met), counting errors ("I take,
   he takes… oops" = blind to in-exchange replies), and the
   tunnel-vision corpus all say so qualitatively, and our own qsearch-0
   lever is the existence proof (TOTAL opponent-recapture blindness →
   organic believable sub-600 blunders). Mechanism: perception pruning
   on OPPONENT plies hides the punishing reply → the bot overvalues
   its move → plays it → an organic blunder whose *severity* is
   whatever the hidden reply wins (the blunder min/max band becomes
   emergent rather than dialed — a control we'd lose; acceptable if
   the emergent distribution is believable). Validation gate before
   deleting either dial: perception must reproduce the measured roles
   of the miss AND blunder curves in the bands. Grid arithmetic
   improves further if both go: drop miss (×3) and blunder (×5), add
   perception (×3–4) → grid shrinks.

8. **Settled-ply redesign precedes the scorer** (the payoff-depth component
   keys on it, and today's value is broken — see below).

---

## Settled-ply: diagnosis, inventory, redesign

### Diagnosis

`compute_settled_ply` (`search/settled.rs`) walks **backward from the
leaf** and returns at the **latest** 2-ply white-POV eval delta ≥ 25 cp
(`SETTLED_THRESHOLD_CP`, an admitted early guess — search/mod.rs:134). On
deep PVs the tail is the search horizon: eval drift, horizon shuffling and
late speculative exchanges routinely exceed 25 cp, so settled lands at/near
the **leaf** almost always (user estimate: >90% of PVs; instrumentation
will confirm). Consequence: `line_material_delta_cp` — the classifier
under the miss/blunder noise branches, i.e. under the current bands —
counts material through the *whole PV* including speculative deep-line
trades, not through the tactic's resolution.

Documentary evidence the semantics were already found wrong once: the
positional-win card (`core/teaching/src/claim.rs`, ~line 2224) explicitly
**rejects settled_ply** ("walks all the way to where the search score
quiesces — by then the attack has been converted into material") and built
its own **forcing-tail** walk.

### Usage inventory (what breaks when semantics change)

Producer — one site: `search/run.rs:183` → `SearchLine.settled_ply` →
`MoveAnalysis.settled_ply` (pass-through, move_analysis.rs:73).

| Consumer | Uses it for | Sensitivity / disposition |
|---|---|---|
| `noise.rs` `line_material_delta_cp` | cap for material walk → miss/blunder pools + capture-rescue swing | **Highest — bands rest on it.** Migrate to `material_settled`. |
| `analysis/material_outcome.rs:135` | cap for capture-event walk → material cards' `last_ply` ("by move N") | Migrate to `material_settled`. (`realized_net_mg_cp` blunder gate re-caps at ply ≤ 1 — insulated.) |
| `analysis/initiative_outcome.rs:163` `compute_eval_swing` | trace index for eval swing + "user still favored" | **Different question** — wants "where do I read the stable eval," not "when did the tactic resolve." Switch to explicit **leaf** read (today's leaf-drag means it's de facto leaf already → no observable change). |
| CLI `format_settled_suffix` (search_report / analysis_report / play/output) + main.rs JSON | the "settled leaf" display markers | Display-only; follows new semantics. |
| `ui/retrospective_view/*` fixtures, `test_support`, `noise_tests` | test fixtures | Mechanical updates. |

Notably absent: tactic detectors and verdict/assessment layer never read it.

### The no-tactic question (resolved)

"Settled = when the tactic resolved" only makes sense if there IS a tactic.
Per-consumer answer:

- **Material consumers:** no problem by construction — a quiet line has
  zero capture/promotion events, delta is 0 wherever the cap lands, and
  "settles immediately, banks nothing" is the *correct* classification.
- **Eval-swing (initiative):** the question bites here — a quiet line under
  event semantics settles at 0 and the swing degenerates. Resolution: that
  consumer was never asking the tactic question; it reads the **leaf**.
- Net: the redesign is a **three-way split** of one overloaded number:
  1. **`material_settled`** — forward event-walk (below); defined for every
     line, "no events → ply 0".
  2. **leaf** — stable-eval read (initiative; any future "representative
     trace" use).
  3. **climax / forcing-tail** — only meaningful given a sacrifice; already
     implemented in the positional-win card; stays as-is.

### New `material_settled` semantics (proposal to validate)

Walk **forward**; captures / promotions / checks count as "still
resolving"; settle at the start of the **first run of N consecutive
non-forcing plies** (N ≈ 3). Rationale:

- **Events, not cp**: kills the 25-cp-wobble false triggers outright
  (user's point: "1 pawn of MATERIAL score" — going all the way to discrete
  events is the robust version of that).
- **Quiet-move-inside-a-tactic** (the original reason for the backward
  walk — skewer/fork quiet first moves): a fork is quiet-move → quiet-flee
  → capture = a 2-quiet-ply gap; N = 3 bridges it. Deflection→fork chains'
  links are mostly checks/captures (forcing) and don't break the run.
- **First resolution, not last shift**: deep-tail speculative trades are
  exactly what must NOT count toward "this move wins material by force." A
  payoff 10 quiet plies later is positional, not banked material — for the
  noise classifier ("what does the bot think it's getting") and the
  teaching cards ("you win a rook by move N"), early-pause semantics is
  the right meaning.

Validation oracle: the lichess tactic detectors. `find_tactic_in_line` /
`TacticHit.pv_ply` give semantically-labeled payoff plies — run the new
logic over detector-labeled PVs (teaching-positions/, bench set, real-game
retrospectives) and assert `material_settled` lands at/just after the
payoff. **Oracle in tests, not a runtime dependency** (noise path stays
cheap and dependency-light). Multi-tactic-per-PV detection ("first
deflection, then fork") is a natural detector extension for *teaching*
surfaces, independent of this plan.

### Knock-on: bands shift

New settled semantics changes the miss/blunder pools → the measured lever
curves and `bands.txt` rungs shift. Fine: the perception lever forces a
re-derivation anyway, and the bands harness (`calibration/run_bands.py`)
makes iteration routine. Do NOT lock bands or run the grid before both
land.

---

## Research synthesis (2026-06-06, five parallel research agents)

Five evidence angles — coaching curricula, Reddit/forum self-reports,
academic/data-driven studies, low-ELO game commentary, engine-humanization
prior art — converged on the feature set below. Full agent reports are in
the session transcript; key sources cited inline.

### Prior-art conclusions (engine-humanization survey)

- **Nobody has built geometric move-visibility filtering.** All existing
  weakening = score-noise, depth limiting, or NN imitation (Maia). The
  axis is unoccupied.
- **The universal critique of every existing weak bot** (SF Skill Level,
  Komodo personalities → chess.com bots, Maia): *"plays like Stockfish
  for 30 moves, then drops the queen"* — perfect play punctuated by
  random catastrophe. The fix critics ask for is **consistent,
  patterned, position-explainable misses** — exactly what a
  deterministic feature-based scorer produces. (talkchess t=73603,
  t=55011; Kaufman on Komodo personalities.)
- **The Beal effect kills score-noise approaches structurally**: random
  eval + normal search still plays ~1700 — depth launders noise back
  into strength. SF skill-level shows the same ("right Elo number, wrong
  behavior" — K+P endgame shuffling). **Pre-search candidate pruning is
  the robust mechanism** (search cannot recover a move that was never in
  the tree) — independent confirmation of our in-search decision.
- **The residual defect everywhere is tactical sharpness surviving
  weakening.** Quiet / backward / deep-payoff moves are where humans go
  blind and where alpha-beta stays superhuman — that intersection is the
  scorer's target.
- **Maia's two gaps are determinism and explainability** — the two
  properties the teaching product needs most; geometric scoring provides
  both. Allie (arXiv 2410.03893) models per-position *effort* (thinking
  time); per-move *visibility* is the complementary unbuilt axis.

### Evidence-ranked feature set

**Tier 1 — multi-angle, some quantitative:**

| Feature | Direction | Evidence |
|---|---|---|
| **Ray occlusion** (move's value rides a screened ray: discovered piece behind the mover, X-ray attacker/defender, target behind a blocker) | HARDER — the strongest single geometric signal | lichess puzzle data: discovered-check/pin median ~2200 vs fork ~1450 (artefact2); X-ray counting errors + discovered attacks + screened-diagonal misses are three independent observed failure families |
| **Salience gradient** capture-check > capture > recapture > quiet-check > quiet | forcing = EASIER (modest boost — low-ELO still misses checks); quiet = HARDER | CCT doctrine universal across coaches; Maia: obvious-recapture = most predictable move; eye-tracking: saccades drawn to checks/captures/high-value targets |
| **Direction** forward < sideways < backward (mover-relative) | backward = HARDER (strong); sideways mild | The Qe7-found/Qe1-never classroom test; GM-level documented misses (Karpov, Topalov — ChessMood); "programmed not to look for them." Caveat: no measured blunder-rate statistic — strong *pedagogical* prior |
| **Knight moves** (non-collinear destination) | HARDER; backward-knight = stacked worst case | "Hardest piece to visualize" consensus; forks appear off-line where no ray is scanned |
| **Distance — as a modulator, NOT standalone** | long = HARDER *only* combined with occlusion / distance-from-action | KEY CORRECTION: a clear long diagonal already pointed at the target is "so obvious it's just a plain blunder" (sniper-bishop threads); eye-tracking visual-span literature supports distance-from-focal-action, not raw move length |

**Tier 2 — attention/state features (cheap from move history; co-equal
with geometry per the coaching + self-report corpora):**

| Feature | Direction | Evidence |
|---|---|---|
| **Distance from opponent's last-move square** | far = HARDER | "Attention follows the last-moved piece" — the single most-cited low-ELO vision failure |
| **Plies since the moving piece last moved** | dormant = HARDER | "Easy to forget a piece you moved several turns ago"; "no trigger" |
| **Own-ply vs opponent-ply asymmetry** | opponent's moves HARDER (tunnel vision / "I saw it for me but not for them") | Most-reported single blunder cause; for the in-search filter this is a *feature*: harsher pruning on opponent plies = the human projection "I can't see it so they won't play it" |
| **Rim/edge origin square** | mild HARDER | weaker than expected — fold into distance-from-action, no standalone weight |

**Special cases:** en passant = HARDER despite being a capture (the only
capture not landing on the victim's square); promotion-to-queen = always
visible (existing easing stays); underpromotion = effectively invisible
(already SF11-faithful behavior).

**Calibration anchors (payoff depth):** <800 ≈ 1–2 plies practical
horizon, ~1200 ≈ 3–4, forced lines much deeper (Heisman "Real Chess" =
3-ply; depth-vs-rating folk numbers). Anderson et al.: inherent position
difficulty predicts blunders at 73% vs skill 55% — difficulty features
dominate rating, which is why this scorer can work at all.

### Blunder causation (follow-up agent, 2026-06-06)

Question: are blunders (losing own material) caused by not seeing the
opponent's refutation — i.e., can perception subsume `blunder_chance`?

- **Supported as the dominant avoidable cause below ~1600.** Heisman's
  *Hope Chess* is the hypothesis verbatim — "didn't see it" is a
  thought-process failure (didn't LOOK at the opponent's forcing
  replies), his #1 diagnosis of why sub-1600 players lose. Maia proves
  blunders are systematic and feature-reproducible (predicts the exact
  human blunder ~25% of the time). No academic work decomposes causes
  (Anderson predicts *when* via position difficulty, not *why*).
- **Lever mapping (corrected by the user 2026-06-06):** Heisman splits
  Hope Chess into **Basic** (1200–1400: never checks the reply at all)
  and **Passive** (1200–1700: checks, but with shallow
  pattern-matching). *Never looking* is the **depth/qsearch** axis —
  the reply simply isn't in the tree. **Perception is the Passive
  refinement**: the player who DID try to check the reply but whose
  pattern-scan missed it *because of board geometry* (the knight-move
  capture, the cross-board rook). qsearch-depth covers the
  seen-but-shallowly-resolved part of Passive; perception covers the
  scanned-but-not-noticed part. Three complementary levers, no
  substitutes.
- **The Einstellung boundary condition (calibration rule):** in
  eye-tracking studies, when the tempting move was *outright losing*,
  even novices saw the danger and avoided it (experts F(1,66)=79.9,
  p<.001) — fixation blindness produces *misses*, not material hangs.
  Therefore **pruning probability must rise with the refutation's
  geometric subtlety, never with its material payoff** — an adjacent
  queen-recapture stays ~always visible even at low perception; a
  knight-move or cross-board refutation is missable. This also answers
  the emergent-severity worry: big hangs will occur predominantly to
  *subtle* refutations, which is exactly the real-world queen-blunder
  shape (and the user's observed cases).
- **Data gap = FOLLOW-UP item (user decision 2026-06-06):** no public
  dataset measures hanging-piece-capture rates by capture geometry.
  Deriving weights empirically from a lichess dump (filter low-rated
  games, label hanging pieces with our engine, record whether/how the
  reply captured, regress P(capture) on geometry) is DEFERRED — v1
  weights are tuned by feel/gut to get the proof-of-concept out. The
  accepted cost: when the empirical calibration eventually lands, the
  weights change and the **full grid re-runs**. Flagged in the backlog
  below.

### Architectural consequence: payoff depth is NOT an in-search feature

The in-search filter prunes moves *before* searching them, so it cannot
know a move's payoff depth — and it doesn't need to. **Depth difficulty
falls out of compounding**: a combination is seen only if every link is
seen, so P(see line) = ∏ P(see move). Deep *quiet* payoffs become
invisible automatically (quiet links are individually low-visibility)
while *forcing* chains survive (checks/captures are high-salience) —
which is exactly the forcing-chain discount every evidence angle
demanded, by construction. The explicit line-level number
(`line_difficulty = ∏ P(see pv[i]) for i ≤ material_settled`) exists
only on the **retrospective** side, where the PV is known and the
question is "was this line humanly findable."

### Scorer shape (v1, weights to feel-tune then freeze)

Per-move `P(see | perception)`, all features cheap at movegen time:

1. **Salience class** (capture-check / capture / recapture / quiet-check
   / quiet / en-passant) — sets the base visibility.
2. **Direction class** (forward / sideways / backward) — multiplier.
3. **Knight bump** — multiplier.
4. **Ray occlusion** — multiplier (v1: mover is a discovered-attack
   vehicle, or the move's ray to its highest-value target is screened;
   reuse the existing alignment scan primitives where possible).
5. **Distance from opponent's last-move square** + **mover dormancy** —
   state multipliers (need last-move + per-piece last-moved-ply, both
   already available / trivially tracked in search state).
6. **Opponent-ply multiplier** — harsher filter on plies where the
   side-not-being-modeled moves.
7. **No pedagogical salience floors** (user decision 2026-06-06,
   reversing the earlier sketch). Checks / recaptures /
   capture-of-last-mover are STRONG salience multipliers, never
   absolute exemptions — the evidence says even these get missed when
   the geometry is hard (queens blundered to a knight-move or
   cross-board-rook capture go untaken; CCT exists as *training*
   precisely because untrained players miss checks; chess.com's Martin
   misses mate-in-1). Our own validation has both halves: the t400
   adjacent-queen-recapture refusal was absurd (geometrically trivial
   → easing was right), while "didn't punish the hung queen because
   the capture was a knight move" is realistic (geometrically hard).
   The perception score distinguishes exactly these; a floor can't.
   The only ABSOLUTE exemptions are mechanical:
   - **in-check nodes are never filtered** (all evasions considered —
     same rule as the qsearch cap);
   - **never empty the candidate list**;
   - **`guaranteed_mate_in` contract patch — KEPT (user decision
     2026-06-06):** when the dial is ≥ 1, exempt *root* checking moves
     from the filter (cheap, bounded; a mate-in-1 is then always
     resolved and the guard fires). Rationale: the dial is a
     **training feature**, not a realism feature — a student has to
     learn to see checkmate threats, so the trainer-bot must reliably
     deliver them. Realism-seeking rungs set the dial to 0 (and even
     then the bot usually plays the mate on eval alone, unless
     perception or avg_rank demotes it). Deeper mates may go
     unresolved at low perception — consistent with the dial's
     documented semantics ("a protection floor, not a search cap",
     commit `7d8c03f`).
   Long-term unification note: the noise layer's capture-rescue easing
   (P(grab) by value/rank) could itself become perception-driven —
   P(rescue) = P(see the capture) — collapsing two mechanisms into one
   curve. Not v1.

Position-level blunder-potential (Anderson β = fraction of candidate
moves that lose) is a **deferred v2 multiplier** — at the root it's
nearly free from the MultiPV deltas; in-tree it is not cheap. Note it,
don't build it.

The believability constraint that killed checkboxes ("fails a 2-ply
tactic but sees a 6-ply one" must be impossible) holds by construction:
one perception dial gates compounded per-move probabilities, so deeper =
strictly less visible at equal salience.

### v1 weight table (proposed 2026-06-06, feel-tune then freeze)

`V(mv) = S × D × K × O × A ∈ (0, 1]`, then the **margin curve**
(user-steepened 2026-06-07; **re-leveled + perception-scaled plateau
after the first playtest**, same day — see "Playtest revision" below):
with margin `m = p − (1 − V)` and
`plateau(p) = 1 − (1 − PLATEAU_FLOOR)·(1 − p)`,

```
P = 1.0                                     if V == 1.0  (no difficulty flags → nothing to miss; exact-match special case is principled: factors are discrete)
P = plateau(p) + (1−plateau(p))·min(1, m/RAMP)   if m ≥ 0
P = plateau(p) · max(0, 1 + m/CLIFF)²            if m < 0  (quadratic cliff to literal 0)
PLATEAU_FLOOR = 0.8 · RAMP = 0.3 · CLIFF = 0.45
```

Properties: perception clears difficulty → P ≥ plateau(p) ramping to a
deterministic 1.0 at margin ≥ 0.3 ("reliably sees" classes — the
believability-consistency ask). The plateau **scales with p** ("how
often you fumble a move you're capable of seeing" shrinks as the scan
sharpens: p=0 → 0.80, p=0.5 → 0.90, p=0.95 → 0.99) and converges
smoothly into the `p ≥ 1.0` bypass instead of jumping. Below
threshold, quadratic cliff hitting **literal zero** at margin −0.45.
Deterministic roll per `(game_seed, zobrist, move)`.
**Opponent-ply asymmetry:** `V^1.5` before the curve on plies where
the side-not-being-modeled moves. Curve visualization:
`calibration/plot_perception_curve.py`.

### Playtest revision (2026-06-07, after the first feel game)

User played a "perfect" bot (d10/q∞/r1) at p=0.5 and won off a
too-weak blunder: in `5kn1/5p2/p2N2p1/8/5P2/1r6/7r/k2R4 b` the bot
played Rh1?? blind to the open-board recapture Rxh1. Autopsy: V =
sideways .70 × A_NEAR .92 = .64 → opp-ply .52 → P = .81 — and **the
deterministic roll makes any per-move miss probability a PERMANENT
game-long blind spot** (19% lottery, hit). Compounding is double:
across a line's plies (a d10 PV's integrity needs every reply-roll to
succeed) and across the game's hundreds of node decisions. Fixes, all
landed:

1. **Direction square-rooted for captures** (the targeted fix — the
   forward-bias evidence is about quiet moves; a sideways rook TAKE
   has a target pulling the eye). Checks excluded on purpose.
2. **Weights re-leveled up** (hardest common stacks ≈ 0.5; the
   weakness now comes from compounding + the residual lottery, not
   from single moves being coin flips — user's diagnosis).
3. **Perception-scaled plateau** (user: "shouldn't the plateau rise
   with perception?" — it also fixes the discontinuity at the bypass).
4. **Endpoint-clutter factor added** (user-requested; penalizes dense
   middlegame tangles, neutral on open boards and opening formations —
   it would NOT have saved the Rh1 case, which is why 1–3 are the
   load-bearing fixes).

Post-revision: Rxh1 reads V ≈ .88 → opp .82 → margin +0.32 → **P = 1.0
deterministic**. The trade accepted: single-move blindness migrates
down-dial (the quiet backward Qe1 at p=.5 is now ~98% seen; the
classroom blindness lives at p≈0–0.2), and a given p plays stronger
than before — where each p lands on the ELO ladder is the grid's job.

**Composition rule:** every sub-factor is an independent multiplier
defaulting to 1.0 when not applicable; `V = S × D × K × (∏O) × (∏A)`,
clamped to a small floor.

**S — salience = RULE-FAMILIARITY ONLY (user recalibration
2026-06-07):** normal moves — quiet, captures, checks, promotions to
queen — are all base **1.00**; "marching a pawn is never hard to see."
What earns a salience penalty is depending on a special rule /
abnormal piece movement:

| class | S |
|---|---|
| any normal move (quiet / capture / check / Q-promotion) | 1.00 |
| castling | 0.80 |
| en passant | 0.55 |
| underpromotion | 0.25 |

The earlier capture>quiet gradient is deliberately dropped: a
recapture's easiness comes out of the ATTENTION factor organically
(the capture square *is* the last-move locus → A = 1.0), not from a
class bonus. Quiet *key* moves are hard because of their geometry
(backward / threading / far endpoints / vehicle) or because their
payoff is beyond the horizon (the depth/qsearch levers' job) — never
because they are quiet.

**D — direction (mover-relative rank delta):** forward 1.00 · sideways
0.85 · backward 0.75 — applied in FULL to quiet moves, **square-rooted
for captures** (the target piece pulls the eye; the forward-bias
evidence is about quiet moves — the Rh1 playtest fix). Checks
deliberately do NOT get the attenuation: missed checks/mates are
documented low-ELO behavior the lever must stay able to produce.

**K — piece:** knight 0.85 · all others 1.00.

**O — ray occlusion:** discovered-attack vehicle (mover unveils a
friendly slider's attack on an enemy piece) ×0.75 · slider path
threads traffic (occupied squares adjacent to the path interior: ≥4 →
×0.85, 2–3 → ×0.92). (No standalone long-move factor — length is
subsumed by the two-endpoint attention term.)

**A — attention (state inputs, neutral when absent):**
**two-endpoint** distance from the opponent's last-move square (a move
is a relation; you must attend BOTH ends — seeing the bishop doesn't
mean seeing its far target, and vice versa):
`A = g(cheby(from, last_to)) × g(cheby(to, last_to))`,
`g: ≤2 → 1.0 · 3–4 → 0.95 · ≥5 → 0.90` (both-far = 0.81) ·
**endpoint clutter** (visual crowding; user-requested from playtest):
occupied squares in the union of the from/to king-rings (endpoints
excluded): 7–9 → ×0.94, ≥10 → ×0.88 (thresholds sit above ordinary
opening formations — 1. e4's ring count is 5 → neutral) · mover
dormancy (≥12 plies unmoved) ×0.90 — dormancy is v1-OPTIONAL (needs
per-piece last-moved tracking; the ctx field defaults neutral).

**Worked archetypes** (own-ply; opponent plies apply V^1.5 first):

| Move | V (revised) | Reads as |
|---|---|---|
| Adjacent queen recapture | **1.00** | literally never declined (it IS the attention locus) ✔ |
| Open-board sideways rook take (the Rh1 case) | √.85×.95 ≈ **.88** → opp .82 | margin +0.32 at p=.5 → **P = 1.0 deterministic** ✔ |
| Backward quiet queen fork (the Qe1 case) | **.75** | p=.7: 1.0 · p=.4: .94 · p=0: **.16** — classroom blindness lives down-dial now |
| Cross-board knight capture of a hung queen | .85×.90 ≈ **.77** | as opponent's refutation: .67 → P = .42 at p=.2 — still the unpunished-queen case ✔ |
| Sniper bishop, threaded + both endpoints far | .85×.81 ≈ **.69** (clear diagonal at target near action: ~1.0) | hard only when screened/remote ✔ |
| Quiet discovered-attack vehicle move | **.75** (stacked with knight/backward/clutter → .4–.5) | hardest motif class; deep intersections still go low ✔ |
| Quiet 3-move plan of normal moves | 1.0³ = **1.00** | ordinary plans fully findable ✔ |

**P(see) reference table** — revised curve (scaled plateau):

| V \ p | 1.0 | 0.7 | 0.4 | 0.2 | 0.0 |
|---|---|---|---|---|---|
| 1.00 (normal move) | 1.00 | 1.00 | 1.00 | 1.00 | **1.00** |
| 0.92 | 1.00 | 1.00 | 1.00 | .90 | .54 |
| 0.80 (castling) | 1.00 | 1.00 | .96 | .84 | .25 |
| 0.60 | 1.00 | 1.00 | .88 | .26 | .01 |
| 0.40 | 1.00 | .96 | .27 | .01 | **0** |
| 0.20 | 1.00 | .57 | .01 | 0 | 0 |

`p = 0` means **maximally geometry-blind, not move-blind**: every
V = 1.0 move is always seen, the cliff floor (literal zero) now needs
V < 0.55 = multi-factor stacks. The scaled plateau means a high-p bot
fumbles cleared moves at `(1−PLATEAU_FLOOR)·(1−p)` — ~1% at p=0.95 —
not a flat 20%.

Line-level findability (retrospective): `∏ P(see pv[i])` over the
mover's plies through `material_settled`, evaluated at a fixed
"strong-human reference" perception (≈0.75) — replaces/augments the
depth-honesty heuristics.

## Sequencing (agreed order)

1. **Settled-ply instrumentation** ← LANDED (`chess-tutor settled-audit`,
   `core/cli/src/settled_audit.rs`, TEMPORARY — remove with this plan).
   Reports settled-distance-from-leaf distribution, material class
   (win/neutral/loss at ±1 pawn, the noise branches' discriminator) under
   current-cap vs a `material_settled` prototype vs full-PV, and the
   tactic-detector-oracle gap. `--depth` repeatable to show depth scaling.
   **Findings (d8/d12/d16 × multi_pv 10, 416 lines/depth, full output at
   `calibration/runs/settled_audit_d8_d12_d16.txt`) — diagnosis CONFIRMED:**
   - **Leaf-drag:** 90.9% (d8) / 89.9% (d12) / 85.8% (d16) of lines settle
     AT the leaf. Degeneration (current-cap delta == full-PV delta):
     98.8% / **100.0%** / 99.3% — at d12, the retrospective default, the
     settled cap did literally nothing on every line.
   - **Material-class distortion grows with depth:** win/neutral/loss
     flips current→prototype: 14.9% (d8) → 28.6% (d12) → 32.9% (d16).
     At d12 the current classifier labels 77 win / 142 loss vs the
     prototype's 35 / 85 — **roughly double**, by counting speculative
     deep-tail trades as banked material. Concrete absurdity: from the
     START POSITION, `1. c4` (slot 5) classifies as a material LOSS
     (−100 @ply 12 — a QGA-style line "loses" the c-pawn twelve plies
     out). Quiet opening moves sit in the blunder pool today.
   - **Detector oracle:** current settled lands p50 9 (d12) / 13 (d16)
     plies PAST the named tactic's key move (≈ the leaf); the prototype
     lands p50 0, p90 7–8 — it tracks the tactic, current doesn't.
   - **Prototype sanity:** settles at ply 0 ("quiet, banks nothing") on
     ~56–58% of lines at every depth — matches chess reality (most
     candidate moves force nothing).
   - **Bonus bug, opposite direction:** PVs of length ≤ 2 always get
     `settled = 0` (the backward walk needs `i ≥ 2`), so a 2-ply PV like
     `Rf6 Nxf5` counts only ply 0 — the immediate recapture LOSS is
     invisible. Note the irony: for 3+-ply short PVs the leaf-drag
     *accidentally rescues* the weak-bot case (counting everything ⊇
     counting the recapture), which is why miss/blunder still "worked"
     in the bands at d2-q1. The redesign must keep that case correct —
     the prototype does (first resolution includes the immediate
     exchange).
   - Prototype `QUIET_RUN_LEN = 3` tightness is debatable on one observed
     shape: `Nbd6+ Kf8 Be5 Nf6 Bxd4 …` closes the window at ply 0 (three
     quiet plies before Bxd4) and forgoes the ply-4 pawn grab — arguably
     correct ("slow plan, not banked tactic"), to be eyeballed during the
     redesign.
   - **Eval read-point comparison** (added on user request — what the
     eval-swing consumer would read at each settled notion): the two
     read points differ by p50 ≈ 104/143/159 engine-cp (d8/d12/d16,
     ≈ 0.5–0.7 pawns), p90 ≈ 470–1062, growing with depth — the split
     is REAL, you cannot use one notion for the other. On
     tactic-labelled non-mate lines, |score − eval(prototype)| is
     p50 ≈ 103–150 engine-cp — positional-drift magnitude, NOT the
     >1000 engine-cp half-a-hanging-queen signature mid-exchange
     reads would show, so the prototype is landing on *resolved*
     positions. |score − eval(current)| ≈ 30 cp is the expected
     tautology (current ≈ leaf ≈ where the search scored). Confirms
     the three-way split: material classifiers → `material_settled`;
     eval-swing → leaf; climax → forcing tail.
2. **Settled redesign** — LANDED. `compute_material_settled`
   (`search/settled.rs`, replacing the eval-delta backward walk;
   `MATERIAL_QUIET_RUN = 3` exported, audit tool imports it so they can't
   drift). `SearchLine.settled_ply` now carries the new semantics —
   noise + material_outcome consumers needed **zero code changes** (they
   cap walks on the field); `initiative_outcome::compute_eval_swing`
   switched to an explicit **leaf** read (behavior-preserving: the old
   leaf-dragging value was de facto the leaf). `SETTLED_THRESHOLD_CP`
   survives display-only (the `--debug` trajectory marker). All 1,332
   workspace tests pass with no fixture fallout. **Regression check
   (re-run audit, d12 × mpv 10): current ≡ prototype — 0 class flips,
   eval-gap 0; corrected classifier: 35 win / 296 neutral / 85 loss (was
   77/197/142).** Knock-on: the bands' miss/blunder pools changed under
   this fix — re-derivation (step 7) is mandatory, as planned.
3. **Difficulty scorer** — LANDED (`core/engine/src/visibility.rs`,
   top-level sibling of `noise.rs` so search doesn't depend on
   `analysis/`). `visibility(pos, mv, ctx)` (V = S×D×K×O×A per the
   table), `p_see` (margin curve), `sees` (zobrist-keyed roll),
   `line_findability` (retrospective `∏ P(see)` to `material_settled`),
   `PerceptionParams`. 16 unit tests pin the archetypes + curve
   reference points. Feel-test flag from the tests: the threading
   neighbourhood counts home-rank clutter, so a queen sortie down a
   half-open file reads 0.75 ("leaving the nest") — if real play shows
   queen sorties over-missed, narrow the neighbourhood.
4. **In-search perception filter** — LANDED.
   `SearchParams.perception: Option<PerceptionParams>` →
   `Search.perception` (≥1.0 normalized to None at `run()`; bypass is
   one dead branch per move — **verified byte-identical**: bench
   16 1 14 = 9,745,694 nodes with and without the code, clean-worktree
   A/B). Filter sits after legality in `negamax_moves` and in qsearch
   (stand-pat is qsearch's natural fallback); in-check nodes never
   filter; opponent plies apply `V^1.5`; inner-node attention locus
   reads the parent stack frame, root locus passed by the caller.
   **Never-empty fallback (N=1)**: `MovesOutcome.unseen_fallback`
   carries the highest-V pruned move; negamax reruns the loop pinned to
   it when a node comes back empty (load-bearing — `move_count == 0`
   still means mate/stalemate); root fallback only for the primary PV
   slot (secondary slots stay empty = the bot's candidate list is the
   seen moves). Root-check exemption wired to
   `guaranteed_mate_in >= 1`. Five search-level tests incl. the
   backward-mate exemption pair. Wired end-to-end:
   `OpponentProfile.perception` (manual `Default` — 1.0, not derived
   0.0) + `perception_params()` → play worker (locus = user's last
   move) / `--perception` on `play`, `uci` (per-game seed), `search`
   (fixed seed 0 for reproducible inspection) / desktop New Game
   "Perception" slider. Analytical paths pass `None` everywhere.
   Smoke: `search "8/4Q3/8/8/8/6K1/8/7k w" --depth 4` finds Qe1#;
   `--perception 0` plays Qd8 — the backward mate is invisible.
5. **Feel-validation + weight FREEZE** (manual games) — pre-grid critical
   path.
6. **Miss-subsumption check** → drop `miss_chance` if perception covers its
   band role.
7. **Re-derive lever curves → bands → lock → grid** (axis swap keeps grid
   ~2880 / ~6.5 h).
8. **Retrospective findability refactor** on the frozen scorer (anytime
   after step 3; separable deliverable).

## Open questions / concerns

- **Empirical grounding of geometric weights — FOLLOW-UP (decided
  2026-06-06)**: v1 ships hand-picked + feel-validated. The in-house
  lichess-dump analysis (P(hung piece captured) regressed on the
  capturing move's geometry — no public dataset exists) is deferred to
  get the proof-of-concept out; when it lands, weights change and the
  **full grid re-runs** (cost accepted by the user).
- **Perception × qsearch interaction**: composes, not replaces —
  qsearch-depth = how far you calculate; perception = which moves exist
  for you. Expect grid interaction terms (like miss×qsearch ≈ 0 at q0).
- **Pilot before grid membership**: 1-D perception sweep on 2–3 bases
  (d1-q0, d2-q1, d2-q2) to measure Elo effect + spot sign-flips (the mask
  lesson) before committing it as a grid axis.
- **Mate-guard residual**: quiet mating move + high `guaranteed_mate_in` +
  low perception → guard can't fire on a move never searched. Test it;
  accept or add a targeted exemption.
- **In-search = deeper intervention**: needs the A/B discipline (bypass
  byte-identical on full strength; weak-bot believability by play), per
  the "SF pruning is a balanced set" lesson — don't trust solo reasoning
  about search changes.
- **GUI rank-slider cap** (1.0–4.0) still needs lifting for basement rungs
  (pre-existing item, unaffected).
