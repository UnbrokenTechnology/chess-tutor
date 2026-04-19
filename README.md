# Chess Tutor

Offline chess learning app that explains **why** moves are good or bad. Cross-platform, one Rust core, thin native UIs.

See [`PLAN.md`](PLAN.md) for the full architecture and roadmap.

## Repository Layout

```
chess-tutor/
├── core/          Rust workspace — Game state, analysis, explainer, FFI, CLI
├── engine/        Vendored cross-check engine (Viridithas fork, Phase 2)
├── desktop/       Rust + egui app (Windows shipping target; also dev on Linux/macOS)
├── apple/         SwiftUI Multiplatform app (iOS, iPadOS, macOS — Universal Purchase)
├── android/       Kotlin + Jetpack Compose app (deprioritised — Phase 5)
├── assets/        Opening book, ECO mapping, other bundled data
└── scripts/       Cross-platform build glue
```

## Quick Start

```sh
# Core library + CLI
cd core
cargo build
cargo test

# Render a board from a FEN
cargo run -p chess-tutor-cli -- board
cargo run -p chess-tutor-cli -- board "r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3"

# Play a game in the terminal — live Unicode board, last-move highlight,
# 5+3 Fischer clock, auto-flip so the mover is always at the bottom.
cargo run -p chess-tutor-cli -- play --time 300 --increment 3 --auto-flip

# Need ASCII pieces (terminal without chess-glyph support)?
cargo run -p chess-tutor-cli -- play --ascii
```

The `play` loop accepts moves in **either** notation:

- **SAN** (algebraic) — `e4`, `Nf3`, `O-O`, `Qxf7#`, `e8=Q`, `Nbd2` for disambiguation
- **UCI** — `e2e4`, `g1f3`, `e1g1` for castling, `e7e8q` for promotion

Commands during play:

- `moves` — every legal move as SAN
- `hanging` — every piece where attackers > defenders
- `attackers e4` — who attacks the given square (both colours)
- `attackers N` / `attackers n` — attackers on each white / black knight. Same letters work for K Q R B P.
- `undo`, `resign`, `flip`, `fen`, `help`, `quit`

```sh
# Windows / Linux / macOS desktop GUI (Phase 4 shell — window boots,
# board rendering lands with Phase 4)
cd ../desktop/chess-tutor-desktop
cargo run --release
```

## Platform Builds

| Target       | Script                                |
|--------------|---------------------------------------|
| Apple        | `scripts/build-xcframework.sh`        |
| Android      | `scripts/build-android-libs.sh`       |
| Opening book | `scripts/build-book.sh`               |

The Windows/desktop target just runs `cargo build --release` inside `desktop/chess-tutor-desktop` — no script needed.

## Status

Phase 1 scaffold. See [`PLAN.md`](PLAN.md) for the checklist.

## License

TBD — depends on the engine choice (see PLAN.md "Key Decisions"). Until that call is made, treat this repo as proprietary / all-rights-reserved.
