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

7. **`miss_chance` is a deletion candidate.** Real misses are
   pattern-shaped (a tactic you aren't trained to see), not coin flips;
   residual randomness (tilt, fatigue) is avg_rank's job. Validation gate
   before deleting: perception must reproduce the role the measured miss
   curves play in the t800–t1500 bands believably. Grid arithmetic is
   neutral: drop the miss axis (×3), add a perception axis (×3) → grid
   stays ~2880 configs / ~6.5 h.

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

## Difficulty scorer sketch (v1, weights to feel-tune then freeze)

Subscores in [0,1], combined (weighted sum or max — decide during build):

- **Direction** (mover-relative): forward easy · sideways harder ·
  backward hardest.
- **Distance**: Chebyshev distance of the move; long = harder.
- **Mover type**: knight moves (esp. retreats) harder; diagonal sliders
  harder than orthogonal.
- **Ray traffic**: slider path passing adjacent-to / between pieces =
  harder.
- **Payoff depth**: plies until `material_settled` claims the line's
  material (0 for immediate; scales up). This is the component that may
  subsume `miss_chance`.
- Salience floors zero the difficulty outright (checks, recaptures,
  capture-of-last-mover).

Monotone-in-depth by construction → "fails a 2-ply tactic but sees a 6-ply
one" cannot happen (the believability constraint that killed checkboxes).

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
3. **Difficulty scorer** (`analysis/`, fixed weights, shared surface).
4. **In-search perception filter** — zobrist-keyed seeding, salience
   floors, never-empty guard, ≥1.0 bypass. A/B: full-strength bench
   byte-identical with bypass; weak-bot games for feel.
5. **Feel-validation + weight FREEZE** (manual games) — pre-grid critical
   path.
6. **Miss-subsumption check** → drop `miss_chance` if perception covers its
   band role.
7. **Re-derive lever curves → bands → lock → grid** (axis swap keeps grid
   ~2880 / ~6.5 h).
8. **Retrospective findability refactor** on the frozen scorer (anytime
   after step 3; separable deliverable).

## Open questions / concerns

- **Empirical grounding of geometric weights**: v1 is hand-picked +
  feel-validated. Maia/lichess data could ground it later; don't block.
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
