# Chess Tutor — Project Plan

An offline chess learning app that explains **why** moves are good or bad, not just which move an engine prefers. Target: bridge a ~1200 ELO player toward ~2000 through deterministic, explainable position analysis.

## Guiding Principles

- **Explanations first, evaluation second.** A perfect explanation for the second-best move beats a silent recommendation of the best.
- **No LLM at runtime, ever.** All explanations come from structured analysis data filled into templates. No hallucination surface.
- **Deterministic analysis pipeline.** Given a FEN, output is fully reproducible.
- **The cross-check engine is a cross-check, not the oracle.** Our analysis runs first; the engine confirms or flags disagreement, and disagreement is itself surfaced as a learning moment.
- **Share the hard work across platforms.** One Rust core; thin native UIs.

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│  Platform Shells (thin)                                 │
│  ├─ iOS / iPadOS / macOS  — SwiftUI (Universal Purchase)│
│  ├─ Android               — Kotlin + Jetpack Compose    │
│  └─ Web (later)           — TS + WASM                   │
└─────────────────────────────────────────────────────────┘
                          │
                          │  FFI (uniffi for Swift/Kotlin,
                          │   wasm-bindgen for web)
                          ▼
┌─────────────────────────────────────────────────────────┐
│  Rust Core (`chess-tutor-core`)                         │
│  ├─ Board & moves         (build on `shakmaty`)         │
│  ├─ Attacker/defender maps + SEE                        │
│  ├─ Tactical motif detector                             │
│  ├─ Positional feature extractor                        │
│  ├─ Opening book (Polyglot reader)                      │
│  ├─ Forcing-line / quiescence search                    │
│  └─ Explanation engine (template-based)                 │
└─────────────────────────────────────────────────────────┘
                          │
                          ▼
┌─────────────────────────────────────────────────────────┐
│  Cross-check engine: Viridithas (Rust, MIT)             │
│  Linked as a library; called for low-depth check only   │
└─────────────────────────────────────────────────────────┘
```

### Why this split

The Rust core is pure logic — no I/O, no UI, no platform APIs. Every analysis feature is unit-testable from a FEN string. Platform shells only handle: rendering the board, user input, calling the core, displaying the explanation text the core returns.

## The `PositionAnalysis` Contract

The core's main output, for any given position:

- **Square-level data**: attackers and defenders per square (bitboards), SEE values for every possible capture
- **Candidate moves**: ranked list with structured annotations — material change, tactical motifs involved, positional consequences, resulting imbalances
- **Tactical motifs detected**: forks, pins (absolute/relative), skewers, discovered attacks, double checks, deflection, interference, overloading, back-rank weakness, smothered-mate patterns, trapped pieces, x-ray attacks
- **Positional features**: pawn structure (passed, isolated, doubled, backward, hanging, islands), open/semi-open files, outposts, bishop pair, bad bishops, fianchetto status, king safety (attackers/defenders on king ring, pawn shelter, open lines toward king), space advantage, piece activity/mobility
- **Opening identification**: ECO code + name + common continuations, if in book
- **Forcing lines**: quiescence-style walk of checks/captures/threats with resulting position assessment
- **Engine check**: top move, eval, agreement/disagreement flag

This is the source of truth the `Explainer` walks to produce prose.

## The Explainer

Template-based, pure Rust, zero ML. Roughly 200–300 stock phrase templates with slot-filling cover ~95% of chess commentary. Phrases prioritized by significance so the app shows the most important insight first, with progressive disclosure for detail.

Every explanation is paired: **your move's explanation** + **engine's top move's explanation** + **the specific difference**. This is the pedagogical feature that neither chess.com nor Lichess provides.

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

### Apple platforms

- Single SwiftUI codebase, platform-conditional views where needed
- **Universal Purchase enabled from day one in App Store Connect** (much harder to add later)
- Supports iPhone, iPad, and Mac from the same purchase
- "Designed for iPad" on Apple Silicon Macs gives free Mac testing during early dev

### Android

- Kotlin + Jetpack Compose
- JNI bindings generated by `uniffi`

### Data assets (bundled)

- Polyglot `.bin` opening book built from master PGN dump (Lichess masters DB or Caissabase)
- ECO code → name mapping
- Viridithas NNUE file (bundled per-platform; size TBD — see "Cross-check engine" above)

## Development Phases

### Phase 1 — Core foundations (Rust only, no mobile)

Goal: prove the analysis pipeline works. Build everything as a Rust CLI that takes a FEN and outputs a JSON `PositionAnalysis` plus an English explanation.

- [ ] Set up Cargo workspace, depend on `shakmaty`
- [ ] Attacker/defender map per square
- [ ] Static Exchange Evaluation (SEE)
- [ ] Detect checks, captures, threats for both sides
- [ ] Basic tactical motifs: fork, pin (absolute + relative), skewer, discovered attack, double check
- [ ] Basic positional features: passed/isolated/doubled/backward pawns, open/semi-open files, bishop pair
- [ ] King safety scoring (attackers on king ring, pawn shelter)
- [ ] Polyglot book reader
- [ ] Quiescence search for forcing lines (capped depth, captures + checks)
- [ ] Template explainer — first pass, ~50 templates covering the above
- [ ] CLI tool: `chess-tutor analyze "<fen>"`
- [ ] Test suite of ~100 FENs with expected tactical findings (build from Lichess puzzle database — it's free and labeled)

### Phase 2 — Cross-check engine integration (Viridithas)

- [x] Decide: Stockfish vs. Rust-native engine → **Viridithas (MIT)**
- [x] Engine trait (`chess_tutor_core::engine::CrossCheckEngine`) landed in scaffold
- [ ] Fork Viridithas, expose search/eval as library API (currently ships as a UCI binary)
- [ ] Wire fork in as a path/git dependency of `chess-tutor-core`
- [ ] Decide NNUE strategy: full net vs. smaller/quantised vs. classical-eval-only (bundle-size call)
- [ ] Cross-check logic: agreement/disagreement flagging against our top candidate
- [ ] Extend explainer to narrate disagreements ("the engine prefers X because …")

### Phase 3 — iOS/macOS app (Universal Purchase)

- [ ] `uniffi` bindings for the core
- [ ] Build script → `ChessTutorCore.xcframework`
- [ ] SwiftUI board view (drag-drop moves, legal move highlighting)
- [ ] Analysis panel that renders `PositionAnalysis` results
- [ ] "Your move vs. best move" comparison view
- [ ] Game import (PGN paste / file)
- [ ] Universal Purchase configured in App Store Connect **before first submission**
- [ ] macOS target via SwiftUI platform conditionals (not Catalyst)

### Phase 4 — Android

- [ ] `uniffi` Kotlin bindings
- [ ] Build script for Android `.so` libs
- [ ] Compose UI port of the core views

### Phase 5 — Web (optional)

- [ ] `wasm-bindgen` target
- [ ] TS frontend (Vite + whatever UI framework)

### Phase 6 — Content & polish

- [ ] Expand explainer templates to ~300
- [ ] Expanded motif library (deflection, interference, overloading, trapped pieces, x-ray, zugzwang hints, common mating patterns)
- [ ] Expanded positional: outposts, minority attack patterns, pawn breaks, space, piece coordination
- [ ] Curated opening trainer with annotated main lines for common openings
- [ ] Puzzle mode sourced from Lichess puzzle database

## Key Decisions

1. **Engine choice — DECIDED: Viridithas (Rust, MIT).** Rationale in "Cross-check engine" above.
2. **Licensing — DECIDED: proprietary / all-rights-reserved.** Cargo manifests use `license = "UNLICENSED"` (the Cargo convention for "not for public distribution"). No public source release.
3. **Apple bundle ID — DECIDED: `com.unbrokentechnology.chesstutor`.** Used for both the iOS and macOS targets in a single Xcode project to enable Universal Purchase. Apple Developer Program membership: active (enrolled February 2026). Remaining to-do before first TestFlight build: register the App ID in the Developer Portal and toggle Universal Purchase in App Store Connect.
4. **Opening book source — DECIDED: Lichess masters DB** (free, permissive). Revisit if coverage of early deviations is weak.
5. **Puzzle data — DECIDED: Lichess puzzle database** (CC0, tagged with motif labels — perfect for validating our tactics detector).

## Data Sources (all free / permissive)

- **Lichess puzzle database** — CC0, millions of tagged puzzles, includes motif tags (perfect for validating tactical detection)
- **Lichess masters / open DB** — master games for opening book
- **Chess Programming Wiki** — reference for SEE, evaluation features, search
- **ECO codes** — public domain
- **Viridithas source** — MIT, Rust-native, plugs in directly as our cross-check engine
- **Stockfish source** — GPLv3, not shipped, but the classical evaluation code is still a reference goldmine of positional heuristics we can draw from (heuristics aren't copyrightable; specific code is)

## Non-Goals

- Maximum playing strength. We are not competing with Stockfish on ELO (and Viridithas is already well past any strength our teaching layer needs).
- Online play, accounts, or multiplayer.
- Runtime ML/LLM inference.
- Cloud dependencies. The app must work on a plane.

## Success Criteria

- Given any reasonable position, the app produces an explanation that accurately names the relevant tactical and positional factors.
- For the Lichess puzzle test set, motif detection matches the provided tags with ≥95% recall on the tactics we explicitly support.
- A 1200-rated user can, over weeks of use, articulate *why* their moves are good or bad using the vocabulary the app has taught them.
- One purchase; app runs on iPhone, iPad, and Mac.
