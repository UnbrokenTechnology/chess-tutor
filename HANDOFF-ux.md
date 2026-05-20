# Handoff: chess-tutor-2 — UX / teaching layer

Forward-looking UX context. The product surface is teaching feedback, not the engine. See [`HANDOFF.md`](HANDOFF.md) for the index, [`CLAUDE.md`](CLAUDE.md) for the mission and ground rules, and [`HANDOFF-perf.md`](HANDOFF-perf.md) for engine perf state (read only if perf becomes relevant to a UX task).

## Current focus: visual learning elements

The platform-portable UI refactor is complete; the next layer of teaching surface is **visual annotations** on the desktop board — arrows for pins / threats / refutation lines, highlighted squares for weak pawns / outposts / king-attack flanks, badges for trap moments in the move list. The architecture is in place; the missing pieces are (a) new facts the engine doesn't yet produce (pin/fork/skewer detection), (b) new view descriptors that carry annotation payloads, and (c) the egui drawing code that paints them.

### What's already available to consume

Renderers already have access to these structured facts via [`Session`](core/ui/src/session.rs). No new plumbing is needed to start drawing arrows from any of them — only new view descriptors + draw code.

- **`Session::history()` → `&[HistoryEntry]`** — per-move records:
  - `retrospective: Option<RetrospectiveResult>` — for user moves, carries `Vec<MoveAnalysis>` with PVs, scores, per-term deltas, ply-by-ply traces. Filled async by the worker.
  - `engine_info: Option<EngineInfo>` — for engine moves, carries score / depth / nodes / nps / elapsed.
  - `noise_pick: Option<NoisePickInfo>` — when noise drove the bot off best.
  - `trap_events: Vec<TrapEvent>` — for moves played while a trap was mid-refutation.
  - `trap_hit: Option<TrapHit>` — for the trigger move of a new trap.
  - `pending_trap_before: Option<PendingTrap>` — internal undo-restore field; renderers can ignore.
- **`Session::pending_trap() → Option<&PendingTrap>`** — live cursor when a trap is mid-refutation. Has `.entry` (the static TrapEntry with its full refutation tree) and `.hit` (the TrapHit snapshot taken at trigger).
- **`Session::trap_threats() → Vec<TrapThreatened>`** — pre-move warnings for the current position. Each entry has `candidate_uci`, `candidate_san`, and the `TrapHit` you'd be handing the opponent. Refresh per frame is fine; the underlying scan is cheap.
- **`Session::run_analysis(pos, SearchParams) → AnalysisOutcome`** — blocking analytical search for ad-hoc queries. Returns `analyses: Vec<MoveAnalysis>` + timing. Already used by the CLI's REPL `search` / `analyze`; the desktop could use it to drive a "what's the engine's plan for this position?" panel.

### Already-spatial data the engine produces

- **Move.from() / Move.to()** — every move trivially renders as a from→to arrow. Last-played move, engine-preferred move, hint suggestions all have this.
- **PV moves** — `MoveAnalysis.pv: Vec<Move>` carries the principal variation. Drawing the first 1-2 plies as chained arrows is natural ("if you'd played e4, here's how the line goes: e4 → e5 → Nf3").
- **TrapHit.main_line_san** — the punisher's scripted refutation as SAN strings. Parsing back to `Move`s and rendering as arrows is straightforward (need a pre-move position to disambiguate, which Session can provide).
- **`Position::king_square(color)`** — for any king-safety annotation; the king's location is always known.
- **Pinned-piece detection** — [`Position::slider_blockers(candidate_attackers, target)`](core/engine/src/position/blockers.rs) returns `(blockers, pinners)` as bitboards in one call. The convenience wrapper [`Position::blockers_for_king(us)`](core/engine/src/position/blockers.rs) returns just the pieces of `us` pinned to their own king. Stockfish's terminology: a *blocker* is a piece that, if removed, would expose `target` to a slider attack; a *pinner* is the attacker behind it. **This is the lowest-cost first pin renderer**: call `slider_blockers(enemy_pieces, king_sq)` for each side, emit arrows from pinner → blocker → king for each bit pair.

### What's missing (needs engine work)

These are general tactical patterns the engine doesn't yet annotate. They'd live in [`core/engine/src/analysis/`](core/engine/src/analysis/) as Phase 5 of the teaching pipeline (called out as deferred in [`analysis/mod.rs`](core/engine/src/analysis/mod.rs) `//!`):

- **Forks** — one piece attacking two-or-more enemy pieces of higher value.
- **Skewers** — like a pin but the more valuable piece is in front.
- **Discovered attacks** — moving piece A unmasks piece B's attack on a target.
- **Double attacks** — the moving piece itself creates two threats.
- **Hanging pieces** — undefended targets after a sequence.

Each would output a structured `TacticHit` (or similar) with the spatial data needed to draw it: from/to squares, kind tag, severity. Parallel to the trap library's `TrapHit` shape but for general patterns, not pre-scripted lines.

**Order of operations to consider:** start by drawing what's already cheap (pins from bitboards, trap arrows from `TrapHit`, retrospective best-move arrow from `MoveAnalysis.pv[0]`). Add fork/skewer/etc. when the visual layer's rendering pipeline is already in place and we know exactly what shape of payload we want.

### View descriptor design sketch

`BoardView` today carries pre-oriented per-cell semantic flags only. Visual annotations want a separate layer — overlay arrows, highlighted squares, badges — drawn *on top* of the board. Suggested shape:

```rust
pub struct BoardView {
    pub rows: [[BoardCell; 8]; 8],
    pub pending_promotion: Option<PromotionPickerView>,
    pub annotations: Vec<BoardAnnotation>,  // NEW
}

pub enum BoardAnnotation {
    Arrow {
        from: Square,
        to: Square,
        kind: AnnotationKind,
    },
    SquareHighlight {
        square: Square,
        kind: AnnotationKind,
    },
    // Future: PieceBadge { square, glyph } for "weak piece" markers etc.
}

pub enum AnnotationKind {
    /// Engine-preferred move you didn't play. Subtle blue/green.
    BestMove,
    /// Pin: pinner → blocker, or blocker → king. Red/amber.
    Pin,
    /// Trap refutation main line. Bold red.
    TrapRefutation,
    /// Threat — your piece is attacked. Yellow.
    Threat,
    /// Custom kinds as we add tactics.
    Generic(&'static str),  // tag the renderer can interpret
}
```

Renderers (egui, CLI text fallback) each map `AnnotationKind` to their own visual language — egui paints actual arrows; CLI could print "→ engine preferred c4" text under the board (or just ignore annotations, treating them as a desktop-only feature for now).

### Where annotations come from (the view-builder)

`Session::build_board_view()` is the natural place. It already gathers `viewed_position` + last-move + selected + legal-moves; adding "and these annotations" is one more reader. The annotations come from:

- The viewed history entry's `retrospective` (best-move arrow, engine-preferred-line arrow).
- `pending_trap` + `trap_threats()` (trap arrows + threat-square highlights).
- The viewed position itself (pin bitboards → pin arrows; check-tint → already in `BoardCell.check_tint`).
- Future Phase-5 tactic library output, once it exists.

Whether annotations are computed per-frame or cached on `HistoryEntry` is a question the renderer doesn't dictate — start with per-frame, profile if it becomes hot.

### Recommended starting slice

1. **`BoardView` grows the `annotations` field.** No behavioural change yet — empty vec everywhere.
2. **Pin renderer:** `Session::build_board_view` reads `Position::blockers` / `Position::pinners`, emits `BoardAnnotation::Arrow { Pin }` from pinner → king through the pinned piece. Desktop's `draw::board` paints them as red arrows.
3. **Trap-refutation arrows:** when `pending_trap.is_some()`, emit arrows for the next expected punisher move and the defender's main-line response (parsed from the static `TrapEntry`). One arrow per active layer of the tree.
4. **Best-move arrow on retrospective panel:** the user just played M but the engine preferred M'; draw an arrow for M'. Lives on the panel-entry's `retrospective.analyses[0].pv[0]`.

Each slice is testable independently — pin detection has unit-test candidates, trap arrows can be verified against the Damiano fixture, best-move arrows are visible-by-eye in a normal game.

---

## Architectural state (recap)

For a fresh context: the refactor that ended just before this handoff was rewritten landed five commits. The shape now is:

- **`chess-tutor-engine`** produces facts. `MoveAnalysis` (term deltas, surprise, verdict, settled-ply, PV, ply traces) for searches. `TrapEvent` / `TrapHit` / `TrapThreatened` for the trap library. Pure data; no platform coupling.
- **`chess-tutor-ui` (`Session`)** owns game state — position, history, opponent profile, book cursor, trap cursor — plus a worker thread that runs searches. `RepaintFn` callback at construction lets the renderer wake its event loop; no other platform types in the API.
- **Renderers** transform facts into platform-specific surfaces.
  - `chess-tutor-narration` is **the text renderer** — same conceptual layer as `draw::board`. Used by desktop's `draw::side_panel` (default opts) and CLI's `play.rs` (with `--no-explain-best`). The core/ui crate does *not* depend on it.
  - `desktop/src/draw/*` paints egui views.
  - `core/cli/src/board.rs` paints ANSI views.

`HistoryEntry` is the persistent record of "everything that happened on this move" — raw data, no formatting. Renderers format on read.

The CLI no longer holds a private engine. Engine play, auto-retrospective, REPL `search`, REPL `analyze` all flow through Session's worker (the last three via `Session::run_analysis` blocking helper).

## Opponent profile / bot variability

Goal: ship bot-tuning toggles so games aren't deterministic from move 1, and so the student can practice against specific openings or weakened opponents. All four pillars — A (skeleton), B (opening book), C (eval signal mask), D (move noise + blunder) — landed in May 2026. Read the [`opponent.rs`](core/engine/src/opponent.rs) module doc for the strict invariant: **analytical paths (retrospective, hint, `analyze`) must never consult the profile** — they need to judge the user's move against true best play.

Phase D surface (delivered 2026-05-16):
- 7 [`NoiseProfile`](core/engine/src/opponent.rs) knobs, all-off by default. Three branches, evaluated in this order — **blunder → wild → softmax**.
  - **Blunder branch** (`blunder_chance`, `blunder_min_loss_cp`, `blunder_max_loss_cp`): pick uniformly from engine-considered lines whose loss vs #1 falls in `[min, max]`. When the band is empty, the picker takes the closest line on each side, with a `BLUNDER_FALLBACK_TOLERANCE = 2.0×` cap on the above-band side — blunders skip entirely rather than throw away a queen. Mate-guarded.
  - **Wild branch** (`wild_chance`): per-move probability of picking uniformly from **all legal moves**, bypassing engine ranking. Only branch that can pick a move the search didn't surface. Mate-guarded.
  - **Softmax branch** (`candidate_pool`, `temperature_cp`): Boltzmann-weighted sampling over the top-K.
  - Plus `guaranteed_mate_in` (default 1) — suppresses blunder + wild when the bot sees a short mate.
- [`noise::pick`](core/engine/src/noise.rs) — pure function `(profile, seed, ply, &lines, &legal_moves) -> NoisePick`. Deterministic given `(seed, ply)`; per-game seed is logged so a varied game can be replayed via `--seed N`.
- CLI flags: `--noise-pool`, `--noise-temp`, `--blunder-chance`, `--blunder-min-loss`, `--blunder-max-loss`, `--wild-chance`, `--guaranteed-mate-in`. REPL: `noise [show | pool N | temp CP | blunder F | min-loss CP | max-loss CP | wild F | guarantee N | reset]`.
- Desktop New Game dialog has the full settings UI. Auto-opens at first launch.
- **Noise picks land on `HistoryEntry.noise_pick`.** Both renderers can read them; CLI tags inline (`[noise: softmax #3 of 6 (-42 cp)]`), desktop only logs to stderr today — a visible per-move badge in the move list is a small follow-on.

Phase D follow-on, deferred:
- **Visible per-move noise badge in the desktop move list.** Data already on `HistoryEntry.noise_pick`. ~5 lines of `draw::side_panel` work.
- **ELO presets.** `--bot-elo 1200` (CLI) + a "Preset" dropdown in the desktop dialog filling in `(pool, temp, blunder, severity, guarantee, wild)`. Defer until the manual knobs feel clunky in real play (so presets get tuned from actual playthrough).
- **Opponent-side retrospective.** A "the bot just played a deliberate mistake — can you find the punishment?" line when `noise_pick.is_some()` and the delta is large. Requires the analytical search to read the bot's profile for this *one* user-facing purpose — currently forbidden by the analytical-paths invariant; needs a carefully-scoped exception.
- **More aggressive defaults.** Once ELO presets are tuned, default new-install to a ~800-ELO preset for a more human-feeling out-of-box opponent.
- **Seed surface in the GUI.** Desktop logs the seed to stderr but doesn't show it in the UI; players who want to replay a varied game can't copy the seed back. Add a status line + paste-in field.

Phase C (eval mask, delivered): 8 toggleable `EvalCategory` values. CLI surface complete; desktop reads the mask but has no UI for editing (the New Game dialog has a collapsible checkbox panel — close enough; if more granular mid-game editing is wanted, add a settings panel mirroring the CLI's `eval-mask` command).

Opening-book follow-on, deferred:
- **Desktop UI for allowed-openings selection.** Default is "every theoretical opening in the TSV" (~3,900 entries via [`all_ids`](core/engine/src/book.rs)). CLI has `openings list / allow PAT / deny PAT / reset / selected`; desktop needs an equivalent inside the New Game dialog so each game can pick its own subset.
- **Teaching-note overlay** — separate `book_notes.toml` keyed by `(eco, name)` with short prose blurbs the GUI surfaces alongside the book line.
- **Desktop UI for opening status** — today the only desktop surface is a stderr log on book moves. Wants a "book: <opening>" badge in the move list or under the board.
- **"New game in book" REPL command** — CLI `openings allow/deny` only takes effect on the next game; a `new-game` REPL verb would re-create the cursor in the current REPL session.
- **Transposition-aware book matching** — current cursor uses move-prefix; transpositions miss. Low priority.

Locked-in book decisions:
- Book entries are discrete TSV rows, not branches.
- Per-ply matching is the only mode (commit `15bb2e8`).
- Default-allowed set is "every TSV entry."
- Seed is random per game, logged in the play prompt.
- London System and other system-by-piece-placement openings are out of scope for the book; system detection is a separate quality issue against [`openings.rs`](core/engine/src/openings.rs).

## Teaching layer, deferred

See [`core/engine/src/analysis/mod.rs`](core/engine/src/analysis/mod.rs) `//!` for full spec on:
- **Phase 2 — cheap-pass + surprise detection** (depth-1 qsearch + SEE for every legal move).
- **Phase 4 — signal-mask** (zero each `EvalTrace` term in turn, re-rank, surface "you'd prefer M' if you undervalued X").
- **Phase 5 — tactic library** (general patterns: pin / fork / skewer / double attack / discovered attack / etc., parallel to `traps/`). **This is the engine-side prerequisite for the richest visual annotations.** Specifically the spatial data (which squares, which arrows) needs to come out of this.

Additional:

- **Drill-down API for compound eval terms.** [`TermId`](core/engine/src/analysis/term_id.rs) collapses ~100+ raw SF11 signals into 47 chess-concept buckets. The narrator sometimes needs to explain *why* a compound term moved — e.g., "your KingDanger went up 80 cp because an enemy bishop now hits the long diagonal and your knight-defender just moved." Design sketch: opt-in `Option<&mut DetailedTrace>` analogous to today's `Some(&mut trace)` pattern, queried only by narrators explaining swings above some threshold. First target: `KingDanger`'s 16-signal blend.
- **Rubinstein trap** — user wants to work out its invariants first. Belongs in the trap library ([`core/engine/src/traps/`](core/engine/src/traps/) — see that module's `//!` for the four-gate validator schema). With trap state now in Session, the GUI gets trap surfacing for free as soon as new entries are added to the library.

## UX / platform, deferred

- **Visual annotations on the board** — the current-focus work above. `BoardView.annotations` field + view-builder reads from existing facts (pinners/blockers bitboards, trap entries, retrospective PVs).
- **Hint panel narration via narration crate refactor.** Hint panel currently shows `mv / score / PV` directly from `MoveAnalysis`. A richer narration should reuse the per-term narrators. Factor `narration::render_report`'s middle section into `render_per_term_narration(out, pre_move_pos, candidate, root_stm)`; expose `format_candidate_explanation(...)` without verdict / engine-preferred framing.
- **Real piece sprites** (cburnett, CC-BY-SA from Lichess). 12 SVGs, `include_bytes!`, drop-in for the desktop's `piece_glyph` mapping in `draw::board`.
- **Bot strength / customization framework.** Long-term: configurable openings, blunder profile, tactical eyesight per bot. Same data shape as the existing `OpponentProfile`; this is about presets + UI, not engine work.
- **FFI crate (`core/ffi/`).** First concrete step toward Apple/Android. Outstanding decisions: UniFFI vs. raw C ABI, in-process vs. out-of-process, how to expose `MoveAnalysis` (and now `BoardAnnotation`) across the boundary.
- **Mobile shells (`apple/`, `android/`).** Consume `chess_tutor_ui` via `core/ffi`. Each platform is a renderer + event dispatcher, ~hundreds of lines, not thousands. Gated on the FFI crate.

### Locked-in design decisions

- **Engine produces facts; renderers render.** Narration crate is one renderer (text); desktop's `draw::*` is another (egui); a future mobile shell is a third. `core/ui` carries the facts as raw data on `HistoryEntry` and via Session accessors — no formatting in the shared layer.
- **Events name intents, not inputs.** `Cancel`, `RequestNewGame`, `Takeback`, `JumpToLive`, `SelectSquare(sq)`, `ConfirmNewGame{...}` — never `EscapePressed` / `NewGameClicked` / `BoardClicked`. See [memory feedback_ui_events_intent_not_input.md](../.claude/projects/C--Users-steve-Repos-work-chess-tutor-2/memory/feedback_ui_events_intent_not_input.md).
- **Cancel resolution lives in the session, not the renderer.** Priority order: promotion picker > dialog > deselect. Renderer just emits `Cancel`.
- **Dialog form: payload-on-confirm.** `ConfirmNewGame { color, fen, depth, noise, eval_mask }` rather than per-field events. Validation (FEN parse, depth bounds) is the session's job. The desktop's egui dialog gets a `&mut NewGameForm` borrow for in-place widget editing — a concession to immediate-mode UI; a platform that can't borrow session state across frames would need a `UpdateNewGameDraft` route added.
- **Worker remains in the shared layer.** Only the `RepaintFn` callback is platform-flavoured.
- **CLI uses Session via blocking helpers** (`wait_for_worker`, `run_analysis`). Sync-feeling REPL on top of an async worker; the GUI keeps polling.

## Live-play tuning

Every retrospective narrator has unit tests for shape, but the wording and thresholds were picked *a priori*. Continued real-game playthrough is how they get tuned. CLI `play` and the desktop GUI retrospective panel are both wired for this. When playing, the most useful failure-mode categories to report:

- **Engine *said* X but narration didn't surface it** → narrator-prose tuning.
- **Narration surfaced X but you can't tell *why* X moved** → drill-down API gap (compound terms).
- **You made move M, engine preferred M', but you don't understand the *category* of mistake** → Phase 4 signal-mask gap.
- **Hint panel told you nothing useful** → hint panel narration refactor.
- **Wording felt off / patronising / vague** → cheapest fix; just tune the strings.
- **(NEW)** **You could see *that* something was wrong but not *where* on the board** → visual annotation gap. This is the working motivation for the visual learning elements push.
