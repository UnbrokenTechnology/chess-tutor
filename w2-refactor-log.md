# W2 Non-Functional Refactor — Working Log

Working log for [ROADMAP.md](ROADMAP.md) **Workflow 2**: split every `.rs`
source file to ≤500 LOC and move inline tests to sibling files, **with no
logic / perf / test-count change**. Mirrors the [parity-audit-log.md](parity-audit-log.md)
pattern. Delete when W2 is complete.

Branch: **main**. (W1 + the merged UX WIP + W2 all live on main now.)

---

## Status: IN PROGRESS — 13 commits landed, ~2 files + 2 checkpoint files remain (pawns done; retrospective_view + main.rs next)

**Done criteria (from ROADMAP):** every `.rs` *source* file ≤500 LOC (data
tables like `psqt` and one cohesive eval term `pawns` are documented
exceptions); tests in sibling files; **bench unchanged**; **test count
unchanged**.

**Test count (must stay):** 728 engine (+4 ignored) + 105 narration + 33 cli
+ 27 ui = **893**.

**Bench invariant (engine files only):** `./target/release/chess-tutor bench
16 1 14` must report **exactly 9,739,495 nodes** (NPS within ~2% noise, ~2.5
M/s). A pure relocation cannot change node count — any deviation means logic
moved, not just code. UI / CLI / desktop files have no perf surface; verify
by build + tests only.

---

## The recipe (per file, one commit each)

1. **Test extraction (no perf surface, can't change release binary):** move
   the inline `#[cfg(test)] mod tests { ... }` block to a flat sibling
   `<name>_tests.rs`, leaving `#[cfg(test)] #[path = "<name>_tests.rs"] mod
   tests;` in the source. The sibling holds the module *body* (starts with
   `use super::*;`), dedented one level. Tier A used a Python helper for this.
2. **Structural split:** create a directory module (`foo.rs` → `foo/mod.rs` +
   `foo/<part>.rs`), or split a flat file. `mod.rs` **glob-re-exports** the
   parts (`pub use part::*;`) so external `crate::path::X` references are
   unchanged. Byte-faithful slices (one-off Python scripts — see "tooling").
3. **`cargo fix --release -p <crate> --all-targets --allow-dirty`** to trim
   per-file unused imports. **Use `--all-targets`, NOT `--lib`** — `--lib`
   removed imports the test block still needed via `super::*` (the traps
   gotcha below).
4. **Verify:** engine → build bin + `bench 16 1 14` == 9,739,495 + engine
   tests; ui/cli/desktop → build + that crate's tests. Then commit.

## Conventions established (follow these for the rest)

- **Cross-module private access → `pub(super)`.** When a split moves a type/fn
  whose private fields/methods are touched by code now in a sibling module,
  widen to `pub(super)` (e.g. `Square.0` → `pub(crate)`; TT `TTEntry`
  load/store/zero/payload/gen_depth + `Cluster.entries` → `pub(super)`; trap
  scan/check fns re-exported). Same in-crate accessibility as the pre-split
  single module — **not a behaviour change**; note it in the commit.
- **Tests that reach private helpers** get declared from the module that
  *owns* those helpers, via `#[path]`. (traps: `logic.rs` declares
  `#[cfg(test)] #[path = "tests.rs"] mod tests;`, so `super::*` in the test
  reaches logic's privates.)
- **Glob re-export** all moved pub items from `mod.rs` to preserve public
  paths.
- **EOL is LF** in the working tree (verified by raw byte read; git
  autocrlf=true stores LF in blobs and would only convert to CRLF on a fresh
  checkout). The earlier "CRLF repo-wide" claim was a `grep $'\r'` quoting
  artifact. New files: write LF (the Write tool already does). Slice scripts
  auto-detect via `b"\r\n" in data`.

## Gotchas hit (don't repeat)

- `cargo fix --lib` dropped `use crate::position::Position` / `san` from
  `traps/mod.rs` that the **test** block used via `super::*` → test build
  broke. Fix: `--all-targets`, and/or declare tests from the owning submodule.
- A doc-comment-aware "anchor that backs up over `//`/`#[`/blank" is right for
  **section boundaries** but wrong for **test extraction** — it backed past
  `#[cfg(test)]` over the `// === Tests ===` separator and wrapped `mod tests
  {` into the sibling (unclosed delimiter). For test blocks, match the *exact*
  `#[cfg(test)]` + `mod tests {` lines, no backing up.

## Tooling

One-off split scripts were written to `C:\Users\steve\split_*.py` and
`extract_tests.py` (OUTSIDE the repo, so not committed; gone after restart —
recreate from the recipe as needed). They: detect CRLF, slice by anchor lines,
add import headers + `use super::*;`, bump `pub(super)`, dedent test bodies.
Each split is small enough to also do by hand.

---

## Landed (9 commits, all on main, tests green, engine bench node-neutral)

| Commit | File | Result |
|---|---|---|
| `3a3d155` | **Tier A** — 11 files | inline tests → `<name>_tests.rs` siblings: bitboard, attacks, magics, movegen, noise, psqt, san, make_move, fen, move_assessment, coaching_view. All ≤500. |
| `867ce7d` | `view.rs` (ui) | → `view/{mod,panels}.rs` (453/355). Pure data; glob re-export. |
| `371242f` | `learning_mode.rs` (ui) | → `learning_mode/{mod,terms}.rs` (325/328). Extracted `term_prompt_copy`. |
| `2e59fb0` | `side_panel.rs` (desktop) | → `side_panel/{mod,cards}.rs` (476/279). `category_glyph`/`sentiment_color` pub(super). |
| `88c1381` | `play.rs` (cli) | → `play/{mod,output,commands,parse}.rs` (413/376/306/128). |
| `a1820b5` | `types.rs` (engine) | → `types/{color,piece,square,direction,value,misc,moves,tests}.rs`. `Square.0`→pub(crate). Bench 9,739,495. |
| `e9b9357` | `traps/mod.rs` (engine) | → `traps/{mod,logic,tests}.rs` (360/445/...). damiano.rs (470) untouched. Bench 9,739,495. |
| `f40c050` | `tt.rs` (engine) | → `tt/{mod,storage,tests}.rs` (356/154/...). Entry/Cluster internals pub(super). Bench 9,739,495. |
| `e2a7649` | `analysis/threats_outcome.rs` (engine) | → `threats_outcome/{mod,types,lists,guaranteed,tests}.rs` (97/107/285/133/387). `list_pressured`→pub(super); `count_hanging` test helper moved into tests.rs. Bench 9,739,495; 728 engine tests. |
| `395ce82` | `eval/mod.rs` (engine, HOT PATH) | → `eval/{mod,core,scale,trace,tests}.rs` (296/216/40/281/267). trace types (EvalTrace/MaterialBreakdown/MobilityBreakdown) → trace.rs; evaluate_inner + piece_value_balance → core.rs; scale_factor → scale.rs. PHASE_MAX/SCALE_NORMAL→pub(super); evaluate_inner/scale_factor→pub(super). `crate::eval::X` paths preserved via glob re-export. Bench 9,739,495, NPS within noise (no `#[inline]` needed — same-crate inlining holds); 728 engine tests. |
| `7077c24` | `eval/pieces.rs` (engine, HOT PATH) | → `eval/pieces/{mod,tables,tests}.rs` (415/104/140). SF11 weight tables (MOBILITY_*, KING_ATTACK_WEIGHT, ROOK_ON_FILE, bonuses) → tables.rs as pub(super), imported via `use tables::*`. `crate::eval::pieces::{evaluate, PiecesBreakdown}` paths unchanged. Bench 9,739,495, NPS within noise (consts are compile-time, fully inlined); 728 engine tests. |
| _pending_ | `pawns.rs` (engine) | tests → `pawns_tests.rs` (335); source 679 LOC **kept whole** (documented >500 W2 exception — one cohesive eval term). Bench 9,739,495; 728 engine tests. |
| `b71517f` | `movepick.rs` (engine, HOT PATH) | → `movepick/{mod,history,picker,helpers,tests}.rs` (266/359/456/88/578). history tables (Butterfly/Continuation/Capture/CounterMove) → history.rs; MovePicker FSM impl → picker.rs; pick_best_index/partial_insertion_sort/mvv_lva/captured_piece_value/is_pseudo_legal → helpers.rs (pub(super)). mod.rs keeps buffer pool + Stage + MovePicker struct + split_bufs. Sibling/child modules reach mod.rs privates (ScoredMove, split_bufs, consts) by descendant access; `pub use history::*` re-export keeps `crate::movepick::X` paths. Byte-faithful slice via /tmp script. Bench 9,739,495, NPS within noise; 728 engine tests. **NB: working-tree EOL is LF, not CRLF (git autocrlf stores LF; the log's earlier CRLF claim was a `grep $'\r'` quoting artifact).** |

`CLAUDE.md` "Separation of concerns" bullet already updated (in `3a3d155`) to
document the `#[path]` sibling-test convention precisely.

## Remaining worklist (do in this order; checkpoint before the last two)

Seam plans below are from a prior structural analysis — trust them, no need to
re-derive.

1. **`retrospective_view.rs`** (ui, 2797 — THE BIG ONE). Plan: directory
   module, one file per `build_*_item`: `retrospective_view/{mod (orchestrator
   `build_retrospective_view` + re-exports), headline, material, threats,
   king_safety, mobility, pawn_structure, passed_pawns, space, pieces,
   secondary, helpers}.rs` + `_tests`. Generic formatters
   (piece_name/article/capitalize/join_with_and/format_score_pawns/
   format_delta_pawns) → `helpers.rs` as `pub(crate)`. Each builder ≤380.
   No perf surface — build + ui tests (27).
2. **`main.rs`** (cli, 832). Mostly Clap arg-def boilerplate. Extract the
   `Cli`/`Command`/`EngineColor` clap definitions to a sibling (e.g.
   `cli_args.rs`) to get `main.rs` ≤500, OR document as a boilerplate
   exception. (Was flagged but not yet in the task list.)

### CHECKPOINT WITH USER before these two (per user, 2026-05-26):

3. **`session.rs`** (ui, 2107, god-object). Plan: `session/{mod (struct +
   ctor + accessors), moves, game_flow, worker, event_dispatch, view_builders,
   game_state, learning}.rs`. Fields → `pub(crate)`. No inline tests. Risky —
   pause for review before starting.
4. **`search.rs`** (engine, 3539, HOTTEST PATH). User chose **decompose +
   bench-lock**: file-split (qsearch, pruning helpers, history helpers,
   settled_ply, aspiration/run, SearchContext/state to siblings) AND decompose
   the ~1373-LOC `negamax` into sub-functions to hit ≤500 — node count
   byte-identical, NPS within noise, `#[inline]` on extracted hot helpers.
   Highest risk; pause for review before starting.

## Key decisions (from the user this session)

- `pawns.rs` kept whole (documented >500 exception).
- `search.rs` negamax: decompose + bench-lock (not a file-split-only exception).
- Checkpoint with the user before `session.rs` and `search.rs`.
- Tier A batched as one commit (release-neutral); Tier B is one-file-per-commit.

## Note on pre-existing clippy state

The merged tree is **not** clippy-clean: ~16 engine warnings predate W2
(mostly `MSRV 1.80 vs stable-since-1.82` from the rust-toolchain pin, plus a
few real lints in pawns/movepick/threats_outcome/search). **Not W2's job** —
W2 only requires *introducing no new* warnings. (HANDOFF's "clippy clean"
claim is therefore aspirational; revisit as a separate cleanup.)
