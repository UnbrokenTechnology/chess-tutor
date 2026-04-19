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
cargo run -p chess-tutor-cli -- play

# Windows / Linux / macOS desktop GUI (Phase 4 shell, board rendering TBD)
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
