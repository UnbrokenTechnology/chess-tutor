# Chess Tutor — Project Plan

An offline chess learning app that explains **why** moves are good or bad, not just which move an engine prefers. Target: bridge a ~1200 ELO player toward ~2000 through deterministic, explainable position analysis.

## Guiding Principles

- **Explanations first, evaluation second.** A perfect explanation for the second-best move beats a silent recommendation of the best.
- **No LLM at runtime, ever.** All explanations come from structured analysis data filled into templates. No hallucination surface.
- **Deterministic analysis pipeline.** Given a FEN, output is fully reproducible.
- **Stockfish is a cross-check, not the oracle.** Our analysis runs first; Stockfish confirms or flags disagreement, and disagreement is itself surfaced as a learning moment.
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
│  Stockfish (C++, bundled as static lib)                 │
│  Called for low-depth cross-check only                  │
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
- **Stockfish check**: top move, eval, agreement/disagreement flag

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

### Stockfish

- Bundled as static C++ library, built for each target platform
- GPLv3 implications: the shipped app must also be GPLv3-compatible. **Decide early** whether this is acceptable, or whether to use a Rust-native engine (`Carp`, `Viridithas`) for the cross-check and avoid the license constraint. For low-depth sanity checks, a weaker engine is fine.

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
- Static Stockfish NNUE file (if using Stockfish)

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

### Phase 2 — Stockfish integration

- [ ] Decide: Stockfish vs. Rust-native engine (GPL decision)
- [ ] Wrap engine behind a trait so it can be swapped
- [ ] Cross-check logic: agreement/disagreement flagging
- [ ] Extend explainer to narrate disagreements

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

## Key Decisions to Make Early

1. **Engine choice**: Stockfish (strongest, GPLv3) vs. Rust-native (cleaner build, weaker play, permissive license). For a low-depth cross-check, a weaker engine is genuinely fine — the analysis layer is doing the teaching.
2. **Opening book source**: Lichess masters DB (free, permissive) vs. building custom from curated PGN. Start with Lichess masters.
3. **Puzzle data**: Lichess publishes its puzzle database under CC0. Use it.
4. **Universal Purchase bundle ID**: pick it before the first TestFlight build.

## Data Sources (all free / permissive)

- **Lichess puzzle database** — CC0, millions of tagged puzzles, includes motif tags (perfect for validating tactical detection)
- **Lichess masters / open DB** — master games for opening book
- **Chess Programming Wiki** — reference for SEE, evaluation features, search
- **ECO codes** — public domain
- **Stockfish source** — GPLv3, classical evaluation code is a goldmine of positional heuristics even if not shipping Stockfish itself

## Non-Goals

- Maximum playing strength. We are not competing with Stockfish on ELO.
- Online play, accounts, or multiplayer.
- Runtime ML/LLM inference.
- Cloud dependencies. The app must work on a plane.

## Success Criteria

- Given any reasonable position, the app produces an explanation that accurately names the relevant tactical and positional factors.
- For the Lichess puzzle test set, motif detection matches the provided tags with ≥95% recall on the tactics we explicitly support.
- A 1200-rated user can, over weeks of use, articulate *why* their moves are good or bad using the vocabulary the app has taught them.
- One purchase; app runs on iPhone, iPad, and Mac.
