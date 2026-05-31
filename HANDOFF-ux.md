# Handoff: chess-tutor-2 — UX / teaching layer

Forward-looking UX context. The product surface is teaching feedback, not the engine. See [`HANDOFF.md`](HANDOFF.md) for the index, [`CLAUDE.md`](CLAUDE.md) for mission + ground rules, [`HANDOFF-perf.md`](HANDOFF-perf.md) for engine perf (only if perf becomes relevant to a UX task).

> **The teaching UX is functional end-to-end with the engine surface.** The card-based retrospective, the three learning-mode axes, the live coaching panel, the intervention pause, board overlays, the trapped-piece overlay, and the tactic/mate cards have all landed (git history carries the wave-by-wave detail; inline `//!` docs carry design rationale). This file tracks **tuning, iteration, and surfaces not yet wired.**

## Learning-mode workflows — current state

Three orthogonal axes for how much the student is guided, in [`LearningPreferences`](core/ui/src/learning_mode.rs), driving three side-panel surfaces (Retrospective, Coaching, Game Review) plus an in-game intervention pause:

- **`AssistanceLevel`** — Off / Prophylactic (*not yet wired* — currently behaves like Off) / **Coached** (live "features to notice" panel; never names a move).
- **`MistakeHandling`** — SilentRetrospective (default) / TeachingMoments / AllMistakes. Drives whether the engine reply pauses after a user move.
- **`BlunderSafety`** — Off / OfferTakeback. Independent of the teaching axis; catches realized material loss.
- **`reveal_best_moves: bool`** (default `false`) — controls whether the retrospective shows the engine's preferred move (SAN, chip, arrow). Off by default: telling the student the answer trains rote memorisation; the per-category cards explain *why* without showing *what*.

Named presets ([`LearningPreset`](core/ui/src/learning_mode.rs)): Practicing / Supported / Coached / Custom. Picker in the side panel; **not yet in the New Game dialog** (backlog).

### The engine-side classifier

[`classify_user_move`](core/engine/src/analysis/move_assessment.rs) is the gate, returning [`MoveAssessment { blunder, teaching }`](core/engine/src/analysis/move_assessment.rs) (both `Option`, independent):

- **Blunder**: realized material loss ≥ 300 cp (`MaterialOutcome::realized_net_mg_cp`), carrying the lost-piece square.
- **Teaching moment**: per-[`TermId`](core/engine/src/analysis/term_id.rs). Must pass the hopeless gate (`best.score > −500 cp`) + noise floor (drop ≥ 30 cp), then one of three triggers fires in priority order: (1) **multi-term** — top two cover ≥ 75% of the drop, both ≥ 25 cp (surfaces both concepts); (2) **absolute-severity** — a single term ≥ 50 cp regardless of share; (3) **single-term dominance** — one term ≥ 60% of the drop and ≥ 25 cp. `MaterialPieceValue` is excluded (the blunder gate handles it). `TermFamily::of(dominant.term)` recovers the category-level group.

### Intervention pause

Data flow: `apply_user_move` → queue Retrospective worker job → on result, `classify_user_move` → `intervention_required(&assessment, &prefs)?` → either continue (queue engine) or set `Session::pending_intervention`. User dispatches `ContinueDespitePrompt` / `RevealMissedConcept` / `TakeBackDuringIntervention`. The [`Intervention`](core/ui/src/view.rs) panel ([`build_intervention_panel`](core/ui/src/learning_mode.rs)) takes priority over Retrospective/Hint while pending. Prompts come from the per-`TermId` `term_prompt_copy` table (44 cases, first-pass wording) — concrete enough to act on without naming the move.

### Coached mode — live coaching panel

[`coaching_view.rs`](core/ui/src/coaching_view.rs) `build_coaching_view(pos, user_color)` — pure snapshot, no search, sub-ms. Shown when assistance == Coached, viewing live, user's turn, game in progress, no higher-priority body active. Surfaces today: "your king is in check" (with checker arrows + response count); "en passant available"; "look for a capture" (opponent's loose pieces, legal-move-filtered); "watch your loose piece" (ours, not filtered — threat is next turn); pawn-weakness cards either side (8 cp threshold); tactic-name hint (PV-reuse + static-scan fallback, High-confidence only, no square annotations); overloaded-piece card. `list_hanging`/`list_see_losing` in [`threats_outcome.rs`](core/engine/src/analysis/threats_outcome.rs) are pub for live consumption.

### Game Review

[`Session::build_game_review`](core/ui/src/session/view_builders.rs) walks every user move's cached analyses through the same classifier → ranked [`GameReviewView`](core/ui/src/view.rs). Gating uses `mistake_handling` (switch to AllMistakes before opening to widen). Click a moment → `JumpToReviewMoment(history_index)`.

### Pedagogical rules in force (codified across builders)

1. Pre-move coaching never names squares (`tactic_card` emits zero annotations).
2. Confidence-Medium tactics don't surface in coaching (gated before card construction; still appear in retrospective today).
3. Card prose uses chess vocabulary where precise (*fork*, *pin*), plain English where the signal doesn't fit (`pattern_phrase` / `pattern_lesson`). See memory [[feedback_teaching_terminology]].

### Signal-honesty constraints (the misfire-prone surfaces)

- **"You can win material" / "their piece loses to a trade"** read the `*_guaranteed` lists (`filter_guaranteed_targets` in [`threats_outcome.rs`](core/engine/src/analysis/threats_outcome.rs)) — an entry survives only if it holds against *every* legal opponent reply. **Known limitation:** the one-ply guarantee filter still passes when the opponent left a piece *en prise as a sacrifice* — every passive reply leaves the bait capturable. The fix needs a second-pass search of our capture + the opponent follow-up. See memory [[project_threat_signal_revisit]].
- **"You won/lost material"** uses `realized_net_mg_cp(root_stm)` scoped to ply ≤ 1 (the user's move + any forced recapture), not the full PV.
- **Material headline** uses classical point parity (P1 N3 B3 R5 Q9 via `PieceType::classical_points`); a `phase_dependent_trade_note` explains cp imbalance when point parity is even.
- **`filter_misleading_hangs`** drops `ours_*` entries that describe a planned recapture on our own ply-0 capture square, or a counter-attack where a higher-value target is the real threat.
- **Forced-consequences card** walks the user's PV one ply for pawn-structure concessions in the opponent's best reply ("if they reply gxh6, they get doubled pawns") — never "this forces."
- Back every win%-based gate with an absolute-cp gate: `win_chances` saturates near 1.0 in winning positions and silently suppresses real teaching moments. See memory [[feedback_winning_position_saturation]].

### Board overlays

Toggleable always-on highlights painted on the live/viewed position, independent of any card (collapsible "Board overlays" section; `Event::ToggleOverlay(OverlayKind)`; sticky, not disk-persisted). Available: My/Opponent space (two-tier), mobility-area-excluded, king rings, pins, attack heatmap, **trapped pieces** (`BadPiece` tint on each doomed piece + muted-red cage of dead escape squares, both sides under one toggle). Data: `compute_overlays(&Position) → OverlayData` → `overlays_view::push_overlay_annotations`. Engine cost: one `Evaluator` priming + a 64-square heat walk per frame (tens of µs; skipped when no overlays active). **To add an overlay:** a bitboard on `OverlayData`, an `OverlayKind` variant + label, a match arm in `overlays_view`, and (if a new colour) an `AnnotationKind` + entry in `draw::board::annotation_square_colors`.

### Card / annotation architecture

`build_retrospective_view(pre, &analyses, user_move, show_all) → RetrospectiveViewModel { headline, items }`; one bordered card per signal with sentiment strip + click-to-expand. Clicking paints the item's spatial story on the board. Key types all in [`core/ui/src/view.rs`](core/ui/src/view.rs): `RetrospectiveItem`, `RetrospectiveCategory`, `Sentiment`, `BoardAnnotation` (`Arrow`/`SquareHighlight`), `AnnotationKind`. `Session::collect_board_annotations` is the single point that populates `BoardView.annotations` (best-move arrow + selected item's annotations). `Session::selected_retrospective: Option<(history_index, item_index)>`, reset on navigation / new game.

`Session.show_all_signals` (default false) lowers per-card thresholds (mobility 20→1 cp with one card per piece-type per side; space 15→1 cp; "other shifts" drops the 50%-coverage filter). Toggle in the panel header.

**Per-card spatial-annotation status** (what's solid vs. still text-only):

| Category | Spatial annotations | Status |
|---|---|---|
| Threats | hanging/SEE square + attacker arrows | ✅ solid |
| Mobility | specific piece(s) + per-square New/LostMobility (via `per_piece_mobility` opt-in tracker) | ✅ solid |
| Space | per-side front + reinforced boxes (`space_bitboards`) | ✅ solid |
| Material | capture resolution squares | OK — no from→to arrows yet |
| King Safety | king-square highlight | OK — no ring squares / per-attacker arrows |
| Pawn Structure | none | needs work — expose squares whose sub-term status changed |
| Passed Pawns / Piece Placement | none | needs work — Score-driven, no square list |
| Secondary | none | OK (fallback list) |

**The per-piece disaggregation pattern** (how Mobility disambiguates "which bishop?"): an opt-in `Evaluator::per_piece_mobility: Option<Vec<(Square, Color, PieceType, Score, Bitboard)>>` (default `None`, zero search cost), populated only on analytical snapshots, read back per-piece by the outcome. Copy this shape for per-piece outpost squares, per-rook open-file, per-pawn structure events. See memory [[feedback_per_piece_disaggregation_pattern]].

### Architectural decisions worth knowing

- **`core/ui` does NOT depend on `core/narration`.** View-model logic reimplements some thresholds the narration crate has for text. Convergence (narration derives text from the view model) is a long-term refactor; today's duplication is accepted.
- **`format_retrospective` (CLI text) is untouched and parallel** — same engine outcomes, different presentation; reads `HistoryEntry.retrospective` directly.
- **Per-frame view-model rebuild** — `build_retrospective_view` recomputes every egui frame (8× evaluator priming, low-ms). Cache on `HistoryEntry` if it becomes a hotspot.
- **Annotation overlay is renderer-neutral** — `BoardView.annotations` is flat data; CLI's ANSI renderer ignores it; a future mobile shell paints its own way. No egui types in `core/ui`.
- **Engine produces facts; renderers render.** Narration (text), `desktop/draw::*` (egui), future mobile = three renderers over the same `HistoryEntry` facts. Events name intents, not inputs (`Cancel`, `RequestNewGame`, `SelectSquare(sq)` — never `EscapePressed`). See memory [[feedback_ui_events_intent_not_input]].
- **Tactics resolve before positional eval.** Chess is two modes; positional advice is only valid in a quiet position. The GUI must gate positional advice behind a tactical-mode check. See memory [[project_tactical_vs_positional_modes]].

## Backlog

### Learning-mode polish (rough priority order)

1. **Persistence design** — storage trait + per-platform impls (filesystem desktop, Core Data / Room mobile), past-games sidebar, user erase / clear-history UX. Foundation for drills + per-concept mastery fading. **Needs a design conversation before code lands** (storage semantics + delete model differ across platforms). User-flagged as the gate for this work. *(This is the "open thread" HANDOFF.md points at.)*
2. **Intervention prompt wording iteration** — `term_prompt_copy` strings are first-pass; real play surfaces where they read patronising/vague. One match expression.
3. **Coached-mode scope expansion** — outposts (`PiecesBreakdown.outposts`), bad bishops (bishop-pawns term), restricted mobility (per-piece tracker), king-safety attacker imbalances, weak squares. Each is a new builder in `coaching_view.rs`.
4. **Coaching "positives" cards** — currently only weaknesses; a "their knight on d5 is a beautiful outpost" / "your rook on the open file" surface helps the student see why a position is good/hard.
5. **Apply readability treatment to retrospective cards** — same `.small().weak()`→legible fix that coaching got. Open question with the user.
6. **Pre-select retrospective card on intervention continue** — auto-select the card matching `TermFamily::of(dominant_term)` so the spatial story shows without hunting.
7. **Learning-mode picker in the New Game dialog** — today only the side-panel picker exists; the dialog needs it for first-launch onboarding.
8. **Phase-dependent trade note: piece-pair framing** — today just cp numbers; add "bishops favor open positions / endgames", "the bishop pair is a long-term asset" for common pairs.

### Retrospective visual surfaces (rough value order)

1. **Pawn-structure highlights** — expose the *squares* of pawns whose sub-term status changed (became doubled/isolated/...) → `SquareHighlight { Bad/GoodPiece }`.
2. **Passed-pawn / piece-placement squares** — same shape: small helpers to expose passed-pawn squares, outposts, trapped-rook squares, weak-queen square.
3. **Material capture arrows** — re-walk `MoveAnalysis.pv` from `pre_move_pos` in `build_material_item` to recover from-squares → `Arrow { Capture }`.
4. **Wire Initiative / Blocked Center / Castling cards** — compute functions exist (`compute_initiative_outcome` etc.); copy the existing builder pattern.
5. **Pin arrows on live position** — `Arrow { Pin }` from `Position::blockers_for_king(us)` when no card selected; cheap, lives in `collect_board_annotations`.
6. **Trap-refutation arrows** — when `pending_trap.is_some()`, parse the trap's main-line SAN → arrows for the punisher + reply.
7. **Trap-threat warnings** — `Session::trap_threats()` returns candidate-uci + `TrapHit`; surface as red square on the at-risk candidate.
8. **Detail-prose convergence** — card `detail` strings duplicate narration wording; eventually have narration derive text from the view model (one source of truth).

### Tactic-library tuning (real-play feedback expected)

- **Walked-into framing** ("if they reply, they get a fork") — may read as nagging; iterate once real games surface it.
- **Mate-pattern detail** — only BackRank / Smothered have detail prose; others ride as heading suffix. Expand when a named-mates teaching pass lands.
- **Confidence::Medium retrospective rollout** — Medium hits still appear in retrospective (coaching is High-only); tighten retrospective to High if misfires come in.
- **PV-freshness gate cost** — the coaching tactic hint silently disappears when the bot deviates from PV[1]; relax to "within X cp of PV[1]" if it fires too rarely.

### Future teaching surfaces (deferred, durable home)

- **`win_chances` adoption** — the `win_chances.rs` sigmoid (lila cp→win%, `k = −0.00368208`) **exists**; the deferred work is *using* it as the threshold to gate which retrospective cards show + expressing blunder/missed-tactic thresholds in win%-lost. **Gotcha:** normalize our cp (PawnEG ≈ 213) to pawn = 100 first, and sanity-check `k` against our SF11-classical eval (lila fit it on NNUE). See memory [[project_win_chances_adoption]].
- **Latent-threat retrospective wiring** *(rescued from the retired PLAN-cli)* — `analysis::find_latent_threats` exists and is surfaced in the CLI (`tactics --latent`) + the `danger:` header, but is **not yet wired into `compute_tactic_outcome`'s `user_walked_into` slot.** Today `user_walked_into` requires the opponent to actually play the tactic; with latent detection the retrospective could fire pre-emptively against any user move that fails to disrupt a standing alignment (the `Qc5+`-walks-into-discovered-attack case). The pre-move coaching surface (`latent_threat_card`) is the other natural consumer.
- **Flank-classified attack signal** (needs design discussion) — kingside (files e–h) vs queenside (a–d) board *halves*, decoupled from king location (our `kingDanger` is king-centric). Pull SF11's `KING_FLANK` / `flank_attacks` when revisited. See memory [[project_flank_attack_classification]].
- **Named-endgame teaching library** — trap-library-shaped, built on the existing `endgame/` specialists (KPK opposition, KBNK right-corner, Lucena/Philidor). Rule text *attached* per recognized endgame, not derived (the bitbase stores win/draw, not the reason). Distinct from lichess endgame *tags* (material-bucket metadata we skip). See memory [[project_endgame_teaching_library]].
- **Named-mate teaching library** — Anastasia / Boden / etc. engine-available (`MatePattern`); 1200 student doesn't need them by name yet.
- **Overloaded retrospective surface** — `find_overloaded` shipped coaching-only per conservative rollout; promote to retrospective if real play shows the strict sole-defender-of-≥2 predicate doesn't misfire. See memory [[project_overloaded_detector]].
- **Drill-down API for compound eval terms** — `TermId` collapses ~100+ raw signals into 47 buckets; narrators sometimes need *why* a compound term moved (e.g. KingDanger's 16-signal blend). Opt-in `Option<&mut DetailedTrace>` analogous to today's trace pattern, queried only for above-threshold swings.
- **Rubinstein trap** — belongs in the trap library ([`core/engine/src/traps/`](core/engine/src/traps/), four-gate validator). User wants to work out its invariants first; GUI surfacing is free once the entry lands.
- **Zugzwang — DROPPED, don't re-propose as a detector.** It's a position state, not an exploitable tactic, and needs a search (static eval can't see it). See memory [[project_zugzwang_dropped]].

## Opponent profile / bot variability — landed, with follow-ons

All four pillars (skeleton / opening book / eval-signal mask / move-noise+blunder) landed May 2026; read [`opponent.rs`](core/engine/src/opponent.rs) `//!` for the strict invariant: **analytical paths (retrospective, hint, `analyze`) must never consult the profile.** Noise picks land on `HistoryEntry.noise_pick`. See memory [[project_opponent_profile_plan]].

Deferred follow-ons:
- **Visible per-move noise badge** in the desktop move list (data already on `HistoryEntry.noise_pick`; ~5 lines).
- **ELO presets** — `--bot-elo 1200` + a desktop "Preset" dropdown filling `(pool, temp, blunder, severity, guarantee, wild)`. Defer until manual knobs feel clunky in real play. See memory [[project_skill_level_and_multipv]] (also: MultiPV for variable-strength bots).
- **Opponent-side retrospective** — "the bot played a deliberate mistake — find the punishment" when `noise_pick.is_some()` and the delta is large. Needs a *scoped* exception to the analytical-paths invariant.
- **More aggressive defaults** — ship a ~800-ELO default once presets are tuned.
- **Seed surface in the GUI** — desktop logs the per-game seed to stderr; add a status line + paste-in field so varied games can be replayed.
- **Desktop UI for allowed-openings + opening-status badge + book teaching-note overlay** — CLI has `openings list/allow/deny`; desktop needs the equivalent inside the New Game dialog, plus a "book: <opening>" badge. Transposition-aware book matching is low priority.

## UX / platform, deferred

- **FFI crate (`core/ffi/`)** — the prerequisite for the platform apps. Outstanding decisions: UniFFI vs raw C ABI, in-process vs out-of-process, how to expose `MoveAnalysis` + `BoardAnnotation` across the boundary.
- **Mobile shells (`apple/`, `android/`)** — consume `chess_tutor_ui` via `core/ffi`; each is a renderer + event dispatcher (~hundreds of lines). Gated on the FFI crate.
- **Hint-panel narration via narration-crate refactor** — hint panel shows `mv / score / PV` directly; factor `narration::render_report`'s middle into `render_per_term_narration(...)` and expose `format_candidate_explanation(...)` without verdict framing.
- **Real piece sprites** (cburnett, CC-BY-SA from Lichess) — 12 SVGs, `include_bytes!`, drop-in for the desktop's `piece_glyph` mapping.
- **Teaching-layer Phases 2 & 4** (see [`analysis/mod.rs`](core/engine/src/analysis/mod.rs) `//!`) — Phase 2 cheap-pass + surprise detection (depth-1 qsearch + SEE per legal move); Phase 4 signal-mask (zero each `EvalTrace` term, re-rank, surface "you'd prefer M' if you undervalued X"). Phase 5 (tactic library) has largely landed.

## Live-play tuning

Every narrator has shape tests, but wording + thresholds were picked a priori — real-game playthrough is how they get tuned (CLI `play` + desktop GUI). Most useful failure-mode categories to report:

- **Engine said X but narration didn't surface it** → narrator-prose tuning.
- **Narration surfaced X but you can't tell *why* X moved** → drill-down API gap (compound terms).
- **You made M, engine preferred M', but you don't understand the *category* of mistake** → Phase 4 signal-mask gap.
- **Hint panel told you nothing useful** → hint-panel narration refactor.
- **Wording felt patronising / vague / wrong** → cheapest fix; tune the strings.
- **You could see *that* something was wrong but not *where* on the board** → visual-annotation gap (the working motivation for the visual learning push).
