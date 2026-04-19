# Chess Tutor — Project Plan

An offline chess learning app that explains **why** moves are good or bad, not just which move an engine prefers. Target: bridge a ~1200 ELO player toward ~2000 through deterministic, explainable feedback on games the user actually plays.

## Guiding Principles

- **Explanations first, evaluation second.** A perfect explanation for the second-best move beats a silent recommendation of the best.
- **No LLM at runtime, ever.** All explanations come from structured analysis data filled into templates. No hallucination surface.
- **Deterministic analysis pipeline.** Given a position, output is fully reproducible.
- **The cross-check engine is a cross-check, not the oracle.** Our analysis runs first; the engine confirms or flags disagreement, and disagreement is itself surfaced as a learning moment.
- **Teach during play, not just after.** Every move the user plays gets classified and, on demand, explained — including what they should have seen. Static board analysis is a subset of this.
- **Share the hard work across platforms.** One Rust core; thin native UIs.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│  Platform Shells (thin)                                     │
│  ├─ iOS / iPadOS / macOS — SwiftUI Multiplatform,           │
│  │                         per-platform view divergence,    │
│  │                         Universal Purchase               │
│  ├─ Windows (and Linux)  — Rust GUI (egui), single binary   │
│  ├─ Android               — Kotlin + Jetpack Compose        │
│  └─ Web (later)           — TS + WASM                       │
└─────────────────────────────────────────────────────────────┘
                          │
                          │  FFI (uniffi for Swift/Kotlin,
                          │   direct Rust dep for egui,
                          │   wasm-bindgen for web)
                          ▼
┌─────────────────────────────────────────────────────────────┐
│  Rust Core (`chess-tutor-core`)                             │
│  ├─ Game state (history, turn, status, PGN I/O)             │
│  ├─ Move application + per-move feedback pipeline           │
│  ├─ Board & moves         (build on `shakmaty`)             │
│  ├─ Attacker/defender maps + SEE                            │
│  ├─ Tactical motif detector                                 │
│  ├─ Positional feature extractor                            │
│  ├─ Opening book (Polyglot reader)                          │
│  ├─ Forcing-line / quiescence search                        │
│  ├─ Bot opponent (capped-strength Viridithas)               │
│  └─ Explanation engine (template-based)                     │
└─────────────────────────────────────────────────────────────┘
                          │
                          ▼
┌─────────────────────────────────────────────────────────────┐
│  Cross-check engine: Viridithas (Rust, MIT)                 │
│  Linked as a library; called for low-depth check + bot play │
└─────────────────────────────────────────────────────────────┘
```

### Why this split

The Rust core is pure logic — no I/O, no UI, no platform APIs. Every feature is unit-testable from a FEN or a PGN. Platform shells only handle: rendering the board, accepting user input, driving the `Game` state, and displaying the prose the core returns.

## The `Game` Contract

The core's primary runtime object. Platform shells instantiate one per active game.

- **State**: starting position, move history (SAN + UCI), current position, side to move, status (ongoing / checkmate / stalemate / 50-move / threefold / resignation / draw offer)
- **Operations**:
  - `apply(Move)` → returns a `MoveReport` (see below). Rejects illegal moves.
  - `undo()` → pops the last move. Keeps explanation caches valid.
  - `legal_moves()`, `legal_moves_from(square)` → for UI highlighting
  - `to_pgn()`, `from_pgn()` → round-trip games
  - `resign(side)`, `offer_draw(side)`, `claim_draw()` → game-state transitions
- **`MoveReport`** (the teaching artefact):
  - Classification: `Best | Excellent | Good | Inaccuracy | Mistake | Blunder | Book | Forced`
  - `PositionAnalysis` for the position *before* the move (the choice the user faced)
  - What they actually played, with its analysis
  - What the best move was, with its analysis
  - The *difference* — what was missed, in one prioritised sentence, with a deep-dive available on demand
  - Engine agreement flag

## The `PositionAnalysis` Contract

The per-position analysis the `Game` pipeline consumes. Unit-testable from any FEN — analysis mode is just "build a `Game` from this FEN and analyse the current position."

- **Square-level data**: attackers and defenders per square (bitboards), SEE values for every possible capture
- **Candidate moves**: ranked list with structured annotations — material change, tactical motifs involved, positional consequences, resulting imbalances
- **Tactical motifs detected**: forks, pins (absolute/relative), skewers, discovered attacks, double checks, deflection, interference, overloading, back-rank weakness, smothered-mate patterns, trapped pieces, x-ray attacks
- **Positional features**: pawn structure (passed, isolated, doubled, backward, hanging, islands), open/semi-open files, outposts, bishop pair, bad bishops, fianchetto status, king safety (attackers/defenders on king ring, pawn shelter, open lines toward king), space advantage, piece activity/mobility
- **Opening identification**: ECO code + name + common continuations, if in book
- **Forcing lines**: quiescence-style walk of checks/captures/threats with resulting position assessment
- **Engine check**: top move, eval, agreement/disagreement flag

This is the source of truth the `Explainer` walks to produce prose.

## The Explainer

Template-based, pure Rust, zero ML. Roughly 200–300 stock phrase templates with slot-filling cover ~95% of chess commentary. Phrases prioritised by significance so the app shows the most important insight first, with progressive disclosure for detail.

Every explanation is paired: **your move's explanation** + **engine's top move's explanation** + **the specific difference**. This is the pedagogical feature that neither chess.com nor Lichess provides.

### Coaching cadence (default)

- After each user move: one-line classification + tagline ("Inaccuracy — you missed a discovered attack on d5").
- Tap/click for the full deep-dive: missed motifs, the forcing line the engine saw, the paired "your move vs. best move" comparison.
- After-game review walks every move in sequence with the deep-dive expanded for every non-book, non-best move.

## Tech Stack

### Core (Rust)

- **`shakmaty`** — board representation, move generation, FEN/SAN/PGN, Zobrist hashing
- **`uniffi`** — auto-generates Swift + Kotlin bindings from a single UDL file
- **`wasm-bindgen`** — web target (later phase)
- Custom modules for SEE, motif detection, positional features, explainer

### Cross-check engine — Viridithas (decided)

- **Viridithas** (https://github.com/cosmobobak/viridithas), Rust, **MIT-licensed**.
- Chosen over Stockfish to avoid GPLv3's App Store friction and the requirement to ship the whole app's source to every paying customer.
- Chosen over Carp and Pleco because Viridithas's license is the only one confirmed permissive; it is also the strongest and most actively maintained of the Rust-native options.
- Ships as a UCI binary, not a library crate. Integration path: fork it, library-ify the search/eval entry points, link directly into `chess-tutor-core` behind the [`engine::CrossCheckEngine`] trait. No subprocess — iOS/Android both make process spawning awkward.
- **Bundle-size caveat:** NNUE nets are typically 30–50 MB. For a low-depth cross-check we can later swap for a smaller/quantised net, or strip NNUE and use classical eval. That's a Phase 2 optimisation, not a scaffold blocker.
- The trait abstraction stays regardless — if we ever need Stockfish-strength analysis on a platform where GPLv3 is acceptable (e.g. a desktop companion), we can plug a second engine in behind the same interface.

### Apple platforms (iOS / iPadOS / macOS)

- **Xcode Multiplatform SwiftUI template**, one Xcode project, one target family, Universal Purchase.
- Substantial per-platform view divergence — not just `#if os(macOS)` sprinkles. iOS gets a touch-first layout with analysis panels that slide under the board; macOS gets a proper desktop layout with a sidebar move list, menu bar, keyboard shortcuts, window-sizing that adapts to game vs. analysis vs. review modes, and multi-window.
- Shared view models and services across platforms; only the top-level scenes and primary views fork.
- FFI through `uniffi`-generated Swift bindings over the Rust core.
- Universal Purchase toggled on in App Store Connect **before the first TestFlight build** (retrofitting later is painful).

### Windows (and Linux) — Rust GUI

- **egui** (MIT/Apache-2.0), single compiled binary, no .NET / runtime dependencies.
- Board rendering via `wgpu` — fast, handles animated piece drops, highlighting, arrows.
- Same binary works on Linux and macOS for development, but **macOS's shipping app is the SwiftUI one** — this is the Windows target plus a dev convenience.
- Depends on `chess-tutor-core` directly as a Rust crate; no FFI layer needed.

### Android (deprioritised — Phase 5)

- Kotlin + Jetpack Compose, JNI bindings generated by `uniffi`.
- No Android hardware on the team at the moment, so this ships after Apple and Windows are solid.

### Data assets (bundled)

- Polyglot `.bin` opening book built from master PGN dump (Lichess masters DB or Caissabase)
- ECO code → name mapping
- Viridithas NNUE file (bundled per-platform; size TBD — see "Cross-check engine" above)

## Development Phases

### Phase 1 — Core foundations (Rust only, no UI)

Goal: prove the game loop + analysis pipeline. The CLI becomes a playable interface (human vs. human over stdin, or human vs. bot) that annotates every move.

- [x] Cargo workspace with `shakmaty` dependency
- [x] `Game` state skeleton (history, apply/undo, legal moves, status)
- [ ] Attacker/defender map per square
- [ ] Static Exchange Evaluation (SEE)
- [ ] Detect checks, captures, threats for both sides
- [ ] Basic tactical motifs: fork, pin (absolute + relative), skewer, discovered attack, double check
- [ ] Basic positional features: passed/isolated/doubled/backward pawns, open/semi-open files, bishop pair
- [ ] King safety scoring (attackers on king ring, pawn shelter)
- [ ] Polyglot book reader
- [ ] Quiescence search for forcing lines (capped depth, captures + checks)
- [ ] Move classification: `Best/Excellent/Good/Inaccuracy/Mistake/Blunder/Book/Forced` (eval-delta + our own heuristics)
- [ ] Template explainer — first pass, ~50 templates covering the above
- [ ] CLI: `chess-tutor analyze "<fen>"`, `chess-tutor play`, `chess-tutor review <pgn>`
- [ ] Test suite of ~100 positions with expected tactical findings, sourced from the Lichess puzzle database

### Phase 2 — Cross-check engine integration (Viridithas)

- [x] Decide: Stockfish vs. Rust-native engine → **Viridithas (MIT)**
- [x] Engine trait (`chess_tutor_core::engine::CrossCheckEngine`) landed in scaffold
- [ ] Fork Viridithas, expose search/eval as library API (currently ships as a UCI binary)
- [ ] Wire fork in as a path/git dependency of `chess-tutor-core`
- [ ] Decide NNUE strategy: full net vs. smaller/quantised vs. classical-eval-only (bundle-size call)
- [ ] Cross-check logic: agreement/disagreement flagging against our top candidate
- [ ] Extend explainer to narrate disagreements ("the engine prefers X because …")
- [ ] Bot opponent: reuse Viridithas with skill-level / contempt / depth caps to hit target ELOs

### Phase 3 — Apple app (iOS / iPadOS / macOS, Universal Purchase)

- [ ] `uniffi` bindings for the core (both `Game` operations and `PositionAnalysis`)
- [ ] Build script → `ChessTutorCore.xcframework`
- [ ] SwiftUI board view — drag-drop moves, legal-move highlighting, piece animation
- [ ] Game loop: play vs. human (local, pass-and-play) and vs. bot
- [ ] Per-move feedback overlay + deep-dive sheet
- [ ] After-game review flow
- [ ] iOS layout: touch-first, analysis under board
- [ ] macOS layout: sidebar move list, menus, keyboard shortcuts, multi-window
- [ ] PGN import (paste / file)
- [ ] Universal Purchase configured in App Store Connect **before first submission**

### Phase 4 — Windows desktop (Rust + egui)

- [ ] `chess-tutor-desktop` crate in the workspace — egui-based single binary
- [ ] Board rendering (wgpu), drag-drop input, highlights, arrows
- [ ] Same game loop features as Apple (local human vs. human, human vs. bot, review)
- [ ] Per-move feedback panel + deep-dive drawer
- [ ] PGN import/export
- [ ] Windows installer (MSIX or plain .exe) — no .NET, no VCRuntime beyond what's statically linked

### Phase 5 — Android

- [ ] `uniffi` Kotlin bindings
- [ ] Build script for Android `.so` libs
- [ ] Compose UI port of the core views

### Phase 6 — Web (optional)

- [ ] `wasm-bindgen` target
- [ ] TS frontend (Vite + whatever UI framework)

### Phase 7 — Content & polish

- [ ] Expand explainer templates to ~300
- [ ] Expanded motif library (deflection, interference, overloading, trapped pieces, x-ray, zugzwang hints, common mating patterns)
- [ ] Expanded positional: outposts, minority attack patterns, pawn breaks, space, piece coordination
- [ ] Curated opening trainer with annotated main lines for common openings
- [ ] Puzzle mode sourced from Lichess puzzle database
- [ ] Clocks / time controls
- [ ] Network play (scope TBD — LAN invite? Lichess API? Own relay?)

## Key Decisions

1. **Engine choice — DECIDED: Viridithas (Rust, MIT).** Rationale in "Cross-check engine" above. Same engine doubles as the capped-strength bot opponent.
2. **Licensing — DECIDED: proprietary / all-rights-reserved.** Cargo manifests use `license = "UNLICENSED"` (the Cargo convention for "not for public distribution"). No public source release.
3. **Apple bundle ID — DECIDED: `com.unbrokentechnology.chesstutor`.** Used for both the iOS and macOS targets in a single Xcode project to enable Universal Purchase. Apple Developer Program membership: active (enrolled February 2026). Remaining to-do before first TestFlight build: register the App ID in the Developer Portal and toggle Universal Purchase in App Store Connect.
4. **Apple UI approach — DECIDED: Xcode Multiplatform SwiftUI with substantial per-platform view divergence.** macOS is a proper desktop experience with sidebar/menus/keyboard shortcuts, not a scaled-up iOS app.
5. **Windows UI approach — DECIDED: Rust + egui, single binary, no .NET / runtime deps.** Same binary works on Linux and macOS for development; macOS's shipping app is SwiftUI.
6. **Opening book source — DECIDED: Lichess masters DB** (free, permissive). Revisit if coverage of early deviations is weak.
7. **Puzzle data — DECIDED: Lichess puzzle database** (CC0, tagged with motif labels — perfect for validating our tactics detector).
8. **Bot opponent — DEFAULT (not final): reuse Viridithas with skill-level / depth caps.** Revisit after Phase 2 if capped-engine play feels too mechanical vs. a bespoke sparring bot.
9. **Coaching cadence — DEFAULT (not final): per-move classification + tagline, deep-dive on demand, full review post-game.** Revisit after Phase 1 when the explainer is real enough to dogfood.
10. **Time controls — DEFERRED to Phase 7.** `Game` carries optional clock fields from day one so we don't have to thread them through later, but the UI ignores them in v1.
11. **Network play — DEFERRED to Phase 7.** Scope decision (LAN invite vs. Lichess API vs. own relay) lives with that phase.

## Data Sources (all free / permissive)

- **Lichess puzzle database** — CC0, millions of tagged puzzles, includes motif tags (perfect for validating tactical detection)
- **Lichess masters / open DB** — master games for opening book
- **Chess Programming Wiki** — reference for SEE, evaluation features, search
- **ECO codes** — public domain
- **Viridithas source** — MIT, Rust-native, plugs in directly as our cross-check engine
- **Stockfish source** — GPLv3, not shipped, but the classical evaluation code is still a reference goldmine of positional heuristics we can draw from (heuristics aren't copyrightable; specific code is)

## Non-Goals

- Maximum playing strength. We are not competing with Stockfish on ELO (and Viridithas is already well past any strength our teaching layer needs).
- Online multiplayer, accounts, or a social layer — at most, defer-phase scoped network play so two local humans can play over a LAN.
- Runtime ML/LLM inference.
- Cloud dependencies. The app must work on a plane.

## Success Criteria

- A 1200-rated user can, over weeks of use, articulate *why* their moves are good or bad using the vocabulary the app has taught them.
- For the Lichess puzzle test set, motif detection matches the provided tags with ≥95% recall on the tactics we explicitly support.
- Given any reasonable position, the app produces an explanation that accurately names the relevant tactical and positional factors.
- Users can play full games against a local opponent or a bot and receive move-by-move feedback that explains not just *what* was wrong but *what they should have seen* — the defining pedagogical feature.
- One Apple purchase; app runs on iPhone, iPad, and Mac.
- Windows app ships as a single binary with no external runtime dependencies.
