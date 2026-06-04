# PLAN — ELO-slider calibration harness

**Status:** BUILD STARTED 2026-06-04. The UCI shim (Component 1) is landed; external
tooling + orchestration are next. Earlier status (RESEARCHED + SKETCHED) preserved below.
**Date:** 2026-06-04. Source: deep-research run (workflow `w9xeyh5e2`), findings folded in below.

## Decisions locked (2026-06-04, with the user)

- **Experimental design = hybrid characterization.** Gather broad data *first* to learn each
  dial's effect and ceilings; the user then sets Elo-banded **bands** (e.g. blunder 10–20% at
  1200) and **forced binaries** (e.g. wild=0 above ~1000, king-safety-mask on/off) as solve-time
  constraints, and a solver fills the free dials. The regression is pure *measurement*; the
  human-realism policy is the user's editable constraint layer (cleaner than a pre-committed
  backbone — the data dictates the shape, the user dictates the policy, and both stay editable).
  Three measurable jobs: **(1) marginal curves** (sweep each dial alone), **(2) ceilings** (toggle
  each eval-mask category + combos at full strength), **(3) interactions** (big Latin-hypercube).
- **Pilot first.** Validate the harness end-to-end + eyeball PGNs for human-likeness + confirm a
  monotone curve before committing the multi-day runs.
- **Extremes (below 1100 / above 1900) via self-play connectivity + extrapolation.** No external
  sub-1000 anchor exists that's human-calibrated; the weak/strong configs tie to the Maia-anchored
  scale transitively through intermediate configs. Sub-1000 numbers are extrapolated, validated by
  hand. (Optional later: our own zero-noise engine as a >2000 ceiling reference.)

## Component status

- ✅ **Dials** — all exist in the engine (`opponent.rs` / `noise.rs`): depth, avg-move-rank,
  blunder chance/min/max, miss, wild, guaranteed-mate-in, eval-mask (8 categories), per-game seed.
- ✅ **Prerequisite** — allowed-openings selector landed (`6aef577`). (Note: the *measurement* uses
  fastchess's external balanced book fed to both engines, not our internal book — so neither side's
  book knowledge skews strength. The internal book is a product feature, orthogonal to measurement.)
- ✅ **Component 1 — UCI shim** — landed as the `chess-tutor uci` subcommand (NOT a separate crate;
  reuses every `play` dial flag + the `worker.rs` search→`noise::pick`→bestmove path). Always
  searches to `--depth` (ignores TC tokens → reproducible per-config strength). Per-game seed =
  `--seed` mixed with a `ucinewgame` counter → one base seed replays a whole run, games still vary.
  Threads repetition history so threefold is handled. `core/cli/src/uci_shim.rs` (+ sibling tests).
- ⬜ **Component 2 — external tooling** (downloads): fastchess, Ordo, lc0, 9 Maia nets, balanced book.
- ⬜ **Component 3 — orchestration + fit**: config generator → fastchess gauntlets → Ordo (`-A`
  anchored to measured Maia ratings) → monotone-aware model fit → constrained solver honoring the
  user's bands/binaries.

## Goal

A single **"opponent Elo" slider** in the product. The user drags it to a target human Elo; we
generate a bot configuration that *plays like* a human of roughly that strength. No memorising named
personalities, no tweaking ten dials per game.

To make the slider real we need a **model that maps our tuning dials → measured Elo**, fit from
offline experiments, and then **invert** it (solver: best-guess → forward-predict → perturb → hit
target). This doc is the plan for building the measurement harness and fitting that model.

## Current dials (inputs to the model)

From the existing opponent-profile work (`project_opponent_profile_plan`):

- **Search depth** — run a search at MultiPV 10 to get candidate moves.
- **Blunder chance** — if a material blunder is available, roll; on hit, filter candidates to only
  blundering moves.
- **Miss chance** — if material can be *won*, roll; on hit, filter to candidates that fail to win it.
- **Blunder min/max material** — separates "blunder a pawn" from "blunder a queen" (e.g. 50% on a
  pawn, never on a queen).
- **Wild move chance** — every move, roll; on hit, pick a uniformly random legal move.
- **Average move rank** — over the filtered MultiPV-10 list, how strongly we bias toward the #1 move
  vs. the #10 move.
- **Guaranteed mate-in** — if a mate-in-N exists, does the bot always see it, or does it still fall
  back to average-move-rank?
- **Eval mask** — when scoring a move, ignore certain positional signals.

## Load-bearing research finding (reshapes the design)

> **Depth-limiting does NOT make an engine play like a human — it just plays *weaker*.**

The Maia paper (KDD 2020) measured 15 depth-limited Stockfish versions: their move-match accuracy
*rises monotonically with the opponent's strength* (depth-15 matches 1900-players more than
1100-players). A throttled-depth engine plays "thin engine moves," not weak-human moves. This is
exactly the failure mode the product exists to avoid — a bot that plays perfectly then hangs its
queen teaches nothing, because real 1200 players don't do that.

**Corollary — the dials that reshape the *move distribution* are the real strength levers; depth is
the weak one.**

- **Eval-noise** is empirically a *smooth, continuous* strength dial — Stockfish's `RandomEvalPerturb`
  experiment swept one parameter across ~3,700 Elo on a single axis (measured on NNUE at 100k nodes,
  so our classical eval will have its *own* curve — must be re-measured, but the shape transfers).
- **Blunder-chance, miss-chance, average-move-rank** reshape the distribution the same way; these are
  the human-realistic levers.
- **Search depth should be a coarse *floor*, not the fine strength knob.**

## Design: 1-D backbone + banked snaps + small correction surface

This replaces the original "blind 8-D regression over thousands of configs" idea. The research says
*one* continuous dial carries most of the strength range, so:

1. **Primary continuous axis.** Pick ONE dial — almost certainly **eval-noise magnitude** or
   **average-move-rank** (both monotone, both reshape the distribution). Calibrate *that single dial →
   Elo* with a cheap **1-D sweep** (~10–15 points). This is the invertible backbone curve.
2. **ELO-banked snaps** for the discrete / personality dials:
   - wild-move chance → 0 above ~1000 Elo,
   - eval-mask layers peel off at rising Elo thresholds,
   - guaranteed-mate-in = f(Elo),
   - allowed openings → chosen randomly per game (see *prerequisite* below),
   - blunder min/max material → banded by Elo.
3. **Small response-surface correction** for the *interactions* that actually move Elo (primarily:
   how each eval-mask layer shifts the backbone). Fit this from a modest **Latin-hypercube** sample
   (~hundreds of configs), fitting only the *deviation* from the backbone — not the full 8-D surface.

Net effect: weeks of compute → an overnight backbone sweep + a few days of interaction probes.

## Anchor opponents: Maia, not Stockfish UCI_Elo

We need reference opponents whose Elo is on the **human** scale (the student is human; we approximate
chess.com/Lichess-style strength). Decision:

- **Anchor to Maia.** Nine GPL neural nets (ELO 1100–1900, 100-pt bands), each trained on ~12M
  Lichess games where both players were in that band. Download the `.pb.gz` weights, run under **lc0**
  with `go nodes 1` (pure policy → stays human-like). The only locally-runnable *human-calibrated*
  anchor.
- **Do NOT anchor to Stockfish `UCI_Elo`.** It's internally the Skill-Level move-degradation wrapper,
  calibrated to the **CCRL engine-vs-engine list** (an inflated, non-human scale), and **floors at
  1320** — it literally cannot represent our core 800–1200 students.

### Two caveats to design around

1. **Maia plays *above* its band label.** The label is a training target ("the average move of an
   X-rated player" is stronger than an X-rated player, who also plays below-average moves). So `1100`
   is not ground truth. **Fix:** the nets also run as public Lichess bot accounts (`maia1`/`maia5`/
   `maia9`) with *measured* Lichess ratings — use those measured numbers as the Ordo anchor values,
   not the labels. *(This lookup is an open item — see below.)*
2. **Below ~1100 there is no trustworthy human anchor.** Maia floors at 1100; UCI_Elo floors at
   engine-scale 1320; depth-limited SF isn't human. For the 600–1000 band (which we care about), we
   must **extrapolate the backbone downward and validate the feel by hand.** Know this before trusting
   any sub-1100 number the harness emits.

## The engine-Elo vs human-Elo scale gap

CCRL/CEGT engine rating lists are a separate, inflated pool vs human FIDE/Lichess/chess.com Elo
(Stockfish's own calibration notes two engines 200 Elo apart in play but 300 apart on CCRL). So a
"1320 UCI_Elo" is an engine-scale number, not a human 1320. Anchoring the rating pool to Maia (human
bands) is what makes our output numbers mean "plays like a ~X human."

## Tooling (all fully offline on Windows — confirmed)

| Layer | Tool | Notes |
|---|---|---|
| Match runner | **fastchess** | Replaced cutechess-cli as Stockfish's Fishtest runner (2024). UCI-only. ~250-thread concurrency. Built-in SPRT. Local binary, no network. |
| Rating calc | **Ordo** (primary) / BayesElo | Reads local PGN → ratings. Ordo `-A` anchors the pool to a fixed reference Elo (→ our Maia anchors). Ordo is what the SF team used to calibrate UCI_Elo. |
| Human anchor | **Maia** nets + **lc0** | 9 downloadable GPL `.pb.gz` files, run offline under lc0. |

The only network activity in the whole project is the one-time download of these binaries + weights.

## Statistics

- **SPRT** (built into fastchess) answers a *pairwise yes/no*: "is config A stronger than B?" with the
  fewest games. Use for **inner-loop dial-sensitivity checks** (does bumping blunder-chance actually
  drop Elo?). It does **not** give an absolute Elo.
- **Absolute per-config Elo** comes from running **Ordo/BayesElo on the full anchored PGN pool**.
- **Games per config:** sources gave no clean formula (flagged open). Rule of thumb: **~±50–60 Elo
  error at ~100 games, narrowing as 1/√N.** Backbone points want a few hundred games each; interaction
  probes can run fewer and lean on the fitted surface.

## UCI shim (test-only — never ships)

fastchess is UCI-only, so the library needs a thin **test binary** that speaks the minimal subset:

```
uci          → emit `id name`/`id author`, any `option` lines, then `uciok`
isready      → `readyok`
ucinewgame   → reset state
position [startpos | fen <FEN>] moves <m1> <m2> ...
go depth N   → search, then: `bestmove <move>`
```

It reads a dial-config (e.g. `--config dials.json`) and exposes that configured bot as a UCI engine.

**Gotchas:** must emit `uciok` and `readyok` or the harness hangs on handshake; fastchess applies a
wall-clock timeout even on `go depth N`, so each search must return promptly (fine — we're depth-budget
by design). Lives in a separate crate (e.g. `core/uci-shim` or under `core/ffi`); the product never
links it.

## Harness architecture

```
┌─ Anchor ladder (download once, run under lc0, offline) ─────────────┐
│  Maia 1100 / 1200 / … / 1900  (9 nets, go nodes 1 = pure policy)     │
│  + their MEASURED Lichess bot ratings → Ordo -A anchor values       │
└─────────────────────────────────────────────────────────────────────┘
                         ▲ plays gauntlet vs ▲
┌─ Our engine, wrapped ──┴───────────────────────────────────────────┐
│  uci-shim test binary  --config <dials.json>                        │
│  reads one dial-config, exposes it as a UCI engine                  │
└─────────────────────────────────────────────────────────────────────┘
                         ▲ driven by ▲
┌─ fastchess (local, concurrent, per-config PGN, fixed opening book) ─┐
│  config_i  vs  Maia ladder   → games_i.pgn (tagged with config id)  │
└─────────────────────────────────────────────────────────────────────┘
                         ▼ rated by ▼
┌─ Ordo  -A <maia anchors>  → elo_i per config ──────────────────────┐
└─────────────────────────────────────────────────────────────────────┘
                         ▼ fit ▼
┌─ Model: backbone Elo = f(primary dial) + Δ(banked settings) ───────┐
│  invert: target Elo → primary-dial value + banked snaps (solver)    │
└─────────────────────────────────────────────────────────────────────┘
```

## Experiment phases

- **Phase 1 — backbone (overnight):** 1-D sweep of the primary strength dial, ~10–15 points, ~200
  games each vs the Maia ladder → invertible Elo-vs-dial curve.
- **Phase 2 — interactions (days):** Latin-hypercube sample of a few hundred configs over primary-dial
  × the banked settings that plausibly shift Elo (eval-mask layers, mate-vision). Fit the response
  surface for the *deviation* from the backbone.
- **Phase 3 — validation:** hold out target Elos (e.g. 950, 1350, 1650); solver emits dials; play
  fresh gauntlets; confirm measured Elo lands within ~±50.

## Open items to resolve before a multi-week run

1. **Measured Maia ratings.** Look up each Maia net's real Lichess bot rating → true anchor values
   (don't trust the band labels).
2. **Primary-dial choice.** Pilot-sweep eval-noise vs average-move-rank; pick whichever gives the
   smoother, widest monotone curve.
3. **Sub-1100 gap.** Decide explicitly how the 600–1000 band is produced (extrapolate backbone) and
   how it's validated (by hand).
4. **fastchess ergonomics.** Re-verify current `master` flags (was v1.8.0-alpha) for per-config PGN
   tagging, deterministic opening book, and crash-resume on a Windows multi-day unattended run.
5. **Determinism interaction.** The product is depth-budget + deterministic for teaching; confirm the
   bot's randomness (blunder/miss/wild rolls) is seeded per-game so harness runs are reproducible.

## Prerequisite (built first, independent of all the above)

**Allowed-openings selector** over the existing book — tell the bot which openings it may play,
selected randomly per game. Eliminates one personality dial and is not invalidated by anything the
calibration discovers. Flag the approach (book-lookup shape, how the allowed set is expressed,
random-per-game selection) against `core/engine`'s current book code before implementing, per repo
ground rules.

## References

- Maia: https://github.com/CSSLab/maia-chess · paper https://www.cs.toronto.edu/~ashton/pubs/maia-kdd2020.pdf
- fastchess: https://github.com/Disservin/fastchess · https://official-stockfish.github.io/docs/fishtest-wiki/Running-Fastchess.html
- Ordo: https://github.com/michiguel/Ordo/wiki/Ordo · BayesElo: https://www.remi-coulom.fr/Bayesian-Elo/
- Stockfish UCI_Elo calibration: PR https://github.com/official-stockfish/Stockfish/pull/2225
- Eval-randomization as a strength dial: https://github.com/official-stockfish/Stockfish/issues/3635
- Allie (human-aligned MCTS, ~49 Elo skill gap 1000–2600): https://arxiv.org/pdf/2410.03893
