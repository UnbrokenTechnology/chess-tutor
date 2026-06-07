# HANDOFF — perception lever + perception-era calibration

Cold-resume snapshot, **2026-06-07** (one very long session). Companions:
[`PLAN-perception.md`] (the design doc — read it), [`bands.txt`] (the live
rung table), [`calibration/ladder.md`] (STALE — see warning below),
[`HANDOFF-weak-bot-tuning.md`] (the prior thread this grew out of).

---

## TL;DR — where we are

Built a **move-visibility ("perception") lever**: the bot prunes
hard-to-see moves from its search, so a weak bot misses *humanly-missed*
moves (geometric blind spots) instead of playing engine-random. It
**LANDED and works** (user feel-verdict: "our blunders looked human,
theirs looked random"). Then **removed miss%/blunder%** entirely
(perception subsumes them) and **rebuilt the calibration ladder** around
the new lever set: **depth · qsearch · perception · avg-rank · endgame**.

**Active work when context ran out:** tuning the perception-era ladder.
A user feel-test exposed that **p0 (zero perception) makes a bot blind to
the opponent's captures → hangs everything unrealistically** at the
~1000 level. Fix in flight: **perception = a linear function of target
ELO** (`clamp((elo-500)/900, 0, 1)`), with avg-rank tuned to hit the
number. A second feel-test then exposed a **real engine bug** (the
self-hang filter only catches the *moved* piece hanging, not a move that
*abandons* an already-hanging piece) — now **FIXED** (2026-06-07) by
rebuilding the filter onto the perception-filtered PV (see "self-hang
filter redesign" below). Basement must be re-measured (the fix makes
weak bots weaker + more realistic).

---

## The perception lever — LANDED (committed)

Core: [`core/engine/src/visibility.rs`] (+ `visibility_tests.rs`). Pure
scorer + in-search filter. Read the file's `//!` and `PLAN-perception.md`
for the full model. Capsule:

- **`visibility(pos, mv, ctx) -> V ∈ (0,1]`** = `S × D × K × O × A`:
  - **S salience** = rule-familiarity ONLY (normal moves incl. captures/
    checks/Q-promo = 1.0; castling .80, en-passant .55, underpromo .25).
  - **D direction** (mover-relative): fwd 1.0 · sideways .85 · backward
    .75, **square-rooted for captures** (target pulls the eye; checks NOT
    attenuated so missed mates stay producible).
  - **K knight** ×.85.
  - **O occlusion**: discovered-vehicle ×.75; **diagonal pinch points**
    ×.70 each (a diagonal step whose BOTH flank squares are occupied —
    the "squeeze through a zero-width corner"; orthogonal slides exempt;
    the gap-width model, user's insight).
  - **A attention**: two-endpoint distance from opponent's last-move sq
    (`g: ≤2→1.0, 3-4→.95, ≥5→.90`, applied to BOTH from+to) × endpoint
    clutter (king-ring occupancy: 7-9→.94, ≥10→.88).
- **`p_see(v, perception)`** = margin curve on `m = p − (1−V)`:
  perception-scaled plateau `1 − 0.2·(1−p)` ramping to deterministic 1.0
  at margin ≥ .30; quadratic cliff to literal 0 at margin ≤ −.45.
  `V==1.0` and `p≥1.0` are exact short-circuits (always seen).
- **`sees(seed, zobrist, move, p)`** — deterministic roll, **NO ply
  mixing** → TT-coherent, stable per-game blind spots (misses the same
  diagonal all game).
- **Opponent plies** apply `V^1.5` before the curve (Hope Chess: subtle
  refutations missed more than subtle opportunities → organic blunders).
- **In-search**: `SearchParams.perception: Option<PerceptionParams>`
  (`level ≥ 1.0` normalizes to None = byte-identical bypass, verified).
  Filter after legality in `negamax_moves` + qsearch; never in check;
  **never-empty fallback** (`MovesOutcome.unseen_fallback` = highest-V
  pruned move, re-run pinned); root-check exemption when
  `guaranteed_mate_in ≥ 1`. `OpponentProfile.perception` (manual Default
  = 1.0) → `perception_params(last_move_to)`. Analytical paths pass None.
- **Surfaces**: `--perception` on `play`/`uci`/`search`; desktop New Game
  "Perception" slider; GUI avg-rank cap lifted 4.0→8.0.
- **Tuning constants** are FROZEN-BY-THE-GRID: re-weighting invalidates
  the ladder + future grid. Empirical weight calibration (lichess-dump
  P(hung-piece-captured) regression) is a DEFERRED follow-up.

### settled-ply redesign (landed, prerequisite work, commit `c2ce8cc`)
`settled_ply` now = **material resolution** (forward walk: last forcing
event before the first run of 3 quiet plies), NOT eval drift. Old
backward 25-cp walk dragged to the leaf ~90% of lines (audit-confirmed;
`1. c4` classified a material loss). `chess-tutor settled-audit` is the
temporary instrumentation. Three distinct notions, don't reconflate:
`material_settled` (noise + material_outcome), **leaf** (initiative
eval-swing), **forcing-tail/climax** (positional-win card).

---

## Committed this session (newest-first, branch `main`)

- `425fc6d` "monotone ladder LOCKED" — **MISLEADING**: its `run_ladder.py`
  actually contains the FINAL (q0) rungs, NOT monotone (heredoc bug —
  see GOTCHA). `ladder.md` in it has a **fictional measured column**.
- `a9f065b` FINAL ladder candidate + dense extremes (`run_extremes.py`).
- `ff5bd60` rung-design sweeps; GUI rank cap → 8.0.
- `b6a73b1` **remove miss/blunder dials** everywhere (engine/ui/cli/
  harness). `NoiseProfile` = {avg_move_rank, guaranteed_mate_in}.
  `NoisePick::Blunder/Miss` gone. Bot-strip chip → Perception.
- `b825e22` pinch model in plan + sweep results.
- `8997a1e` threading → **diagonal pinch points** (gap-width model).
- `7ec33dc` perception sweep harness + first dial→Elo curves.
- `ffff4bb` perception revision (capture-attenuation, scaled plateau,
  clutter) after first playtest.
- `fc51532` **perception lever LANDED**.
- earlier: `3bef920`/`6fc91e9`/`777f0ad`/`2da51ab`/`f973d47` (plan
  iterations + 5-agent research synthesis), `c2ce8cc` (settled redesign),
  `436674f` (settled audit instrumentation).

### Uncommitted working tree (decide what to keep)
- `?? bands.txt` — the live human-readable rung table (q1-floor version,
  then overwritten with ramp values mid-session; **currently shows the
  q1-floor pre-ramp basement** — regenerate after the ramp is nailed).
- `?? calibration/run_q1floor.py` `run_ramp_cells.py` `run_rampbase.py`
  `run_bands.py` — sweep scripts (run_bands is legacy/stale).
- `M calibration/run_ladder.py` — has the **ramp rungs** (via Edit tool;
  the guard confirmed). NOT committed.
- `M build_ladder.py` `design_ladder.py` `HANDOFF-weak-bot-tuning.md` —
  pre-existing legacy modifications from before this session; the legacy
  ladder scripts are SUPERSEDED (they construct miss/blunder configs and
  now fail loudly).
- `ladder.md` is committed-but-STALE (the fictional monotone table) —
  **regenerate honestly** from the real ramp numbers before re-locking.

---

## ⚠️ CRITICAL GOTCHA — heredoc writes to run_ladder.py don't persist

`python - <<'PYEOF' ... f.write('run_ladder.py') ... PYEOF` **silently
failed to persist** all session — printed "ok", file unchanged. Proof:
`git show 425fc6d:calibration/run_ladder.py` contains FINAL rungs, not
the monotone rungs I "wrote". So the **monotone ladder was never actually
measured** (every run secretly re-ran FINAL → identical RMSE 39). The
**harness Edit/Write tools persist correctly.** RULE: edit `run_ladder.py`
**only via Edit/Write tools**, and **guard every run** by checking the
`--design-only` output for an expected dial before playing (a one-liner
`if ... | grep -q "perception 0.55"; then run; else abort`). Other
files (bands.txt, new scripts) wrote fine via heredoc — the failure was
specific to overwriting this existing file (cause unknown; possibly an
open handle from a concurrent run).

## ⚠️ Measurement gotcha — extreme-band float

Weak rungs (sub-~800) lose ~100% to everything above them, so in a sparse
pool their Ordo rating **floats down hundreds of Elo** (t500 read −56 in
one pool, 510 in another — same config). FIX: measure extremes **densely
with boundary anchors** (the `run_extremes.py` / `run_rampbase.py`
pattern: a dense self-connected sub-ladder + 2-3 stable rungs pinned at
known values). `run_rampbase.py` anchors on the q1floor d1q1-p0 points
(992/750/606) and its anchors reproduced (999/773/576) → reliable. Also:
Maia anchors **compress ~150-250 Elo** in weak-heavy pools (loose
multi-anchor doesn't fully fix scale) → treat low-pool absolutes as
±100 soft; shapes robust.

---

## Calibration ladder — current state (IN PROGRESS)

Lever set: **depth · qsearch · perception · avg-rank · endgame**. Targets
100-2500 by 100, **lichess-anchored** (chess.com ~200-350 lower). The
old miss/blunder bands are dead.

### Key measured structure (durable, drives the grid design)
- **Perception is a VERY strong lever**, ~150-200 Elo per +0.1 *even at
  basement rank* (e.g. d1q1 p0.55 r2.6 = 1530, vs p0 r2.6 ≈ 700s).
- **Perception × qsearch sub-additive**: spans ~195 Elo on d1q0 (blind
  base, nothing to reveal) vs ~960 on d4. Power scales with base.
- **Knee climbs with depth** (d1q1/d2q2 ≈ 0.6, d4 ≈ 0.6-0.8, ≈ inert by
  0.6 at d6+). Useful range below the knee.
- **Top quantizes to depth** (p inert up there): d5≈2150, d6≈2350,
  d7≈2475-2555, d8≈2750. Finer top lever (node caps) deferred.
- **q1 is a HUMAN floor; q0 is sub-human.** q0 = can't see the immediate
  recapture → parks its queen in front of a pawn (the search's own #0
  hangs). q1 sees recaptures. New basement uses **q1 minimum**; t100-t400
  dropped (chess.com floor ~250 ≈ our 500). q1-p0 rank curve (dense,
  reliable): r1=1166, r2=992, r3=750, r4=606, r5=444, r6=343, r8=258.

### The LINEAR-PERCEPTION ramp (current direction)
`perception = clamp((elo-500)/900, 0, 1)` (0 at t500, 1.0 at t1400+),
rank tuned to hit the target. Reliable basement ranks (from
`run_rampbase.py`, anchored): d1q1, formula-perception, to hit targets
needs **rank 4.3-5.7** (perception is so strong that rank must climb
high + comes out non-monotone). `run_rampbase.py` measured cells:
```
t500 p0.00 r4.6=527 | t600 p0.10 r4.0=799 r4.5=740 | t700 p0.20 r3.4=1047 r4.0=915
t800 p0.35 r3.0=1295 r3.6=1166 | t900 p0.45 r2.5=1484 r3.2=1325 | t1000 p0.55 r2.0=1716 r2.6=1530
```
The high/wobbly ranks raised an **open design question** (below).

### Honest note on what's actually measured vs not
- Basement d1q1-p0 rank curve: reliable (`run_q1floor.py`, dense).
- Ramp basement cells: reliable (`run_rampbase.py`, anchored).
- Mid/upper (t1400+, d2q2-p1.0-rank + depth): measured in the FINAL pass
  (which is what 425fc6d/a9f065b actually ran) at RMSE ~39 — those rungs
  (t1400 d2q2 p1.0 r1.7≈1400 ... t1800 r1.2≈1800, t1900 d4 r1.6, t2000
  d4, t2100 d5 r1.2, etc.) are trustworthy.
- The "monotone" ladder.md table is FICTIONAL (heredoc bug). Ignore it.

---

## RESOLVED — the self-hang filter redesign (2026-06-07)

User feel-test (FEN `r1b1k1nr/ppp2ppp/2np4/8/1b2P2q/P1N5/1PP1NPPP/
R1BQKB1R b`): a d1-q1-p0.55-r2.5 bot played **Bxa3**, throwing the bishop
to the pawn. Root cause: the **black bishop on b4 was already hanging**;
the search correctly ranked the bishop-losing moves low, but the old
self-hang filter (`self_hang_pawns`, a full-strength SEE oracle) only
checked whether the **MOVED** piece lands en prise — it never saw a move
that *abandons* an already-hanging different piece, so Kf8/Qxe4/Nf6 sailed
through and rank-noise sampled one.

**What we built instead of the originally-proposed SEE extension** (key
insight from the user: the old SEE oracle was *perception-blind* — it
overrode the perception lever's realistic blindness; and the Bxa3 case
wasn't even a perception miss, since a pawn capture is high-salience and
the search *did* see the loss). The filter now reads each line's
**settled material delta straight off the perception-filtered PV** and
drops a line iff it is (a) **down material** (`deltas[i] < 0`) AND (b)
**avoidably so** (`max(deltas) > deltas[i]` — a better line keeps more).
Consequences:
- **Perception-aware for free.** A loss the bot never saw (the punishing
  capture was pruned by the perception filter, so it's absent from the
  PV) reads as safe → NOT filtered → the bot commits the realistic,
  geometry-shaped blunder. A loss it *did* see (a fresh recapture at the
  attention locus, present in the PV) is filtered. This is exactly the
  "opponent-just-attacked-my-queen is hard to miss, but a quietly-hanging
  piece is missable" distinction the user asked for.
- **Catches abandoned pieces** (and the moved piece) uniformly — it's any
  material the line gives up, not a moved-piece-specific SEE check.
- **Drop probability is now rank-dependent** (mirrors capture-rescue):
  `P(drop) = min(1, lost_pawns / (SELF_HANG_C·(rank−1)))`, magnitude =
  saveable material `max(deltas)−deltas[i]`. `SELF_HANG_C = 3` → a queen
  is saved through **rank 4** (it takes rank > 4 to hang a queen); rook
  ~56% at r4; weaker bots save less. (Old curve was rank-INdependent
  `v/9`.)
- **Old SEE function `self_hang_pawns` deleted**; reuses the `deltas`
  already computed for capture-rescue (cheaper — no per-line SEE clone).
- **q0 loses self-hang protection by design** (its PV has no recapture →
  delta 0 → never filtered → parks its queen, which is what q0 *means*).
  q0 is off the product ladder; reachable only via the advanced dropdown.

Landed in `core/engine/src/noise.rs` (+ rewritten `noise_tests.rs`: new
`self_hang_saves_a_perceived_queen_drop`, `self_hang_ignores_an_unperceived_hang`,
`self_hang_filters_an_abandoned_piece`). All 935 engine + full workspace
tests pass, clippy clean. **NOT yet committed.** **Makes weak bots weaker
+ more realistic → basement must be re-measured (`run_rampbase.py`).**

## OPEN — basement design decision (A/B/C, user to choose)
Perception is so strong that the linear formula forces high/wobbly
basement ranks. Options put to the user:
- **(A)** Linear formula as-is, accept rank 4.3-5.7 in the basement.
- **(B)** Keep **p0 through the LOW basement (t500-t700)** — a 600 player
  genuinely DOES hang blindly, so p0 is believable there; ramp perception
  only from ~t800 where "hangs everything" stops being believable. Keeps
  basement ranks sane (the q1floor p0 curve). **My recommendation.**
- **(C)** Add an eval-mask to the basement (weaken via "doesn't grasp
  king-safety" instead of high rank) — more human, reintroduces a dial;
  user wants eval-mask in the FINAL solver anyway (needs full grid +
  scipy regression), not now.

## OPEN — pending feel-tests
- **A**: `d1 q1 p0.55 r2.5 Basic` — DONE. p0.55 fixes the queen-parking
  (✓ believability); user stomped it (expected — user is ~1400-1500
  lichess vs a 1000 bot).
- **B**: `d1 q1 p0.1 r5.5 none` — NOT YET PLAYED. The crux: does high rank
  read as "weak human" or "nonsense"? Decides A vs B/C above.

---

## NEXT STEPS (in order)

1. **Self-hang filter fix — DONE** (see RESOLVED section; PV-delta,
   perception-aware, rank-scaled). Remaining: **commit it**, then
   **re-measure the basement** (`run_rampbase.py`) — it affects every weak
   bot, so the q1floor / rampbase curves shift and must be re-derived
   before locking rungs / running the grid.
2. **Resolve the basement design** (A/B/C) — gated on feel-test B and the
   self-hang fix (which softens the high-rank tension).
3. **Finalize the perception-era ladder** (25 rungs, perception-monotone,
   q1 floor) → regenerate `bands.txt` + an honest `ladder.md` →
   confirming pass via `run_ladder.py` (EDIT TOOL ONLY + guard).
4. **Commit the calibration set** (bands.txt, the new run_*.py scripts,
   run_ladder.py) once rungs are locked.
5. **User chess.com feel-tests** at the lichess→chess.com offset (test a
   500-ELO bot vs chess.com's 250 Martin, etc.).
6. **Re-spec + run the full grid** — now depth × qsearch × **perception**
   × rank × eg × masks (miss/blunder GONE). Bake in the measured
   structure (perception×qsearch sub-additivity, depth-scaling knee,
   sample perception densely below the knee). Then scipy regression →
   invertible model → the single "opponent ELO" slider + advanced
   dropdown.

## Constraints / repo rules (carried)
- avg_rank must be a **0.1 multiple** (GUI step); perception on a 0.05
  grid. Never anchor a rung the product can't reproduce.
- Never run workspace-wide `cargo fmt`; bench single-threaded; release
  builds for perf. Commit straight to `main`.
- Flag engine changes (like the self-hang fix) before implementing.
- Analytical paths (retrospective/hint/analyze) NEVER read perception /
  noise / eval-mask / qsearch-cap / endgame-skill — full strength always.
