# Roadmap

**Temporary planning doc.** Four sequential big-rock workflows that need to land in this order. Each one substantially churns the code the next one would touch, so doing them out of order multiplies the work. Delete this file once workflow 4 is complete.

## Status at a glance

| Workflow | State |
|---|---|
| **W1 — SF11 parity audit** | ✅ **Complete.** Done-criteria met (d=14 1.48× SF, d=20 2.04× SF). Full log in [`parity-audit-log.md`](parity-audit-log.md). |
| **W2 — Non-functional refactor** | 🔶 **IN PROGRESS** — 8 commits landed (Tier A + view/learning_mode/side_panel/play/types/traps/tt). ~6 files + 2 checkpoint files remain. Full state in [`w2-refactor-log.md`](w2-refactor-log.md). |
| **W3 — Tactic library port** | ⬜ Not started. |
| **W4 — Broader lichess audit** | ⬜ Not started. |

## Origin — why this roadmap exists

The actual product is the **teaching UX**, not the engine. This roadmap is a detour we took *out of* that UX work, for a concrete reason:

While building the teaching layer we hit a wall — the coach could only reason from the **current-position positional eval**. It had no way to walk the principal variation, so it could never say "you missed a tactic here" or "you had a forced mate." Teaching off static eval alone is the myopia the [prior attempt](CLAUDE.md) died of.

The fix was to walk the PV. Rather than reinvent tactic detection, we looked for prior art and found lichess already does exactly this ([`reference/lichess-puzzler/`](reference/lichess-puzzler/), `tagger/cook.py`). The plan became "port the lichess tactic tagger so the coach can name what happened in the line." But scoping that port surfaced two blockers: our engine code was (a) a tangle of oversized files and (b) **not functionally equivalent to SF11** — so any PV the tactic detectors read came from a search exploring far too many nodes, and we couldn't tell a detector bug from a search bug.

Hence the detour, in dependency order: **parity (W1) → clean the code (W2) → port lichess (W3 tactics, W4 broader audit) → resume the teaching UX.** The in-progress teaching-UX code is **parked** until W4 completes — see [`HANDOFF-ux.md`](HANDOFF-ux.md) for where to pick it back up.

## Why this order

- **Refactor before parity audit** = we refactor code that's about to change again, plus we may split a file in a way that obscures a divergence we'd have spotted in a side-by-side review.
- **Tactic port before parity audit** = we build new teaching features on top of an engine with known correctness/perf bugs. Tactic detectors read PVs from a search that's exploring 10x too many nodes; any "weird" PV could be a bug in the detector or a bug in the search, and we can't tell.
- **Tactic port before refactor** = we add ~600 LOC of new code to files that are about to be split anyway. Wasted work resolving merge conflicts.
- **Lichess feature audit before tactic port** is *not* necessary — we already know we want the tactic library, and starting it doesn't lock anything else out. But auditing for additional features only makes sense once the tactic port has surfaced the lichess codebase's structure to us in detail.

So: parity → refactor → tactic port → broader lichess audit.

---

## Workflow 1: Stockfish 11 parity audit

> ✅ **COMPLETE (2026-05-26).** Full file-by-file walk done across all 9 SF11 file-groups; results in [`parity-audit-log.md`](parity-audit-log.md). Headline outcomes:
> - The **"~10×" symptom below was stale** — the audit's first finding was that the real gap was 2.0–3.3×, not 10×.
> - **Two genuine correctness bugs** found and fixed: E1 (king-ring double-pawn removal used the wrong color) and P1 (en-passant square set on every double push → TT/repetition key divergence from SF).
> - The SF11 pruning stack then landed as **balanced bundles** (never piece-by-piece): faithful quiet-LMR, capture-LMR, S4 refined-eval pruning gates, the NMP family, and the B1+B3 structural-NPS rework (`Position::legal` + prune-before-`do_move` + cached check info).
> - **Done-criteria met:** d=14 (TT=16,1T) **1.48× SF** (was 2.03×); d=20 (TT=128,1T) **2.04× SF** (was 3.3×). The residual is now a diffuse **NPS** gap (~0.65× SF) living in movegen/eval/TT-entry-size, not pruning — out of scope for this workflow.
>
> What remains is W2.

### Symptom

Our engine's NPS is in the right neighborhood of SF11 (~2.4 M/s single-thread vs. SF11's ~2.7 M/s on the same hardware — close), but **we explore roughly 10x as many nodes** to reach the same depth. NPS × node-count gives the time delta, and the time delta is mostly nodes.

A 10x node-count gap means we're searching positions SF11 prunes. The candidates:
- **Move ordering imperfect**: good moves explored late means we miss cutoffs that SF11 takes. Killers, history, counter-moves, capture ordering, hash-move handling all matter here.
- **Pruning conditions slightly off**: cutoff thresholds, depth conditions, or stm-relative-sign mistakes. Each off-by-one re-explores a subtree.
- **Reductions too conservative**: LMR amount too small or applied to too few moves.
- **Extensions too aggressive**: more depth = exponentially more nodes. Check / passed-pawn / singular-reply extensions all stack.
- **TT replacement / probing imperfect**: more re-searches of positions we already know about.

### Known divergences already noted in memory (signal we're not unique to current investigation)

These are entries from `~/.claude/projects/.../memory/` that flag specific places we've already caught a divergence from SF11 — every one of these was discovered by side-by-side comparison, which is exactly the methodology this workflow scales up:

- `feedback_lmr_base_divergence.md` — LMR was 1–3 plies more aggressive than SF11; fixed 2026-05-14.
- `project_qsearch_depth_landed.md` — qsearch was ignoring depth entirely; FEN 19 d=20 went 391M → 7.8M (50x) when fixed.
- `project_aspiration_depth_reduce_landed.md` — aspiration fail-high reduction structure ported (but SF11's delta tuning didn't, deliberately).
- `project_fen26_check_extension_investigation.md` — universal moveCountPruning collapsed cold d13 484M → 226k (2,140x).
- `feedback_passed_pawn_ext_chain.md` — passed-pawn extensions feed outlier deep tails; SF11's fix is lmrDepth pruning.
- `feedback_lever2_regressed.md` / `feedback_singular_extensions_third_attempt.md` — attempts to add SF features bundled together regressed catastrophically.

The pattern across all of these: we either skipped an SF11 feature, simplified one, or implemented one whose effect interacts with another we hadn't implemented yet. **The audit needs to systematically find the rest.**

### Approach

File-by-file diff. For each major SF11 source file, open it alongside the corresponding Rust file, walk the functions in order, log every divergence. A divergence isn't necessarily a bug (we've deliberately deferred some SF features) but every one needs an explicit "deferred because X" or "fix" decision.

Priority order (rough — search.cpp first because it's where node-count loss lives):

| SF11 source | Rust counterpart | Why |
|---|---|---|
| `search.cpp` | `core/engine/src/search.rs` | Owns the 10x node gap. Walk every pruning condition, extension, reduction. |
| `movepick.cpp` | `core/engine/src/movepick.rs` | Move ordering directly drives cutoff effectiveness. |
| `evaluate.cpp` | `core/engine/src/eval/*.rs` | Eval cost matters per node; also wrong eval misleads search. |
| `position.cpp` + `position.h` | `core/engine/src/position/*.rs` + `core/engine/src/types.rs` | StateInfo / make_move / undo_move correctness. |
| `movegen.cpp` | `core/engine/src/movegen.rs` | Move generation edge cases. |
| `tt.cpp` + `tt.h` | `core/engine/src/tt.rs` | Probe and store paths; replacement policy. |
| `bitboard.cpp` + `bitboard.h` | `core/engine/src/bitboard.rs` + `core/engine/src/magics.rs` + `core/engine/src/attacks.rs` | Likely already close; quick pass for completeness. |
| `pawns.cpp` + `material.cpp` | `core/engine/src/pawns.rs` + `core/engine/src/eval/...` | Hashed eval — easy to get cache-key wrong. |
| `psqt.cpp` | `core/engine/src/psqt.rs` | Table-driven; mostly numerical correctness. |

For each file:
1. Open SF11 source and Rust counterpart side-by-side.
2. Walk function-by-function, noting any logical divergence in a per-workflow log (a new section in this doc, or a `parity-audit-log.md`).
3. For each divergence, classify: **fix immediately** (clear bug), **defer with reason** (e.g., we chose to defer SF feature X because Y), or **investigate** (might be either, needs bench guard).
4. Land fixes one at a time with the SF11-mirrored 45-position bench ([`core/cli/src/bench_fens.rs`](core/cli/src/bench_fens.rs), invoked via `./target/release/chess-tutor bench <tt_mb> <threads> <depth>`) before/after so we don't regress while fixing.

The bench is **the** reference instrument for this workflow. SF11's own `benchmark.cpp` `Defaults` array minus the one Chess960 entry — our 45 positions match SF11's 45 standard positions bit-for-bit. That gives apples-to-apples comparison: run `stockfish bench 16 1 <depth>` on the SF11 binary built from `reference/Stockfish-sf_11/src/` and compare aggregate node counts directly to our run at the same `<tt>`/`<threads>`/`<depth>`.

### Done criteria

- Node count within ~2x of SF11 on the 45-position bench at d=20 (TT=128 MB, threads=8) and at d=14 (TT=16 MB, threads=1).
- Every "deferred" divergence has an explicit rationale documented.
- No regressions on the existing 888-test suite.

### Notes / open questions

- Build SF11 locally: `make -C reference/Stockfish-sf_11/src/ build` (or `profile-build` for proper opt). That gives `stockfish.exe` to compare bench output against.
- The `feedback_bench_annotation.md` memory note flags that TT size and depth must annotate every node count we quote. Audit log should preserve that.
- Per `feedback_pruning_bundles.md`, every fix must be A/B'd individually against the bench before bundling — not "land 5 fixes and run bench once."

---

## Workflow 2: Non-functional refactor

> 🔶 **IN PROGRESS (2026-05-26).** Live state, recipe, conventions, gotchas,
> the done/remaining worklist, and per-file seam plans are in
> **[`w2-refactor-log.md`](w2-refactor-log.md)** — read it before resuming. In
> short: Tier A (test extraction) + 7 structural splits done across 8 commits;
> engine bench held node-neutral (d=14 = 9,739,495). Remaining: threats_outcome,
> eval(+pieces), movepick, retrospective_view, pawns (keep-whole), main.rs;
> then **checkpoint with the user before session.rs and search.rs**.

### Goal

Get every `.rs` file under 500 LOC without changing runtime logic or measurable performance. Big files mean Claude reads ~10kLOC of unrelated code on every session that touches them, and the human reader (you) has the same problem with worse working memory.

### Current state (largest files, LOC — refreshed 2026-05-26 post-W1/post-merge)

```
3539 core/engine/src/search.rs              W1 churned this (now larger — the parity bundles landed here)
2797 core/ui/src/retrospective_view.rs      ★ obvious split candidate (parked teaching-UX work added ~1.2k LOC here)
2107 core/ui/src/session.rs                 ★ god-object — view-building / worker dispatch / state mutation
1718 core/engine/src/movepick.rs            W1 churned this
1260 core/engine/src/types.rs               Mostly enums + impls; clean splits along piece / move / value boundaries
1180 core/cli/src/play.rs                   REPL command handlers — natural split per-verb
1059 core/engine/src/eval/mod.rs            Already a module dir; can promote sub-functions to siblings
1025 core/engine/src/traps/mod.rs           Same as eval/mod.rs
1012 core/engine/src/pawns.rs               Single eval term; harder to split usefully
 976 core/engine/src/analysis/threats_outcome.rs
 967 core/engine/src/noise.rs
 855 core/engine/src/analysis/move_assessment.rs   parked teaching-UX module (W3-adjacent)
 832 core/cli/src/main.rs
 811 core/engine/src/position/make_move.rs
 799 core/ui/src/view.rs                    Just struct definitions; split by surface
 741 desktop/src/draw/side_panel.rs
 ...
```

~20 files over 500 LOC. The three biggest (`search.rs`, `retrospective_view.rs`, `session.rs`) are the load-bearing ones. Note: `search.rs` and `movepick.rs` were churned by W1 (parity) as predicted; `retrospective_view.rs` / `session.rs` / `move_assessment.rs` carry the parked teaching-UX work merged from `main` — refactoring them touches paused-but-not-abandoned code, so split along seams that survive when that work resumes.

### Approach

Per file, identify natural seams:
- **By responsibility** — `session.rs` mixes session state, view building, worker dispatch, event handling. Each could be its own file.
- **By feature** — `retrospective_view.rs` builds one card per signal; each `build_*_item` function (already exists) is a natural per-file split.
- **By type cluster** — `types.rs` has Square, File, Rank, Color, PieceType, Move, Score, Value. Could become `types/square.rs`, `types/move.rs`, etc.

Refactoring patterns we have available in Rust:
- **Module split** — the cheapest seam; move related functions to a submodule, re-export from `mod.rs`.
- **Enum dispatch over `if/else` waterfalls** — `match` on an enum is what you'd reach for Strategy pattern for in Java/C#. Mostly already idiomatic here.
- **Context objects** — replace long arg lists with a struct passed by `&mut`. We use this already (`Evaluator`, `SearchContext`, etc.).
- **Trait objects** — only when actual polymorphism warrants the indirection cost; not free in Rust.

### Test extraction (decided)

Existing `#[cfg(test)] mod tests { ... }` blocks come out of the source files and land in sibling `<name>_tests.rs` files (declared from the parent module as `#[cfg(test)] mod tests;` — Rust finds the file by name). This preserves private-symbol access (the test module still sees `super::*` for items not `pub`) without inflating the source file's LOC.

Pattern:
```
core/engine/src/analysis/move_assessment.rs        (source)
core/engine/src/analysis/move_assessment_tests.rs  (tests, NEW)
core/engine/src/analysis/mod.rs                    (declares both: `mod move_assessment; #[cfg(test)] mod move_assessment_tests;` — or use a `#[path]` attribute from `move_assessment.rs` itself)
```

For files that are *already* being split into a directory module (e.g., `search.rs` → `search/` containing `mod.rs` plus per-heuristic files), tests go into `search/tests.rs` and are declared `#[cfg(test)] mod tests;` from `mod.rs`. Same access guarantees.

Crate-root `tests/` (cargo integration tests, public API only) is allowed for genuinely-public-API tests but is *not* the default — most of our tests reach into private surfaces, and moving them to `tests/` would force `pub`-pollution.

The CLAUDE.md "Separation of concerns" bullet that previously mandated inline `#[cfg(test)] mod tests` blocks is updated as part of this workflow.

### Constraints

- **No logic changes.** A non-functional refactor is exactly that. If a refactor reveals a bug, that's a separate change with its own test/bench.
- **No performance regression.** Run release bench (the 45-position SF11-mirrored one) before/after; node count + nps both must match.
- **No API breakage outside the file.** Crate-public symbols stay public at the same paths (re-exports as needed).
- **One file per commit.** Smallest reviewable unit — both for the human and for `git bisect`-ing a regression.

### Done criteria

- Every `.rs` *source* file ≤ 500 LOC. (Exception: data tables like `psqt.rs` may exceed this purely from numeric content; flag and discuss case-by-case.)
- Tests live in sibling files, not inline. Test file size is unconstrained — they're loaded only by `cargo test`, not by every Claude session reading the source.
- Bench numbers unchanged.
- Test count unchanged.

### Notes / open questions

- Some refactors might be net-negative for readability — e.g., splitting `pawns.rs` into a directory because it's 1012 lines might fragment what's actually one cohesive evaluation term. **Decide per-file whether splitting helps or hurts.**

---

## Workflow 3: Tactic library port (lichess-puzzler)

### Scope

The tactic library — already scoped in detail in [`HANDOFF-ux.md`](HANDOFF-ux.md) "Tactic library design brief" section. Ships in four waves:

1. **Ship 1** (~600 LOC): Fork + Hanging-piece capture + Removing-the-defender. Retrospective surface only. Direct hand-transliterations from `reference/lichess-puzzler/tagger/cook.py`.
2. **Ship 2**: Pin, Skewer, Discovered attack, Back-rank mate, Trapped piece.
3. **Ship 3**: Coaching panel (Cβ) — surface tactic *names* pre-move ("There's a fork available.") without naming the location.
4. **Ship 4**: Second wave of patterns — overloading, deflection, in-between, decoy, interference, mate patterns. Plus sacrifice classification.

Long-term goal: parity with lichess's full 30-tag taxonomy. Why: validated against millions of puzzles, strongest open-source benchmark, defensible default for "this is how chess teaching tools name tactics."

### Reference

- [`reference/lichess-puzzler/`](reference/lichess-puzzler/) — cloned (AGPL-3.0, never shipped). `tagger/cook.py` is the load-bearing source.
- Licensing posture in [`CLAUDE.md`](CLAUDE.md) "Secondary reference" section.
- Memory: `project_tactic_library_reference.md`.

### Done criteria

- The 8-pattern minimal taxonomy from the design brief is implemented and surfaced.
- Coaching panel offers named-pattern hints (Cβ).
- No misfires in the standard misfire-vectors documented in the design brief (chess.com failure modes).

---

## Workflow 4: Broader lichess feature audit

### Scope

By the time workflow 3 lands we'll have read most of `reference/lichess-puzzler/` and a fair chunk of related lichess code. Workflow 4 is the systematic pass: **what other pieces of lichess infrastructure are worth porting (or borrowing as design references)** for our teaching product?

Candidates worth at least a look:

- **Puzzle *generator*** (`reference/lichess-puzzler/generator/`) — detects "a tactic exists here" by engine-eval-swing. Mostly already covered by our existing `MoveVerdict ∈ {Inaccuracy, Mistake, Blunder}` plus PV-walk, but worth confirming we don't miss a heuristic.
- **Puzzle *validator*** (`reference/lichess-puzzler/validator/`) — sanity checks on candidate puzzles (uniqueness of solution, opponent's reply forced, etc.). Same patterns could validate our own retrospective annotations.
- **Other lichess repos**:
  - [`lichess-org/lila`](https://github.com/lichess-org/lila) (the main server) — Scala. Probably nothing directly portable but the analysis-board UX is the gold standard reference.
  - [`lichess-org/scalachess`](https://github.com/lichess-org/scalachess) — Scala chess logic. Likely overlaps Stockfish.
  - [`lichess-org/lila-tablebase`](https://github.com/lichess-org/lila-tablebase) — endgame tablebase server. Out of scope (we don't ship tablebases).
- **Their analysis-board explanation surface** — lichess's "tactical motif" annotations on past games. Look at the UX, see what they surface in what priority.
- **Opening explorer** — they have a database of master/online games keyed by position. Probably out of scope unless we want a "common openings" trainer.

### Approach

A research pass, not an implementation pass. Output is a follow-on workflow plan (or a "nothing else worth porting" verdict).

1. Walk the lichess organisation's repo list.
2. For each repo, read the README and skim the architecture.
3. For each component, classify: **port** (idea is novel and useful), **reference** (worth reading but not porting), **skip** (out of scope).
4. Update HANDOFF-ux.md (or write a new sub-handoff) with the verdict.

### Done criteria

- A documented decision on every major lichess component: port / reference / skip.
- New workflow plans for any "port" items.

---

## When this doc is no longer needed

Delete it once workflow 4 is complete. Long-term context for future sessions lives in HANDOFF.md / HANDOFF-ux.md / HANDOFF-perf.md.
