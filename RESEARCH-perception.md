# Research — move-perception / engine-humanization evidence base

> Extracted verbatim from the (now-retired) `PLAN-perception.md` design doc.
> The perception lever it informed has **shipped** (`core/engine/src/visibility.rs`;
> design rationale in that file's `//!`). This file is kept as the durable
> evidence base — raw material for a proper "how our weak bots play like
> humans" README / marketing write-up later. It is research, not a task list.
>
> Five parallel research agents (2026-06-06) gathered the evidence below;
> full agent reports are in the session transcript, key sources cited inline.

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
  weights change and the **full grid re-runs**.

---

## Sources (calibration-harness research)

- Maia: <https://github.com/CSSLab/maia-chess> · paper <https://www.cs.toronto.edu/~ashton/pubs/maia-kdd2020.pdf>
- Allie (human-aligned MCTS, ~49 Elo skill gap 1000–2600): <https://arxiv.org/pdf/2410.03893>
- Eval-randomization as a strength dial (Beal effect): <https://github.com/official-stockfish/Stockfish/issues/3635>
- Stockfish UCI_Elo calibration: <https://github.com/official-stockfish/Stockfish/pull/2225>
- talkchess engine-humanization threads: t=73603, t=55011
