# chess-tutor-2

A fresh start. The prior attempt lives at `~/Repos/work/chess-tutor/` and is abandoned for reasons documented below — read that section before repeating any of its mistakes.

> **Session start:** read [`HANDOFF.md`](HANDOFF.md) before doing anything. It's the current-state snapshot: what modules are built, which is next, key design decisions, and gotchas. This file (CLAUDE.md) is evergreen guidance; HANDOFF.md is the moving target.

---

## Mission

Build a **chess tutor** that plays at roughly 2000 ELO and — critically — **surfaces its reasoning** move-by-move so a human student can learn *why* a move is good or bad, not just *that* it is.

The user is a ~1200 ELO player who:
- Does not hang pieces.
- Notices hanging enemy pieces and available tactics.
- Avoids walking into most enemy tactics.
- Loses to 1400+ bots with **zero blunders and zero mistakes**, because the position slowly deteriorates into a zugzwang-ish trap where the "best" move gives up the queen.

The gap between 1200 and 1600+ is **positional strategy**: holes, space, scope, pawn chains, outposts, weak squares, initiative, which of three non-losing moves actually builds toward a winning endgame vs. which walls you into zero-option territory. Commercial chess engines can't teach this because they are neural-net black boxes: they output `+0.4` with no explanation. Even classical engines that *have* a decomposable evaluation don't expose the decomposition to the user.

**This project exists to close that gap.** It is not "another chess engine." It is a teaching tool whose UI surface is the engine's internal reasoning.

## What makes this project different

The engine's public API must return, for every candidate move:
- The move itself.
- The final numerical score (like every engine does).
- **The full breakdown of signals that produced the score**, in human-readable form — material, mobility (per piece type), king safety, pawn structure, passed pawns, space, threats, initiative, per-piece positional terms, tactical motifs, etc.
- Which signals *changed* between "before this move" and "after this move," so the student sees the cause-and-effect.

If a feature of the engine can't be explained, it doesn't belong in the engine. Every weight, every term, every threshold must be traceable back to a named chess concept that a 1200 ELO player can read about.

## What we are NOT building

- A neural net. NNUE is banned. The whole point is that evaluations must be human-decomposable.
- A world-class engine. 2000 ELO is the target. Beating Stockfish 16 is not.
- An online service. The product runs **fully offline on-device**, no network, no account.
- A subscription product. See commercialization below.

## Technical approach: port Stockfish 11's classical evaluation to Rust

Stockfish 11 is the **last version before NNUE** (Stockfish 12 introduced the neural net). Its entire strength comes from the hand-crafted classical evaluation we want to teach from. The reference source is extracted at [`reference/Stockfish-sf_11/`](reference/Stockfish-sf_11/) (from the ZIP at the repo root, kept for reference only — not shipped).

### The explainability layer is nearly free

Stockfish's own `evaluate.cpp` already decomposes the score into named terms and exposes a `trace()` function that prints the per-term breakdown (see `Eval::trace` at evaluate.cpp:851 and the `Trace` namespace at the top of the file). The decomposition: `Material | Imbalance | Pawns | Knights | Bishops | Rooks | Queens | Mobility | King safety | Threats | Passed | Space | Initiative | Total`, each with middlegame/endgame values for both colors. Tracing is a compile-time template parameter (`Evaluation<TRACE>` vs. `Evaluation<NO_TRACE>`) so the un-traced evaluator pays zero cost.

**Our Rust port should mirror this exactly**: a generic const-bool (or cfg flag) that toggles signal capture, and the ability to return — alongside the usual `Value` — a structured `EvalTrace` the UI layers can render. Because the decomposition already exists in Stockfish, the teaching layer is a plumbing task, not a reimplementation task. This eases the "engine strength first" constraint: we build the trace struct from day one, but we don't have to design a separate "explainability system" on top.

### Board representation

**Roll our own**, mirroring Stockfish's layout: bitboards per piece type + per color, a `Position` type with `do_move` / `undo_move`, and a `StateInfo` that captures the information needed to undo (pawn key, material key, non-pawn material, castling rights, rule50, plies-from-null, ep square, captured piece, checkers, blockers, pinners, check squares). Not shakmaty.

Rationale:

- The whole port strategy is **verify eval-term-by-eval-term against the reference** for the same FEN. Any divergence in how the two libraries represent pins, en passant legality, check detection, or move generation edge cases becomes a debugging rabbit hole. Matching the reference's data layout 1:1 makes every discrepancy a bug in our port (investigable) rather than a semantic mismatch between two libraries (argue forever).
- Porting `evaluate.cpp` becomes near-mechanical when our `Position` has the same accessors Stockfish uses (`pieces<PAWN>()`, `attacks_from<KNIGHT>(sq)`, `psq_score()`, `non_pawn_material()`, etc. — renamed to Rust idiom, same semantics).
- Shakmaty is likely not what made the prior attempt slow. More probable culprits there: cloning positions instead of make/unmake, no transposition table, missing/mis-ordered pruning, recomputing eval from scratch per node instead of incrementally. We'd inherit none of those by using shakmaty — but we'd inherit a translation layer.
- Downside: we write more code. Mitigation: Stockfish's `bitboard.{h,cpp}`, `position.{h,cpp}`, and `movegen.cpp` are ~2500 lines total and are the most mechanical part of the port. Magic numbers are facts; attack tables are facts; the make/unmake structure is public knowledge.

Stockfish 11 is GPLv3. The user has consulted a lawyer and the operating assumption is:

> Copyright protects *expression*, not *ideas*. Algorithms, data structures, evaluation terms, weight tables, architectural patterns, and concepts are not copyrightable. A rewrite that carries over the ideas and authors every line of new code by hand — reading the reference, then typing the Rust ourselves — is its own copyrighted work and is **not a derivative work** that triggers GPL. Mechanical hand-transliteration (matching identifier names, mirroring statement order, adapting comments) is fine; what we cannot do is literally copy-paste source bytes, transpile the reference programmatically, link/wrap the reference binary, or distribute the reference source.

This means when porting:
- **DO** carry over: bitboard layout, magic-number move generation, alpha-beta w/ aspiration windows, null-move pruning, LMR, futility pruning, quiescence search, transposition table, late-move reduction formula, the evaluation term list, the weight tables (numerical weights are facts, not expression), the pawn hash, the material hash, the general module decomposition. **Hewing closely to the reference's structure, identifier names, and comments is fine and often preferable** — the goal is parity with what's known to work, not novelty. Default to "what does Stockfish do here" as the answer.
- **DO NOT**: copy-paste C++ source bytes into the Rust file. Run a transpiler over the reference. Link or wrap the Stockfish binary. Distribute the reference source (the repo is private; `reference/` is kept locally for development and never shipped).
- **Weight tables** (like `MobilityBonus[][32]`) — these are numerical facts. Carry over the numbers. Cite the source in a top-of-file comment: *"Evaluation weights derived from Stockfish 11 (GPLv3), which is a published reference implementation of classical chess evaluation. Weights are factual numerical data and are used under fair use / idea-expression dichotomy."*

### Secondary reference: lichess-puzzler (AGPL-3.0)

[`reference/lichess-puzzler/`](reference/lichess-puzzler/) is a clone of [github.com/ornicar/lichess-puzzler](https://github.com/ornicar/lichess-puzzler), the open-source pipeline lichess uses to generate and tag its puzzle database. **AGPL-3.0, never shipped, never modified** — same posture as the Stockfish reference.

It's the load-bearing reference for tactic naming. The taxonomy (fork / pin / skewer / discovered-attack / removing-the-defender / etc.) and the per-pattern predicates in [`reference/lichess-puzzler/tagger/cook.py`](reference/lichess-puzzler/tagger/cook.py) have been validated against millions of real puzzles — porting the *ideas* gives us parity with the strongest open-source benchmark on the planet without re-deriving the heuristics. Tagger architecture: confirm a tactic exists by engine-eval swing (separate pass), *then* walk the solution PV to assign pattern tags. We follow the same split — `MoveVerdict ∈ {Inaccuracy, Mistake, Blunder}` plus a winning `best` line is our "tactic exists" gate; per-pattern detectors then label the PV.

Same operating assumption as the Stockfish reference: hand-transliterate the predicates by reading `cook.py` and typing the Rust ourselves. Mirroring lichess's predicate shapes and naming is *encouraged* — those choices are validated against millions of puzzles, and we want parity with the strongest open-source benchmark, not novelty. What we cannot do is copy-paste the python, transpile it programmatically, or distribute the reference. Top-of-file citation comment goes on `tactic_outcome.rs` analogous to the Stockfish-derived weight files.

The Stockfish 11 evaluation decomposes into these top-level terms (from `evaluate.cpp`): **Material, Imbalance, Mobility, Threat, Passed, Space, Initiative**, plus per-piece positional terms (King, Knight, Bishop, Rook, Queen), plus Pawn structure, plus King Safety. These are exactly the concepts a teaching tool needs.

Files of interest in the reference (in rough priority order for porting):
- `types.h` — piece/square/color/score/value types and the `Score` (midgame,endgame) packing.
- `bitboard.{h,cpp}` — bitboard ops, magic-number sliders, PEXT alternative.
- `position.{h,cpp}` — position representation, make/unmake move, Zobrist.
- `movegen.cpp` — legal move generation by piece type.
- `psqt.cpp` — piece-square tables.
- `pawns.cpp` + `material.cpp` — hashed pawn/material evaluation.
- `evaluate.cpp` — the ~33K lines of classical eval. **This is the teaching core.** The `Trace` namespace at the top shows the decomposition we want to surface.
- `search.cpp` — alpha-beta + all pruning heuristics.
- `movepick.cpp` — move ordering, killers, history heuristic.
- `tt.{h,cpp}` — transposition table.
- `uci.cpp` — ignore. We're not implementing UCI (the CLI will be our own interface).

## Licensing and commercialization

- Repo is **private**. Source code will not be published. This is the main reason GPLv3's copyleft doesn't bind us: GPL triggers on *distribution*, and the binary we ship is an independent work under our own proprietary license (per the idea/expression reasoning above).
- Product: a **one-time-purchase app at ~$15** on each platform's store (App Store, Play Store, Microsoft Store). No subscription. No network. Comparable in positioning to buying Silman's *How to Reassess Your Chess* — but interactive, adaptive to what the student needs, and with a real playing opponent.
- Because the repo is private and the product is compiled, no source is distributed. This is a deliberate commercial decision, not an accident — do not push this repo to a public remote, and do not add code from any GPL project whose copyrighted *expression* we'd be shipping.

## Monorepo structure

Current layout. The Rust workspace lives at the repo root so cross-directory members (`desktop/` next to `core/`) work cleanly — cargo refuses workspace members above their declared root, so a `core/Cargo.toml` workspace can't include `../desktop/`.

```
chess-tutor-2/
├── CLAUDE.md                     # this file
├── Cargo.toml                    # Rust workspace root
├── reference/Stockfish-sf_11/    # reference source (GPLv3, NOT shipped, NOT modified)
├── core/
│   ├── engine/                   # chess-tutor-engine: the library (pure lib, no CLI deps)
│   ├── cli/                      # chess-tutor-cli: interactive testing CLI
│   └── ffi/                      # chess-tutor-ffi: C ABI for Swift/Kotlin bindings (TODO)
├── desktop/                      # chess-tutor-desktop: Rust egui app (Windows primary)
├── apple/                        # Swift/SwiftUI multi-platform app (TODO)
└── android/                      # Kotlin/Jetpack Compose app (TODO)
```

Confirmed decisions (2026-04-22):

- **Engine is a pure library; CLI is a separate crate.** The CLI is just "another UI" alongside Apple/Android/egui. Keeping the engine free of CLI dependencies (line editors, colored-output crates, arg parsers) keeps it small when linked into mobile apps via FFI.
- **The CLI keeps the prior repo's ANSI board renderer.** The old project at `~/Repos/work/chess-tutor/core/chess-tutor-cli/src/board.rs` renders a chess board in the terminal using 256-color ANSI backgrounds (chequered squares), Unicode chess glyphs with an `--ascii` fallback, amber highlight for last-move squares, board flip for Black's perspective, and a Windows-specific pawn workaround (Windows Terminal forces U+265F to an emoji, so both pawns use U+2659 with SGR foreground colors to distinguish sides). It looks really good and the user wants it preserved. Port that file over mostly verbatim — it takes a FEN string as input, so it's decoupled from engine types, and it's the user's own code (no license concerns).
- **egui for Windows desktop.** Chosen over .NET/MAUI/Tauri/etc. for two reasons: (1) Rust toolchain is already installed, no second development environment; (2) egui compiles to a single stand-alone binary — no runtime install burden on the user. macOS is covered by the Swift app. Linux is likely free as a side effect of egui being cross-platform; not a primary target.

## Agent-facing CLI commands

The `chess-tutor` CLI is **designed for agent consumption**. Before reasoning about a chess position by hand (who attacks what, what's pinned, what tactics exist), run the relevant subcommand — the engine already knows, and the answer is one command away. The case-study post-mortems in [`teaching-positions/`](teaching-positions/) show what happens when an agent skips this and reconstructs geometry by hand: it gets pins and discovered-attack alignments wrong, repeatedly.

All FEN-taking commands print a self-describing summary header (POV, score units, opening, legal-move count) and accept `--json` for machine-parseable output. Score POV defaults to white-POV (matches chess.com); pass `--stm` for engine-side-to-move. Score scale is pawns (chess.com-comparable) alongside engine-cp (PawnEG=213 scale).

The header also carries a **`danger:` block** when the side to move faces a *standing (latent) threat* — a discovered attack, pin, skewer, or loose defender the opponent can cash if you play a move that doesn't address it. This is the load-bearing line for the most common agent failure mode here: analysing what *you* can do while missing what the *opponent* has loaded against you. **A positive→negative eval swing means you *allowed* one of these** — so read `danger:` before trusting any "I have a good move" conclusion.

| Command | Use it when |
|---|---|
| `chess-tutor board [FEN]` | render a position as ANSI board |
| `chess-tutor moves [FEN]` | list legal moves, each annotated with `(check, captures Xy (N pts), promotion, en passant)` |
| `chess-tutor eval [FEN]` | per-term eval trace with directional gloss on every term; `--glossary` for the standalone term dictionary |
| `chess-tutor opening [FEN]` | identify the ECO opening |
| `chess-tutor square <SQ> [FEN]` | per-square dossier — who attacks/defends, pin status, discovered-attack vehicle status, SEE for cheapest capture |
| `chess-tutor threats [FEN]` | unified hanging / SEE-losing / pinned / overloaded / trapped, for both sides |
| `chess-tutor forcing [FEN]` | every check / capture / promotion for both sides (opponent's options via null-move) |
| `chess-tutor attacks [FEN]` | full (attacker, target) ledger, sorted by highest-value target first |
| `chess-tutor alignments [FEN]` | pure geometric ray scan — every discovered-attack / pin / skewer candidate on the board |
| `chess-tutor tactics [FEN]` | named-pattern detector for both sides + overloaded-defender scan. The side-to-move's best tactic is **escape-checked**: a real pin/fork with a forcing out shows an `escape:` line, so you won't trust a move that loses to a zwischenzug. `--latent` adds standing threats; `--check-followups` adds two-step forcing lines |
| `chess-tutor explain [FEN]` | one-shot aggregator: summary + threats + tactics (latent + check-followups) + a depth-N search. "Give me everything on this position" — the right first call when you're unsure which surface you need |
| `chess-tutor search [FEN]` | depth-N search with PV + score; `--annotate` adds a tactic-pattern summary line (plus an `escape:` line when the top-PV tactic has a forcing refutation) |

**Rule of thumb — use the tool, don't reason by hand**: if you find yourself thinking *"the rook on e1 sees through the bishop to attack the queen"*, stop and run `chess-tutor square e5 <FEN>` or `chess-tutor alignments <FEN>` first. Trust the tool over mental reconstruction.

**Rule of thumb — chess.com is accurate but opaque; we are transparent**: chess.com's eval bar uses NNUE Stockfish, which beats our classical SF11 port in head-to-head. When chess.com and our engine disagree on a position's evaluation, **default to "chess.com is right, why did we miss it"**, not "chess.com is wrong". The most common *apparent* disagreement is POV / scale confusion (chess.com is white-POV pawns; our raw engine `Value` is side-to-move engine-cp at PawnEG=213 — both surfaces are now labelled in the CLI, but earlier debugging sessions burned hours on this). Our engine isn't *more right* than chess.com — **it knows *why* it's right**, which is the whole product surface. The teaching layer's job is to extract chess-com-level conclusions *with explanations a 1200 player can learn from*; "more accurate than chess.com" is never the goal.

**Rule of thumb — a detected tactic with an escape is still real.** The `escape:` line on `tactics` / `search --annotate` names the opponent's *forcing out* (a check, an in-between capture, a both-defending move, a retreat, a counter-threat) — it does **not** void the pattern. "There's a pin, but they can break it with `…Bxh2+`" is the lesson, not "there's no pin." Likewise the `danger:` header names the opponent's standing resource against *you*. Read both before concluding a position is won or lost.

Full design + roadmap: [`PLAN-cli.md`](PLAN-cli.md) (the command surface) and [`PLAN-tactic-escape.md`](PLAN-tactic-escape.md) (the escape-detection model). The named-pattern detector (`tactics`), the aggregator (`explain`), the latent-threat scanner (`--latent` / `danger:`), and escape detection have all landed.

## What we learned from the prior attempt (chess-tutor/)

The prior repo tried two strategies, both failed:

1. **Bespoke rule layers.** Attackers/defenders, tactical motif detection, positional feature list. Produced nice per-move commentary but **lost** to 1400 bots because picking the move with the "most positional benefit" isn't the same as picking the move that searches well. Positional benefit without search is myopic.

2. **"Port Stockfish 11 but expose the 120 signals."** Several thousand lines of Rust written over ~24 hours. The result: **slow** (multiple seconds per move), **weak** (blundered mate-in-1, lost to 1400 bots). Likely causes: (a) some search pruning missing or miswired, (b) evaluation terms present but weights or non-linearities wrong, (c) move generation correctness bugs, (d) no performance work (no magic bitboards, no incremental update, no proper TT).

The lesson for this rewrite: **engine correctness and performance come first; teaching UX comes second.** The prior attempt tried to build bespoke explanation systems *instead of* a strong engine; the result was a pretty-but-weak tool that taught the student to play weakly.

Order of operations:

1. Correct, fast Rust engine that plays 1800+ ELO and beats 1400 bots reliably. The internal `EvalTrace` struct exists from day one (it's near-free and essential for debugging — QA strategy is "for this FEN, does our per-term output match Stockfish's?").
2. Teaching UX on top of the trace: how "this move changes King Safety from −24 to +8" is presented to the student, per-move before/after diffs, curriculum around the signals.

Do not invest in the step-2 UX until step 1 is solid. A weak engine with pretty explanations teaches the student to play weakly.

## Ground rules for working in this repo

### Before implementing anything substantial

Flag the approach with the user first. The prior attempt burned a day and a night on code that got thrown away. Short exchanges before writing thousands of lines of Rust — especially around search architecture, data representation choices (8x8 array vs. bitboards vs. hybrid), and the evaluation decomposition — are cheap insurance.

### When in doubt, read the reference

Every Stockfish 11 decision encodes decades of chess knowledge. If you're tempted to deviate from how Stockfish 11 does something, you're probably wrong. Read `reference/Stockfish-sf_11/src/<thing>.cpp` first, understand why it's done that way, then decide if the Rust idiom genuinely wants to express it differently. "I think I know a cleaner way" is almost always worse than "Stockfish does it this way."

### Performance matters

A teaching tool that takes 5 seconds per move is not usable. Budget: sub-second evaluation of the top candidate moves at the user's search depth, on a mobile device. This forces bitboards (magic or PEXT), incremental make/unmake, transposition table, and the full Stockfish pruning stack. Don't defer performance to "a later optimization pass" — bake it into the design from day one.

### Always run release / profiling builds for perf-sensitive work

`cargo run` produces the **debug** profile, which is 20–200× slower than release. A startup that takes 200 ms in release can take 4 s in debug; a per-move retrospective at 10 ms in release can take 2 s in debug. **Any "this is too slow" observation needs to be made against a release or `profiling` build, never a debug build.** The user spent a full session investigating phantom slowness before realizing they'd been running `cargo run`.

For day-to-day playtesting: `cargo build --release --bin chess-tutor`, run `target/release/chess-tutor.exe`. For perf investigation (VTune, Superluminal, WPA): `cargo build --profile profiling --bin chess-tutor` — same optimisations as release but with PDBs so symbolic profilers show Rust function names. The `profiling` profile is defined in the repo-root `Cargo.toml`.

### Determinism is a teaching requirement

The user wants "consistent advice in consistent positions." Two consequences:
- **Use depth-budget search, not time-budget.** Time-budget search would let CPU speed and background load leak into outputs (slower hardware = more time = different move). Even with a node cap as a safety net, the primary stop condition is "depth N completed."
- **Analytical commands must not mutate engine state.** REPL `search`, `analyze`, and the auto-retrospective all clone the play engine before searching. The play engine's TT only updates on actual game moves. This means repeated `search` / `analyze` calls in any order produce the same answer for the same position. Don't reintroduce shared mutable state without a similarly-clean isolation strategy.

### Deferred optimization: bake the magic attack tables into the binary

Current magic bitboards (`core/engine/src/magics.rs`) find magics at first use via a seeded xorshift search, then build the ~900 KB rook + bishop attack tables on the heap. This costs tens of ms of startup on a release build. Eventually we want to harvest the magic numbers from one local run, paste them in as `const` arrays, and either keep startup-time table building (~0 ms) or bake the whole 900 KB attack table as `static` in the binary. The user has approved the size trade-off — 900 KB is nothing on a modern mobile install, and startup latency matters more. Do this when we're integrating with the first platform app (or sooner if cold-launch time shows up as a pain point).

### Separation of concerns

Prefer many small focused files over grab-bag ones. A file that does one thing is easier to review, test, and reason about. Specifically:
- Each evaluation term in its own file (`eval/mobility.rs`, `eval/king_safety.rs`, `eval/passed_pawns.rs`, ...).
- Each search heuristic in its own file where it's self-contained.
- Tests live in sibling `<name>_tests.rs` files — declared *inside the source file* as `#[cfg(test)] #[path = "<name>_tests.rs"] mod tests;` (the `#[path]` is what points a flat sibling at the `tests` child module; without it `mod tests;` would look for a `<name>/tests.rs` subdirectory). Directory modules use `<name>/tests.rs` declared `#[cfg(test)] mod tests;` from `mod.rs`. Either way the test module stays a child of its source module, so it keeps `super::*` private-symbol access while the source file stays readable. The sibling file holds the module *body* (starts with `use super::*;`), not a wrapping `mod tests { }`. Cargo's crate-root `tests/` directory is allowed for tests that only need the public API, but is not the default — most of our tests reach into private surfaces.

### Don't add features the task doesn't require

See the top-level instructions. This applies especially during the engine port: do not invent abstractions Stockfish doesn't have, do not add configurability for things that have one right value, do not add "future-proofing." Port the minimum, get it working, move on.

### Never push this repo to a public remote

See commercialization. If the user ever asks to create a GitHub remote, confirm public vs. private before running the command, and default to private.

## Environment notes

- Windows 11, bash shell available (git-bash). Use Unix syntax — `/dev/null`, forward slashes, etc. — not PowerShell.
- The user's Rust is installed and usable from bash. The old repo used `rust-toolchain.toml` to pin — we'll do the same.
- Current date context: project started 2026-04-22. The prior repo's last commit is 2026-04-21.
