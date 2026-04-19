# Windows desktop app (Rust + egui)

Single-binary Rust GUI built on `egui`. No .NET, no VCRuntime beyond what's statically linked, no installer dependencies.

This crate is **not** a member of the `core/` Cargo workspace. It's a standalone Cargo project that path-depends on `../core/chess-tutor-core`. This keeps `core/` as a pure-library workspace so that mobile FFI builds don't have to resolve egui's graphics deps.

## Run locally

```sh
cd desktop/chess-tutor-desktop
cargo run --release
```

Works on Windows (the shipping target), Linux, and macOS — the last two are dev conveniences. macOS's real shipping build is the SwiftUI app under `../apple/`.

## Layout

```
desktop/
└── chess-tutor-desktop/
    ├── Cargo.toml
    └── src/
        ├── main.rs          entry point + eframe App impl
        ├── board.rs         board rendering + input
        ├── game_view.rs     move list, clocks (later), feedback panel
        └── theme.rs         piece sprites, colours, dark/light
```

## Packaging (later)

- Windows: ship a signed `.exe` directly. Consider WiX/MSIX only if we need Start-menu integration and auto-update.
- Linux: `.AppImage` or plain static binary.
