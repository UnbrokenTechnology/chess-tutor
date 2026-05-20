# Handoff: chess-tutor-2 — UX / teaching layer

Forward-looking UX context. The product surface is teaching feedback, not the engine. See [`HANDOFF.md`](HANDOFF.md) for the index, [`CLAUDE.md`](CLAUDE.md) for the mission and ground rules, and [`HANDOFF-perf.md`](HANDOFF-perf.md) for engine perf state (read only if perf becomes relevant to a UX task).

## Current focus: teaching UI iteration

The engine is now performant enough for the planned mobile use case: at depth 12–14 the GUI feels real-time, retrospective is sub-300 ms on hard positions and ~100 ms on typical middlegames, full d=20 bench is 43 s with 8 threads (was an unfinishable multi-hour run a week ago). Further engine perf work has diminishing returns relative to the teaching-UX work that is now the bottleneck on the actual product.

See [`core/engine/src/analysis/mod.rs`](core/engine/src/analysis/mod.rs) `//!` for the design brief on the move-analysis pipeline (Phase 2 cheap-pass / surprise detection, Phase 4 signal-mask, Phase 5 tactic library) and the `narration` crate for the prose layer. Continued real-game playthrough is how the wording and thresholds get tuned — every retrospective narrator has unit tests for shape but the prose itself was picked a priori.

## Opponent profile / bot variability

Goal: ship bot-tuning toggles so games aren't deterministic from move 1, and so the student can practice against specific openings or weakened opponents. All four pillars — A (skeleton), B (opening book), C (eval signal mask), D (move noise + blunder) — have landed. Read the [`opponent.rs`](core/engine/src/opponent.rs) module doc for the strict invariant: **analytical paths (retrospective, hint, `analyze`) must never consult the profile** — they need to judge the user's move against true best play.

Phase D surface (delivered 2026-05-16):
- 7 [`NoiseProfile`](core/engine/src/opponent.rs) knobs, all-off by default. Three branches, evaluated in this order — **blunder → wild → softmax**. Blunder is the calibrated mistake signal (always picks a worse-than-best move when it fires), so it gets first crack; wild is chaotic and fills whatever budget remains.
  - **Blunder branch** (`blunder_chance: f32`, `blunder_min_loss_cp: i32` default 100, `blunder_max_loss_cp: i32` default 400): pick uniformly from engine-considered lines whose loss vs #1 falls in the band `[min, max]`. When no line falls in the band the picker pools the line(s) with the largest loss strictly below the band's lower edge with the line(s) with the smallest loss strictly above the upper edge, and picks from that pool — but lines further from the band on either side are excluded. The above-tier admission additionally has a **tolerance cap** (`BLUNDER_FALLBACK_TOLERANCE = 2.0×` max-loss): a closest-above line is admitted only if its loss is at most `max_loss × tolerance`. In positions where the only non-#1 alternatives are catastrophic (e.g. engine sees a forcing tactic, every other move loses 20+ pawns), the cap rejects them all and the blunder is **skipped** entirely — bot plays #1, and a stderr log notes that the configured rate is being slightly under-delivered. That's the load-bearing property: bots configured for "small blunders only" never throw away a queen when the only sub-band alternative is a piece sacrifice. Mate-guarded.
  - **Wild branch** (`wild_chance: f32`): per-move probability of picking uniformly from **all legal moves**, bypassing the engine ranking entirely. The only branch that can pick a move the search didn't surface — i.e. genuinely beginner-level mistakes like leaving a piece in a pawn's path. Mate-guarded.
  - **Softmax branch** (`candidate_pool: usize`, `temperature_cp: i32`): Boltzmann-weighted sampling over the top-K when both pool > 1 and temperature > 0.
  - Plus `guaranteed_mate_in: u32` (default 1) — suppresses blunder + wild branches when the bot sees a mate up to and including that depth, so mate-in-1 is never thrown away.
- [`noise::pick`](core/engine/src/noise.rs) — pure function `(profile, seed, ply, &lines, &legal_moves) -> NoisePick`. `NoisePick::Line(idx)` for normal/sampled/blunder picks; `NoisePick::Wild(mv)` when the wild branch fires. Deterministic given `(seed, ply)`; per-game seed is logged so a varied game can be replayed by passing `--seed N` back.
- CLI flags: `--noise-pool N`, `--noise-temp CP`, `--blunder-chance F`, `--blunder-severity CP`, `--wild-chance F`, `--guaranteed-mate-in N`. REPL `noise [show | pool N | temp CP | blunder F | severity CP | wild F | guarantee N | reset]`. Noise-driven engine moves are annotated `[noise: #K of N (-XX cp)]` (sampled) or `[noise: wild — engine preferred X (+Y)]` (wild) so the student knows the bot is off the best line.
- Desktop: reads `self.opponent.noise` when queuing the play search; sets `params.multi_pv = noise.effective_multi_pv()`. Worker computes legal moves, calls the picker, reports `NoisePickInfo::Sampled` or `NoisePickInfo::Wild` (logged to stderr for now; visible per-move tag in the move list is a follow-on). Wild moves get no `engine_info` badge in the move list (no search-line for that exact move).
- Desktop New Game dialog: full settings UI with sliders for the six noise knobs + collapsible eval-mask checkboxes + "Reset bot to defaults" button. The dialog auto-opens at first launch (no Cancel — only path forward is to commit a configuration) so the first thing the user does is pick difficulty. Subsequent New Game clicks pre-populate from the current game, so tweaking between games is incremental rather than from-scratch.
- Perf: off-profile is no-overhead — `is_off()` short-circuits and the engine keeps the single-PV fast path. When sampling is on, MultiPV costs roughly `K×` the single-PV time per move. Wild + softmax-only profiles keep the single-PV fast path because wild doesn't read the lines; only `blunder_chance > 0` widens MultiPV to at least [`BLUNDER_POOL_MIN`](core/engine/src/opponent.rs) (6).

Phase D follow-on, deferred:
- **Visible per-move noise tag in the move list.** Worker reports `NoisePickInfo` but it's only logged to stderr. The GUI move list should show a small badge ("noise: wild" or "blunder #6") on the corresponding move so the student can see at a glance which bot moves were deliberately weakened.
- **ELO presets**. `--bot-elo 1200` (CLI) and a "Preset" dropdown in the desktop dialog that fills in `(pool, temp, blunder, severity, guarantee, wild)` for you. Initial values were sketched in the design discussion: 100 ELO is wild-heavy, 1400 ELO is mostly softmax with a tiny blunder rate. Defer until the manual knobs feel clunky in real play (so we can tune the preset values from actual playthroughs).
- **Opponent-side retrospective**. Retrospective currently only fires on USER moves. A separate "the bot just played a deliberate mistake — can you find the punishment?" line when `noise_pick.is_some()` and `delta_from_top_cp <= -blunder_min_loss_cp` would be a powerful teaching surface, but requires resolving the analytical-paths invariant (the analytical search would need to know what the bot's noise profile is *for the user's retrospective only*, not for the bot's own decision).
- **More aggressive defaults**. Current defaults are all-off; once we have ELO presets and they're tuned from playthrough, the desktop dialog could default to a middle preset (~800 ELO) so a fresh install gives a more human-feeling opponent out of the box.
- **Seed surface in the GUI.** Desktop logs the seed to stderr but doesn't show it in the UI; players who want to replay a varied game can't easily copy the seed back. Add a status line under the move list or in the New Game dialog with the active seed + a way to paste one in to replay.

Phase C surface (delivered):
- 8 toggleable [`EvalCategory`](core/engine/src/opponent.rs) values: `pawn-structure`, `pieces`, `mobility`, `king-safety`, `threats`, `passed-pawns`, `space`, `initiative`. Material and imbalance are deliberately not exposed (disabling them produces gibberish play, not a teaching scenario).
- CLI: `--disable-eval CATEGORY[,CATEGORY...]` startup flag + REPL `eval-mask list / disable CAT / enable CAT / reset` (toggles take effect on the next engine move).
- Desktop: reads `self.opponent.eval_mask` when queuing the play search; no UI for editing yet — wants a settings panel mirroring the CLI surface.
- Perf: TT=16 1T d=13 bench identical to pre-Phase-C (8 per-category branches fold under branch prediction on the empty-mask hot path).

Opening-book follow-on work, deferred:
- **Desktop UI for allowed-openings selection (highest priority).** Default is now "every theoretical opening in the TSV" (~3,900 entries via [`all_ids`](core/engine/src/book.rs)). Users will want a settings panel to narrow that set — both *positive* selection ("I want to practice the Caro-Kann this session") and *negative* ("never play the Sicilian against me, I don't know the theory"). The CLI already has `openings list / allow PAT / deny PAT / reset / selected`; the GUI needs an equivalent surface, ideally as a filter list inside the New Game dialog so each game can pick its own subset without leaving the table. The underlying mechanism (`BookSelection::Allowed(Vec<OpeningId>)`) and the per-ply matching engine already support arbitrary subsets.
- Teaching-note overlay — separate `book_notes.toml` keyed by `(eco, name)` with short prose blurbs the GUI surfaces alongside the book line. Empty to start; populate the marquee openings first.
- Desktop UI for opening status — today the only desktop surface is a stderr log on each book move. Wants at minimum a "book: <opening> (ECO Name)" badge in the move list or under the board so the user sees the opening name at a glance.
- "New game in book" REPL command — CLI `openings allow/deny` only takes effect on the next game; a `new-game` REPL verb would re-create the cursor in the current REPL session.
- Transposition-aware book matching — current cursor uses move-prefix matching, so games that transpose into a curated line via a different move order won't be recognised. Low priority (most book moves are reached via the canonical order; teaching-tool users typically play standard sequences).

Decisions locked in:
- Book entries are discrete TSV rows, not branches — "Caro-Kann Variation X" is its own opening, separate from "Caro-Kann".
- Per-ply matching is the only mode (no game-start pre-commit) — see commit `15bb2e8` for the rationale.
- Default-allowed set is "every TSV entry" — see commit landing this note for the rationale (the 8-entry curated default was too narrow; users want variety AND the freedom to filter).
- Seed is random per game, logged in the play prompt; pass `--seed <n>` to replay.
- London System and other piece-placement-defined "systems" are out of scope for opponent profile; system detection is a separate quality issue against [`openings.rs`](core/engine/src/openings.rs).

## Teaching layer, deferred

See [`core/engine/src/analysis/mod.rs`](core/engine/src/analysis/mod.rs) `//!` for full spec on:
- **Phase 2 — cheap-pass + surprise detection** (depth-1 qsearch + SEE for every legal move).
- **Phase 4 — signal-mask** (zero each `EvalTrace` term in turn, re-rank, surface "you'd prefer M' if you undervalued X").
- **Phase 5 — tactic library** (general patterns: pin / fork / skewer / double attack / discovered attack / etc., parallel to `traps/`).

Additional:

- **Drill-down API for compound eval terms.** [`TermId`](core/engine/src/analysis/term_id.rs) collapses ~100+ raw SF11 signals into 47 chess-concept buckets. The narrator sometimes needs to explain *why* a compound term moved — e.g., "your KingDanger went up 80 cp because an enemy bishop now hits the long diagonal and your knight-defender just moved." Design sketch: opt-in `Option<&mut DetailedTrace>` analogous to today's `Some(&mut trace)` pattern, queried only by narrators explaining swings above some threshold (per-node cost paid only on rare detailed paths). First target: `KingDanger`'s 16-signal blend.
- **Rubinstein trap** — user wants to work out its invariants first. Belongs in the trap library ([`core/engine/src/traps/`](core/engine/src/traps/) — see that module's `//!` for the four-gate validator schema).

## UX / platform, deferred

- **Hint panel narration via narration crate refactor.** Hint panel currently shows `mv / score / PV`; richer narration should reuse the per-term narrators. Factor `narration::render_report`'s middle section into `render_per_term_narration(out, pre_move_pos, candidate, root_stm)`; expose `format_candidate_explanation(...)` without verdict / engine-preferred framing.
- **Real piece sprites** (cburnett, CC-BY-SA from Lichess). 12 SVGs, `include_bytes!`, drop-in for `piece_glyph` callers.
- **Promotion picker UI.** Currently auto-queens. Inline 4-piece overlay near the target square is standard.
- **Visual annotations on retrospective.** GUI eventually draws arrows / highlights tied to specific narrator clauses. Requires changing narration output from flat `String` to a list of clauses with optional annotation payloads (square sets, arrows, kind tag).
- **Bot strength / customization framework.** Long-term: configurable openings, blunder profile, tactical eyesight per bot.
- **FFI crate (`core/ffi/`).** First concrete step toward Apple/Android. Outstanding decisions: UniFFI vs. raw C ABI, in-process vs. out-of-process, how to expose `MoveAnalysis` across the boundary.

## Platform-portable UI refactor (in progress)

Goal: collapse the GUI/CLI duplication and put `chess-tutor-desktop` on the same footing as future Apple/Android renderers — each platform shell becomes a thin unidirectional renderer of view descriptors, with all session state + game logic in shared Rust. CLI's `play.rs` (1,718 lines) and `desktop/src/main.rs` (previously 1,927 lines) duplicate game state today; the refactor pays off twice over.

Steps 1–4 have landed:

- **Steps 1–3** built the platform-portable split: `desktop/src/main.rs` shrunk from 1,927 lines to ~75; `core/ui` exports [`Session`](core/ui/src/session.rs), [`view`](core/ui/src/view.rs) descriptors, [`event::Event`](core/ui/src/event.rs), and a `RepaintFn` callback. desktop's `App` is a thin newtype wrapping `Session`.
- **Step 4** put the CLI on the same `Session`. `core/cli/src/board.rs` now consumes a `BoardView` (same descriptor the egui shell paints) and `core/cli/src/play.rs` shrunk from 1,718 lines to ~870 by delegating position / history / opponent / book to `Session`. The CLI keeps its own retrospective rendering (deeper / configurable via `--retrospective-depth` and `--no-explain-best`) and its own analytical engine for `search` / `analyze` REPL commands.

Session API surface added for the CLI: `Session::start_game(pos, EngineMode, depth, OpponentProfile)`, `wait_for_worker()` (blocking variant of `poll_worker`), `set_log_to_stderr(false)` and `set_auto_retrospective(false)` opt-outs, `play_user_move(Move)`, and accessors for `position` / `history` / `opponent` / `is_engine_thinking` / `game_outcome`. `engine_plays: Option<Color>` became `engine_plays: EngineMode` to support self-play (`--engine-color both`).

Two minor CLI surface changes from the migration:
- `undo` now rewinds the user-move + engine-reply pair (matching the desktop's takeback) instead of one ply at a time.
- The `--time-ms`, `--reset-engine-per-move`, and `--search-progress` flags were dropped — time-budget violates the determinism contract anyway, and the diagnostics had no Session equivalent.

Remaining step:

1. **Mobile shells** (`apple/`, `android/`) consume `chess_tutor_ui` via `core/ffi`. Each platform is a renderer + event dispatcher, ~hundreds of lines, not thousands. The FFI crate itself is a separate prerequisite — outstanding decisions (UniFFI vs. raw C ABI, in-process vs. out-of-process, how to expose `MoveAnalysis` across the boundary) are tracked in the "FFI crate" entry above.

### Locked-in design decisions

- **Events name intents, not inputs.** `Cancel`, `RequestNewGame`, `Takeback`, `JumpToLive`, `SelectSquare(sq)`, `ConfirmNewGame{...}` — never `EscapePressed` / `NewGameClicked` / `BoardClicked`. The shared layer is consumed by GUI, CLI, and (eventually) mobile; input-mechanism names are lies in at least one of those. See [memory feedback_ui_events_intent_not_input.md](../.claude/projects/C--Users-steve-Repos-work-chess-tutor-2/memory/feedback_ui_events_intent_not_input.md).
- **Cancel resolution lives in the session, not the renderer.** Priority order: promotion picker > dialog > deselect. Renderer just emits `Cancel`.
- **Dialog form: payload-on-confirm.** `ConfirmNewGame { color, fen, depth, noise, eval_mask }` rather than per-field events. Validation (FEN parse, depth bounds) is the session's job. Add a `UpdateNewGameDraft` route only if a platform's framework forces session-owned form state.
- **`piece_glyph` ships as a helper, not in the view.** Descriptors carry `Piece`; renderers pick Unicode / sprite / SVG. The shared layer can offer `piece_glyph(Piece) -> char` for CLI/prototype use.
- **Worker remains in the shared layer.** Only the repaint callback is platform-flavoured.

## Live-play tuning

Every retrospective narrator has unit tests for shape, but the wording and thresholds were picked *a priori*. Continued real-game playthrough is how they get tuned. CLI `play` and the desktop GUI retrospective panel are both wired for this. When playing, the most useful failure-mode categories to report:

- **Engine *said* X but narration didn't surface it** → narrator-prose tuning.
- **Narration surfaced X but you can't tell *why* X moved** → drill-down API gap (compound terms).
- **You made move M, engine preferred M', but you don't understand the *category* of mistake** → Phase 4 signal-mask gap.
- **Hint panel told you nothing useful** → hint panel narration refactor.
- **Wording felt off / patronising / vague** → cheapest fix; just tune the strings.
