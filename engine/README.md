# Cross-check engine vendoring

**Decision:** Viridithas (https://github.com/cosmobobak/viridithas), Rust, MIT.

Viridithas ships as a UCI binary crate, not a library. We fork it here and
expose the search + evaluation entry points as a Rust library API, then depend
on the fork from `chess-tutor-core` via a path (local dev) or git (CI)
dependency. No subprocess — iOS and Android both make process spawning
awkward.

## Phase 2 TODO

- Fork `cosmobobak/viridithas` under `unbrokentechnology/viridithas-lib`.
- Expose `search(position, depth) -> (best_move, eval)` as a public API.
- Add as dependency of `chess-tutor-core`.
- Implement `chess_tutor_core::engine::CrossCheckEngine` for it.
- Decide NNUE strategy: full net vs. smaller/quantised vs. classical-eval-only
  (drives the bundle-size call — see `PLAN.md`).

## Why not Stockfish

See `PLAN.md` → "Cross-check engine". Summary: GPLv3 would force the whole
app's source open to every paying customer, and Stockfish + App Store have a
standing compatibility conflict we don't want to fight.
