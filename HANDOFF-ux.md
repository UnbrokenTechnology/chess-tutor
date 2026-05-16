# Handoff: chess-tutor-2 — UX / teaching layer

Forward-looking UX context. The product surface is teaching feedback, not the engine. See [`HANDOFF.md`](HANDOFF.md) for the index, [`CLAUDE.md`](CLAUDE.md) for the mission and ground rules, and [`HANDOFF-perf.md`](HANDOFF-perf.md) for engine perf state (read only if perf becomes relevant to a UX task).

## Current focus: teaching UI iteration

The engine is now performant enough for the planned mobile use case: at depth 12–14 the GUI feels real-time, retrospective is sub-300 ms on hard positions and ~100 ms on typical middlegames, full d=20 bench is 43 s with 8 threads (was an unfinishable multi-hour run a week ago). Further engine perf work has diminishing returns relative to the teaching-UX work that is now the bottleneck on the actual product.

See [`core/engine/src/analysis/mod.rs`](core/engine/src/analysis/mod.rs) `//!` for the design brief on the move-analysis pipeline (Phase 2 cheap-pass / surprise detection, Phase 4 signal-mask, Phase 5 tactic library) and the `narration` crate for the prose layer. Continued real-game playthrough is how the wording and thresholds get tuned — every retrospective narrator has unit tests for shape but the prose itself was picked a priori.

## Opponent profile / bot variability

Goal: ship bot-tuning toggles so games aren't deterministic from move 1, and so the student can practice against specific openings or weakened opponents. All four pillars — A (skeleton), B (opening book), C (eval signal mask), D (move noise + blunder) — have landed. Read the [`opponent.rs`](core/engine/src/opponent.rs) module doc for the strict invariant: **analytical paths (retrospective, hint, `analyze`) must never consult the profile** — they need to judge the user's move against true best play.

Phase D surface (delivered 2026-05-16):
- 6 [`NoiseProfile`](core/engine/src/opponent.rs) knobs, all-off by default. Three branches, evaluated in this order — **blunder → wild → softmax**. Blunder is the calibrated mistake signal (always picks a worse-than-best move when it fires), so it gets first crack; wild is chaotic and fills whatever budget remains.
  - **Blunder branch** (`blunder_chance: f32`, `blunder_severity_cp: i32` default 100): pick uniformly from engine-considered lines that trail #1 by at least the severity gap. When no line clears the gate (quiet positions where the top-6 are all near each other), fall back to `lines.last()` — the worst engine-considered move — rather than #1. This is deliberate: ensures a weakened bot's position gradually deteriorates over a quiet game instead of mysteriously snapping back to perfect play in tactically-tight stretches. Mate-guarded.
  - **Wild branch** (`wild_chance: f32`): per-move probability of picking uniformly from **all legal moves**, bypassing the engine ranking entirely. The only branch that can pick a move the search didn't surface — i.e. genuinely beginner-level mistakes like leaving a piece in a pawn's path. Mate-guarded.
  - **Softmax branch** (`candidate_pool: usize`, `temperature_cp: i32`): Boltzmann-weighted sampling over the top-K when both pool > 1 and temperature > 0.
  - Plus `guaranteed_mate_in: u32` (default 1) — suppresses blunder + wild branches when the bot sees a mate up to and including that depth, so mate-in-1 is never thrown away.
- [`noise::pick`](core/engine/src/noise.rs) — pure function `(profile, seed, ply, &lines, &legal_moves) -> NoisePick`. `NoisePick::Line(idx)` for normal/sampled/blunder picks; `NoisePick::Wild(mv)` when the wild branch fires. Deterministic given `(seed, ply)`; per-game seed is logged so a varied game can be replayed by passing `--seed N` back.
- CLI flags: `--noise-pool N`, `--noise-temp CP`, `--blunder-chance F`, `--blunder-severity CP`, `--wild-chance F`, `--guaranteed-mate-in N`. REPL `noise [show | pool N | temp CP | blunder F | severity CP | wild F | guarantee N | reset]`. Noise-driven engine moves are annotated `[noise: #K of N (-XX cp)]` (sampled) or `[noise: wild — engine preferred X (+Y)]` (wild) so the student knows the bot is off the best line.
- Desktop: reads `self.opponent.noise` when queuing the play search; sets `params.multi_pv = noise.effective_multi_pv()`. Worker computes legal moves, calls the picker, reports `NoisePickInfo::Sampled` or `NoisePickInfo::Wild` (logged to stderr for now; visible per-move tag in the move list is a follow-on). Wild moves get no `engine_info` badge in the move list (no search-line for that exact move).
- Desktop New Game dialog: full settings UI with sliders for the six noise knobs + collapsible eval-mask checkboxes + "Reset bot to defaults" button. The dialog auto-opens at first launch (no Cancel — only path forward is to commit a configuration) so the first thing the user does is pick difficulty. Subsequent New Game clicks pre-populate from the current game, so tweaking between games is incremental rather than from-scratch.
- Perf: off-profile is no-overhead — `is_off()` short-circuits and the engine keeps the single-PV fast path. When sampling is on, MultiPV costs roughly `K×` the single-PV time per move. Wild + softmax-only profiles keep the single-PV fast path because wild doesn't read the lines; only `blunder_chance > 0` widens MultiPV to at least [`BLUNDER_POOL_MIN`](core/engine/src/opponent.rs) (6).

Phase D follow-on, deferred:
- **Visible per-move noise tag in the move list.** Worker reports `NoisePickInfo` but it's only logged to stderr. The GUI move list should show a small badge ("noise: wild" or "blunder #6") on the corresponding move so the student can see at a glance which bot moves were deliberately weakened.
- **ELO presets**. `--bot-elo 1200` (CLI) and a "Preset" dropdown in the desktop dialog that fills in `(pool, temp, blunder, severity, guarantee, wild)` for you. Initial values were sketched in the design discussion: 100 ELO is wild-heavy, 1400 ELO is mostly softmax with a tiny blunder rate. Defer until the manual knobs feel clunky in real play (so we can tune the preset values from actual playthroughs).
- **Opponent-side retrospective**. Retrospective currently only fires on USER moves. A separate "the bot just played a deliberate mistake — can you find the punishment?" line when `noise_pick.is_some()` and `delta_from_top_cp <= -blunder_severity_cp` would be a powerful teaching surface, but requires resolving the analytical-paths invariant (the analytical search would need to know what the bot's noise profile is *for the user's retrospective only*, not for the bot's own decision).
- **More aggressive defaults**. Current defaults are all-off; once we have ELO presets and they're tuned from playthrough, the desktop dialog could default to a middle preset (~800 ELO) so a fresh install gives a more human-feeling opponent out of the box.
- **Seed surface in the GUI.** Desktop logs the seed to stderr but doesn't show it in the UI; players who want to replay a varied game can't easily copy the seed back. Add a status line under the move list or in the New Game dialog with the active seed + a way to paste one in to replay.

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
