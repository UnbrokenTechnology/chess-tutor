# HANDOFF — perception lever + perception-era calibration (durable findings)

Companion to the **live** calibration work in [`HANDOFF-solver.md`](HANDOFF-solver.md)
(the grid re-run + lookup bake — start there). This file is the durable
record of the perception lever and the perception-era ladder: what's true and
feel-validated, with the session play-by-play retired to git history. Harness
internals: [`calibration/HANDOFF-calibration.md`](calibration/HANDOFF-calibration.md).
Evidence base: [`RESEARCH-perception.md`](RESEARCH-perception.md).

---

## The perception lever — LANDED

A **move-visibility lever**: the bot prunes hard-to-see moves from its search,
so a weak bot misses *humanly-missed* moves (geometric blind spots) instead of
playing engine-random. Feel-verdict: "our blunders looked human, theirs looked
random." With it landed, **miss%/blunder% were removed entirely** (perception
subsumes them). Lever set is now: **depth · qsearch · perception · avg-rank ·
endgame**.

Core: [`core/engine/src/visibility.rs`](core/engine/src/visibility.rs) (read
its `//!` + `RESEARCH-perception.md` for the full model). Capsule:

- **`visibility(pos, mv, ctx) -> V ∈ (0,1]`** = `S × D × K × O × A`: salience
  (rule-familiarity only), direction (fwd/side/back, √ for captures), knight
  ×.85, occlusion (discovered-vehicle + diagonal pinch points), attention
  (two-endpoint distance from opponent's last move + king-ring clutter).
- **`p_see(v, perception)`** = perception-scaled plateau ramping to a
  deterministic 1.0, quadratic cliff to literal 0 below threshold. `V==1.0`
  and `p≥1.0` short-circuit (always seen). Opponent plies apply `V^1.5`
  (Hope Chess).
- **`sees`** keys a deterministic roll on `(game_seed, zobrist, move)` with
  **no ply mixing** → TT-coherent, stable per-game blind spots.
- **In-search filter** after legality (never in check; never-empty fallback;
  root-check exemption when `guaranteed_mate_in ≥ 1`). `perception ≥ 1.0`
  normalizes to a byte-identical bypass. Analytical paths pass `None`.
- **Self-hang filter** (`noise.rs`) reads each line's settled material delta
  off the **perception-filtered PV** and drops a line iff it is down material
  AND avoidably so — perception-aware for free (a loss the bot never saw isn't
  in the PV → not filtered → it commits the realistic blunder), catches
  abandoned pieces, rank-dependent drop probability.
- **Tuning constants are FROZEN BY THE GRID** — re-weighting invalidates the
  ladder + grid. Empirical weight calibration (lichess-dump regression) is a
  DEFERRED follow-up; v1 weights are feel-tuned.

### settled-ply (prerequisite, landed)
`settled_ply` = **material resolution** (forward walk to the first run of 3
quiet plies), NOT eval drift. The old backward 25-cp walk dragged to the leaf
~90% of lines. Three distinct notions, don't reconflate: `material_settled`
(noise + material_outcome), **leaf** (initiative eval-swing),
**forcing-tail/climax** (positional-win card).

---

## Perception-era ladder — structure (carries into the grid/lookup)

Lever schedule (one dial at a time, qsearch before depth): `t500-700 d1q1 ·
t800-1200 d1q2 · t1300-1500 d2q2 · t1600-1800 d2qinf · t1900-2500 d4..d7 qinf`.
Perception ramps `clamp((elo-300)/900, 0, 1)` (0 at t300, 1.0 at t1200), inert
above the ~0.6 knee → the active lever only in the basement; rank does the
mid/top. eg tiers: t500-900 basic, t1000-1900 inter, t2000+ full.

Durable structure:
- **Perception is a VERY strong lever** (~150-200 Elo per +0.1 even at basement
  rank) and **monotone in Elo** by construction (saturates at 1.0 by t1400) →
  the solver never sees a dial flip.
- **avg-rank is U-shaped**: high in the basement (no other lever there), ~1.0
  through the perception-driven middle, rising again from t1400 as the
  judgment lever.
- **Perception × qsearch is sub-additive**: ~195 Elo span on d1q0 (blind base,
  nothing to reveal) vs ~960 on d4. Power scales with the base.
- **Perception's knee climbs with depth** (d1q1/d2q2 ≈ 0.6, d4 ≈ 0.6-0.8,
  inert by 0.6 at d6+). Useful range is below the knee.
- **q1 is a HUMAN floor; q0 is sub-human** (can't see the immediate recapture →
  parks its queen). q0 is off the product ladder (advanced dropdown only).
- **The top (>2100) quantizes to depth**: d5≈2150, d6≈2350, d7≈2475-2555,
  d8≈2750. Smooth 100-pt rungs up there need a finer lever (node caps) —
  deferred, above the product core.

The ladder was assembled t500-t2500 (mid LOCKED against Maia at RMSE ~35;
t2000-t2500 provisional ±100 pending a dedicated top pass). It remains the
**feel-validated default-slider path** in `core/engine/src/calibration.rs`
(`config_for_elo`); the grid lookup (see HANDOFF-solver) only drives the
advanced-tab delta display + the iterative inverse.

---

## CHESS.COM OFFSET ≈ 0 (feel-validated 2026-06-07)

The earlier "lichess runs ~200-350 above chess.com" was a **basement-floating
artifact**, not a real scale gap. Three independent feel-tests on the fixed
ladder disprove it:
- **t500 ≫ Martin** (chess.com 250): t500 positionally crushed Martin.
- **t1200 = the user's level** (a chess.com ~1200 player): felt like a peer.
- **t1400 beat Mateo** (chess.com 1400) and chess.com's own Game Review rated
  our bot 1300 / Mateo 800 for that game.

So **target Elo ≈ chess.com Elo directly** — no offset shift on the slider.
Caveat: the lichess↔chess.com gap may be band-dependent; this is the floor-to-
mid read. Implication: the floor could extend DOWN below t500 believably
(self-hang + eg1 basement reaches chess.com's ~250 tier) if sub-500 rungs are
ever wanted.

---

## Humanity at higher Elo (open, user-flagged, revisit later)

Low-Elo bots feel human (believable blunders ✓). At higher Elo the user (a
chess.com ~1200) couldn't judge — couldn't parse the positional moves. Two
entangled gaps: (1) **legibility** — the teaching layer names concepts but
doesn't tie them to the specific puzzling move tightly enough for a 1200 to
learn (the next teaching-UX frontier); (2) **plan-coherence** — at d2 the bot
can't form multi-move plans; moves come from the SF11 classical eval, giving
*emergent* (not intentional) coherence. Perception fixes *misses*, not plan
narrative — the threaded-intent gap is a distinct, harder future lever. User
shelving until stronger at chess.

## Measurement lessons (durable)
- **Lopsided games carry ~no info about gap SIZE** (info ∝ p(1−p)). A region
  reachable from the anchors only through near-shutout links floats. Fix: an
  unbroken chain of competitive (~100-Elo) links to the anchors (basement via
  a locked mid; tune the mid FIRST, then let it bridge the basement up).
- **Maia is a noisy ruler** — non-transitive + compressed (~290-Elo span).
  Absolute calibration is ±~100 regardless; optimize SHAPE, let chess.com
  feel-tests pin the offset.
- **eg-skill buys endgame conversion without touching middlegame strength** (it
  fires only in recognized endgames) — weak bots need it to convert won
  endgames instead of shuffling to a draw.

## Constraints (carried)
- avg_rank must be a **0.1 multiple** (GUI step); perception on a 0.05 grid.
- Never run workspace-wide `cargo fmt`; bench single-threaded; release builds;
  commit straight to `main`.
- Flag engine changes before implementing.
- Analytical paths (retrospective/hint/analyze) NEVER read perception / noise /
  eval-mask / qsearch-cap / endgame-skill — full strength always.
