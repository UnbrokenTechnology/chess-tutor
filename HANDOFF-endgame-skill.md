# HANDOFF — endgame-skill lever + the playtest that drove it

Session snapshot, **2026-06-05**. Captures the Martin-vs-bot playtest, the
Q→B underpromotion bug it surfaced, the **endgame-skill tier lever** we
built in response, and the calibration-harness changes from the same
session. Read alongside [`HANDOFF-calibration.md`](calibration/HANDOFF-calibration.md).

---

## TL;DR

- A manual playtest (chess.com's **Martin**, ~250, vs our `d1-q0-r1`
  bot) ended in a draw because our bot **failed to convert a won
  endgame** — it underpromoted a pawn to a **bishop instead of a queen**,
  reached KBNK, and shuffled to a threefold draw.
- Root cause of the underpromotion: an **SF11-inherited eval quirk**. The
  `kbnk` specialist returns `KNOWN_WIN (10000) + PushToCorners (≤6400)`,
  which **out-ranks a queen** (which routes through generic `kxk` =
  `material + KNOWN_WIN + PushToEdges(≤100)`). There is no `kqnk`
  specialist; SF11 has the same gap and relies on deep search finding the
  queen's fast mate to hide it. Our crippled weak bots (qsearch=0, depth 1)
  can't see that mate, so the static inversion decides the move.
- We **did not** "fix" the eval (it's SF-faithful, and search hides it at
  strength — deferred). Instead we built the lever the bots actually
  needed:
- ✅ **`EndgameSkill` tier lever LANDED** (commit `ed31ca0`) — a
  play-engine-only difficulty ladder. A weak bot is denied the harder
  closed-form endgame specialists and falls back to classical eval, so it
  **misplays endgames like a human of its level** (shuffles a won KQ,
  botches KBNK, stalemates) — and, as a free side effect, **queens
  instead of underpromoting** (no `kbnk` override to invert the ranking).

---

## The playtest (what it showed)

`d1-q0-r1` (depth 1, qsearch 0 "tactically blind", best-move) played Black
vs Martin (White). Both sides hung pieces throughout the middlegame — but
our bot **won material and reached −5.17 (a clean knight up)** by move 15.
It then traded down to K+N+P vs K, promoted the pawn **to a bishop**
(→ KBNK), and wiggled the knight into a **threefold-repetition draw** that
chess.com flagged as a missed win.

**Reframes this produced:**
- `d1-q0-r1` is **not** a true ~250 bot — it's *positionally ~1800,
  tactically blind*. It outplayed Martin on position and only drew via the
  endgame failure. So the "felt like 250" reads as **a thrown won game**,
  which biases its measured Elo *downward*.
- This is a chimera: a real ~250 human is weak at *both* tactics and
  position. The pure `qsearch=0` lever alone produces an unrealistic
  weakness profile — which is *why* the strength model pairs qsearch
  (tactical horizon) with eval-masks (positional sense), noise, **and now
  endgame-skill**.

---

## The underpromotion bug (full diagnosis)

Confirmed with the CLI on the post-promotion FEN
`8/8/6K1/8/4k3/8/1n6/b7 w` (White = lone king; Black = K+B+N, choosing the
promotion; more-negative white-POV = better for Black):

| Promotion | Material | Eval (white-POV) | Path |
|---|---|---|---|
| **Bishop** | K+B+N vs K | **−72.96** | `kbnk.rs` → `KNOWN_WIN + PushToCorners` |
| Queen | K+Q+N vs K | −63.28 | generic `kxk.rs` → `material + KNOWN_WIN + PushToEdges` |
| (ref) Q alone | K+Q vs K | −59.62 | generic `kxk.rs` |

Bishop scores ~9.7 pawns *better* than a queen → the engine underpromotes.

**It's SF11-faithful, not a port bug.** Diffed against
`reference/Stockfish-sf_11/src/endgame.cpp`: our `kxk.rs`, `kbnk.rs`,
`KNOWN_WIN = 10000`, and the `PushToCorners` table (6400…) are byte-for-byte
SF11. SF11 also has no KQN specialist and the same static inversion. SF
never visibly suffers because **deep search + full quiescence finds the
queen's short forced mate** (a mate score ≫ any `KNOWN_WIN` static), so the
queen wins at the root. The inversion only decides a move when search is
too shallow to see the mate — i.e. our `qsearch=0`/depth-1 weak bots (and,
confirmed, full-strength too if the mate is beyond the search horizon: a
`search --depth 16` from a far KNP position still played `e8=B`).

**Decision: full-strength eval fix DEFERRED** (revisit only if it misfires
in real teaching analysis). The weak-bot manifestation is handled by the
endgame-skill lever below.

---

## The endgame-skill lever (LANDED — commit `ed31ca0`)

`EndgameSkill { None, Basic, Intermediate, Full }` in
[`core/engine/src/endgame/mod.rs`](core/engine/src/endgame/mod.rs). A
difficulty-ordered ladder; `probe_with_skill(pos, skill)` consults a
specialist only if `skill >= its tier`, else falls through to a coarser
one (or to classical eval at `None`).

| Tier | Adds | "knows…" | ~human |
|---|---|---|---|
| 0 `None` | — (classical eval only) | nothing; misplaces kings, stalemates, queens-not-underpromotes | sub-1000 |
| 1 `Basic` | `KXK` (KQK/KRK + K+pawns generic) | trivial major-piece mates | ~1000 |
| 2 `Intermediate` | `KPK` bitbase, KQKP/KRKP/KQKR/KRKB/KRKN, KNNK-draw | opposition + piece technique | ~1400 |
| 3 `Full` | `KBNK`, `KNNKP`, fortress scaling fns | the hard mates + fortress draws | ~1800+ |

**Plumbing — mirrors `eval_mask` exactly** (it's the established
play-engine-weakening pattern; see the `eval_mask` field on `Search`):
- `material::evaluate_with_skill(pos, skill)` → `probe_with_skill`.
- `eval::evaluate_with_pawn_cache(pos, cache, mask, eg_skill)` →
  `evaluate_inner` → `Evaluator::new_with_pawns(.., eg_skill)`. Full-
  strength entry points (`evaluate`, `evaluate_with_trace`) pass `Full`.
- `Search.eg_skill` field, set from `SearchParams.endgame_skill` at
  `run()`; passed at every eval call site (negamax, qsearch, static_eval).
- `OpponentProfile.endgame_skill` (default `Full`) → `SearchParams`
  → the play worker in `core/ui/src/session/worker.rs` reads
  `self.opponent.endgame_skill`. **Strict invariant: analytical /
  retrospective / hint paths always pass `Full`** so teaching judges true
  best play (`core/ui/src/worker.rs` retrospective, `analysis/*`).

**Surface:** `--endgame-skill TIER` on `chess-tutor uci` (harness) and
`chess-tutor search` (inspection); `EndgameSkill::from_tier(u8)` maps
`0→None,1→Basic,2→Intermediate,_→Full`. `--endgame-skill` omitted ⇒ `Full`.

**Verified:** `chess-tutor search "8/4P3/4K1N1/8/8/8/8/6k1 w" --depth 16
--endgame-skill 0` → `e8=Q … Qf2#` (queens + mates) vs `e8=B` at full
books. KBNK eval by tier: 0 = −7.3 (plain classical material, the
believable "no technique" value), 2 = −55 (generic KXK), 3 = −71 (the
inflated `kbnk` specialist). All 911 engine + 113 CLI + 127 UI tests pass.
Unit tests `skill_tiers_withhold_harder_specialists` +
`low_skill_prefers_queen_over_bishop_promotion` pin both the fix and the
deferred Full-tier quirk.

**GUI:** ✅ the New Game dialog has an **Endgame skill** slider (None /
Basic / Intermediate / Full, default Full at far right), in the strength
grid below the Tactical-vision slider. Wired through `NewGameForm` →
`start_new_game` → `OpponentProfile.endgame_skill` → play search.

**Follow-up not yet done:**
- **Endgame-skill as a grid dimension** — it's a new believable-floor
  lever (tier 0 = botches endgames), a candidate axis in the grid re-spec.

---

## Calibration-harness changes this session (see HANDOFF-calibration.md)

- **Grid REDESIGNED** (`run_grid.py`/`grid.py`): depth × qsearch-depth ×
  rank × blunder × miss × **eval-mask combos** (safety / positional folded
  in as a real axis to capture the mask×tactical sign-flip), mate-in pulled
  to its own `run_mate_sweep.py`. 2880 configs, ~6.5 h. (commits `4d5d105`,
  prior).
- **`peek_grid.py`** added — rate finished grid batches mid-run into a
  separate `grid_peek` output (isolated from the live run). Fixed the
  `sims=0` Ordo parse (error column omitted at `-s 0`) (commit `e37bfd9`).
- **First grid run was ABORTED** — its peek showed the pool has **no floor
  below ~1128** (`ref-d1-q0`), so sub-~900 configs go all-loss and Ordo
  excludes them (231 unratable). Configs are non-seeds → they only play the
  18-bot pool, never each other. **Fix not yet built:** a *ratable basement
  cluster* of weak reference rungs (≈250–800, the lowest two close enough
  to trade wins, since an excluded all-loss rung cascades and helps no one).
  `floor_calibrate.py` (measure candidate rungs first) is the next step.
- **Open: Maia-anchoring validity** — our `q0` bots are positionally-1800 /
  tactically-blind chimeras; head-to-head vs human-like Maia may
  over/under-state them. Plus the lichess→chess.com offset (a post-fit shift).

### Floor candidate from playtest 2 (~100 Elo)

A second manual game (2026-06-05) — **`d1-q0-r2` + 10% blunder + 10% miss,
no eval mask** (all positional signals on), our bot Black vs Martin White
— **Martin won**. It felt like a believable ~100-Elo floor:
- `q0` (no tactical vision) → misses hung pieces and can't follow up the
  positional plans its full eval suggests (overextends).
- `r2` (avg-move-rank 2) is the key ingredient: it routinely plays the
  *2nd*-best move, so it **fails to punish even obvious one-move blunders**
  — every time Martin hung his queen, "capture the queen" was best and the
  bot played the second-best instead, letting the queen live and clean up.
- The 10% blunder / 10% miss may be **unnecessary** — `d1-q0-r2` alone
  likely produces this.

**Bug found + fixed in the same game (`7d8c03f`):** `guaranteed_mate_in=1`
still missed a mate-in-1. The mate guard suppressed the miss/blunder
branches but **not** the variety (`avg_move_rank`) branch, so rank-2
demoted off the mate. Now all three branches respect the guard; mates
deeper than the guarantee stay unprotected (normal noise — "can't see that
far"). The dial is a *protection floor*, not a search cap.

**Takeaway for `floor_calibrate.py`:** `d1-q0-r2` is a strong **~100-Elo
floor-rung candidate** (the basement the weak grid configs need to be
ratable). `avg_move_rank` is the lever that makes a bot *stop punishing
hung material* — exactly the sub-200 human behavior. Pair it with weaker
rungs (more rank / blunder) for the all-loss-trading basement. Needs
confirmation in the harness, but it's the most promising floor seen so far.
(Also a data point for the eventual endgame-skill grid axis: this bot ran
`Full` endgame books — re-test at tier 0 for even-weaker, no-technique play.)

### Material-aware rank/miss — believability fix (SPEC, 2026-06-05)

**Problem.** The variety (`avg_move_rank`) and `miss` levers pick by
*rank*, blind to *material*. They demote/decline off a blindingly-obvious
winning capture — chess.com validation of `t400` showed it refuse to
recapture a hanging queen (next to a rook) for **3 turns**, and hang B/Q/R
vs Martin. cp can't fix it: a *subtle* only-move and an *obvious* recapture
both show a big cp gap to #2. **Material** is the discriminating signal.

**Fix (no new user lever).** Before the miss/variety demotion, look at the
**material swing you'd throw away** by not playing the best move (reuse
`line_material_delta_cp` already in `noise.rs`). Play the best move with
probability **P that increases with that swing** — *graduated, not a hard
rule*: ~0.97 for a queen, ~0.9 rook, ~0.75 minor, low for a pawn; always
above what raw rank demotion gives. If the roll says "take," return
`Line(0)`; else fall through to normal miss/variety. (One fixed curve, not
a dial.)
- **Preserves the wanted feel:** a move that's positionally worse but
  *materially fine* is still demotable (the "subtle only-move / zugzwang
  misjudgment" feel). A move that hangs a piece is (usually) excluded.
- **NOT targeted:** missing a subtle *defensive* only-move (the eval-bar
  "only move that worked" case) — that's the zugzwang feel we keep.
- Geometry/ease (long-range snipe stays missable) is a later layer.

**Consequences (important):**
- Hanging material then comes only from **tactical blindness (qsearch)** or
  the **intentional blunder lever** — never incidentally from rank. The
  three weakening levers become orthogonal.
- All noisy bots get **STRONGER** → **re-run `build_ladder` / `design_ladder`**
  to re-measure (the ladder Elos shift up).
- **Reopens r5/r6/r7** as useful basement rungs — r4 was the floor only
  because high rank caused catastrophic *incidental* hangs; with those
  gone, high rank is sane-but-weak. → likely **raise the GUI rank-slider
  cap back above 4** after re-measuring (it was just set to 1.0–4.0).

---

## Open threads / next steps (in rough order)

1. ✅ **GUI knobs**: qsearch-depth slider (0–10, 10=∞ default), endgame-skill
   slider, avg-rank slider re-scaled to **1.0–4.0 by 0.1** (was 1–10/0.5 —
   couldn't express 1.9).
2. ✅ **Seed-ladder bootstrap built** (`build_ladder.py` → `design_ladder.py`)
   — full round-robin of a small candidate set + Maia, one Ordo pass. This
   *supersedes* the old `floor_calibrate.py` plan. Learnings:
   - **rank is ~linear in [1,2], slope SCALES with base strength**
     (~240/unit blind → ~550 q1 → ~1090 d4): a strong base×rank interaction
     the grid regression must capture.
   - **miss ~2.3× stronger than blunder per %** (miss declines a *win*),
     both ~linear, combined sub-additive.
   - `design_ladder.py` inverts these to design a rung per target Elo;
     forward model holds at **bias −17, RMSE 124** (errors structured: blind
     rungs land +90 strong as miss does less when shallow; d4 rank noisy).
   - A measured ladder now spans −140 → 2397; runs/ has the CSVs.
3. ⏳ **Hand-validation IN PROGRESS** (user): `t400`/`t1100`/`t1500` vs
   chess.com bots → the **lichess→chess.com offset** (constant? band-
   dependent?) + a believability check. Then snap & lock.
4. **Lock the seed pool** into `pools.py` (`REFERENCE_BOTS`) from the
   measured ladder (relabel by measured Elo, cull to ~150 spacing, + a
   floor filler), then **re-run the big grid** against the dense pool.
5. **Re-spec the grid** to add the **endgame-skill** dimension (+ the floor).
6. **Desktop New Game combo** for endgame-skill (slider already wired
   in-app; this item is stale — DONE).

**Constraint for the grid + the eventual solver:** `avg_move_rank` must be
a **0.1 multiple** (GUI slider step) — never measure/anchor a rung the
product can't reproduce. `build_ladder` used 0.1s; `design_ladder` now
rounds. (memory: `feedback_calibration_rank_granularity`.)

## Commit pointers (this session, on main)
`ed31ca0` endgame-skill lever · `dc4291f` qsearch slider · `c2d0087`
endgame-skill GUI · `7d8c03f` mate-guard variety fix · `build_ladder.py` /
`design_ladder.py` (ladder bootstrap) · avg-rank slider commit.
