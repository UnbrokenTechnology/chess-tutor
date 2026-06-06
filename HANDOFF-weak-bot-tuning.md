# HANDOFF — weak-bot believability + the ELO seed-ladder

Comprehensive reset snapshot of the bot-tuning / calibration thread,
**2026-06-05**. Read this cold to resume. Companions:
[`HANDOFF-calibration.md`](calibration/HANDOFF-calibration.md) (harness
internals), [`HANDOFF-endgame-skill.md`](HANDOFF-endgame-skill.md)
(endgame lever + earlier playtest detail), [`PLAN-elo-calibration.md`].

---

## TL;DR — the goal and where we are

**Goal:** a single "opponent ELO" slider. To build it we measure
`dials → ELO` offline (bot configs vs the Maia ladder via fastchess+Ordo)
and fit an invertible model. Two intertwined work streams this session:

1. **Believable weak bots** — the levers must produce play that *looks*
   like a weak human, not a strong engine that throws games. All the
   engine fixes including miss-gating have now **landed**.
2. **A dense, Maia-anchored seed ladder** — built via `build_ladder.py`
   (measure knobs) → `design_ladder.py` (design a rung per target ELO from
   the measured linear models). A measured ladder exists; it needs **one
   more re-measure** now that the believability fixes are all in.

**Immediate next step:** **re-run `build_ladder` + `design_ladder`** (the
easing + miss-gating changes shifted all noisy-bot ELOs up — miss is now
weaker per % since it no longer declines immediate captures), then continue
chess.com hand-validation → pin the lichess→chess.com offset → lock the
seed pool into `pools.py`.

---

## The weakening levers (engine, play-engine-only)

All live on `OpponentProfile`; analytical engines (retrospective/hint/
analyze) NEVER read them. Plumbed `OpponentProfile → SearchParams →
Search`, mirrored in the harness `BotConfig` (Python) → `chess-tutor uci`
flags.

| Lever | What it does | Human analog |
|---|---|---|
| **depth** | IDS depth (a high floor; d1≈1750 no-noise) | coarse strength floor |
| **qsearch-depth** | quiescence horizon cap; `0` = tactically blind (hangs to recaptures), `None`=full | tactical vision (100→1000) |
| **avg_move_rank** | plays the Nth-best move on average (normal dist around the dial) | the **main** weak-bot knob; ~linear |
| **blunder** chance + min/max material | deliberately hang material in a band | realistic slips |
| **miss** chance | decline a forced material win | "saw the tactic, didn't play it" |
| **guaranteed_mate_in** | always find mates ≤ N (protects them from all noise) | mate vision floor |
| **eval masks** (8 cats) | blind to a positional concept | positional gaps (1000→2000) |
| **endgame-skill** (NEW) | tier of closed-form endgame books allowed | doesn't know KBNK etc. |

### Engine changes THIS session (all committed)
- **endgame-skill tier lever** (`ed31ca0`) — `EndgameSkill {None,Basic,
  Intermediate,Full}`; `probe_with_skill` gates each specialist by tier.
  A weak bot falls back to classical eval (botches KQ/KBNK, stalemates).
  GUI slider wired (`c2d0087`). Harness `BotConfig.endgame_skill` +
  `--endgame-skill` (`a9f2a14`). **Also fixed the SF11-inherited Q→B
  underpromotion for weak bots for free** (no `kbnk` override → classical
  → queen out-ranks B+N). Full-strength underpromotion deferred (it's
  SF-faithful and search-hidden).
- **mate-guard fix** (`7d8c03f`) — the variety branch now respects
  `guaranteed_mate_in` (it didn't; a g-mate-in-1 bot demoted off a
  mate-in-1).
- **Material easing on the rank lever** (`6c05643`, retuned `452a886`) —
  THE big believability fix. See next section.
- **GUI**: qsearch slider 0–10 (10=∞ default) `dc4291f`; avg-rank slider
  rescaled **1.0–4.0 by 0.1** `a988d0a` (was 1–10/0.5, couldn't set 1.9).

---

## Material easing (the rank lever) — the believability keystone

**Problem found in chess.com validation:** the `avg_move_rank` (variety)
lever was **material-blind** — it would demote off an *immediate winning
capture*, so a weak bot left a free queen for turns, **sidestepped a check
instead of taking the checker**, and stopped rooks shy of captures.

**Fix (`noise.rs::pick`):** when a rank demotion would throw away an
immediate winning capture (`PV[0]` is a capture securing material `V`
pawns over the demoted move), the bot still plays it with

> **`P(grab) = min(1, V / (6·(rank − 1)))`**

— rises with value (queen>rook>minor), falls with rank, **capped at 1**.
The `6` ([`CAPTURE_RESCUE_C`]) sets two anchors: **queen at rank 2 always
grabbed, minor at rank 2 ~50%**; `rank==1` always grabs; weak bots (high
rank) still miss high-value pieces sometimes. Only *immediate* captures
are rescued — a subtle quiet best-move stays demotable (the wanted
"looks-like-zugzwang misjudgment" feel). Net principle: **hanging material
comes only from tactical blindness (qsearch) or the deliberate blunder
lever, never incidentally from rank.**

**Consequence:** all noisy bots got STRONGER and **r5/r6/r7 reopened** as
basement rungs (high rank is now sane-but-weak). → **must re-measure the
ladder**, and likely **raise the GUI rank cap above 4** afterward.

### LANDED: gate `miss` on 2-ply material (obvious grab vs combination)
`miss` used to *also* decline immediate captures (it fires before variety),
so a `t400` with 26% miss sidestepped ~¼ of capture moments regardless of
the easing. **Fix shipped in `noise.rs::pick`:** the miss branch now also
requires **`two_ply_material_cp(PV[0]) < WIN_MATERIAL_CP`** — i.e. the best
line is **not already up a pawn-or-more after its first move + the
opponent's reply**. This single test (no `first_move_is_capture` disjunction
needed — a non-capture start is always ≤0 at two plies) captures everything:
- **Obvious grab → exempt** (handled by the value-easing): a hanging-piece
  capture (`Qxd5`, +900 at 2 plies), an even trade settled in hand.
- **Combination → still missable:** a quiet first move (a fork, 2-ply 0); an
  even trade that wins on the follow-up (a discovered attack, 2-ply 0); a
  real **sacrifice** (Damiano-style `Nxe5 …dxe5`, 2-ply *negative*, material
  returns later). The user's catch — a capture-first PV that's really a
  tactic — is now handled, not deferred. (Note: a *deep* quiet sac whose
  material only returns past ply 2 still reads as a grab; that's the known
  limit of a 2-ply read, acceptable — those bots search deep anyway.)
- **`miss` = missed a *combination you had to see*, never a piece sitting
  in front of you.** ← now enforced for both quiet-start AND capture-start
  combinations.
- **Tests:** `miss_declines_a_material_winning_best_move` →
  `miss_declines_a_combination_winning_best_move` (knight-fork win on new
  `fork_root()`); added `miss_does_not_decline_an_immediate_capture` (locks
  the exemption), `miss_declines_a_capture_first_sacrifice` (new `sac_root()`
  Damiano fixture), and `two_ply_material_separates_grab_from_combination`
  (classifier: +900 / 0 / −200). `miss_takes_precedence_over_blunder` moved
  to the fork fixture.

### Also noted, NOT yet built — the symmetric half
The rank lever can also **demote *to* a move that hangs your own material**
(observed: `t400` hung its queen via a bad check-block `Qc6`). The easing
only protects the *capture* side. A "don't demote into a ≥minor self-hang
(that's the blunder lever's job)" filter is the symmetric fix — deferred;
weak bots blundering their own pieces is more human than refusing free
material, so lower priority.

---

## chess.com hand-validation findings (the offset + feel)

Playing our configs' moves against chess.com bots (Martin = cc 250):
- **Scale offset:** our **lichess/Maia** numbers run **~200–350 above
  chess.com** at the low end (lichess inflation). So `t400` (lichess ~480)
  ≈ chess.com ~150–250 (Martin-tier) — *consistent*, not a model failure.
  Pin it by bracketing each test bot down to its ~50/50 chess.com crossover.
- **Believability (pre-easing-retune):** capture-shyness (fixed via the
  C=6 easing + pending miss-gating); **king-wiggling** — 3 causes:
  (1) check-sidesteps [easing], (2) declining a free minor [easing],
  (3) pure wiggles in a *lost* position — the endgame king-PSQT slightly
  favors king moves on a flat lost board (confirmed via `search --multi-pv`:
  `Kb6`/`Kc7` were the engine's top two, all within ~20cp). #3 is mostly a
  **symptom of collapsing early** — fix the captures → it stays in the game
  → far less wiggling; residual lost-position wiggle is cosmetic.
- **Validate against the MEASURED ELO**, not the `t###` label (bias +45,
  bots land a touch strong).

---

## The seed ladder (calibration/)

**Why a dense ladder:** the big grid uses the **seed-swap** — 2880 configs
play a fixed seed pool but never each other (keeps games O(configs×pool)).
That only works if the pool is a **dense ladder** (no gap >~250 ELO), else
configs strand as all-win/all-loss. We have only **3 measured human
anchors** (maia-1100/1500/1900 = lichess 1565/1680/1855), so the dense
rungs must be **our own bots** — placed by the measurement (Ordo rates the
connected graph anchored on the 3 Maia), not assumed.

**Scripts (calibration/):**
- `build_ladder.py` — **Phase A**: full round-robin of ~20 hand-picked
  candidates + 9 Maia, one Ordo pass (loose-anchored on the 3 measured
  Maia). ~11–16k games, ~3–5 min. Prints rated ladder + gap/cull report +
  greedy seed suggestion. Used to **measure the knob curves**.
- `design_ladder.py` — **Phase A round 3**: INVERTS the measured linear
  models (`RANK_CURVES`, `BASES`, miss/blunder slopes) to **design a rung
  per target ELO** (base sets band, rank fine-tunes, miss/blunder = small
  garnish whose ELO cost is subtracted from the rank job; endgame tiered by
  band via `tier_for`), then plays + reports **predicted-vs-measured**.
  Rank rounded to **0.1** (GUI step). `--manual-anchor NAME=ELO` adds a
  hand-validated rung as a loose anchor.
- `run_grid.py` / `harness/grid.py` — the eventual **big grid** (2880
  configs: depth×qsearch×rank×blunder×miss×masks). NOT re-run yet.
- `peek_grid.py` — rate finished grid batches mid-run (sims=0 fast).
- `harness/`: `engines.py` (BotConfig→uci args; now has endgame_skill),
  `pools.py` (REFERENCE_BOTS + Maia + MASK_GROUPS), `experiment.py`
  (seed-swap driver), `gauntlet.py` (fastchess), `rate.py` (Ordo, loose
  multi-anchor), `anchors.py` (MEASURED_RAPID = the 3 points).

### What the knobs measure (POST material-easing, 2026-06-05)
- **rank is ~linear in [1,2]; slope scales with base vision** but the
  easing NARROWED that dependence: blind `d1-q0` ~−268/unit, sighted
  `d1-q1` ~−455/unit (was −240 vs −548 pre-easing). Full r1..r7 sweep on
  the blind base: 942/838/674/489/262/128/15/−113 — the new floor is
  r7≈−113.
- **miss ≈ 2.3× stronger than blunder per %** (miss declines a *win*),
  both ~linear (~−5/% miss, ~−2.5/% blunder), combined sub-additive.
- **d4 rank still unmeasured** (slope guess −700/unit) — the weakest part
  of the forward model; add a measured `d4` rank sweep next pass.
- **Forward-model accuracy** (`design_ladder`, post-recompute): **bias +45,
  RMSE 72** (was 124) — errors structured/correctable. A real interaction
  the big grid's regression must capture: **base × rank**.

### Current measured ladder (lichess/Maia scale, STALE after the pending
miss-gating — re-measure before locking)
`floor 146 · t300 372 · t400 476 · t500 588 · t600 697 · t700 764 · t800
910 · t900 885 · t1000 986 · t1100 1124 · t1200 1194 · t1300 1295 · t1400
1461 · t1500 1433 · t1600 1617 · t1700 1744 · t1800 1969 · ceil-d4 1965 ·
d5 2161 · d6 2371`. Maia anchored tight (1571/1702/1826). The config→dials
table is regenerable from `design_ladder` + the results CSV.

---

## NEXT STEPS (in order)

1. ~~**Implement miss-gating** (2-ply material discriminator) + rewrite its
   tests.~~ **DONE** (this session) — *including* the 2-ply sacrifice
   refinement. Only the symmetric self-hang filter ("don't demote *into* a
   ≥minor self-hang") remains deferred.
2. **Re-run `build_ladder`** (re-measure knobs post-easing+miss) →
   **recompute `design_ladder` models** → **re-run `design_ladder`**.
   (Add a measured `d4` rank sweep to fix the upper band.)
3. **Continue chess.com validation** of t400/t1100/t1500 → pin the
   lichess→chess.com offset (constant vs band-dependent).
4. **Raise the GUI avg-rank cap above 4** (r5/r6/r7 reopened) once the
   re-measure shows how high rank must go to reach the floor.
5. **Lock the seed pool** into `pools.py REFERENCE_BOTS` (relabel by
   measured ELO, cull to ~150 spacing, + a floor filler).
6. **Re-spec + run the big grid** against the dense pool (add the
   endgame-skill dimension) → fit the `dials→ELO` model → invert (solver).

## Constraints to honor (memories)
- `avg_move_rank` must be a **0.1 multiple** (GUI step) — never measure a
  rung the product can't reproduce.
- Never run **workspace-wide `cargo fmt`** (repo isn't fmt-clean).
- Judge teaching verdicts by eval-delta vs our OWN engine, never absolute.
- chess.com is accurate-but-opaque; we're transparent — never frame us as
  "more right".

## Key commits (this session, on main)
`ed31ca0` endgame-skill · `c2d0087` endgame GUI · `7d8c03f` mate-guard ·
`6c05643`+`452a886` material easing · `dc4291f` qsearch slider · `a988d0a`
rank slider · `a9f2a14` harness endgame · `3b71186` post-easing re-measure ·
`build_ladder.py`/`design_ladder.py` (ladder bootstrap).
