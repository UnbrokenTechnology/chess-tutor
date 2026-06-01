# Handoff: chess-tutor-2

State index for fresh contexts. **Read [`CLAUDE.md`](CLAUDE.md) first** for evergreen guidance (mission, legal/licensing, ground rules); this file and its split-outs are forward-looking only — git history covers what's been built, inline module docs (`//!`) cover design rationale.

## What this app is

A **chess tutor**, not a chess engine. The product surface is move-by-move teaching feedback for ~1200 ELO students climbing toward the 1600+ range. Strength is a means: 2000-ish ELO is enough to pose interesting positions; explainability is the actual product. Three pillars:

1. **The engine** — Stockfish-11 classical port (NNUE banned). 2000 ELO verified empirically. Search has most of the SF11 pruning stack; eval decomposes into 45 named sub-terms keyed by `TermId`, each with mg/eg components and a per-term tapered cp delta the teaching layer reads.
2. **The teaching layer** — [`core/engine/src/analysis/`](core/engine/src/analysis/) — see that module's `//!` for the design principles. Traces every UI claim back to a concrete engine signal: term deltas, structured outcome snapshots, surprise classification, verdict.
3. **The teaching crate** (`core/teaching/`, formerly `core/narration/`) — the single prose translator. Carries the language-free **Claim IR** (`claim::Claim`, one variant per teaching point, mover-relative, **never says "you"**) + the salience builders (`claims_for` / per-category `*_claims`) + the one **`phrase(&Claim, &PhrasingContext) -> Phrasing`** translator where perspective ("you" vs "they"), the chess.com reframe, verbosity, and i18n live. Both the GUI (`core/ui`) and CLI consume Claims and call `phrase`; the engine stays pure. Public CLI surface unchanged in shape: `format_retrospective(pre_move_pos, &[MoveAnalysis], user_move, &NarrationOptions, perspective) -> String` (now a pure claims + phrase join — no hardcoded-prose path left). **Rust owns all prose; mobile receives final strings over FFI, not the IR** (see CLAUDE.md "Prose ownership"). This reverses the old "each platform writes its own prose" guidance.

UIs: CLI (`chess-tutor`), egui desktop (`chess-tutor-desktop`), planned Apple + Android. FFI crate (`core/ffi/`) is the prerequisite for the platform apps and doesn't exist yet.

Tests: **891 engine (+4 ignored) + 150 teaching + 103 cli + 92 ui = 1236 passing**, clippy clean across all targets.

## Status: the engine detour is COMPLETE — teaching UX is the active work

The teaching UX (below) was parked behind a three-part engine detour (the coach could only reason from static eval, so it couldn't honestly say "you missed a tactic" / "you had a mate"). All of it has now landed:

- **SF11 parity** ✅ — engine is functionally faithful to Stockfish 11 (node gap d=14 1.48× / d=20 2.04× SF; two correctness bugs fixed + the SF11 pruning stack landed as balanced bundles). Current figures + deferred levers: [`HANDOFF-perf.md`](HANDOFF-perf.md); the file-by-file audit history is in git.
- **Non-functional refactor** ✅ — every source `.rs` ≤ 500 LOC bar documented exceptions (`pawns.rs` 687, one cohesive eval term; data tables); tests in sibling files.
- **lichess tactic-library port** ✅ — the full taxonomy worth porting is engine-available (waves 1–6). Git history carries the wave-by-wave detail; the durable summary is memory `project_tactic_library_reference`.

### Engine-available tactic surface (what the teaching UX consumes)

All in [`core/engine/src/analysis/`](core/engine/src/analysis/); [`tactic_outcome/mod.rs`](core/engine/src/analysis/tactic_outcome/mod.rs) `//!` carries provenance (hand-transliterated from lichess `cook.py` under the idea/expression dichotomy — see CLAUDE.md).

- **`compute_tactic_outcome(best_ma, user_ma, pre_pos, root_stm, prior_move) -> TacticsOutcome`** — three independent `Option<TacticHit>` slots: `user_played_tactic`, `user_missed_tactic` (gated by **win% gap OR absolute cp gap** so winning-position saturation doesn't suppress real misses), `user_walked_into`. Each slot has a paired **`user_*_escape: Option<TacticEscape>`** companion — the forcing refutation of that tactic (for `walked_into`, *the user's own* escape from the opponent's tactic). `prior_move: Option<PriorMove>` feeds the recapture guard (a real retrospective caller passes the opponent's actual prior move). Consumed by `core/ui/src/retrospective_view/tactic.rs` (which renders a per-slot escape sentence).
- **`find_tactic_in_line(pre, line, mover, prior_move) -> Option<TacticHit>`** — single-line variant for the coaching surface, no played/missed/walked-into framing. Consumed by `Session::coaching_tactic_hint()` (PV-reuse path) to mine the previous retrospective's PV for a pre-move tactic name.
- **`find_best_tactic_in_position(pos, mover, prior_move) -> Option<TacticHit>`** — static fork-shape scan over every legal move (the detector chain is purely predicate-based; no search needed). Consumed by `Session::coaching_tactic_hint()`'s **static-scan fallback** when PV-reuse can't fire (move 1 of a game, bot deviated). Pattern-severity tiebreaker (Fork beats TrappedPiece at equal material gain) mirrors the detector chain's priority order.
- **`find_tactic_escape(pos, hit, owner) -> Option<TacticEscape>`** ([`tactic_escape.rs`](core/engine/src/analysis/tactic_escape.rs)) — structural escape-hatch check: does the opponent have a forcing reply (`EscapeKind` = ForcingCheck / Zwischenzug / DefendsBothTargets / AdequateRetreat / CounterAttack) that prevents a detected tactic's *expected capture*? Analyses Pin / Fork / RemovingDefender / Skewer / DiscoveredAttack only; **no eval thresholds** — reuses `is_in_bad_spot`. `TacticEscape = { refutation: Move, kind: EscapeKind, expected_target }`. A real pin/fork with an escape is still a real tactic; this names the out without suppressing the hit. Needs `TacticHit.key_move`. Consumed by `compute_tactic_outcome` and the `tactics` / `search --annotate` CLI surfaces. Design rationale: [`tactic_escape.rs`](core/engine/src/analysis/tactic_escape.rs) `//!`.
- **`TacticHit`** = `{ pattern: TacticPattern, mate_pattern: Option<MatePattern>, sacrifice: bool, primary_piece, targets, material_gain, confidence, pv_ply, key_move: Option<Move> }`. `pattern` names the lesson; `mate_pattern` / `sacrifice` are orthogonal annotations that ride alongside (a fork-into-back-rank-mate is `Fork` + `Some(BackRank)`, exactly as lichess tags both). `key_move` is the move occupying `pv_ply` in the analysed line — stamped centrally in `detect_line_tactic`, feeds escape detection and lets callers name the move.
- **`TacticPattern`** (each has `heading()`): Fork, HangingCapture, RemovingDefender, TrappedPiece, Pin, Skewer, DiscoveredAttack, DiscoveredCheck, DoubleCheck, Sacrifice, Intermezzo, Deflection, Attraction, Interference, Clearance, XRay, AttackingF2F7, UnderPromotion, Checkmate.
- **`MatePattern`** (terminal-node mates; `heading()` + `surfaced_by_default()` → `true` only for BackRank/Smothered, the rest engine-available but a named-library for later): BackRank, Smothered, Anastasia, Hook, Arabian, Boden, DoubleBishop, Dovetail.
- **`find_overloaded(pos, victim) -> Vec<OverloadedPiece>`** ([`overloading.rs`](core/engine/src/analysis/overloading.rs)) — a *pre-move scan* (strict sole-defender-of-≥2), deliberately **not** in the tactic chain; its own analytical surface. Consumed by `coaching_view::overloaded_card`.
- **Trapped-piece overlay, engine side:** `OverlayData.{white,black}_trapped` + `{white,black}_trapped_cage` (cage-union bitboards) + `analysis::trapped_cages(pos, colour)` (per-piece dead-escape "cage" — kept for any future arrow surface). Consumed by `overlays_view` via `OverlayKind::TrappedPieces`.
- **`win_chances(Value) -> f64`** ([`win_chances.rs`](core/engine/src/analysis/win_chances.rs)) — lila cp→win% sigmoid (normalizes our PAWN_EG=213 → conventional pawn=100 first).

### First UI wiring pass landed (2026-05-27)

The five surfaces from the prior NEXT list shipped end-to-end:

- **TrappedPieces overlay** ✅ — `OverlayData.{white,black}_trapped_cage` (union of dead escape squares), `OverlayKind::TrappedPieces`, `AnnotationKind::TrappedEscape` (muted-red cage tint over `BadPiece` on the doomed piece). Toggle checkbox auto-renders via `OverlayKind::ALL`.
- **Retrospective Tactic card** ✅ — `RetrospectiveCategory::Tactic` + `core/ui/src/retrospective_view/tactic.rs` builder, fed by `compute_tactic_outcome`. `prior_move` threaded into `build_retrospective_view` via the new `Session::prior_move_for`. Played/missed/walked-into all surface; missed card suppresses spatial annotations when `reveal_best_moves` is off.
- **Mate cards** ✅ — `MatePattern::surfaced_by_default()` drives the back-rank / smothered heading suffix on Checkmate hits; non-default named mates are still detected, just don't pop a card.
- **Coaching tactic hint** (E, PV-reuse variant) ✅ — `Session::coaching_tactic_hint()` mines the previous user-move retrospective's `analyses[user_move].pv[2..]` and gates on `history[u+1].mv == pv[1]`. New public `find_tactic_in_line(pre, line, mover, prior)` does the detection. Confidence::High only, no annotations (pedagogical rule).
- **Overloaded coaching card** (D) ✅ — `find_overloaded(pos, !user_color)` consumed in `coaching_view::overloaded_card`. Defender → `BadPiece`, each duty → `Threat`, defender→duty arrow → `Defender`. Strict sole-defender-of-≥2 predicate keeps misfires low.

### NEXT: tuning + the surfaces not yet wired

Pick up from **[`HANDOFF-ux.md`](HANDOFF-ux.md)**. The product has three teaching surfaces below, all card-based and all reading the same engine outcomes:

1. **Retrospective panel** — after-the-fact analysis of the user's last move. Cards per signal (material, threats, king safety, mobility, pawn structure, passed pawns, piece placement, secondary, **forced consequences of opponent's best reply**). Best-move reveal is opt-in (`LearningPreferences.reveal_best_moves`, default off).
2. **Coaching panel** (live) — features-to-notice for the position the user is about to move from. Shown when `AssistanceLevel::Coached` is active. Surfaces hanging-piece opportunities (filtered through legal moves so in-check / pinned cases don't lie), en-passant captures, pawn weaknesses on either side, and a "your king is in check" card. Never names a move.
3. **Game Review** (post-game / on-demand) — ranked list of significant moments derived from the same classifier that drives in-game intervention. Click any moment to jump the rest of the UI there.

Plus the **intervention pause**: when `MistakeHandling::TeachingMoments` or blunder safety is on, the engine reply is held after a user move until the classifier decides whether to pause and surface a "you missed something / take back / continue" prompt. Gate is tight by design — single dominant eval-term family + share threshold + position-not-hopeless, so noise / engine subtlety don't interrupt play.

→ **[`HANDOFF-ux.md`](HANDOFF-ux.md)** — teaching layer state, learning-mode design, deferred Phase 2/4/5 work, narration tuning, UX platform tasks, live-play tuning loop. Read this when iterating on teaching UX.

→ **[`HANDOFF-perf.md`](HANDOFF-perf.md)** — current bench figures (d=14 1.48× / d=20 2.04× SF), single-thread determinism policy, deferred perf/strength levers, and the "SF pruning is a balanced set" lesson. Read only when returning to engine perf / strength work.

**Open thread for the teaching UX:** persistence design — game history on disk so past games are reviewable across launches and the foundation for drills / per-concept mastery fading. Desktop and mobile storage models differ (filesystem vs platform storage); user erase / clear-history UX needs design before code lands. See HANDOFF-ux's "Learning-mode polish" backlog (item 1, Persistence design).

## Build / dev commands

```bash
cargo test --release       # default; debug is 20–200× slower (magic search)
cargo build --release      # → target/release/chess-tutor[-desktop].exe
cargo clippy --all-targets

# Profiling build (release-equivalent + debuginfo for VTune):
cargo build --profile profiling --bin chess-tutor
# → target/profiling/chess-tutor.exe

# Bench (SF11-compatible — `<tt_mb> <threads> <depth> [fen_file] [limit_type]`):
./target/release/chess-tutor bench 16 1 13                              # 1 thread, shared TT
./target/release/chess-tutor bench 128 8 20 default depth --new-game-between-positions  # 8 threads, cold TT
./target/release/chess-tutor bench 16 1 13 path/to/fens.txt             # custom positions

# Play (CLI) and multi-thread retrospective:
./target/release/chess-tutor play --threads 4              # 4 thread engine moves; retrospective uses all cores
./target/release/chess-tutor play --deterministic          # retrospective single-thread (bit-reproducible narration)
```

## Heap allocation policy

Per-search or per-engine allocations are fine. **Per-node allocations are not** — use stack arrays or pool from a thread-local. The `MovePicker` buffer pool (thread-local `Vec<Box<MoveBufs>>`) is the canonical pattern; copy it for any new feature that needs per-call scratch.

## Pointers to inline design briefs

- **Teaching analysis pipeline**: [`core/engine/src/analysis/mod.rs`](core/engine/src/analysis/mod.rs) `//!`
- **Teaching translation layer (Claim IR + `phrase`)**: [`core/teaching/src/lib.rs`](core/teaching/src/lib.rs) `//!`, [`claim.rs`](core/teaching/src/claim.rs) (the language-free IR + salience builders), [`phrasing.rs`](core/teaching/src/phrasing.rs) (`PhrasingContext`, `Perspective`, the single `phrase` translator — home of "you"/"they" + the chess.com reframe)
- **Tactic library (engine)**: [`core/engine/src/analysis/tactic_outcome/mod.rs`](core/engine/src/analysis/tactic_outcome/mod.rs) `//!` (predicate provenance; types + `compute_tactic_outcome`), `detectors.rs` (the per-pattern chain), `mate.rs` (named mates); plus [`overloading.rs`](core/engine/src/analysis/overloading.rs) + [`win_chances.rs`](core/engine/src/analysis/win_chances.rs)
- **Trap library schema + four-gate validator**: [`core/engine/src/traps/mod.rs`](core/engine/src/traps/mod.rs) `//!`
- **Engine public API surface**: [`core/engine/src/engine.rs`](core/engine/src/engine.rs)
- **Search structure + pruning stack**: [`core/engine/src/search/`](core/engine/src/search/) (`mod.rs` `//!` + per-phase files: `negamax`, `pre_loop`, `move_loop`, `move_search`, `loop_helpers`, `qsearch`, `run`, `settled`, `state`)
- **Move picker pipeline**: [`core/engine/src/movepick.rs`](core/engine/src/movepick.rs) `//!`
- **TT layout**: [`core/engine/src/tt.rs`](core/engine/src/tt.rs) `//!`
- **Repo layout, mission, ground rules**: [`CLAUDE.md`](CLAUDE.md)
