# Handoff: chess-tutor-2 — UX / teaching layer

Forward-looking UX context. The product surface is teaching feedback, not the engine. See [`HANDOFF.md`](HANDOFF.md) for the index, [`CLAUDE.md`](CLAUDE.md) for the mission and ground rules, and [`HANDOFF-perf.md`](HANDOFF-perf.md) for engine perf state (read only if perf becomes relevant to a UX task).

## Current focus: teaching UI iteration

The engine is now performant enough for the planned mobile use case: at depth 12–14 the GUI feels real-time, retrospective is sub-300 ms on hard positions and ~100 ms on typical middlegames, full d=20 bench is 43 s with 8 threads (was an unfinishable multi-hour run a week ago). Further engine perf work has diminishing returns relative to the teaching-UX work that is now the bottleneck on the actual product.

See [`core/engine/src/analysis/mod.rs`](core/engine/src/analysis/mod.rs) `//!` for the design brief on the move-analysis pipeline (Phase 2 cheap-pass / surprise detection, Phase 4 signal-mask, Phase 5 tactic library) and the `narration` crate for the prose layer. Continued real-game playthrough is how the wording and thresholds get tuned — every retrospective narrator has unit tests for shape but the prose itself was picked a priori.

## Opponent profile / bot variability (in flight)

Goal: ship bot-tuning toggles so games aren't deterministic from move 1, and so the student can practice against specific openings or weakened opponents. Phase A landed an empty [`OpponentProfile`](core/engine/src/opponent.rs) skeleton; Phase B landed the opening-book pillar (curated default subset, CLI `openings` command, `--no-book` flag, book cursor in play loop + desktop). Read the [`opponent.rs`](core/engine/src/opponent.rs) and [`book.rs`](core/engine/src/book.rs) module docs for the strict invariant: **analytical paths (retrospective, hint, `analyze`) must never consult the profile** — they need to judge the user's move against true best play.

Remaining pillars, one PR each, A/B-tested against a real game between landings:

- **Phase C — eval signal mask.** Bitset over [`TermId`](core/engine/src/analysis/term_id.rs); each eval term gates its contribution. Needs perf check after — eval is hot-path. Lets the student spar against an opponent who "doesn't know about" king safety / pawn structure / etc.
- **Phase D — move noise.** Search with `multi_pv = K`, sample top results weighted by score gap; with `blunder_chance` extend the pool. Reuses existing MultiPV infrastructure.

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
