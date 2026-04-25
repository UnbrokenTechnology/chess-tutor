# Handoff: Stockfish 11 Rust port — current state

A snapshot so a fresh context can pick up where we left off without reading the whole conversation. **Read [`CLAUDE.md`](CLAUDE.md) first** for mission + legal/licensing + evergreen guidance; this file is the moving target.

## TL;DR

- **Engine**: Classical Stockfish-11 evaluation fully ported (all terms, granular sub-term breakdown). Movegen perft-verified. Search has the full pruning stack (null-move, LMR, LMP, futility, SEE, check ext, mate-distance, aspiration, PVS). MultiPV works. Endgame specialists: KXK / KBNK / KPK (bitbase) / KNNK / KNNKP. Plays 2000 ELO (user-verified vs chess.com bots).
- **CLI** (`chess-tutor`): subcommands `board`, `moves`, `eval`, `search`, `opening`, `play`. Lenient SAN + UCI input. Move analysis building blocks exposed: `--multi-pv N`, settled-ply display, `--debug` per-ply trajectory.
- **Teaching-layer pieces landed**: Opening book (lookup + live banner in `play`); Trap library schema + Damiano refutation + CLI wiring + pending-trap state machine; Teaching-analysis pipeline Phase 0 (granular `EvalTrace` sub-terms, including the mobility split: `MobilityBreakdown` with knight/bishop/rook/queen sub-terms), Phase 1 chunks 1-2 (MultiPV, per-ply `EvalTrace` capture, 2-ply settled-ply detection), Phase 1 chunk 3 (trace-diff + `MoveAnalysis` + `analyze_position` + `chess-tutor search --analyze` + REPL `analyze`), Phase 1 chunk 4 (force_include + MoveVerdict classifier + shallow-vs-deep SurpriseKind + auto-retrospective in REPL), Phase 1 chunk 5 (`MaterialOutcome` structured data + PV capture-sequence renderer in retrospective; 75% cumulative-prefix secondary terms), Phase 3 Tier-1 hanging-piece detection (`ThreatsOutcome` + `HangingPiece` + CLI narrator with attacker annotations: "hanging knight on d2 (attacked by the e3 pawn)"), Phase 3 Tier-1 SEE-losing-exchange detection + narrator, Phase 3 Tier-1 Stockfish pressure-pattern parity (`PressuredPiece` + `PressureKind::{MinorOnMajor, RookOnQueen, SafePawnThreat}` + CLI narrator with kind-specific verbs: "harried" / "pressured" / "kicked"; CLI-side de-dup against hanging+SEE-losing lists), Phase 3 Tier-2 king-safety outcome (`KingSafetyOutcome` + pre/post `KingSafetySnapshot` capturing `king_sq` + `king_attackers_count` + `king_attacks_count` + pawn-shelter mg/eg + `phase`; CLI narrator with bidirectional teaching ("Your king is more exposed" / "Your king is safer"), flank-aware phrasing ("2 attackers on the kingside" / "queenside attackers down to 1" / fallback "king ring"), and endgame-phase shelter suppression below phase 32), Phase 3 Tier-3 pawn-structure + mobility outcomes (`PawnStructureOutcome` wrapping pre/post `PawnsBreakdown` per side; CLI narrator per-category phrasing for worsening/improving on both sides: "doubled a pawn", "isolated a pawn", "created a backward pawn", "exposed a weak pawn", "walked into a pawn lever", "broke pawn connections" + improved counterparts; `MobilityOutcome` wrapping pre/post `MobilityBreakdown`; CLI narrator picks the biggest-|delta| piece type per side with phrasings like "Your knight mobility dropped (+0.60 → +0.20)" and "You restricted the opponent's rook mobility (...)"), surprise-tag precision fix (suppress misleading "refutes it" on Good moves; tighten verdict→surprise pairing), brilliancy surfacing (`!` SAN annotation + "Well spotted" line on Best+LooksBadButGood; engine-preferred sharp moves flagged on the "Engine preferred" line), Phase 0 king + passed sub-term splits (`KingBreakdown { shelter, danger, pawnless_flank, flank_attacks }`, `PassedBreakdown { rank_bonus, king_proximity, free_advance, stopper_penalty }`; `king_danger` stays atomic; passed halving applies componentwise — bit-exactness drifts ≤1 cp per passer, within weight-tuning noise), and Phase 3 Tier-4 outcomes (`PassedPawnsOutcome` wrapping pre/post `PassedBreakdown` per side with 4-category per-passer-sub-term narration: "a passer pushed forward", "king race improved", "the promotion path cleared", "a passer reached an easier file" + worsened counterparts; `PiecesPositionalOutcome` wrapping pre/post `PiecesBreakdown` per side with 11-category narration: "a minor claimed an outpost", "a rook claimed the open file", "a bishop claimed the long diagonal", "a rook escaped its trap", etc., under "Your piece placement improved/weakened:" and "You weakened the opponent's piece placement:" subjects).
- **Tests**: **554 engine + 123 cli = 677 passing**, clippy clean, rustfmt clean.

```bash
cd core && cargo test --release       # ~0.2s; debug mode ~1.4s due to magic search
cd core && cargo clippy --all-targets

# For perf investigation — release-equivalent w/ debuginfo for VTune:
cd core && cargo build --profile profiling --bin chess-tutor
# Output: core/target/profiling/chess-tutor.exe
```

## Next session: tune retrospective phrasing against real-game output

Engine and search are in good shape after the 2026-04-24+ work below. The user's next priority is **revisiting the retrospective output** — playing real games, reading the prose, and filing specific phrasing/threshold tweaks. Most of the narrators have unit tests for shape but the wording was picked a priori; pressure-testing it against actual positions is what's left.

## What landed in the 2026-04-24 perf + correctness session

This session tackled the live-play hangs / perf / determinism issues that surfaced on top of the earlier "quick wins." All landed:

- **CLI determinism in analytical commands.** `Engine: Clone` (with manual `Clone` for the atomic-bearing `TranspositionTable` / `TTEntry` / `Cluster`). REPL `search`, `analyze`, and the auto-retrospective all now clone the play engine *before* running so they inherit warm TT state but don't mutate it. Repeated `search` / `analyze` calls in any order produce the same answer for the same position.
- **REPL `search` default → MultiPV=1.** Was 2; the user observed `search` recommending a different move than `--engine-color white` actually played, because they used different MultiPV. `search 1` now matches engine play; `search N` for `N > 1` shows alternatives (a known SF-quirk: different MultiPV values can produce different top moves at the same depth, owing to per-slot TT/history state).
- **Bounded repetition scan.** `Search::is_repetition` now scans only the last `pos.halfmove_clock()` entries of `path_keys`, not the full vector. Positions before the most recent pawn move / capture cannot physically repeat, so scanning them is wasted work — but more importantly, the unbounded scan was *creating* spurious repetition matches across stale `game_history` entries, which manifested as "everything draws" subtree explosions in late self-play games. Two test FENs needed adjustment to use realistic `halfmove_clock` values matching seeded path_keys.
- **Engine-turn node cap (5M nodes).** `play_engine_turn` sets `max_nodes: Some(5_000_000)` as a hard safety net. At ~4 M nodes/s, this bounds engine-move latency to ~1.3 s on pathological positions. Doesn't cap analytical commands — those don't run automatically and the user can wait for full depth.
- **Draw-value jitter.** `is_repetition` / 50-move-rule returns now produce `Value(±1 + contempt)` based on `nodes & 1` and asymmetric contempt-around-root. The jitter (depth ≥ 4) gives alpha-beta a tiebreak so subtree-of-draws positions don't tie at exactly 0 cp — that was the hang trigger. Below depth 4, returns flat `Value::DRAW` to avoid distorting qsearch.
- **Eval-level contempt = 2 cp.** `CONTEMPT_CP` lives in search.rs. Started at 20 cp (caused weird self-play asymmetric losses traced to TT pollution across root_stm flips — see math below); reduced to 2 cp as a "small enough to be noise" compromise that still gives a tiny "play on" preference. **Open question:** dropping to 0 may be cleaner; we tested 0/2/20 and 2 cp was an acceptable middle ground but not validated against a benchmark suite.
- **Movegen allocation refactor.** New `MoveList` type — stack-allocated `[Move; 256]` + `len`. Public movegen API: `generate_pseudo_legal_moves(pos, &mut MoveList)` and `generate_legal_moves(pos, &mut MoveList)`. Convenience wrappers `pseudo_legal_moves_vec(pos) -> Vec<Move>` and `legal_moves_vec(pos) -> Vec<Move>` for non-hot-path callers (CLI, tests, traps, endgame, san, search-tests). The hot path (movepick's `generate_captures` / `generate_quiets` / `generate_evasions` / `is_pseudo_legal`, plus search's root-move generation) writes to stack `MoveList`s. VTune trace previously showed ~21% of CPU in heap allocator on this pattern.
- **Diagnostic CLI flags on `play`** (keep these; they're useful for any future debugging):
  - `--show-fens`: print FEN before every turn.
  - `--reset-engine-per-move`: call `engine.new_game()` before every engine move (clears TT + history). Diagnostic only.
  - `--search-progress`: write iterative-deepening + root-move + aspiration-window + node-count progress to stderr during every engine search. Includes a "still alive" heartbeat every 500k nodes from `check_should_stop`.
  - `--explain-best`: from the earlier session — fall through to full per-term narration on Best verdicts instead of short-circuiting after the headline.
- **Move-number always printed.** `move N: white to move.` line, using `Position::fullmove_number()`.
- **Verbose progress output in search.** When `SearchParams::verbose_progress = true`, search prints depth start/finish, per-root-move start, aspiration-window changes (attempt + window + result FAIL-LOW/FAIL-HIGH/OK), and a node-counter heartbeat. All to stderr; doesn't affect normal play.

## Cargo profile: `[profile.profiling]`

Added in `core/Cargo.toml`. Inherits `release` opts but keeps `debug = true` and `strip = false` so VTune / Superluminal / WPA show Rust function names. Build: `cargo build --profile profiling --bin chess-tutor` → `core/target/profiling/chess-tutor.exe`. Use this binary for any perf investigation.

**Critical:** plain `cargo run` is the **debug** build (~20-200× slower than release). The user spent a session investigating "4 s startup, 2 s retrospective" only to discover those were debug-mode artifacts; release/profiling builds run startup in ~0.2 s and retrospective in ~10 ms. **Always use `--release` or `--profile profiling` for any performance-sensitive testing.**

## Suspect: contempt + cross-search TT

Eval-level contempt (any non-zero `CONTEMPT_CP`) bakes side-dependent bias into TT entries written during a search. When the next move's search has the opposite `root_stm`, those entries are read with a wrong sign, polluting cutoff decisions by up to `2 × CONTEMPT_CP`. The math is more subtle than a simple ply-parity correction — contempt contribution at internal nodes depends on tree-shape (leaf depth distribution), which we don't preserve in the TT entry.

Stockfish has the same issue in principle; they tune contempt against benchmark suites and the noise is dominated by other factors. We don't have that calibration loop. If self-play asymmetries reappear, the first thing to try is `CONTEMPT_CP = 0` and confirm whether the bias is the cause vs. routine 2000-ELO blunders.

## Live-play feedback 2026-04-24 — landed quick wins

These all landed in the prior session (still-relevant context for understanding the retrospective layout):

Phase 3 Tiers 1-4 are complete. Phase 0 sub-term splits cover every classical eval term that's splittable (pawns 6, pieces 11, mobility 4, threats 9, king 4, passed 4 = 38 granular sub-terms + 3 net + 1 aggregate Space). The teaching-analysis pipeline now has a `XxxOutcome` struct for every major term group except `Space` / `Imbalance` / `Initiative` (by design — see "don't split" section in the playbook).

### Live-play feedback 2026-04-24 — landed quick wins

Real-game session revealed these phrasing/UX issues; short fixes landed this session:

- **Retrospective cumulative threshold 75% → 50%** (`cli/src/retrospective/secondary_terms.rs::RETROSPECTIVE_TOP_PERCENT`). At 75% the fallback line was 7–9 terms per move, mostly noise; 50% typically lands on the 2–4 terms that drove the swing. The `search --analyze` default stays at 75 — that surface is explicitly opt-in to higher detail.
- **Sign-grouped `Helped` / `Hurt` lines instead of one mixed `Shifts:` line.** `render_secondary_terms` now takes `root_stm`, flips signs so positives = "helped the player," and partitions into two lines sorted by magnitude. When specialised narrators already fired, the headings switch to `Also helped` / `Also hurt`. Fixes a latent perspective bug where black-to-move's deltas rendered white-POV.
- **`TermId::pretty_label()` — plain-English student-facing labels** (`engine/src/analysis/term_id.rs`). The kebab-case `label()` stays for eval-report tables; `pretty_label()` replaces it in the retrospective's fallback line. Examples: `threats.slider-on-queen → "slider pressure on queen"`, `pawns.weak-unopposed → "weak pawns"`, `mobility.bishop → "bishop activity"`, `king.danger → "king safety"`. Un-narrated sub-terms now read naturally in the Helped / Hurt lines.
- **Mobility narration: threshold 30 → 50 cp, phrasing "mobility" → "activity"** (`cli/src/retrospective/mobility_narration.rs`). The 30-cp threshold fired on almost every opening move because any nudge to an enemy pawn shifts the mobility-area bitmap; 50 cp cuts noise without hiding real piece-reach changes. "Activity" reduces the surprise of hearing "bishop mobility improved" when the bishops haven't moved — Stockfish's mobility term measures weighted squares-attacked-in-safe-area, not legal-move count. The underlying concept still bleeds through on large shifts (e.g. a pawn move that opens the bishop's diagonal), but the word choice no longer promises legal-move gains.
- **`--explain-best` flag + REPL `explain-best [on|off]` toggle.** Default off: `Best` verdicts short-circuit after the congratulatory headline, as before. With the flag on, Best falls through to the same per-term narration non-Best verdicts get, so the student learns *why* their move was best — not just that it was. `BestAvailable` (position already lost) still short-circuits regardless; the deltas there are noise around a catastrophic baseline.

### Next session priorities

1. **Instrument profiling first.** Don't guess at startup / retrospective latency — measure. The two suspected culprits for the 4 s startup (magic tables, opening-book SAN replay) and the ~2 s per-move retrospective (full-depth MultiPV-3 with `force_include`) are currently unverified. Add a `--trace-startup` or Criterion-style bench before tuning. See "Suspected-but-unverified" below.
2. **After profiling, pick the highest-leverage item** from the medium-effort list.

### Medium-effort follow-ups still pending from the 2026-04-24 feedback

- **REPL `analyze <move>`** — currently `analyze` takes `[N] [P]`; extending to `analyze <move>` would let the student analyze a specific candidate before committing, without violating the design-brief rule against pre-commit every-move analysis (this is user-initiated per move, not automatic). Partial alignment with the existing `on-demand` analysis mode in the pipeline design brief.
- **Retrospective latency (~2 s per human move).** Runs a full-depth search with `multi_pv=3` + `force_include=[user_mv]` — same wall-time as the engine's own move. No fix on the roadmap; options to explore after profiling: cap retrospective depth below search depth, reuse the just-completed engine search's TT warmth, run async while the human picks their next move.
- **Per-piece-type mobility raw square counts** — HANDOFF's older "per-piece-type mobility counts" docket item. Surface the raw squares-attacked-in-safe-area count alongside the cp value: "knight activity improved (+0.60 → +0.80 — sees 7 squares, up from 5)". Requires tracking popcount alongside the Score per type. Would help demystify the "activity" term when it fires on moves where no piece of that type actually moved.

### Suspected-but-unverified (next session profiles these)

- **~4 s CLI startup.** Two plausible contributors:
  - Magic-bitboard search at `LazyLock` init (CLAUDE.md estimates "tens of ms" per process; may be wrong).
  - Opening-book init: `engine/src/openings.rs` replays Lichess TSVs through the SAN parser at first use to build a ~3,500-entry `HashMap<EPD, OpeningIdentification>` — plausible 4 s culprit.
  - Fix path: bake magics as `const` (CLAUDE.md "Deferred optimization"); pre-serialise the opening-book map and load via `bincode` or `rkyv`.
- **~2 s per-move retrospective pause.** The retrospective runs a full-depth search — same cost as the engine's own move — but the student has already waited for that. Two extra costs on top: the MultiPV=3 slot-search + trace capture on every ply. Whether the 2 s is dominated by the extra MultiPV passes or the `analyze_position` bookkeeping is unknown pre-profile.

### Still on the docket from earlier tuning rounds

**Priority for the next session** (after profiling): continue **playing real retrospective games** end-to-end. Every narrator has unit tests, but the phrasing was picked a priori — ongoing real-game use is how the thresholds + wording get fine-tuned. Expected workflow:

1. `chess-tutor play` against the engine (set a bot strength) for 10-20 moves.
2. Read every retrospective line. Is the verdict right? Does the term attribution point at the right chess concept? Does a beginner understand the phrasing, or is it engine jargon leaking through?
3. File specific wording changes and threshold re-tunes; update the per-subterm phrases in `cli/src/retrospective/*.rs` where needed.

### Immediate tuning candidates flagged during implementation

- **Passed-pawn phrasing uses chess jargon**: "king race", "stopper penalty" might still feel technical. Consider simpler phrasings like "a passer pushed forward" is fine; "the king couldn't catch the passer" is clearer than "king race worsened." Re-tune once real positions surface.
- **Piece-placement clauses are generic ("a minor", "a bishop")**: narrowing to specific pieces ("your knight", "the bishop on c4") needs per-piece attribution the current `PiecesBreakdown` doesn't carry. Defer — requires tracking which square triggered which sub-term, a bigger refactor.
- **Piece-placement per-side multi-line case**: the current shape emits at most 2 lines per retrospective (ours + theirs). If both outposts and rooks and bishops all move in one PV, all three show up in one comma-joined clause. That reads OK but could be split into one line per sub-term when there are ≥3 clauses. Ship-and-see.
- **PawnStructureOutcome vs PassedPawnsOutcome ordering**: both can fire on the same move (pawn push makes the pawn passed + affects structure). Current narrator runs them sequentially; no de-dup. Verify with real games whether this reads as redundant or complementary.
- **Threshold `PASSED_DELTA_THRESHOLD_CP = 25`**: may be too high for the king-proximity sub-term, which shifts smoothly as pieces move. Lower threshold here is fine if it catches relevant king-race moments.

### Optional next-level work (judgement calls, deferred for now)

- **`KingDangerOutcome` — punted as redundant**: HANDOFF previously flagged this as optional "once the king split lands." Decision: **skip**. The existing `KingSafetyOutcome` already surfaces the raw scalars (attackers_count, attacks_count, shelter_mg/eg) that drive the Tier-2 narration — those are direct human-scale quantities. A parallel `KingDangerOutcome` wrapping pre/post `KingBreakdown` would only duplicate the teaching story at a more abstract Score level. Reconsider if real output shows cases where the breakdown surfaces teaching moments the scalars miss (e.g., `danger` sub-term shifts while attackers_count stays constant — possible via flank pressure or pinned-defender changes).
- **Per-passer `Vec<PasserDetail>`**: the narrow 4-sub-term `PassedBreakdown` landed; the wider per-passer shape (naming specific passers by square and rank) would unlock phrasing like *"Your a-pawn on rank 6 now queens in 3 moves."* But `Vec` in `EvalTrace` breaks `Copy`, which ripples through search / TT / history. If the 4-sub-term narration feels flat in practice, plumb a parallel per-passer struct on `PassedPawnsOutcome` (outcomes don't need `Copy`) rather than on `EvalTrace`. That's the cleanest compromise.
- **Threats outcome narrowing**: now that `ThreatsBreakdown` exposes 9 sub-terms, the retrospective's `render_threats` could consume only the specific TermIds its detectors cover (`ThreatsHanging` for hanging-piece detection, `ThreatsByMinor` / `ThreatsByRook` for pressure patterns, etc.) leaving the other threat sub-terms visible in the generic Shifts line. Current behaviour: fires the outcome → consumes *all 9*. Narrower consumption would surface knight-on-queen / slider-on-queen / restricted shifts the bespoke detectors don't narrate.

### Also on the docket

- **Tune `MoveVerdict` thresholds** — `Good` firing at 47 cp in the opening still feels too harsh after earlier testing. Revisit `BEST_LOSS_MAX` / `GOOD_LOSS_MAX` in `engine/src/analysis.rs`.
- **`MaterialOutcome` PSQ annotation** — currently captures drive material narration but PSQ shifts (e.g., knight moves to a better square, gaining ~15 cp of positional material) slip silently into `Shifts: material -1.24`. Could label those as "positional PSQ shift" when no captures are present.
- **Pawn-structure file-level detail** — deferred Tier-3 polish. Current narration says "doubled a pawn" without naming the file; a re-walk of `pawns.rs` classification with per-pawn data would let us say "doubled pawns on the c-file."
- **Per-piece-type mobility counts** — current mobility narration shows cp (e.g., "+0.60 → +0.20"). Could also surface raw square counts ("knight sees 5 squares, down from 8") — requires tracking popcount alongside the mobility Score per type.

### Deferred (will remain deferred a while)

- **Cheap-pass evaluator** (Phase 5): depth-1 qsearch + SEE over every legal root move. Enables surprise tagging on moves below the MultiPV horizon — where "bad moves that look tempting to a 400-1000 player" live.
- **Tier 5 / 6 narration**: Space, Imbalance, Initiative. Each is a single net term (Space has a per-colour pair), so an outcome would consume a single `TermId` and probably only fire in endgame-transition positions. Low priority — ship Tier 1-4 phrasing polish first.
- **Phase 4**: signal-mask (zero each term, re-rank, record `MaskedHint`).
- **Phase 5**: cheap-pass + tactics library (absolute pin, relative pin, fork, skewer, double attack).
- **Phase 6**: platform-specific renderers (Swift, Kotlin, egui).

---

## Phase 0 sub-term split playbook

*(Canonical recipe. Re-read before starting any new sub-term split.)*

A step-by-step checklist for decomposing a monolithic eval term into a `XxxBreakdown` with per-sub-term `Score` fields. This pattern was applied to pawns, pieces, mobility, threats, king, and passed — **every splittable term now has a breakdown**. This section remains as canonical reference in case a new term is added or a breakdown needs to be widened (e.g., per-passer detail on passed). Skip space / initiative / imbalance / material — see "don't split" at bottom.

### The pattern in one sentence

The term's `evaluate()` function produces a `XxxBreakdown` struct (named `Score` fields per sub-term) instead of a single `Score`; `.total()` recovers the aggregate; each sub-term becomes a `TermId` variant; the CLI `eval_report` renders an aggregate row + indented sub-rows; the retrospective consumes all sub-term TermIds when its outcome fires.

### Engine checklist

1. **Define the breakdown** in the term's home file (e.g., `eval/king.rs`, `eval/passed.rs`):
   - Struct with named `Score` fields, one per sub-term.
   - `#[derive(Clone, Copy, Debug, PartialEq, Eq)]`.
   - `pub const fn zero()` — an all-zero breakdown.
   - `pub fn total(&self) -> Score` — sum of every field. Used by the main evaluator to recover the aggregate.
   - For terms that accumulate while iterating piece types, add `pub(crate) fn add_for(&mut self, pt: PieceType, bonus: Score)` — match on the piece type, silent no-op for unused slots. (Pattern from `MobilityBreakdown`.)

2. **Refactor `evaluate()`**:
   - Change return type from `Score` → `XxxBreakdown`.
   - Replace `score += ...` accumulations with `breakdown.<field> += ...` per sub-term.
   - The Stockfish numerical weights stay intact; only the aggregation shape changes.

3. **Re-export** from `eval/mod.rs`: `pub use xxx::XxxBreakdown;` (alongside the existing `PawnsBreakdown` / `PiecesBreakdown` / `MobilityBreakdown` / `ThreatsBreakdown` re-exports).

4. **Update `Evaluator`** — ONLY if the term's scratch state lives there (applies to mobility; does not apply to pawns/pieces/king/threats, whose `evaluate()` is read-only on `Evaluator` and returns the breakdown directly):
   - Field type `[Score; 2]` → `[XxxBreakdown; 2]`.
   - `new()` default uses `[XxxBreakdown::zero(); 2]`.

5. **Update `EvalTrace`**:
   - Field type `[Score; 2]` → `[XxxBreakdown; 2]`.
   - `zero()` default uses `[XxxBreakdown::zero(); 2]`.
   - Add `pub fn xxx_total(&self, color: Color) -> Score` — `self.xxx[color.index()].total()`.

6. **Update `evaluate_inner`** in `eval/mod.rs`:
   - Where the term is summed into the running `score`, call `.total()`: e.g., `score += white_threats.total() - black_threats.total();`.
   - The `t.xxx = [white_xxx, black_xxx]` assignment in the trace-build block already works because both sides are now `XxxBreakdown`.

7. **Update `TermId`** in `analysis.rs`:
   - Remove the monolithic variant (e.g., `TermId::Threats`).
   - Add one variant per sub-term, named `XxxSubtermName` (e.g., `ThreatsHanging`, `KingShelter`). Keep consistent with existing naming — `Pawns*` / `Pieces*` / `Mobility*` / `Threats*`.
   - Grow the `const ALL: [TermId; N]` array: update `N` by hand, add the new entries in the declaration order.
   - Add `label()` arms using kebab-case `"xxx.sub-term"` (e.g., `"threats.by-safe-pawn"`).
   - Add `net_score()` arms — per sub-term, return `t.xxx[0].<field> - t.xxx[1].<field>`.

### CLI checklist

8. **`cli/src/eval_report.rs`**:
   - Import `XxxBreakdown` from `chess_tutor_engine::eval`.
   - Remove the term from the `aggregates: &[(&str, [Score; 2])]` slice.
   - Add a block that prints the aggregate row followed by indented sub-rows — mirror the `pawns` / `pieces` / `mobility` / `threats` blocks near the top of `render()`.
   - Add a `xxx_sub_rows(w, b) -> impl Iterator<Item = (&'static str, Score, Score)>` helper alongside `pawns_sub_rows` etc.

9. **`cli/src/retrospective.rs`** — if any `consumed_terms.push(TermId::Xxx)` existed for the monolithic variant, swap to `consumed_terms.extend_from_slice(&[TermId::XxxSubA, TermId::XxxSubB, ...]);`. Pattern from the `render_pawn_structure` / `render_threats` / `render_mobility` wiring.

### Test checklist

10. **In the term's `eval/xxx.rs::tests`** — any helper that returned `Score` now returns `XxxBreakdown`. Existing assertions on `.mg().0` become either `.total().mg().0` (aggregate) or `.<field>.mg().0` (specific sub-term — prefer this when the test is *about* a specific pattern, e.g., "hanging rook should fire the `hanging` sub-term"). Add `breakdown_total_equals_sum_of_subterms`.

11. **In `analysis.rs::tests`** — audit `TermId::Xxx` references:
    - `compute_term_deltas_returns_all_terms_and_is_sorted`: if it sets a value on `trace.xxx[0]`, specify the specific sub-term field; update the expected `deltas[N].term` to the new variant.
    - `cumulative_prefix_*` tests use `TermId::Xxx` as a generic term identifier — swap to any sub-term variant (e.g., `TermId::MobilityKnight`, `TermId::ThreatsHanging`).

12. **In `cli/src/eval_report.rs::tests`** — add `renders_xxx_sub_terms` spot-checking a few sub-row labels (mirrors `renders_pawns_and_pieces_sub_terms` / `renders_mobility_sub_terms_by_piece_type` / `renders_threats_sub_terms`).

### Gotchas learned the hard way

- **Unused-import warning**: if `XxxBreakdown` is imported at the top of `analysis.rs` only so tests can build literals, the non-test code may never name the type and clippy complains. Move the import *inside* `#[cfg(test)] mod tests { use chess_tutor_engine::eval::XxxBreakdown; }`. (Pattern applied to `KingSafetySnapshot` and almost caught out with `ThreatsBreakdown`.)
- **Non-`Score` fields break symmetry tests**: adding `king_sq: Square` to `KingSafetySnapshot` broke `snapshot_king_safety_startpos_has_zero_attackers_and_is_symmetric`'s `assert_eq!(w, b)` — white king e1 ≠ black king e8. Switch to field-by-field equality.
- **The `ALL` array length**: `const ALL: [TermId; N]` is compile-time sized. Update `N` by hand or the compile fails cryptically.
- **Snapshot helpers** (`snapshot_king_safety`, `snapshot_mobility` in `analysis.rs`): terms that need an `Evaluator`-primed snapshot follow this exact priming sequence — `Evaluator::new(pos)` + `initialize(White)` + `initialize(Black)` + `pieces::evaluate(&mut e, White)` + `pieces::evaluate(&mut e, Black)` — then read `e.xxx[color]`. Match this when building `XxxOutcome` snapshots for a new term.
- **Weights are facts**: when refactoring `evaluate()`, don't rewrite the numerical weights. Keep the `const` tables untouched; only change how the per-sub-term sums are accumulated.

### Files touched per split

- `engine/src/eval/<term>.rs` — breakdown struct, refactored `evaluate()`, `tests` module updates.
- `engine/src/eval/mod.rs` — re-export, `Evaluator` / `EvalTrace` field type (if applicable), `xxx_total()` helper, `evaluate_inner` consumer.
- `engine/src/analysis.rs` — `TermId` split, `ALL`, `label`, `net_score`, test updates.
- `cli/src/eval_report.rs` — import, aggregate+sub-rows render, `xxx_sub_rows` helper, test.
- `cli/src/retrospective.rs` — `consumed_terms` wiring (only if there's an existing consumer).

### Remaining split candidates

None. All splittable terms are split. **Widening** candidates remain (per-passer detail on passed, per-piece detail on pieces, per-pawn detail on pawn structure) — these are richer refactors that would pedagogically help ("Your c-pawn is doubled" vs. "doubled a pawn") but require data plumbing beyond the basic sub-term split. See the "Next session" section above for the current thinking on each.

### Don't split

- **Space** (`eval/space.rs`) — single counted value × piece-count scaling. No internal structure worth surfacing.
- **Initiative** (`eval/initiative.rs`) — single complexity correction, already net (no colour split).
- **Imbalance** (`material.rs`) — table lookup, monolithic.
- **Material** (PSQ sum maintained incrementally in `Position`) — not an eval-pass term; not a split candidate.

---

## Teaching Analysis Pipeline design brief

*(Canonical spec. Re-read before starting any teaching-pipeline chunk.)*

The goal: for a given position, produce a rich per-move analysis that goes far beyond "here's the best move + PV". The student should see **why** each candidate is good or bad, which signals contributed most, where the position "settles" along the PV, what tactical content the move creates / allows / resolves, and where the full evaluator disagrees with a shallow one (the "looks good but isn't" / "looks bad but is" classes). Traps stay separate — they're memorization, not skill.

Explicitly **not** chess.com's guess-and-narrate style. Everything the UI says must trace back to concrete engine data, not pattern-matched templates against aggregate scores.

### Core data model

Stockfish-internal cp everywhere inside the engine; UI layer converts to qualitative language at render time.

```rust
pub struct MoveAnalysis {
    pub mv: Move,
    pub score: Value,                     // side-to-move pov at configured depth
    pub pv: Vec<Move>,
    pub ply_traces: Vec<EvalTrace>,       // one per ply — for settled-ply detection
    pub settled_ply: Option<usize>,       // index into ply_traces of the "aha" moment
    pub pre_move_trace: EvalTrace,        // at the root position
    pub term_deltas: Vec<TermDelta>,      // sorted by |delta| desc
    pub cheap_score: Option<Value>,       // Phase 2: depth-1 qsearch + SEE
    pub surprise: Option<SurpriseKind>,   // Phase 2: LooksGoodButBad / LooksBadButGood
    pub masked_rankings: Vec<MaskedHint>, // Phase 4: "if you zero term X, this move ranks #1"
    pub tactics: Vec<TacticHit>,          // Phase 5: created / allowed / resolved
    pub verdict: MoveVerdict,             // Phase 6: Best / Good / Dubious / Mistake / Blunder / Surprise
}

pub struct TermDelta {
    pub term: TermId,                     // granular post-Phase-0 term id
    pub delta_mg: i32,                    // post.mg − pre.mg
    pub delta_eg: i32,
    pub delta_tapered: i32,               // same phase taper the main evaluator applies
    pub piece_involved: Option<Piece>,    // None for aggregate terms (defer)
}

pub struct TacticHit {
    pub kind: TacticKind,
    pub interaction: TacticInteraction,   // Created / Allowed / Resolved
    pub pieces: Vec<(Square, Piece)>,
    pub label: &'static str,
}
```

### Settled-ply detection (landed)

See the settled-ply section in "What's already landed" below. The heuristic: compare same-side-to-move plies (2 apart) to filter the side-to-move sawtooth; `settled_ply` = largest index with a ≥ 25 cp white-POV shift. Use `ply_traces[settled_ply]` as the trace to diff against `pre_move_trace`.

### Cumulative-threshold term selection

Don't use "top N terms" for narration — "show the smallest prefix that accounts for ≥ X% of total |delta|" is right. A one-term blunder produces a one-term list; a subtle positional combo produces 4–5. Start with X = 75%; tune as real output lands.

### Cheap-pass + surprise detection (Phase 2)

Depth-1 qsearch + SEE for every legal move. Compare the cheap ranking against the full-depth MultiPV ranking:
- **LooksGoodButBad** — cheap says top-k, deep says bad. "Think about the opponent's reply."
- **LooksBadButGood** — cheap says bad, deep says top-k. Sacrifices, deflection, positional tempo.

Workaround available today: `--multi-pv = legal_count` gives a real deep score for every root move, enabling shallow-vs-deep delta comparison without the cheap-pass machinery. Phase 2 is a latency optimisation, not a correctness prerequisite.

### Signal-mask (Phase 4)

Zero each `EvalTrace` term in turn and re-rank. If zeroing term X changes the top move from M to M', record `MaskedHint { masked_term: X, would_prefer: M' }` on M. "You'd prefer M' if you undervalued X — but X is what makes M the best here."

### Tactic library (Phase 5)

Parallel to `engine/src/traps/` but for general patterns, not named refutations. `engine/src/tactics/`. Each tactic is a detector `fn (pos, mv) -> Option<TacticHit>`. Classifications: Created / Allowed / Resolved. First-pass catalogue: absolute pin, relative pin, fork, skewer, double attack. Deferred: discovered attack, deflection, overloading, x-ray, interference. Run on demand only — prior repo ran tactics inside search and killed perf.

### Phase-aware narration (Phase 3)

Each `TermId` gets a template that takes `(delta_mg, delta_eg, phase, piece_involved, position_context)` and produces a qualitative sentence. Phase is explicit because mobility-in-MG reads differently from mobility-in-EG. Templates live UI-side (CLI to start; platform apps will have their own). Tier-ordered rollout:

| Tier | ELO range | Terms |
|---|---|---|
| 1 | 800–1200 | Material, Threats (hanging pieces, undefended attacks) |
| 2 | 1000–1400 | King safety (pawn shelter, flank attacks) |
| 3 | 1200–1500 | Pawn structure sub-terms, Mobility |
| 4 | 1400–1700 | Piece positional sub-terms, Passed pawns |
| 5 | 1600+ | Space |
| 6 | 1800+ | Imbalance, Initiative |

### Analysis triggers

Exactly two entry points:
- **On demand** — user asks for a hint / analyzes a position. No latency budget; full search depth available.
- **Retrospective** — after the most-recently-played move. "Was that move good? Alternatives? Did you miss a tactic?" Analyzes the pre-move position, treats the played move as one candidate.

Every-move pre-commit analysis ("would this be a blunder?") is explicitly NOT a mode. Too expensive, too hand-holdy.

### Open questions (revisit when real output lands)

- **Settled-ply threshold**: currently 25 cp. Real positions may want higher or a different metric (largest single jump? variance-based?).
- **Piece attribution**: Material/PSQ are trivial. Threats/King Safety/Mobility aggregate over many pieces; may need scratch state or pattern-matching at template time.
- **Template format**: single sentence vs. paragraph? Probably configurable per-term. Material deltas are one-liners ("you won a knight"); king-safety may warrant two sentences.

### What's explicitly NOT part of this pipeline

- **Traps.** Memorisation of named patterns. Lives in `engine/src/traps/`.
- **Opening identification.** Already in `engine/src/openings.rs`. May be referenced as *context* for a template, doesn't drive classification.
- **Game-level post-game commentary** (blunder summary, ELO estimate, etc.). Separate product surface.

---

## What's already landed (reference for next session)

### Phase 0: EvalTrace granular refactor

- `pawns.rs` aggregate decomposed into **`PawnsBreakdown`** (6 sub-terms: connected, isolated, backward, doubled, weak_unopposed, weak_lever). Passed-pawn still separately in `eval/passed.rs`.
- `eval/pieces.rs` aggregate decomposed into **`PiecesBreakdown`** (11 sub-terms: outposts, reachable_outposts, minor_behind_pawn, king_protector, bishop_pawns, long_diagonal_bishop, rook_on_queen_file, rook_on_open_file, rook_on_semiopen_file, trapped_rook, weak_queen).
- `eval/mod.rs` mobility aggregate decomposed into **`MobilityBreakdown`** (4 sub-terms: knight, bishop, rook, queen). Added alongside the pawn/piece split as a third granular Phase-0-style refactor. `Evaluator.mobility: [MobilityBreakdown; 2]` and `EvalTrace.mobility: [MobilityBreakdown; 2]`. `pieces.rs::evaluate_piece_type` accumulates via `e.mobility[us_idx].add_for(pt, mobility_bonus(pt, mob))`. `king.rs` and the main evaluator call `.total()` to recover the pre-split aggregate. `TermId::Mobility` split into `MobilityKnight` / `MobilityBishop` / `MobilityRook` / `MobilityQueen`. CLI `eval_report` prints `mobility` aggregate + indented knight/bishop/rook/queen sub-rows mirroring pawn/piece rendering.
- `eval/threats.rs` aggregate decomposed into **`ThreatsBreakdown`** (9 sub-terms: by_minor, by_rook, by_king, hanging, restricted, by_safe_pawn, by_pawn_push, knight_on_queen, slider_on_queen). `threats::evaluate` now returns `ThreatsBreakdown` instead of `Score`. `EvalTrace.threats: [ThreatsBreakdown; 2]`. Main evaluator calls `.total()` on both sides' breakdowns when summing into the running score. `TermId::Threats` split into 9 variants `ThreatsByMinor` / `ThreatsByRook` / `ThreatsByKing` / `ThreatsHanging` / `ThreatsRestricted` / `ThreatsBySafePawn` / `ThreatsByPawnPush` / `ThreatsKnightOnQueen` / `ThreatsSliderOnQueen`. `ALL` grew 28 → 36 across the mobility and threats splits. CLI `eval_report` prints `threats` aggregate + indented sub-rows. Retrospective's `render_threats` consumes all 9 `TermId::Threats*` variants on fire (was the single `TermId::Threats`).
- All four breakdown structs re-exported from `chess_tutor_engine::eval`. Each has `.zero()` const fn + `.total()` → `Score`. `EvalTrace` has `.pawns_total(color)` / `.pieces_total(color)` / `.mobility_total(color)` / `.threats_total(color)` for backwards-compat aggregate access.
- `PawnsEval.scores[c]` is a cached aggregate; equals `PawnsEval.breakdowns[c].total()` by construction.
- `eval/pieces.rs::evaluate` returns `PiecesBreakdown`; helpers take `&mut PiecesBreakdown` and attribute weights per-field.

### Phase 1 chunk 1: MultiPV

- `SearchParams.multi_pv > 1` now runs Stockfish-style per-PV-slot search. `Vec<RootMove>` + `pv_idx` cursor; iterative deepening walks slots 0..multi_pv restricting root moves per slot.
- Stable-sort `root_moves[pv_idx..]` after each slot. Post-IDS final sort on `[0..multi_pv]` smooths cross-slot TT volatility so output is strict descending.
- **TT save at root gated to `pv_idx == 0`** so secondary-slot searches don't clobber the best-move entry.
- `Engine::search` returns `Vec<SearchLine>` directly (empty = terminal position).
- **Known quirk (not fixing)**: different `--multi-pv` values can produce slightly different top-line scores for the same top move — each slot's search modifies TT + butterfly history, so state at later depths depends on multi_pv. Internally consistent at a given value. Stockfish has the same behaviour.

### Phase 1 chunk 2: per-ply traces + settled-ply

- `SearchLine.trace: EvalTrace` → **`SearchLine.ply_traces: Vec<EvalTrace>`**, one per PV ply. `trace_along_pv` walks via do/undo and calls `evaluate_with_trace` at each ply. Leaf = `ply_traces.last()`.
- **`SearchLine.settled_ply: Option<usize>`** computed by `compute_settled_ply(&ply_traces, root_stm)`. Uses **2-ply comparison** (same-side-to-move plies) because the 1-ply sawtooth is routinely 100–300 cp in quiet positions — a 1-ply threshold never fires in real search output.
- **`EvalTrace::white_pov_value(stm_at_eval: Color) -> Value`** — tempo-free white-POV. Subtracts tempo, flips sign on black plies. Use this for any cross-ply comparison.
- **`search::stm_after_ply(root_stm, ply) -> Color`** — the alternation helper.
- `SETTLED_THRESHOLD_CP` (public const in `search`): 25 engine-cp ≈ 1/10 pawn. Tuneable; may want larger for noisier positions.

### Phase 1 chunk 2 CLI

- Single-PV `chess-tutor search` prints `settled: ply N of M (SAN)` under `pv:`.
- MultiPV rows get `[settles ply N]` / `[settles leaf]` per-row suffix, both one-shot and REPL.
- **`chess-tutor search --debug`** dumps per-PV ply-by-ply trajectory: `pre` baseline row, each ply with SAN + white-POV score + Δ, `*` on settled index. Essential for tuning thresholds and explaining "why did the eval jump here?".
- REPL `search` defaults to 2 PVs; `search N` takes an explicit count. Scores render as pawns (`+0.42`) / mate notation (`#5`, `-#3`).

### Phase 1 chunk 3: trace-diff + `MoveAnalysis`

- **`engine/src/analysis.rs`** — `TermId` enum (25 variants: 3 single-valued + 5 per-colour scalar + 6 pawn sub-terms + 11 piece sub-terms), `TermDelta` (`delta_mg`, `delta_eg`, `delta_tapered`, `piece_involved: Option<Piece>`), `compute_term_deltas(pre, post, phase, sf)` sorts by `|delta_tapered|` desc, `cumulative_prefix(deltas, percent)` selector.
- **`MoveAnalysis`** struct holds `mv`, `score`, `depth`, `pv`, `ply_traces`, `settled_ply`, `pre_move_trace`, `term_deltas`. Phase-2+ fields (`cheap_score`, `surprise`, `masked_rankings`, `tactics`, `verdict`) deferred. `.diff_trace()` returns the trace used for diffing (settled-ply / leaf / baseline fallback).
- **`analyze_position(engine, pos, params) -> Vec<MoveAnalysis>`** — computes `pre_move_trace` once via `evaluate_with_trace`, runs `engine.search`, wraps each line. Caller controls breadth via `params.multi_pv`.
- **Tapered-cp formula** uses the post-move trace's `phase` and `scale_factor` — same formula the main evaluator applies: `(mg * phase + eg * (128 - phase) * sf / 64) / 128`. Engine-internal cp (PawnEG = 213); UI-side conversion to pawns (`/100`) lives in the CLI renderer.
- **`piece_involved`** intentionally always `None` for this phase — aggregate terms would need `Evaluator` scratch state to attribute correctly. Phase 3 narration templates may revisit.

### Phase 1 chunk 3 CLI

- **`chess-tutor search [FEN] --analyze [--top-percent 75]`** — same search as non-analyze path, but output surfaces the teaching pipeline. Per-move rows show score, SAN, settled-ply, then an indented table of cumulative-prefix term deltas (smallest prefix whose `|delta|` sums to ≥ top-percent of the total) with labels like `pawns.connected`, `pieces.bishop-pawns`. Ends with `... N more terms cover the remaining X%` when the prefix doesn't cover everything.
- **`--analyze --debug`** combines with the per-ply trajectory dump — useful for tuning the settled-ply threshold against real output.
- **`cli/src/analysis_report.rs`** renders `&[MoveAnalysis]` given the root `Position` (for SAN).
- **REPL `analyze [N] [P]`** (default N=3, P=75) — same pipeline, inside the `play` loop. Sits alongside `search [N]`.

### Phase 1 chunk 4: force_include + verdict + surprise + auto-retrospective

- **`SearchParams::force_include: Vec<Move>`** — guarantees these moves appear in the returned `SearchLine` list even when they fall outside the natural MultiPV top-k. Each forced move that isn't already in top-k gets its own dedicated single-move IDS pass via `Search::run_forced_slots`. Illegal moves silently dropped; dupes deduped. Output is re-sorted by score after the forced pass so "best first" ordering holds across natural + forced moves. ~100 LOC in `search.rs`, 9 dedicated tests.
- **`MoveVerdict::{Best, Good, Inaccuracy, Mistake, Blunder, BestAvailable}`** + `classify_move(user_score, best_score)` + `MoveAnalysis::classify(best_score)`. Thresholds in engine-cp: Best ≤ 15, Good ≤ 50, Inaccuracy ≤ 120, Mistake ≤ 350, Blunder > 350. `BestAvailable` fires when best itself is ≤ -500 cp *and* user is within the Best band — avoids congratulating the student in already-lost positions. Thresholds tuned by feel; revisit with real output.
- **`SurpriseKind::{LooksGoodButBad, LooksBadButGood}`** + `detect_surprise(ma, root_stm)` + `MoveAnalysis::surprise(root_stm)`. Compares `ply_traces[0]` (shallow static eval, white-POV tempo-free) against `score` (deep). Triggers when the delta exceeds ±150 cp. **Scope limit**: only fires on moves in MultiPV top-k (or `force_include`d), since moves below rank N have no valid `ply_traces`. Moves below top-k need cheap-pass (Phase 5).
- **Auto-retrospective in `chess-tutor play`** (`cli/src/retrospective.rs`) — after every successful human move, analyze the pre-move position with `force_include=[user_mv]` + `multi_pv=3`, classify, render ~4 lines:
  - Line 1: verdict headline with SAN + `??` / `?` annotations for Blunder / Mistake.
  - Line 2: engine's preferred move + expected opponent reply (pulled from `user.pv[1]`).
  - Line 3: dominant `TermDelta` (label + value).
  - Line 4 (optional): surprise tag when shallow-vs-deep disagreed.
- **REPL `retrospect [on|off]`** toggle (default on). Pause per move is roughly same as engine's move time.

### Phase 1 chunk 5: material narration — structured data + capture-sequence renderer

- **`MaterialOutcome { events, net_mg_cp, net_eg_cp, last_ply }`** + **`CaptureEvent { ply, captor, captor_piece, captured_piece, square, value_mg, value_eg }`** — structured per-capture story. Engine computes, UI renders. Handles en passant (captured pawn recorded at `to` square) and promotions (captor is pre-promotion piece). Castling explicitly not a capture.
- **`compute_material_outcome(ma, pre_move_pos, root_stm)`** — walks the user's PV from the pre-move position through the settled ply (or PV end), recording captures chronologically. Net values in engine-cp using MG/EG piece-value tables; sign is root-STM POV.
- **CLI renderer in `cli/src/retrospective.rs`** — when the PV has captures, emits `"Forced sequence: Nxe5 Nxe5 d4 ... — you lose a pawn (knight + bishop for pawn + bishop + pawn)."`. Net computed using **classical** piece values (pawn=1, minor=3, rook=5, queen=9) so a chess player reads intuitive magnitudes. When there are no captures, falls through to the cumulative-75% term prefix: `"Shifts: material -1.24, pawns.connected -0.28, ..."`.
- **Design invariant established**: each term's richer narration is a structured `TermXOutcome` struct returned by the engine (paralleling `MaterialOutcome`), with the CLI rendering it as prose. This scales to the Tier 2+ terms (Threats, King, etc.) without mixing engine logic and phrasing.

### Phase 3 Tier 1 (partial): ThreatsOutcome — hanging-piece narration

- **`ThreatsOutcome { ours_hanging, theirs_hanging, ours_hanging_delta, theirs_hanging_delta, last_ply }`** + **`PieceLocation { square, piece }`** — structured data. "Hanging" = attacked by enemy AND not defended by any friendly piece. V1 scope: hanging only; SEE-losing-exchange detection (lower-value attackers, multi-attacker overloads) deferred — that's a 1400+ concept.
- **`compute_threats_outcome(ma, pre_move_pos, root_stm)`** — clones pre-move pos, replays PV through settled ply, counts hanging pieces on both sides from `root_stm`'s POV, deltas against pre-move baseline.
- **CLI renderer in `cli/src/retrospective.rs`** — emits `"You leave a hanging knight on d2."` / `"You expose the opponent's bishop on c5."` when deltas are positive (i.e., *newly* hanging, not just still-hanging). Extracts `TermId::Threats` from the generic Shifts list when narration fires, so no redundancy.
- **Tier 1 completion still pending**: move toward richer threat patterns (minor-on-major, rook-on-queen, undefended-attackers) per evaluator's threats.rs breakdown. Piece-attribution on hanging pieces landed in the same chunk.

### Phase 3 Tier 1 (continued): attacker annotation on HangingPiece

- **`HangingPiece { location: PieceLocation, attackers: Vec<PieceLocation> }`** replaces the flat `PieceLocation` entries on `ThreatsOutcome`. Attackers are gathered from the attackers bitboard, ordered by ascending square index for deterministic output.
- **CLI narration** grew attacker phrasing: `"attacked by the e3 pawn"` / `"attacked by the e3 pawn and b5 bishop"` / `"attacked by the e3 pawn, b5 bishop, and d1 queen"` (Oxford comma for 3+). Multi-piece hanging lines use `—` + `;` separator so each entry keeps its own attacker parenthetical.
- Verified end-to-end on the Italian-Nxe5 fork: renders `"pawn on f2 (attacked by the e4 knight); pawn on c5 (attacked by the e4 knight)"` — the student can SEE the fork from the output alone.

### Phase 0 (completion): King + Passed sub-term splits

- **`KingBreakdown { shelter, danger, pawnless_flank, flank_attacks }`** — 4 named `Score` sub-terms. Signs baked in so `.total()` is a plain field-sum: shelter is the raw `pawns::king_safety(pos, us)` output; the other three are *already-negated* penalty contributions. `danger` stays atomic — the quadratic `Score::new(king_danger² / 4096, king_danger / 16)` derives from a scalar blend of ~10 raw signals (safe checks, attacker count × weight, weak-ring squares, etc.) that's pedagogically irreducible. `TermId::King` split into `KingShelter` / `KingDanger` / `KingPawnlessFlank` / `KingFlankAttacks`. `EvalTrace.king: [KingBreakdown; 2]` + `king_total(color)` helper. CLI `eval_report` renders aggregate + 4 sub-rows.
- **`PassedBreakdown { rank_bonus, king_proximity, free_advance, stopper_penalty }`** — 4 named `Score` sub-terms, stopper baked-in negative. Candidate-passer halving applies per-component (via `Score`'s componentwise division), so each sub-term retains its own halving. Bit-exactness drifts by ≤1 cp per passer from the reference's "halve the aggregate bonus" formulation — within weight-tuning noise. `TermId::Passed` split into `PassedRankBonus` / `PassedKingProximity` / `PassedFreeAdvance` / `PassedStopperPenalty`. `EvalTrace.passed: [PassedBreakdown; 2]` + `passed_total(color)` helper. CLI `eval_report` renders aggregate + 4 sub-rows.
- Post-split `TermId::ALL` is **42 variants** (was 36 pre-king-split): 3 net (Material/Imbalance/Initiative) + 1 per-colour scalar (Space) + 6 Pawns + 11 Pieces + 4 Mobility + 9 Threats + 4 King + 4 Passed.

### Phase 3 Tier 4: PassedPawnsOutcome + PiecesPositionalOutcome

- **`PassedPawnsOutcome { ours_pre, ours_post, theirs_pre, theirs_post: PassedBreakdown, last_ply }`** — pre/post snapshots of the 4-sub-term `PassedBreakdown` on both sides. `snapshot_passed(pos, our_color)` runs the standard `Evaluator::initialize` + `pieces::evaluate` priming (same as mobility / king) and calls `passed::evaluate(&e, our_color)`. CLI narration (`cli/src/retrospective.rs`): `PASSED_DELTA_THRESHOLD_CP = 25`. Per side, four line generators with worsened-wins-over-improved precedence via `or_else`. Per-category phrasing: `rank_bonus → a passer pushed forward / a passer fell back`, `king_proximity → king race improved / king race worsened`, `free_advance → the promotion path cleared / the promotion path got crowded`, `stopper_penalty → a passer reached an easier file / a passer drifted to a harder file`. Lines: *"Your passed pawns improved: a passer pushed forward."* / *"You weakened the opponent's passed pawns: the promotion path got crowded."* Consumes all 4 `TermId::Passed*` variants when any line fires.
- **`PiecesPositionalOutcome { ours_pre, ours_post, theirs_pre, theirs_post: PiecesBreakdown, last_ply }`** — pre/post snapshots of the 11-sub-term `PiecesBreakdown` on both sides. `snapshot_pieces_both(pos)` runs the full `Evaluator::initialize` + `pieces::evaluate(White)` + `pieces::evaluate(Black)` sequence in a single pass and returns `(PiecesBreakdown, PiecesBreakdown)` — callers map to ours/theirs by `root_stm`. Matches the canonical WHITE-then-BLACK order the main evaluator uses. CLI narration: `PIECES_POSITIONAL_DELTA_THRESHOLD_CP = 15`. Per side, four line generators with worsened-wins precedence. Per-category phrasing keeps piece subjects in each clause so the outer "Your piece placement improved:" / "The opponent's piece placement weakened:" wrapper clarifies ownership. Highlights: `outposts → a minor claimed/lost an outpost`, `rook_on_open_file → a rook claimed/left the open file`, `long_diagonal_bishop → a bishop claimed/left the long diagonal`, `trapped_rook → a rook escaped its trap / a rook got trapped`, `weak_queen → the queen came under minor-piece pressure / shook off minor-piece pressure`. Consumes all 11 `TermId::Pieces*` variants when any line fires.
- **KingDangerOutcome explicitly skipped**: after the king split landed, we evaluated whether a pre/post `KingBreakdown` outcome would add teaching value over the existing `KingSafetyOutcome` (which narrates from raw scalars — attackers_count, attacks_count, shelter_mg/eg). Decision: skip as redundant. Revisit if real retrospective output shows `danger` or `flank_attacks` sub-term shifts that the scalar-based narration misses.

### Phase 3 Tier 3: PawnStructureOutcome + MobilityOutcome

- **`PawnStructureOutcome { ours_pre, ours_post, theirs_pre, theirs_post: PawnsBreakdown, last_ply }`** — pre/post snapshots of the 6-sub-term `PawnsBreakdown` on both sides. Snapshot helper just passes through `pawns::evaluate(pos).breakdowns[color]` — no `Evaluator` priming needed since pawn-structure scoring doesn't depend on piece attack tables.
- **`MobilityOutcome { ours_pre, ours_post, theirs_pre, theirs_post: MobilityBreakdown, last_ply }`** — pre/post snapshots of the 4-sub-term `MobilityBreakdown` on both sides. Snapshot helper runs the standard `Evaluator::initialize` + `pieces::evaluate` priming (same as `snapshot_king_safety`) and reads `e.mobility[color]`.
- **CLI narration in `cli/src/retrospective.rs`**:
  - **Pawn structure**: `PAWN_STRUCTURE_DELTA_THRESHOLD_CP = 15`. Per side, four line generators (`our_pawns_worsened_line`, `our_pawns_improved_line`, `their_pawns_worsened_line`, `their_pawns_improved_line`) — worsened wins over improved via `or_else` when both fire on the same side. Per-category phrasing: `connected → broke pawn connections / connected pawns`, `isolated → isolated a pawn / reconnected an isolated pawn`, `backward → created a backward pawn / freed a backward pawn`, `doubled → doubled a pawn / resolved a doubled pawn`, `weak_unopposed → exposed a weak pawn / covered a weak pawn`, `weak_lever → walked into a pawn lever / resolved a pawn lever`. Multiple triggered sub-terms are comma-joined in one line. Lines: *"Your pawn structure weakened: doubled a pawn, exposed a weak pawn."* / *"You weakened the opponent's pawn structure: ..."* / *"The opponent's pawn structure improved: ..."*. Consumes all 6 `TermId::Pawns*` variants when any line fires.
  - **Mobility**: `MOBILITY_DELTA_THRESHOLD_CP = 30`. `mobility_biggest_shift(pre, post)` returns the piece type with the largest |delta_mg|; if below threshold → no line. Lines: *"Your knight mobility dropped (+0.60 → +0.20)."* / *"Your bishop mobility improved (...)"* / *"You restricted the opponent's rook mobility (...)"* / *"The opponent's queen mobility improved (...)"*. Consumes all 4 `TermId::Mobility*` variants when any line fires.

### Phase 3 Tier 2: KingSafetyOutcome (with polish)

- **`KingSafetySnapshot { king_sq, attackers_count, attacks_count, shelter_mg, shelter_eg }`** — raw scalar snapshot. `king_sq` carried so UI layers can categorize the king's location without the Position back; `attackers_count` = number of enemy pieces hitting our king ring; `attacks_count` = total attacks on squares immediately adjacent to our king; `shelter_*` = pawn-shelter Score components in engine-cp.
- **`KingSafetyOutcome { ours_pre, ours_post, theirs_pre, theirs_post, last_ply, phase }`** + 8 delta accessors (attackers / attacks / shelter mg / shelter eg, per side). `phase` = game-phase blend at settled ply (128 = mg, 0 = eg) for endgame gating.
- **`snapshot_king_safety(pos, our_color)`** — runs the standard `Evaluator::initialize` + `pieces::evaluate` priming sequence and reads `e.king_attackers_count[enemy]` / `e.king_attacks_count[enemy]`, then calls `pawns::king_safety(pos, our_color)` for the shelter Score. Same priming the main evaluator does.
- **`compute_king_safety_outcome(ma, pre_move_pos, root_stm)`** — snapshot pre, replay PV to settled ply, snapshot post, attach phase via `material::evaluate(post).game_phase.0`.
- **CLI narration in `cli/src/retrospective.rs`** — four line generators (`our_king_exposure_line`, `their_king_exposure_line`, `our_king_safer_line`, `their_king_safer_line`), each returning `Option<String>`. Exposure fires on attackers ≥ +1 OR shelter ≤ −25 cp; safer fires on attackers ≤ −1 OR shelter ≥ +25 cp. **Per-side precedence**: when exposure + safer both fire on the same side (contrived case), exposure wins — worsening is more urgent teaching.
- **Flank-aware phrasing** (`flank_side_label(king_sq)`): kings on files a-c label "queenside," f-h label "kingside," d-e fall back to generic "king ring." Exposure uses "N attackers on the kingside (up from M)"; safer uses "kingside attackers down to N (from M)." Shelter clauses use "shelter weakened" / "shelter cracked" / "shelter strengthened" per direction.
- **Endgame shelter suppression** (`KING_SHELTER_ENDGAME_PHASE_CUTOFF = 32`): below the cutoff, shelter clauses are silenced in all four line generators — attackers clauses still fire. If shelter was the only trigger, the whole line goes silent. Rationale: pawn cover doesn't matter once heavy pieces have traded off; narrating it would just add noise.

### Phase 3 Tier 1 (continued): Stockfish pressure-pattern parity

- **`PressuredPiece { location, attackers, kind }`** + **`PressureKind::{MinorOnMajor, RookOnQueen, SafePawnThreat}`** — structured per-(target, kind) data for Stockfish-evaluator threat patterns that fire even when SEE doesn't (i.e., positional pressure that forces the target to relocate). Same `PieceLocation` shape as hanging/SEE-losing.
- **`ThreatsOutcome.ours_pressured` / `theirs_pressured`** (+ deltas) — populated by `compute_threats_outcome`. **No engine-side de-dup** with hanging or SEE-losing: a low-value attacker against a higher-value target almost always also wins SEE, so suppressing at the engine layer would empty the lists. Engine reports the patterns honestly; CLI suppresses redundant render.
- **Patterns implemented**:
  - `MinorOnMajor` — knight or bishop attacks an enemy rook or queen.
  - `RookOnQueen` — rook attacks the enemy queen.
  - `SafePawnThreat` — a pawn whose own square is unattacked by any of `side`'s pieces, attacking an enemy non-pawn. Simpler "not threatened back" definition than Stockfish's "strongly safe" — same teaching outcome.
- **Deferred** (defer until real output asks for them): `KnightOnQueen` (knight one move from queen, in safe square) and `SliderOnQueen` (slider on doubly-defended ray to queen) — more abstract at 1200 ELO.
- **CLI narration in `cli/src/retrospective.rs`** — passive verb chosen per kind (`harried` for MinorOnMajor, `pressured` for RookOnQueen, `kicked` for SafePawnThreat — established chess slang). Single entry: `"Your rook on a1 is harried (attacked by the c2 knight)."`. Multi: `"Your pieces are under pressure — rook on a1 harried (attacked by the c2 knight); bishop on f6 kicked (attacked by the e5 pawn)."`. CLI-side de-dup: pressures whose target square is already in `ours_hanging`/`ours_see_losing` (or theirs) are suppressed before render.

### Phase 3 Tier 1 (continued): SEE-losing-exchange detection

- **`ThreatsOutcome.ours_see_losing` / `theirs_see_losing`** (+ deltas) — pieces that are defended but still lose material in an enemy-initiated exchange, per `Position::see_ge(cheapest_enemy_capture → piece, Value(1))`. Reuses `HangingPiece` for the carrier struct (the same location + attackers shape applies; the outer field tells callers which teaching story to tell).
- **`list_see_losing`** uses the cheapest enemy attacker as the initial capture since standard SEE resolves optimally from there. `see_ge` reads the mover colour from `piece_on(from)` — NOT the position's side-to-move — so no stm-flipping needed.
- **Known false-negative cases** (both acceptable — better to miss than to mis-flag):
  - Cheapest attacker is pinned: the initial capture would be illegal; `see_ge` doesn't check legality of the first move, so it might still return true. Conservative — we prefer false negatives.
  - Promotion-rank capture: `Move::normal` doesn't encode the promotion, and `see_ge` short-circuits non-`Normal` moves to `Value::ZERO >= threshold`, failing strictly-positive thresholds.
- **CLI narration**: `"Your knight on e5 is defended but loses material to the exchange (attacked by the d6 pawn and g4 knight)."` — separate from hanging lines, distinct phrasing, fires independently.

### Surprise-tag precision fix

- **`select_surprise_phrase(verdict, surprise) -> Option<&'static str>`** (in `cli/src/retrospective.rs`) filters the shallow-vs-deep tag. Fires only for (Best/Good × LooksBadButGood) — positive teaching ("sacrifice works") — and (Inaccuracy/Mistake × LooksGoodButBad) — the main negative-teaching case. Suppresses contradictory pairs (Inaccuracy + LooksBadButGood), verdict-is-already-clear pairs (Blunder), and lost-position pile-ons (BestAvailable).
- **Phrasing** softened from `"follow-up refutes it"` → `"looks reasonable short-term — the follow-up favors the opponent"`. "Refutes" has a specific chess meaning (refuting a trap/opening) that didn't fit our shallow-vs-deep gap threshold.
- User-reported trigger: `"You played d5 — Good (Δ -0.30). ... (looked fine at a glance — follow-up refutes it.)"` — the verdict says the move is Good, so the tag was contradictorily framing it as a trap. Now suppressed in that configuration.

### Brilliancy surfacing (Best + LooksBadButGood, engine-preferred sharp moves)

- **User-side brilliancy** (`Best` + `LooksBadButGood`): previously lost to the `MoveVerdict::Best` early return before the surprise tag could fire. Fixed — Best now emits the headline with `!` annotation on the SAN and a dedicated follow-up line ("Well spotted — this looks risky at first glance, but the longer line pays off.").
- **Engine-preferred brilliancy**: when the user didn't play the best move but the best move itself is `LooksBadButGood`, the `"Engine preferred X (+Y)"` line now annotates X with `!` + a short explanation ("a sharp move that looks risky but pays off in the longer line"). Helps the student recognize that the engine's choice was non-obvious.
- **`!` SAN annotation convention**: extended `verdict_annotation` to `sharp_or_verdict_annotation(verdict, is_sharp)` — `!` takes precedence over `""` / `?` / `??` when `is_sharp` is true. Sharp = verdict ∈ {Best, Good} AND surprise == LooksBadButGood.
- **`format_engine_preferred_line(best_san, score_str, is_sharp)`**: extracted pure helper so the brilliancy rendering is unit-testable without needing a Position + StdoutLock.

### Opening book

- `engine/src/openings.rs` + `engine/data/openings/{a,b,c,d,e}.tsv` (Lichess CC0). Bundled via `include_str!`, replayed through our SAN parser at first use to build `HashMap<EPD, OpeningIdentification>`. ~3,500+ entries.
- Public: `identify(&Position) -> Option<OpeningIdentification>`, FEN/EPD variants. `chess-tutor opening [FEN]` for one-shot; REPL banner in `play` on every opening transition (`>> B90  Sicilian Defense: Najdorf Variation`). Descriptive only; does not influence move selection.

### Trap library

- `engine/src/traps/` — schema (`TrapEntry`, `Invariant`, `PunisherMove`, `DefenderOption`), 4-gate validator (trigger → invariants → SEE backstop → main-line verify), public scan APIs (`scan_threats` / `scan_after_move`).
- **Invariant kinds** (11): `PieceOn`, `SquareEmpty`, `AllEmpty`, `AnyPieceOfColor`, `PieceCount`, `NoPieceInMask`, `AttackerCountByColor`, `NotAttackedBy`, `AttackersSubsetOf`, `AttackersEqual`, `RayClear`. `check_invariant` public so UIs can render per-invariant explanations.
- **Damiano Defense** (`traps/damiano.rs`) — 7 invariants, 2-level branching tree with `is_main_defense` + `punisher_follow_up`. +100 cp gain.
- **Pending-trap state machine** (`PendingTrap`, `TrapExpectation`, `TrapEvent`, `advance_pending`) walks the refutation tree move-by-move once a trap fires, so students see per-ply narration through the whole sequence. `HistoryEntry` snapshots pre-move state so `undo` walks the cursor backward.
- CLI wiring in `play.rs`: `scan_threats` before the human's prompt (warns on bad moves); `scan_after_move` after every played move (emits trap-fired banners). Banners compose with opening-name banners.
- **Unit convention for trap gains (`terminal_gain_cp`, `main_line_gain_cp`)**: conventional cp (pawn=100, knight=300, bishop=325, rook=500, queen=900). Private to `traps/mod.rs::material_delta_for`; **does not leak into search or evaluation** — engine internals speak Stockfish-internal cp (PawnEG=213). Don't cross the streams.

### Endgame specialists

KXK, KBNK, KPK (196_608-entry retrograde bitbase in `bitbases.rs`), KNNK (unconditional draw), KNNKP (technique gradients: pawn-distance-from-promotion + king tropism + nearest-knight-to-weak-king). Wired via `endgame::probe(pos) -> Option<Value>` which the main evaluator's endgame short-circuit trusts.

Deferred: `KRKP`/`KRKB`/`KRKN`/`KQKR`/`KQKP` (user knows these manually), pawn-heavy scaling functions (`KBPsK`, `KRPKR`, etc.).

---

## Key design decisions (don't re-litigate)

- **Engine is a pure library.** CLI is a separate crate. Platform apps (Swift, Kotlin, egui) consume via FFI. See CLAUDE.md.
- **Board layout matches Stockfish 1:1** — bit 0 = a1 through bit 63 = h8, row-major. Non-negotiable; lets us verify eval term-by-term against Stockfish's `d` output.
- **Numerical weights carry over verbatim; code is independently authored.** See CLAUDE.md for legal reasoning (idea/expression dichotomy). Never copy variable names, comments, or code structure from the reference.
- **`Score` is packed mg+eg in i32** using Stockfish's rounding trick. Addition/subtraction/negation/integer multiplication distribute componentwise — verified. Don't use floats. **Division does NOT distribute** — `impl Div<i32>` is decompose-recompose.
- **All incremental Position state is maintained in `remove_piece_mailbox_and_bitboards` / `put_piece_mailbox_and_bitboards`.** These are the two chokepoints every move passes through. When adding an incremental field, update both and add a `compute_*_from_scratch` test oracle.
- **Perft is the certification standard.** Any movegen change must pass the four perft positions in `movegen.rs::tests`.
- **Parallelism-ready APIs.** Standing policy: don't have to parallelise up front, but APIs must accept eventual multi-threading. TT's `&self` + atomic entries is the concrete example — single-threaded today but no API change needed when threads arrive.
- **Skill level + MultiPV > 1 for variable-strength bots** are in-scope later. Plan search API so `Skill::enabled()`-style override can be added without rewriting core move selection.
- **TT entries are 16 bytes atomic-packed** (vs Stockfish's 10 bytes racy). Trade memory density for Rust-idiomatic all-atomic, no `unsafe`. Don't "fix" back to 10 bytes.
- **Move picker takes `pos` and `history` per-call, not as borrowed fields.** Borrow-checker forced this; threading them in at each `next_move(pos, history, skip_quiets)` call lets search mutate freely between calls. Don't reintroduce borrowed fields.
- **Illegal FENs are dangerous.** `from_fen` validates piece counts + kings but *not* "side-to-move's opponent isn't in check." Test FENs must have the non-moving side not-in-check. Bit me 3× across eval/SEE tests.
- **Magic bitboards use runtime xorshift search at LazyLock init.** Deterministic seeds; tens of ms per process. Bake into binary when integrating first platform app.

## Gotchas

- **Rust 1.80+** required (uses `LazyLock`). `rust-toolchain.toml` pins stable.
- **`cargo test --release`** is ~10× faster than debug because of the magic search. Use release by default.
- **`Piece::index()`** returns 1..=6 (white) and 9..=14 (black) — gaps at 0/7/8/15. Tables indexed by piece use size 15/16 with unused slots.
- **`Square::NONE`** is `Square(64)`; valid squares 0..=63. Never silently construct it.
- **FEN validator rejects missing/extra kings.** Test FENs must have both kings.
- **`PieceType` discriminants are 1..=6.** `PieceType::index()` returns 1 for Pawn, 6 for King. Tables use `[_; 7]` with slot 0 empty, or `[_; 6]` with -1 shift. Prefer the former.
- **`Direction` is `i8`.** Internal arithmetic goes through `i16` to avoid overflow.
- **Legal-move filter uses do/undo**, not pin/checker analysis. Correct but slow; optimise only if profiler says so.
- **`Bitboard` lacks `BitOrAssign<Square>` / `BitXorAssign<Square>`.** Use `bb = bb | sq` or extend the impls.
- **TT `key16 == 0` is the empty-slot sentinel.** ~1/65 536 real keys collide (accepted). Test keys must use non-zero top 16 bits.
- **`Position::see_ge` only runs the full algorithm for normal moves.** Castling/EP/promotion short-circuit to `Value::ZERO >= threshold`.
- **Per-term eval tests must bootstrap the evaluator.** Build an `Evaluator`, call `initialize(White)` + `initialize(Black)`, then `pieces::evaluate(&mut e, White)` + `pieces::evaluate(&mut e, Black)` *before* calling the term under test. Pattern in `eval/king.rs::tests`.
- **Test FENs need realistic material for space / king-safety terms.** `space::evaluate` short-circuits below 12,222 total non-pawn material.
- **`cargo test --release <test_name_filter>`** avoids ~5s release recompile on single-test iteration.

## Repo layout

```
chess-tutor-2/
├── CLAUDE.md                    # evergreen guidance, auto-loaded
├── HANDOFF.md                   # this file — current state snapshot
├── reference/Stockfish-sf_11/   # extracted reference source, read-only
├── rust-toolchain.toml
└── core/
    ├── Cargo.toml               # workspace root (members: engine, cli)
    ├── engine/                  # chess-tutor-engine (lib)
    │   └── src/
    │       ├── lib.rs           # pub mod … (alphabetical)
    │       ├── analysis.rs      # teaching pipeline: TermId, TermDelta, MoveAnalysis, analyze_position
    │       ├── types.rs         # enums + newtypes + piece values
    │       ├── bitboard.rs      # Bitboard + ops + const masks
    │       ├── attacks.rs       # const attack tables
    │       ├── magics.rs        # slider magic bitboards
    │       ├── zobrist.rs       # hash keys (main + pawn + ep + side + castling)
    │       ├── psqt.rs          # piece-square tables
    │       ├── material.rs      # game phase, imbalance, scale factor
    │       ├── position.rs      # board state, FEN, do/undo, blockers/pinners
    │       ├── movegen.rs       # pseudo-legal + legal + perft
    │       ├── pawns.rs         # pawn structure (PawnsBreakdown) + king shelter/storm
    │       ├── tt.rs            # transposition table
    │       ├── movepick.rs      # staged move picker + butterfly history
    │       ├── search.rs        # alpha-beta + qsearch + pruning + MultiPV + settled-ply
    │       ├── engine.rs        # public API: Engine, SearchParams, SearchLine
    │       ├── san.rs           # Standard Algebraic Notation (parse + format)
    │       ├── openings.rs      # ECO/name lookup via bundled Lichess TSVs
    │       ├── traps/          # refutation-tree library
    │       │   ├── mod.rs       # schema + invariants + validator + scan APIs + pending-trap FSM
    │       │   └── damiano.rs   # Damiano Defense refutation
    │       ├── bitbases.rs      # retrograde-computed endgame bitbases (KPK)
    │       ├── endgame.rs       # specialised endgame evaluators (KXK, KBNK, KPK, KNNK, KNNKP)
    │       └── eval/            # main evaluator (EvalTrace lives in mod.rs)
    │           ├── mod.rs       # Evaluator + initialize + tapered assembly + scale_factor + EvalTrace
    │           ├── pieces.rs    # per-piece-type positional terms + mobility (PiecesBreakdown)
    │           ├── king.rs      # pawn-shelter + kingDanger + flank penalties
    │           ├── threats.rs   # hanging / minor / rook / king / pawn-push / queen-attack
    │           ├── passed.rs    # passed-pawn scoring
    │           ├── space.rs     # middlegame space
    │           └── initiative.rs  # complexity-driven mg/eg correction
    └── cli/                     # chess-tutor-cli (bin name `chess-tutor`)
        └── src/
            ├── main.rs          # clap dispatch: board/moves/eval/opening/search/play
            ├── board.rs         # ANSI board renderer
            ├── uci.rs           # Move ↔ UCI string
            ├── eval_report.rs   # EvalTrace pretty-printer
            ├── analysis_report.rs  # MoveAnalysis pretty-printer (--analyze output)
            ├── retrospective.rs    # post-move verdict renderer (auto-fires in play)
            └── play.rs          # interactive REPL
```

Future files (create when porting the corresponding piece):
- `engine/src/tactics/` — Phase-5 tactic detectors
- `engine/src/timeman.rs` — proper time management (eventual)

## Prior-attempt context

Predecessor repo (`~/Repos/work/chess-tutor/`) failed at ~800 ELO — the real failure mode wasn't "got search wrong" but the sequencing: hand-crafted bespoke signals first, naive search bolted on after. This rewrite copies Stockfish 11's data structures + algorithms + weight tables essentially 1:1 (independent Rust expression, carried-over numerics) so we inherit strength, and builds the teaching layer as a thin presentation over Stockfish's `Trace` decomposition — not a separate engine. Strength verified empirically vs chess.com 2000 bots. UCI adapter explicitly out of scope (fully-offline teaching tool, not tournament engine).

## Deferred work

- **Time management** (`timeman.rs`) — proper allocation from game time + increment + moves-to-TC. Today `max_time` is a simple deadline.
- **Deferred pruning** (ordered by historical Stockfish gains): continuation history + counter-move heuristic, IID, singular extensions, probcut, razoring.
- **Baked-in magic attack tables** — harvest magic numbers from one local run, paste as `const`. Saves ~tens of ms per process start. Do when integrating first platform app.
- **Remaining endgame specialists** — KRKP/KRKB/KRKN/KQKR/KQKP + pawn-heavy scaling functions.
- **Rubinstein trap** — user needs to work out its invariants first.
