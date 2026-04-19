# Chess Tutor

Offline chess learning app that explains **why** moves are good or bad. Cross-platform, one Rust core, thin native UIs.

See [`PLAN.md`](PLAN.md) for the full architecture and roadmap.

## Repository Layout

```
chess-tutor/
├── core/          Rust workspace — analysis, tactics, positional, explainer, FFI, CLI
├── engine/        Vendored cross-check engine (Viridithas fork, Phase 2)
├── apple/         SwiftUI app (iOS, iPadOS, macOS — Universal Purchase)
├── android/       Kotlin + Jetpack Compose app
├── assets/        Opening book, ECO mapping, other bundled data
└── scripts/       Cross-platform build glue
```

## Quick Start (Rust core)

```sh
cd core
cargo build
cargo test
cargo run -p chess-tutor-cli -- analyze "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"
```

## Platform Builds

| Target       | Script                                |
|--------------|---------------------------------------|
| Apple        | `scripts/build-xcframework.sh`        |
| Android      | `scripts/build-android-libs.sh`       |
| Opening book | `scripts/build-book.sh`               |

## Status

Phase 1 scaffold. See [`PLAN.md`](PLAN.md) for the checklist.

## License

TBD — depends on the engine choice (see PLAN.md "Key Decisions"). Until that call is made, treat this repo as proprietary / all-rights-reserved.
