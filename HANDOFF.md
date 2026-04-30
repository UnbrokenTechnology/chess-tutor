# Handoff: chess-tutor-2 — current state

A snapshot for a fresh context to pick up the next task. **Read [`CLAUDE.md`](CLAUDE.md) first** for evergreen guidance (mission, legal/licensing, ground rules); this file is forward-looking only — git history covers what's been built, inline module docs (`//!`) cover design rationale.

## What this app is

A **chess tutor**, not a chess engine. The product surface is move-by-move teaching feedback for ~1200 ELO students climbing toward the 1600+ range — a market that classical engines and modern NNUE engines both fail to serve, the former because they only output a number and the latter because their evaluation is opaque. Strength is a means: 2000-ish ELO is enough to pose interesting positions; explainability is the actual product. Three pillars:

1. **The engine** is a classical Stockfish-11 port (NNUE banned). 2000 ELO verified empirically. Search has the full SF11 pruning stack; eval decomposes into 45 named sub-terms keyed by `TermId`, each with mg/eg components and a per-term tapered cp delta the teaching layer reads.
2. **The teaching layer** lives in [`core/engine/src/analysis/`](core/engine/src/analysis/) — see that module's `//!` doc for the design principles. Per-move output traces every UI claim back to a concrete engine signal: term deltas, structured outcome snapshots (king safety, threats, mobility, pawn structure, passed pawns, piece placement, material capture sequences), surprise classification, and a verdict.
3. **The narration crate** (`core/narration/`) renders structured outcomes into prose. Public surface: `format_retrospective(pre_move_pos, &[MoveAnalysis], user_move, &NarrationOptions) -> String`.

UIs: CLI (`chess-tutor`), egui desktop (`chess-tutor-desktop`), planned Apple (Swift/SwiftUI) + Android (Kotlin/Compose). FFI crate (`core/ffi/`) is the prerequisite for the platform apps and doesn't exist yet.

Tests: **582 engine + 105 narration + 46 cli = 733 passing**, clippy clean.

## Build / dev commands

```bash
cargo test --release       # default; debug is 20–200× slower (magic search)
cargo build --release      # → target/release/chess-tutor[-desktop].exe
cargo clippy --all-targets

# Profiling build (release-equivalent + debuginfo for VTune):
cargo build --profile profiling --bin chess-tutor
# → target/profiling/chess-tutor.exe
```

## Active plan: per-node + per-depth perf pass

We're chasing wall-clock-to-depth, primarily so the engine is responsive on iPhone-class hardware. Reference: cold startpos depth-12 search runs at **~0.87 Mnps**, depth-14 at **~1.30 Mnps**, warm interactive play at **~1.55 Mnps**. Stockfish 11 single-thread is ~3–5 Mnps on similar hardware, so we're ~3–4× behind per node. Roughly half of that gap is per-node code cost, half is nodes-per-depth (move ordering).

`Engine::last_nodes() / last_elapsed() / last_nps()` surface the stats; CLI `play` and `search` print them after each move. Auto-retrospective also prints its own timing line.

Five tasks tracked as `TaskCreate` items #5–#9. Order matters because #9 depends on the per-ply stack added in #7.

### #5 — `MAX_PLY` 246 → 64 (standalone, do first)

`pub const MAX_PLY: usize = Value::MAX_PLY as usize;` at [`core/engine/src/search.rs:35`](core/engine/src/search.rs). Verify whether `Value::MAX_PLY` ([`core/engine/src/types.rs`](core/engine/src/types.rs)) is a separate constant — if so, change both.

PV table is sized `MAX_PLY × MAX_PLY × 8 bytes`: 246² × 8 = ~485 KB allocated and zero-written per `Engine::search` call. With 64: 32 KB. **~450 KB saved per call**. Killers + pv_length save another ~5 KB combined.

Trade-off: bails earlier on extension-stacking pathological positions. Realistic check sequences hit threefold or 50-move rule before 30 plies, so 64 leaves comfortable headroom for `max_depth=20` plus extensions.

### #6 — Counter-move heuristic

New table on `Engine`: `counter_moves: Box<[[Move; 64]; 7]>` (~7 KB), indexed `[prev_piece_kind][prev_to_sq]`. Cleared on `new_game`. Update on β-cutoff for a quiet move: `counter_moves[prev_piece][prev_to] = our_move`.

**Separate `MovePicker` stage**, not score-boost — when the counter-move triggers a β-cutoff, the picker exits before generating, scoring, or sorting any quiets. Stage order becomes `MainTt → CaptureInit → GoodCapture → Killer0 → Killer1 → CounterMove → QuietInit → Quiet → BadCapture` (mirrors Stockfish's REFUTATION_PHASE). Validation mirrors `is_valid_killer`.

Expected: **~7–8 % nps** from skipped quiet generation + ~10 Elo of move-ordering improvement.

### #7 — Continuation history + `improving` flag (folded together)

Per-ply search stack on `Search`, one heap alloc at `Search::new`, sized `MAX_PLY+4` (Stockfish convention for safe pre/post-indexing). Each entry: `(moved_piece, to_sq, static_eval)` for the move played at that ply.

Four `[[i16; 64]; 7]` continuation-history tables (~1 KB each, ~4 KB total). Stockfish maintains 1-ply-ago, 2-ply-ago, 4-ply-ago + an aggregate. On β-cutoff: bump our move's score in the 1/2/4-ply tables, decrement losers tried before. `MovePicker` quiet scoring becomes `score = butterfly + cont1 + cont2 + cont4`.

`improving = stack[ply].static_eval > stack[ply-2].static_eval` (with in-check guards — consult Stockfish 11 `search.cpp` for exact fallback rule). Used to: loosen futility margins; lower late-move-pruning threshold; reduce LMR by 1 ply when improving; tighten null-move pruning when *not* improving.

Expected: ~10 % time-to-depth from continuation history (~20 Elo) + ~3–5 % from improving (~10 Elo). Per-ply stack also unlocks #9.

### #8 — Capture history (independent; can land any time)

`Box<[[[i16; 7]; 64]; 7]>` (~12 KB), indexed `[moving_piece][to_sq][captured_piece]`. Update on β-cutoff for a capture: bump winner, decrement losing captures tried before. Used as a tiebreaker on top of MVV-LVA inside `GoodCapture`.

Expected: ~3 % time-to-depth, ~10 Elo.

### #9 — Singular extensions (blocked by #7)

Add `excluded_move: Option<Move>` to `negamax`. Precondition: depth ≥ 8, TT bound ∈ {Exact, Lower}, `tt_depth ≥ depth − 3`, no excluded_move already (recursion guard). Verification search at `(depth - 1) / 2` with window `[singular_beta - 1, singular_beta]` where `singular_beta = tt_value − 2*depth`. If verification fails low, set `extension = 1` for the TT move. If `singular_beta ≥ beta`, multi-cut shortcut: return `singular_beta`. Skip TT writes when `excluded_move.is_some()`. `MovePicker` skips the excluded move.

Expected: ~15 % time-to-depth at depth 20+, ~70 Elo.

### Design decisions confirmed

- Per-ply stack: `Vec` on `Search` (one heap alloc per `Search::new`), not stack-allocated.
- Counter-moves: separate stage, not score-boost (the perf benefit is skipping quiet generation entirely on cutoff).
- `improving`: folded into #7 since both share the per-ply stack.
- `cutNode` (Stockfish's PV-vs-cut-node distinction): not adding for now. `is_pv` is close enough.

### Heap allocation policy

Per-search or per-engine allocations are fine. **Per-node allocations are not** — use stack arrays or pool from a thread-local. The `MovePicker` buffer pool (thread-local `Vec<Box<MoveBufs>>`) is the canonical pattern; copy it for any new feature that needs per-call scratch.

## Open dockets (not in the active plan)

### Engine perf, deferred until after the active plan

- **King-safety hash table** (~5–8 % nps). Key is `(pawn_key, king_sq, castling_rights)`. Same template as the pawn-structure cache.
- **Material hash table** (~3–5 % nps). [`material.rs`](core/engine/src/material.rs) flagged this internally; one hash on the material key.
- **Incremental `pos.occupied()`** as a `by_all: Bitboard` field. Toggle in `remove_piece` / `put_piece`. Tiny per-call but called many times in eval / movegen / SEE.
- **PEXT bitboards** for slider attacks (~5–10 % on slider terms). Requires "Haswell or newer" build target. Defer until everything else is done.
- **`ENGINE_TURN_NODE_CAP` review** — currently a flat 5M at [`core/cli/src/play.rs:35`](core/cli/src/play.rs). Engine play hits the cap consistently at depth 20 (5001216 nodes per move), so the cap is too tight to actually reach the requested depth in normal positions. Historically necessary because some closed positions ran 30+ minutes uncapped. Worth running a few "well-behaved" depth-20 positions uncapped to pick a number in the 15–50M range, or making the cap depth-aware (e.g. `4 * 6^depth.min(20)`).

### Engine strength, deferred

- **Time management** (`core/engine/src/timeman.rs` — file doesn't exist). Today `max_time` is a simple deadline. Proper allocation needs game time + increment + moves-to-TC.
- **Baked-in magic attack tables**. Magic numbers are searched at process start (LazyLock + xorshift); harvest from one local run, paste as `const`. Saves ~tens of ms per process start. Do when integrating the first platform app.
- **Remaining endgame specialists** — KRKP / KRKB / KRKN / KQKR / KQKP + pawn-heavy scaling functions (`KBPsK`, `KRPKR`, etc.).
- **Rubinstein trap** — user wants to work out its invariants first.

### Teaching layer, deferred

See [`core/engine/src/analysis/mod.rs`](core/engine/src/analysis/mod.rs) `//!` doc for full spec on:
- **Phase 2 — cheap-pass + surprise detection** (depth-1 qsearch + SEE for every legal move).
- **Phase 4 — signal-mask** (zero each `EvalTrace` term in turn, re-rank, surface "you'd prefer M' if you undervalued X").
- **Phase 5 — tactic library** (general patterns: pin / fork / skewer / double attack / discovered attack / etc., as a parallel module to `traps/`).

### UX / platform, deferred

- **Hint panel narration via narration crate refactor.** Hint panel currently shows `mv / score / PV`; richer narration should reuse the per-term narrators. Concretely: factor `narration::render_report`'s middle section into `render_per_term_narration(out, pre_move_pos, candidate, root_stm)`; expose `format_candidate_explanation(...)` that uses it without the verdict / engine-preferred framing.
- **Real piece sprites** (cburnett, CC-BY-SA from Lichess). 12 SVGs, `include_bytes!`, drop-in for `piece_glyph` callers.
- **Promotion picker UI.** Currently auto-queens. Inline 4-piece overlay near the target square is the standard pattern.
- **Visual annotations on retrospective.** GUI eventually draws arrows / highlights tied to specific narrator clauses. Requires changing narration output from flat `String` to a list of clauses with optional annotation payloads (square sets, arrows, kind tag).
- **Bot strength / customization framework.** Long-term: configurable openings, blunder profile, tactical eyesight per bot. Engine APIs already accommodate `Skill::enabled()`-style overrides.
- **FFI crate (`core/ffi/`).** First concrete step toward Apple/Android. Decisions outstanding: UniFFI vs. raw C ABI, in-process vs. out-of-process, how to expose `MoveAnalysis` across the boundary.

### Live-play tuning

Every retrospective narrator has unit tests for shape, but the wording and thresholds were picked *a priori*. Continued real-game playthrough is how they get tuned. The CLI `play` and the desktop GUI's retrospective panel are both now wired for this.

## Pointers to inline design briefs

- **Teaching analysis pipeline**: [`core/engine/src/analysis/mod.rs`](core/engine/src/analysis/mod.rs) `//!`
- **Trap library schema + four-gate validator**: [`core/engine/src/traps/mod.rs`](core/engine/src/traps/mod.rs) `//!`
- **Engine public API surface**: [`core/engine/src/engine.rs`](core/engine/src/engine.rs)
- **Search structure + pruning stack**: [`core/engine/src/search.rs`](core/engine/src/search.rs) `//!`
- **Move picker pipeline**: [`core/engine/src/movepick.rs`](core/engine/src/movepick.rs) `//!`
- **TT layout**: [`core/engine/src/tt.rs`](core/engine/src/tt.rs) `//!`
- **Repo layout, mission, ground rules**: [`CLAUDE.md`](CLAUDE.md)
