# Handoff: chess-tutor-2 — UX / teaching layer

Forward-looking UX context. The product surface is teaching feedback, not the engine. See [`HANDOFF.md`](HANDOFF.md) for the index, [`CLAUDE.md`](CLAUDE.md) for the mission and ground rules, and [`HANDOFF-perf.md`](HANDOFF-perf.md) for engine perf state (read only if perf becomes relevant to a UX task).

## Current state: interactive card-based retrospective (2026-05-20)

The retrospective panel is no longer a wall of monospace text. The desktop now renders one bordered card per teaching signal (material, threats, king safety, mobility, pawn structure, passed pawns, piece placement, secondary terms), with a sentiment-tinted strip, glyph, heading, score-delta chip, and click-to-expand detail. Clicking a card paints the item's spatial story (square highlights + arrows) on the board.

### Signal honesty pass (2026-05-21)

Two teaching surfaces were tightened so they describe what actually resolved / is guaranteed, not engine speculation:

- **"You can win material" / "Their piece loses to a trade"** used to fire off the static [`ThreatsOutcome::theirs_hanging`](core/engine/src/analysis/threats_outcome.rs) / `theirs_see_losing` snapshots — true at the moment after our move, but routinely refuted by the opponent's next move (1.Nf3 attacks e5, ...Nc6 defends). The engine now also computes `theirs_hanging_guaranteed` / `theirs_see_losing_guaranteed` via [`filter_guaranteed_targets`](core/engine/src/analysis/threats_outcome.rs), which keeps an entry only if it survives every legal opponent reply. Both UI cards and the CLI narrator (`render_threats` in [`threats_narration.rs`](core/narration/src/threats_narration.rs)) read the guaranteed lists; the raw lists stay available for callers that want the static view.
- **"You won material" / "You lost material"** used to walk the full PV up to `settled_ply` and could fire past-tense framings off a capture 15 plies deep. `MaterialOutcome` now exposes `realized_events()` / `realized_net_mg_cp(root_stm)` accessors that scope to ply ≤ 1 (the user's move plus any forced opponent recapture). The UI material card consumes the realized surface; CLI's `material_narration.rs` keeps using the full `events` slice because its framing is explicit ("Best line: …"), which is already honest about being hypothetical.

**Known limitation** (revisit later — see [memory project_threat_signal_revisit.md](../.claude/projects/C--Users-steve-Repos-work-chess-tutor-2/memory/project_threat_signal_revisit.md)): the one-ply guarantee filter still passes when the opponent left a piece en prise as a sacrifice to set up a tactic. Every passive reply leaves the bait capturable, so the filter calls it "guaranteed," but the right move for the student is to refuse the capture. Detecting this needs a second-pass that searches our response (take vs. refuse) and evaluates the position after the opponent's follow-up.

### "Show all signals" preference

`Session.show_all_signals: bool` (default `false`) drives two behaviors when on:

- **Mobility cards** surface one card per piece type per side (up to 8) instead of just the biggest shift. Default threshold is 20 cp (`MOBILITY_DELTA_THRESHOLD_CP` in [`retrospective_view.rs`](core/ui/src/retrospective_view.rs)); under "Show all" the threshold drops to 1 cp so a bishop's 12→13 reach surfaces too.
- **"Other shifts"** drops its 50%-coverage `cumulative_prefix` filter and lists every non-zero residual term.

Toggle lives in the retrospective panel header (`desktop/src/draw/side_panel.rs`); emits `Event::ToggleShowAllSignals`. Sticky for the session, no disk persistence yet. The flag flows into `build_retrospective_view(pre, &analyses, user_move, show_all)`.

### Data flow at a glance

```
                              MoveAnalysis  (engine search output, raw)
                                    │
                                    ▼
    core/engine/src/analysis/*_outcome.rs    ──► structured outcomes
    (compute_threats_outcome, compute_mobility_outcome, …)
                                    │
                                    ▼
    core/ui/src/retrospective_view.rs        ──► RetrospectiveViewModel
    (build_retrospective_view)                   { headline, items: [RetrospectiveItem…] }
                                    │
                  ┌─────────────────┴───────────────────┐
                  ▼                                     ▼
         RetrospectiveKind::                Session::collect_board_annotations
         UserMoveReady{view_model,          reads the *selected* item's
                       selected_item}       annotations and the always-on
                  │                         best-move arrow → BoardView.annotations
                  ▼                                     │
         desktop/src/draw/side_panel.rs                 ▼
         draws cards, emits                  desktop/src/draw/board.rs
         Event::SelectRetrospectiveItem      paints arrows + square highlights
```

CLI text path (`chess_tutor_narration::format_retrospective`) is **untouched** and parallel — same engine outcomes, different presentation.

### Key types (all in `core/ui/src/view.rs`)

- **`RetrospectiveViewModel { headline, items }`** — top-level view model returned by `build_retrospective_view`.
- **`RetrospectiveHeadline`** — verdict label, sentiment, user/best scores, SAN annotation (`!`/`?`/`??`), optional teaching note. Carries `best_move_annotation: Option<BoardAnnotation>` for the always-on arrow.
- **`RetrospectiveItem { category, heading, summary, detail, score_delta_pawns, sentiment, annotations }`** — one card. `annotations` is the per-card spatial story.
- **`RetrospectiveCategory`** — `Material | Threats | KingSafety | PawnStructure | Mobility | PassedPawns | PiecePlacement | Initiative | BlockedCenter | Castling | Space | Secondary`. Drives card glyph + theming.
- **`Sentiment`** — `Positive | Negative | Mixed | Neutral`. Drives card border + chip color (green / red / amber / grey).
- **`BoardAnnotation`** — overlay layer on `BoardView`:
  - `Arrow { from: Square, to: Square, kind: AnnotationKind }`
  - `SquareHighlight { square: Square, kind: AnnotationKind }`
- **`AnnotationKind`** — `BestMove | Capture | Threat | Attacker | Defender | KingRing | GoodPiece | BadPiece | NewMobility | LostMobility | Highlight`. Each renderer maps to its own palette; desktop's mapping is in `draw::board::annotation_square_colors` + `arrow_color`.

### Selection state

`Session::selected_retrospective: Option<(history_index, item_index)>` tracks which card is selected. Driven by `Event::SelectRetrospectiveItem(usize)` (toggle: clicking the same card again deselects). Reset automatically on `ViewHistoryIndex` (browsing to a different move) and on `start_new_game`.

`Session::collect_board_annotations()` is the single point where the BoardView's annotation layer is populated. It pulls:
1. The viewed entry's `headline.best_move_annotation` (always-on if user wasn't best).
2. The selected item's `annotations` (when one is selected for the *currently viewed* entry).

### Per-card annotation status

What each card produces today, and what's still rough:

| Category        | Annotations                                                                                                              | Quality      |
|-----------------|--------------------------------------------------------------------------------------------------------------------------|--------------|
| Material        | Square highlight on each capture's resolution square.                                                                    | OK — no from→to arrows yet (would need to re-walk PV in the builder). |
| Threats         | `SquareHighlight { Threat / GoodPiece }` on the hanging/SEE-losing piece + `Arrow { Attacker }` from each attacker.       | ✅ Solid.    |
| King Safety     | `SquareHighlight { KingRing / GoodPiece }` on the king's square.                                                          | OK — could add ring squares + per-attacker arrows. |
| Mobility        | `SquareHighlight { GoodPiece / BadPiece }` on the **specific** piece(s) whose per-square mobility delta aligns with the card. Uses `Evaluator::per_piece_mobility` opt-in tracker; threshold + alignment filter. | ✅ Solid.    |
| Pawn Structure  | None (text-only).                                                                                                         | Needs work. |
| Passed Pawns    | None (Score-driven, no square list).                                                                                      | Needs work. |
| Piece Placement | None (same shape as passed pawns).                                                                                        | Needs work. |
| Secondary       | None — it's the fallback "Helped / Hurt" list, not spatial.                                                              | OK as-is.   |

### How the mobility per-piece tracker works (engine-side)

A real example of the trick we used to disambiguate "which bishop?" when an aggregate breakdown collapses per-piece detail.

- **`Evaluator::per_piece_mobility: Option<Vec<(Square, Color, PieceType, Score)>>`** in [`core/engine/src/eval/mod.rs`](core/engine/src/eval/mod.rs).
- Default `None` — `pieces::evaluate`'s mobility loop checks `if let Some(vec) = e.per_piece_mobility.as_mut()` and pushes only when populated. Single tagged-union test, branch-predicts to skip; bench unchanged (≈2.4 Mnps single-thread depth-13).
- `compute_mobility_outcome` sets it to `Some(Vec::new())` for the analytical snapshot, then reads back per-piece records into `MobilityOutcome.ours_per_piece_pre/post` and `theirs_per_piece_pre/post`.
- View builder (`highlight_specific_pieces` in [`core/ui/src/retrospective_view.rs`](core/ui/src/retrospective_view.rs)) keys by square: same-square pre/post → per-square delta; post-only (the moved piece) → full post score. Filters to deltas aligned with the card's sentiment + above 15 cp threshold; falls back to the largest aligned contributor if nothing crosses.

**This is the pattern** for surfacing piece-specific spatial annotations from any aggregate eval term. The same shape (opt-in `Option<Vec<...>>` tracker on `Evaluator`, populated only on analytical paths) is what to copy for: per-piece outpost squares, per-rook open-file detection, per-pawn structure events, etc.

### Architectural decisions worth knowing

- **`core/ui` does NOT depend on `core/narration`.** View-model logic in `retrospective_view.rs` reimplements some thresholds + categorization that the narration crate has for text. The alternative — having one depend on the other — makes both crates harder to evolve, since narration was designed as a sibling text renderer (same conceptual layer as `draw::board`). Convergence (have narration derive text from the view model) is a long-term refactor; today's duplication is accepted.
- **Per-frame view-model rebuild.** `Session::build_retrospective_view` recomputes the entire view model every egui frame from the stored `Vec<MoveAnalysis>`. Each `compute_*_outcome` does a fresh evaluator priming (`Evaluator::new` + `initialize(W)` + `initialize(B)` + `pieces::evaluate(W)` + `pieces::evaluate(B)`); doing this 8× per frame is ≈low-ms. If it becomes a hotspot, cache the view model on `HistoryEntry` keyed by a "have the analyses arrived" bit.
- **`format_retrospective` (CLI text) is untouched.** CLI tests (105 in narration) didn't move; the prose surface stayed identical. The CLI doesn't go through `SidePanelView::RetrospectiveKind::UserMoveReady` at all — it reads `HistoryEntry.retrospective` directly and formats with `format_retrospective`.
- **Selection persistence model is intentional.** Selection is tied to `(history_index, item_index)` rather than a content-based key so navigating away clears the highlight; coming back to the same move shows a clean board until you click again. If we want sticky selection, change the dispatch in `Session::dispatch::ViewHistoryIndex` to not null `selected_retrospective`.
- **Annotation overlay is renderer-neutral.** `BoardView.annotations` is a flat data list; the CLI's ANSI renderer just ignores it. A future iOS / Android shell paints its own way. No egui types leak into `core/ui`.

### Next polish items

In rough order of value:

0. **Threat-signal sacrifice-tactic check** — the one-ply guarantee filter in `filter_guaranteed_targets` still mis-says "you can win material" when the opponent left a piece as a sacrifice to set up a tactic. Add a second-pass that searches our candidate capture and evaluates the resulting position; if our eval drops below the material gain, drop the guarantee. See [memory project_threat_signal_revisit.md](../.claude/projects/C--Users-steve-Repos-work-chess-tutor-2/memory/project_threat_signal_revisit.md) for the full plan (and a few smaller refinements: fork/discovered-attack framing, target-chasing, pressure-list pass through the same filter).
1. **Pawn-structure highlights** — extend `PawnStructureOutcome` (or add a sibling function) to expose the *squares* of pawns whose sub-term status changed (became doubled, isolated, etc.). View builder turns those into `SquareHighlight { BadPiece / GoodPiece }`.
2. **Passed-pawn / piece-placement squares** — same shape: the engine outcomes are Score-driven; add small helpers to expose passed-pawn squares, outposts, trapped-rook squares, weak-queen square. Then the cards get spatial stories.
3. **Material capture arrows** — re-walk `MoveAnalysis.pv` from `pre_move_pos` inside `build_material_item` to recover from-squares; emit `Arrow { Capture }` for each capture. Today only the destination squares highlight.
4. **Wire Initiative / Blocked Center / Castling / Space cards.** The narration crate has these; the view builder doesn't yet build cards for them. Compute functions exist (`compute_initiative_outcome`, etc.); copy the pattern from the existing builders.
5. **Pin arrows on live position.** When no card is selected, draw `Arrow { Pin }` from `Position::blockers_for_king(us)` so threats are visible during normal play (not just when looking back at a move). Cheap — lives in `collect_board_annotations`. Add an `AnnotationKind::Pin` if you want a distinct color.
6. **Trap-refutation arrows.** When `pending_trap.is_some()`, parse the trap's main-line SAN back to `Move`s and emit arrows for the next punisher move + defender reply.
7. **Trap-threat warnings.** `Session::trap_threats()` returns candidate-uci + `TrapHit` for moves the user shouldn't play; surface as red square on the at-risk candidate.
8. **Detail prose convergence.** Card `detail` strings duplicate narration crate wording. Eventually have the narration crate derive text from the view model, so there's one source of truth for category copy.

### What's already available to consume (reference)

For future visual work, these `Session` accessors give you everything you'd need without new plumbing:

- **`Session::history()` → `&[HistoryEntry]`**:
  - `retrospective: Option<RetrospectiveResult>` — user moves, raw `Vec<MoveAnalysis>` with PVs, scores, per-term deltas, ply traces.
  - `engine_info: Option<EngineInfo>` — engine moves.
  - `noise_pick: Option<NoisePickInfo>` — when noise drove the bot off best.
  - `trap_events: Vec<TrapEvent>` — moves played during mid-trap refutation.
  - `trap_hit: Option<TrapHit>` — trigger move of a new trap.
- **`Session::pending_trap() → Option<&PendingTrap>`** — `.entry` (static TrapEntry with full refutation tree) + `.hit` (snapshot at trigger).
- **`Session::trap_threats() → Vec<TrapThreatened>`** — pre-move warnings for the live position; each carries `candidate_uci`, `candidate_san`, and the `TrapHit` you'd be handing the opponent.
- **`Session::run_analysis(pos, SearchParams) → AnalysisOutcome`** — blocking analytical search for ad-hoc queries.

Already-spatial data the engine produces:

- **`Move::from()` / `Move::to()`** — every move trivially renders as a from→to arrow.
- **`MoveAnalysis.pv: Vec<Move>`** — principal variation; first 1-2 plies make chained arrows.
- **`TrapHit.main_line_san`** — punisher's scripted refutation; parse back via `san::parse_on(&mut pos, san)`.
- **`Position::king_square(color)`** — for any king annotation.
- **`Position::blockers_for_king(us)`** — pieces of `us` pinned to their own king (bitboard).
- **`Position::slider_blockers(candidate_attackers, target)`** — fuller pin geometry: `(blockers, pinners)` for arbitrary target square.

What's missing (needs engine work) — **Phase 5 tactic library** in [`core/engine/src/analysis/`](core/engine/src/analysis/):

- **Forks / skewers / discovered attacks / double attacks / general hanging detection.** Each would output a structured `TacticHit` parallel to the trap library's `TrapHit`. Render as arrows + square highlights using the existing `BoardAnnotation` layer.

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

- **Visual annotations beyond what cards produce** — pin arrows on the live position, trap-refutation arrows, trap-threat warnings. The `BoardView.annotations` infrastructure is in place; these all live as additional readers inside `Session::collect_board_annotations`. See "Next polish items" in the current-state section above.
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
