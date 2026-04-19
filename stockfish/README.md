# Stockfish (Phase 2)

Added once the engine-choice decision lands (see `PLAN.md` → "Key Decisions").

Plan:

1. Add `https://github.com/official-stockfish/Stockfish` as a git submodule under `vendor/`.
2. Build a static lib per target via the scripts in this directory.
3. Wire it behind the `chess_tutor_core::engine::CrossCheckEngine` trait.

## Per-platform build scripts

| Script              | Produces                                           |
|---------------------|----------------------------------------------------|
| `build-ios.sh`      | `libstockfish-ios.a` + simulator variants          |
| `build-android.sh`  | `libstockfish.so` per Android ABI                  |
| `build-macos.sh`    | `libstockfish-macos.a` (arm64 + x86_64 fat)        |

## License

Stockfish is **GPLv3**. Shipping it forces the whole app to be GPLv3-compatible. If that is unacceptable, swap in a Rust-native engine (`Carp`, `Viridithas`) behind the same trait — for a low-depth cross-check this is fine.
