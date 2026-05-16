# Handoff: chess-tutor-2 — UX / teaching layer

Forward-looking UX context. The product surface is teaching feedback, not the engine. See [`HANDOFF.md`](HANDOFF.md) for the index, [`CLAUDE.md`](CLAUDE.md) for the mission and ground rules, and [`HANDOFF-perf.md`](HANDOFF-perf.md) for engine perf state (read only if perf becomes relevant to a UX task).

## Current focus: teaching UI iteration

The engine is now performant enough for the planned mobile use case: at depth 12–14 the GUI feels real-time, retrospective is sub-300 ms on hard positions and ~100 ms on typical middlegames, full d=20 bench is 43 s with 8 threads (was an unfinishable multi-hour run a week ago). Further engine perf work has diminishing returns relative to the teaching-UX work that is now the bottleneck on the actual product.

See [`core/engine/src/analysis/mod.rs`](core/engine/src/analysis/mod.rs) `//!` for the design brief on the move-analysis pipeline (Phase 2 cheap-pass / surprise detection, Phase 4 signal-mask, Phase 5 tactic library) and the `narration` crate for the prose layer. Continued real-game playthrough is how the wording and thresholds get tuned — every retrospective narrator has unit tests for shape but the prose itself was picked a priori.

## Opponent profile / bot variability (in flight)

Goal: ship bot-tuning toggles so games aren't deterministic from move 1, and so the student can practice against specific openings or weakened opponents. Phases A (skeleton), B (opening book), and C (eval signal mask) landed. Read the [`opponent.rs`](core/engine/src/opponent.rs) module doc for the strict invariant: **analytical paths (retrospective, hint, `analyze`) must never consult the profile** — they need to judge the user's move against true best play.

Remaining pillar:

- **Phase D — move noise / blunder.** Bot occasionally plays a not-quite-best move (variety between equally-good replies, plus an opt-in "exploitable blunder" knob so the student gets practice spotting and punishing mistakes).

### Phase D design sketch (read before coding)

**Two related knobs in [`NoiseProfile`](core/engine/src/opponent.rs)** (currently empty stub):

- `candidate_pool: usize` (default `1` = no noise) — search width to consider when sampling.
- `temperature_cp: i32` (default `0`) — softmax temperature in centipawns over the score gap from #1. Low = peakier ("usually #1"); high = flatter ("often picks #2-#K when scores are close").
- `blunder_chance: f32` (default `0.0`, range `0.0–1.0`) — probability per move of skipping the natural top-K and picking a deliberately worse move.
- `blunder_severity_cp: i32` (default `100`) — how much worse a blunder must be vs. #1 to count.

(Exact field names / ranges open; pick during implementation. The four knobs above are the *concept* shape — the user's stated needs were "occasional good-but-not-best move" and "occasional blunder to exploit.")

**Mechanism — sampling layer sits in the play loop, not the engine:**

1. Play search runs with `SearchParams::multi_pv = profile.noise.candidate_pool` (today both CLI play and desktop hard-code `1`).
2. Engine returns ranked `SearchLine`s as today.
3. New helper (e.g. `noise.rs` mirroring `book.rs`): `NoiseProfile::pick(seed, &[SearchLine]) -> usize` returns the index into the result list. Softmax weighted by score deltas; blunder branch widens the candidate pool when it fires.
4. Play loop applies `lines[pick]` instead of `lines[0]`.

**Strict invariant (same as Phases B / C):** analytical paths must not sample. Retrospective / hint / `analyze` still ask for `multi_pv = 3` for their own ranking purposes, but the user's move is still judged against `lines[0]` (true best). Search params on analytical paths build `noise: ...` as today.

**Determinism:** derive a per-move seed from `profile.seed` + `move_number`. Same starting seed + same human moves = same bot moves and same noise picks. Per the existing `feedback_determinism` rule.

**Integration points (mirror Phase C):**

- `core/engine/src/opponent.rs`: extend `NoiseProfile` fields, add `Default` impl. Constructors `new_random` / `with_seed` populate sensible defaults (likely `pool=1`, `temperature=0`, `blunder=0.0` — all-off until user opts in).
- New `core/engine/src/noise.rs` (or fold into `opponent.rs` if small): the `pick` helper + small RNG. No engine plumbing — sampling happens after search returns.
- CLI: `--noise-pool N`, `--noise-temp CP`, `--blunder-chance F` startup flags + REPL `noise [show | pool N | temp CP | blunder F | reset]` command. Updates `cfg.opponent.noise` in-place like `eval-mask` does.
- CLI [`play_engine_turn`](core/cli/src/play.rs) and desktop [`maybe_queue_engine_search`](desktop/src/main.rs): set `params.multi_pv = profile.noise.candidate_pool.max(1)`; after the search, call the noise picker to choose which line index to play. The book branch in both files still short-circuits — sampling only applies when the engine actually searches.

**Perf note:** `multi_pv > 1` costs roughly `K×` the single-PV time (each slot is a separate IDS pass). Acceptable — the user opted into weakened play. No need to gate on `noise.candidate_pool > 1` to skip MultiPV when off, since `multi_pv.max(1) == 1` already takes the fast path.

**Open design questions to flag before/during implementation:**

- Is "blunder" a separate knob or just very-high temperature? Argument for separate: temperature samples from an existing top-K cluster (which is usually close in score), while blunders pick moves intentionally outside the cluster.
- ELO presets later? `--bot-elo 1200` could pick `(pool, temp, blunder)` for you. Defer unless the manual knobs feel clunky in practice.
- Does the retrospective ever narrate bot blunders? Today retrospective only fires on USER moves. A separate "opponent commentary" line ("the bot just played a mistake — can you find the punishment?") would be a nice teaching surface but is out of scope for Phase D.

Phase C surface (delivered):
- 8 toggleable [`EvalCategory`](core/engine/src/opponent.rs) values: `pawn-structure`, `pieces`, `mobility`, `king-safety`, `threats`, `passed-pawns`, `space`, `initiative`. Material and imbalance are deliberately not exposed (disabling them produces gibberish play, not a teaching scenario).
- CLI: `--disable-eval CATEGORY[,CATEGORY...]` startup flag + REPL `eval-mask list / disable CAT / enable CAT / reset` (toggles take effect on the next engine move).
- Desktop: reads `self.opponent.eval_mask` when queuing the play search; no UI for editing yet — wants a settings panel mirroring the CLI surface.
- Perf: TT=16 1T d=13 bench identical to pre-Phase-C (8 per-category branches fold under branch prediction on the empty-mask hot path).

Opening-book follow-on work, deferred:
- Grow the curated default from 8 entries; current list lives in [`CURATED`](core/engine/src/book.rs) (covered by the `every_curated_entry_resolves` regression test).
- Teaching-note overlay — separate `book_notes.toml` keyed by `(eco, name)` with short prose blurbs the GUI surfaces alongside the book line. Empty to start; populate the marquee openings first.
- Desktop UI for opening selection / status — today the only desktop surface is a stderr log on new-game. Wants at minimum: a "book: <opening>" line under the move list, plus a settings panel mirroring the CLI `openings` command.
- "New game in book" REPL command — CLI `openings allow/deny` only takes effect on the next game; a `new-game` REPL verb would re-pick a cursor in the current REPL session.
- Transposition-aware book matching — current cursor drops on any move-order divergence from the canonical line, even when the resulting position is the same. Low priority (curated lines are mostly canonical move orders).

Decisions locked in:
- Book entries are discrete TSV rows, not branches — "Caro-Kann Variation X" is its own opening, separate from "Caro-Kann".
- Curated default subset on by default once Phase B ships.
- Seed is random per game, logged in the play prompt; pass `--seed <n>` to replay.
- London System and other piece-placement-defined "systems" are out of scope for opponent profile; system detection is a separate quality issue against [`openings.rs`](core/engine/src/openings.rs).

## Teaching layer, deferred

See [`core/engine/src/analysis/mod.rs`](core/engine/src/analysis/mod.rs) `//!` for full spec on:
- **Phase 2 — cheap-pass + surprise detection** (depth-1 qsearch + SEE for every legal move).
- **Phase 4 — signal-mask** (zero each `EvalTrace` term in turn, re-rank, surface "you'd prefer M' if you undervalued X").
- **Phase 5 — tactic library** (general patterns: pin / fork / skewer / double attack / discovered attack / etc., parallel to `traps/`).

Additional:

- **Drill-down API for compound eval terms.** [`TermId`](core/engine/src/analysis/term_id.rs) collapses ~100+ raw SF11 signals into 47 chess-concept buckets. The narrator sometimes needs to explain *why* a compound term moved — e.g., "your KingDanger went up 80 cp because an enemy bishop now hits the long diagonal and your knight-defender just moved." Design sketch: opt-in `Option<&mut DetailedTrace>` analogous to today's `Some(&mut trace)` pattern, queried only by narrators explaining swings above some threshold (per-node cost paid only on rare detailed paths). First target: `KingDanger`'s 16-signal blend.
- **Rubinstein trap** — user wants to work out its invariants first. Belongs in the trap library ([`core/engine/src/traps/`](core/engine/src/traps/) — see that module's `//!` for the four-gate validator schema).

## UX / platform, deferred

- **Hint panel narration via narration crate refactor.** Hint panel currently shows `mv / score / PV`; richer narration should reuse the per-term narrators. Factor `narration::render_report`'s middle section into `render_per_term_narration(out, pre_move_pos, candidate, root_stm)`; expose `format_candidate_explanation(...)` without verdict / engine-preferred framing.
- **Real piece sprites** (cburnett, CC-BY-SA from Lichess). 12 SVGs, `include_bytes!`, drop-in for `piece_glyph` callers.
- **Promotion picker UI.** Currently auto-queens. Inline 4-piece overlay near the target square is standard.
- **Visual annotations on retrospective.** GUI eventually draws arrows / highlights tied to specific narrator clauses. Requires changing narration output from flat `String` to a list of clauses with optional annotation payloads (square sets, arrows, kind tag).
- **Bot strength / customization framework.** Long-term: configurable openings, blunder profile, tactical eyesight per bot.
- **FFI crate (`core/ffi/`).** First concrete step toward Apple/Android. Outstanding decisions: UniFFI vs. raw C ABI, in-process vs. out-of-process, how to expose `MoveAnalysis` across the boundary.

## Live-play tuning

Every retrospective narrator has unit tests for shape, but the wording and thresholds were picked *a priori*. Continued real-game playthrough is how they get tuned. CLI `play` and the desktop GUI retrospective panel are both wired for this. When playing, the most useful failure-mode categories to report:

- **Engine *said* X but narration didn't surface it** → narrator-prose tuning.
- **Narration surfaced X but you can't tell *why* X moved** → drill-down API gap (compound terms).
- **You made move M, engine preferred M', but you don't understand the *category* of mistake** → Phase 4 signal-mask gap.
- **Hint panel told you nothing useful** → hint panel narration refactor.
- **Wording felt off / patronising / vague** → cheapest fix; just tune the strings.
