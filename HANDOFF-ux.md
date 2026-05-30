# Handoff: chess-tutor-2 — UX / teaching layer

Forward-looking UX context. The product surface is teaching feedback, not the engine. See [`HANDOFF.md`](HANDOFF.md) for the index, [`CLAUDE.md`](CLAUDE.md) for the mission and ground rules, and [`HANDOFF-perf.md`](HANDOFF-perf.md) for engine perf state (read only if perf becomes relevant to a UX task).

> ▶️ **First UI wiring pass landed 2026-05-27.** Five surfaces from the prior "NEXT" list shipped: trapped-piece overlay, retrospective Tactic + mate cards, coaching tactic hint (PV-reuse, no new search), and overloaded-piece coaching card. See [`HANDOFF.md`](HANDOFF.md) "First UI wiring pass" for the exact landing list. **The teaching UX is now functional end-to-end with the engine surface; this handoff is here for tuning, iteration, and the surfaces not yet wired** (mate-pattern detail beyond default-surfaced, walked-into framing iteration, overloaded retrospective surface if misfire rate stays low in real play).

## Current state: learning-mode workflows (2026-05-25)

The product now has three orthogonal axes for how much the student is guided during play, named [`LearningPreferences`](core/ui/src/learning_mode.rs). They drive three side-panel surfaces (Retrospective, Coaching, Game Review) plus an in-game intervention pause. The 2026-05-20 retrospective card system is the foundation — every new surface reuses its categories, sentiments, and annotation layer.

### The three axes

- **`AssistanceLevel`** — Off / Prophylactic (not yet implemented) / **Coached**. When `Coached`, a live coaching panel surfaces features-to-notice for the current position on the user's turn. Never names a move.
- **`MistakeHandling`** — SilentRetrospective (default) / TeachingMoments / AllMistakes. Drives whether the engine reply pauses after a user move so the student can re-examine or take back.
- **`BlunderSafety`** — Off / OfferTakeback. Independent of the teaching axis; specifically catches realized material loss.
- **`reveal_best_moves: bool`** (default `false`) — controls whether the retrospective shows the engine's preferred move (SAN, score chip, board arrow). Off by default — telling the student the answer trains rote memorisation; the per-category cards explain *why* the move was inaccurate without showing *what* to play.

Named presets in [`LearningPreset`](core/ui/src/learning_mode.rs): Practicing / Supported / Coached / Custom. Picker lives in the side panel; the New Game dialog doesn't yet have one.

### The engine-side classifier

[`chess_tutor_engine::analysis::classify_user_move`](core/engine/src/analysis/move_assessment.rs) is the gate. Returns [`MoveAssessment { blunder: Option<BlunderInfo>, teaching: Option<TeachingInfo> }`](core/engine/src/analysis/move_assessment.rs) — both fields independent.

- **Blunder**: realized material loss ≥ 300 cp from the engine `MaterialOutcome::realized_net_mg_cp`. Carries the lost-piece square so the prompt can highlight.
- **Teaching moment**: per-[`TermId`](core/engine/src/analysis/term_id.rs) — surfaces a specific, concrete concept the student can act on. The position must always pass the hopeless gate (`best.score > -500 cp` default — when even the best move loses badly, taking back doesn't help) and the noise floor (drop ≥ 30 cp). One of three trigger scenarios must then fire, evaluated in priority order:
  1. **Multi-term** — top two terms cover ≥ 75% of the drop with both clearing the severity floor (25 cp). Surfaces both concepts. Catches the "two distinct weaknesses in one move" case (40/40/20-split).
  2. **Absolute-severity escape** — a single term clears 50 cp on its own. Surface it regardless of share. Catches the "one loud signal alongside scattered smaller ones" case.
  3. **Single-term dominance** — a single term carries ≥ 60% of the drop *and* clears 25 cp severity. The original concept-shaped gate.

`TeachingInfo.dominant: TermContribution` is the primary term; `TeachingInfo.secondary: Option<TermContribution>` is populated only by Scenario 1. The broader [`TermFamily`](core/engine/src/analysis/move_assessment.rs) groups (KingSafety, PawnStructure, PiecePlacement, Mobility, Threats, PassedPawns, Space, Initiative, Development, Imbalance, Material) are useful for category-level renderers and are recoverable via `TermFamily::of(dominant.term)`. `MaterialPieceValue` is excluded from teaching-moment gating because the blunder gate already handles it.

### Intervention pause data flow

```
apply_user_move(mv)
   │
   ▼ (sets awaiting_intervention_decision = true when learning prefs care)
queue Retrospective worker job
   │
   ▼ (worker returns)
handle_worker_result(Retrospective)
   │ classify_user_move(pre_pos, &analyses, mv, gating_config_for(prefs))
   ▼
intervention_required(&assessment, &prefs)?
   ├─ false → maybe_queue_engine_search()  (game continues normally)
   └─ true  → Session::pending_intervention = Some(PendingIntervention)
              (maybe_queue_engine_search holds while .is_some())

User dispatches one of:
   ContinueDespitePrompt → clear pending, queue engine
   RevealMissedConcept  → set concept_revealed=true (no state change)
   TakeBackDuringIntervention → undo move, clear pending
```

The intervention panel ([`SidePanelBody::Intervention`](core/ui/src/view.rs) + [`build_intervention_panel`](core/ui/src/learning_mode.rs)) takes priority over Retrospective and Hint surfaces while `pending_intervention.is_some()`. Prompts are built from `TeachingInfo.dominant_term` via the per-`TermId` `term_prompt_copy` table in [`learning_mode.rs`](core/ui/src/learning_mode.rs) — concrete enough that the student can act on the prompt without the engine telling them the move. Wording is first-pass and ready to iterate from real play.

### Game Review surface

[`Session::build_game_review`](core/ui/src/session/view_builders.rs) walks every user move's cached retrospective analyses through the same classifier and returns a ranked [`GameReviewView`](core/ui/src/view.rs) of significant moments. The classifier's gating uses `LearningPreferences.mistake_handling` — switching to AllMistakes before opening Review widens the list. Top-bar "Review Game" button (enabled once any user move's retrospective has arrived) toggles `Session::game_review_open`. Click a moment → `JumpToReviewMoment(history_index)` sets `viewing_index` and pre-selects nothing (user clicks through cards themselves).

### Coached mode — live coaching panel

[`core/ui/src/coaching_view.rs`](core/ui/src/coaching_view.rs) — `build_coaching_view(pos, user_color) → CoachingViewModel`. Pure snapshot of the live position, no engine search, sub-ms in release. Shown when `coaching_should_show()`: assistance == Coached, viewing live, user's turn, game in progress, no higher-priority body active (intervention / game review / hint).

What it surfaces:
1. **"Your king is in check"** card (when `pos.in_check()`) with checker arrows, king-square highlight, and a count of king-move vs. block-or-capture responses. Never names a move.
2. **"En passant capture available"** card when any legal move is `MoveKind::EnPassant`. Computes the captured-pawn square as `Square::new(to.file(), from.rank())`; highlights both that square and the destination so the unusual geometry is visible.
3. **"Look for a capture"** card — opponent's undefended pieces, **filtered by the legal-move destination set** so in-check / pinned-attacker cases don't surface false opportunities.
4. **"Watch your loose piece"** card — our undefended pieces. Not legal-move-filtered (the threat is about opponent's next turn, which depends on what we move first).
5. **Pawn weakness** cards on either side — doubled / isolated / backward / weak-unopposed pawns. Threshold 8 cp (lower than the retrospective's 15 cp because structural concessions are inherently small-cp but pedagogically valuable).

`list_hanging` and `list_see_losing` in [`threats_outcome.rs`](core/engine/src/analysis/threats_outcome.rs) are pub for live (pre-user-move) consumption.

Currently narrow in scope: doesn't yet name outposts, weak squares, bad bishops, restricted mobility, king-safety attacker imbalances. Adding any of these is a new builder in `coaching_view.rs` — the infrastructure (`CoachingViewModel`, side panel wiring, engine helpers) is in place.

### Retrospective tuning landed this session

Several "the card is technically true but misleading" cases got capture-aware suppression:

- **Material card now uses classical point parity** (P:1, N:3, B:3, R:5, Q:9 via [`PieceType::classical_points`](core/engine/src/types.rs)) instead of cp net for the headline. B↔N is "Even trade"; B-for-R is "You won material." When point parity is even but cp lean ≥ 30 in either mg or eg, a [`phase_dependent_trade_note`](core/ui/src/retrospective_view.rs) explains the imbalance ("the engine reads this slightly in your favor — -0.61 pawns at endgame phase").
- **`filter_misleading_hangs`** drops `ours_hanging` / `ours_see_losing` entries that describe either (1) a planned recapture on the same square as our ply-0 capture (Bxh6 → bishop on h6 not really "hanging") or (2) a counter-attack where `theirs_hanging_guaranteed` has a strictly higher-value target (we hang a bishop while threatening the queen — opponent's best response addresses the queen, not the bishop).
- **King-protector card** suppresses for the captured side when any minor of that side came off the board (arithmetic from removal, not repositioning), and suppresses the "drifted away" variant for the capturing side when our ply-0 was a minor's capture (the drift enabled the capture).
- **Forced-consequences card** ([`build_forced_consequences_items`](core/ui/src/retrospective_view.rs)) walks the user's PV one ply and surfaces pawn-structure concessions in the opponent's best reply. "If they reply gxh6, they get doubled pawns." Threshold 8 cp because the doubled-pawn penalty is small-cp but pedagogically valuable. Never says "this forces" — only "if they reply."

### Coaching panel UI styling (2026-05-25)

The coaching panel's intro and card prose were originally `.small().weak()` (≈10pt faded grey), which the user reported as illegible. Now: intro/empty-state use `.italics()` only at default size; card heading 15pt bold; card summary `.weak()` only (no size shrink); card detail default size. Same `.small().weak()` pattern still applies to the **retrospective panel** — open question whether to apply the same treatment there too.

### What changed at a glance — files

- New: [`core/ui/src/learning_mode.rs`](core/ui/src/learning_mode.rs), [`core/ui/src/coaching_view.rs`](core/ui/src/coaching_view.rs), [`core/engine/src/analysis/move_assessment.rs`](core/engine/src/analysis/move_assessment.rs).
- Touched: `core/ui/src/{session,event,view,retrospective_view,lib}.rs`, `core/engine/src/analysis/{mod,threats_outcome}.rs`, `core/engine/src/types.rs`, `desktop/src/draw/{side_panel,top_bar}.rs`.

### Next polish items (learning-mode work)

In rough order:

1. **Persistence design** (long-standing #5 from the original sequencing). Storage trait + per-platform impls (filesystem on desktop, Core Data / Room on mobile), past-games sidebar, user erase / clear-history UX. Foundation for drill modes and per-concept mastery fading. **Needs a design conversation before code lands** — storage semantics differ across platforms, and the user-facing "delete this game" model deserves thought (per-game delete vs. clear-all vs. retention policy). User has explicitly flagged this as the gate for #5 implementation.
2. **Intervention prompt wording iteration.** Strings in [`term_prompt_copy`](core/ui/src/learning_mode.rs) are first-pass keyed by `TermId` (44 cases). Real play will surface where the wording feels patronising / vague / wrong. Easy to iterate; strings live in one match expression.
3. **Coached mode scope expansion.** Surface outposts (engine knows them via `PiecesBreakdown.outposts`), bad bishops (bishop pawns term), restricted mobility (per-piece tracker exists), king-safety attacker imbalances, weak squares (squares no enemy pawn can attack). Each is a new builder in `coaching_view.rs`.
4. **Apply readability treatment to retrospective panel cards.** Same `.small().weak()` pattern as coaching had pre-fix. Question still open with the user.
5. **Threat-signal sacrifice-tactic check** — the one-ply guarantee filter still mis-says "you can win material" when the opponent left a piece as a sacrifice to set up a tactic. See [memory project_threat_signal_revisit.md](.claude/projects/C--Users-steve-Repos-work-chess-tutor-2/memory/project_threat_signal_revisit.md).
6. **Coaching "positives" cards.** Currently only surfaces weaknesses. A "their knight on d5 is a beautiful outpost" / "your rook on the open file" surface would help the student see why their position is hard / good.
7. **Pre-select retrospective card on intervention continue.** When user continues past a teaching moment, automatically select the card matching `TermFamily::of(dominant_term)` so they can see the spatial story without hunting.
8. **Learning-mode picker in New Game dialog.** Today only the side-panel picker lets the user pick a preset; the dialog should have it for first-launch onboarding clarity.
9. **Phase-dependent trade note: piece-pair framing.** Today it just gives cp numbers. Adding "bishops favor open positions and endgames" / "the bishop pair is a long-term asset" for the common piece-pair cases would make the note much more pedagogically valuable.

### Known limitations

- **Counter-attack hang filter only catches discovered-attack-style patterns.** If our hanging piece IS the only attacker on the high-value target, the guaranteed list (correctly) doesn't include the target — opponent's "take our piece" response removes the threat. That's the right behavior, but the student might still find the trade-off worth taking; we don't have a "positional compensation" surface for non-material counter-threats (e.g. forcing checkmate threats, huge positional gains).
- **Coaching `Prophylactic` assistance level exists in the enum but isn't yet wired.** The `Coached` level shows features-to-notice; `Prophylactic` was meant to show opponent threats specifically. For now selecting `Prophylactic` produces the same panel state as `Off`.
- **CLI doesn't have the intervention / game-review surfaces.** Both are desktop-only. The CLI flows still work through the original retrospective text path.

---

## Tactic library — landed 2026-05-27

The full taxonomy (W4-impl waves 1–6, see memory `project_tactic_library_reference`) is engine-available and the UI is wired through. The architecture stabilized at:

- **Engine**: [`compute_tactic_outcome`](../core/engine/src/analysis/tactic_outcome/mod.rs) returns `{ user_played, user_missed, user_walked_into }` from one PV-walk per move. Mate geometry rides alongside on `TacticHit.mate_pattern`; sacrifice classification rides on `TacticHit.sacrifice`. Single-line variant `find_tactic_in_line` powers the coaching surface.
- **Retrospective**: `RetrospectiveCategory::Tactic` cards in [`tactic.rs`](../core/ui/src/retrospective_view/tactic.rs). Played/missed/walked-into each get a card. Missed-tactic annotations suppressed when `reveal_best_moves` is off — the heading still names the pattern, the squares stay hidden.
- **Coaching**: `Session::coaching_tactic_hint()` (queries.rs) mines the previous retrospective's `analyses[user_move].pv[2..]` and gates on `history[u+1].mv == pv[1]` (a fresh-PV check that costs nothing). When that hits, [`coaching_view::tactic_card`](../core/ui/src/coaching_view.rs) names the pattern with no spatial annotations — student must locate it themselves.

### Pedagogical rules in force (codified across both builders)

1. Pre-move coaching never names squares — `tactic_card` produces zero annotations by design.
2. Confidence-Medium tactics don't surface in coaching — gated in `build_coaching_view` before card construction.
3. Card prose uses chess vocabulary where it's precise (*"fork"*, *"pin"*); plain English where the engine's signal doesn't fit (see `pattern_phrase` / `pattern_lesson` in both files).

### Tuning followups (real-play feedback expected)

- **Walked-into framing**: currently "If they reply, they get a fork" — may need iteration once real games surface where the framing reads as nagging vs. instructive.
- **Mate-pattern detail**: only BackRank and Smothered have detail prose. Other surfaced mates ride as heading suffix only. Expand when a named-mates teaching pass lands.
- **Confidence::Medium retrospective rollout**: today Medium hits still appear in retrospective (only coaching filters High-only). If misfire feedback comes in, tighten retrospective to High too.
- **PV-freshness gate cost**: today the coaching tactic hint silently disappears when the bot deviates from PV[1] (any cause). If real play shows this firing too often (bots picking different tie-broken moves at the same eval), relax to "noise_pick.is_none() or bot move within X cp of PV[1]".

### What stayed deferred

- **Zugzwang**: too search-expensive for live use. Memory `project_zugzwang_dropped` (it's a position state, not an exploitable tactic).
- **Named-mate teaching library**: 1200 student doesn't need Anastasia / Boden / etc. by name yet. Engine-available; UI exposure is later.
- **Overloaded retrospective surface**: shipped as coaching-only per the conservative-rollout note in memory `project_overloaded_detector`. Promote to retrospective if real play shows the predicate doesn't misfire.

---

## Backlog: future teaching surfaces (post-W4)

Confirmed-but-deferred teaching features surfaced in discussion (2026-05-27). None were in the W4-impl engine waves (now complete); this is their durable home. Roughly ordered by readiness, not priority.

1. **`win_chances` adoption (near-term).** Port lichess/lila's cp→win-probability sigmoid (`win% = 2/(1+e^(k·cp)) − 1`, `k = −0.00368208`, lila PR #11148 — an empirically *fitted* constant, a numerical fact). Primary use the user endorsed: **a threshold to gate which retrospective cards show**, and to express blunder / missed-tactic thresholds in win-probability lost rather than raw cp (so the bar means the same thing at equality and at +1500). Folds into W4-impl wave 3 (it's also the lever for the one-ply-guarantee misfire fix). **Gotcha:** our internal cp scale is PawnEG ≈ 213, not the conventional pawn = 100 the constant assumes — normalize to pawn = 100 before applying, and sanity-check / consider refitting `k` against our SF11-classical eval (lila fit it on NNUE evals). See memory `project_win_chances_adoption`.

2. **Overloaded-piece detector (build-from-scratch).** chess.com surfaces "a piece is overloaded so you can win material," so it's a parity target the user wants — accuracy of chess.com's own detector is unknown, treat ours as independent. **lichess's `cook.py:overloading` is a `return False` stub** (never implemented), so there's nothing to transliterate: design our own pre-move scan — find an enemy piece defending ≥ 2 of our winnable targets such that forcing it off one (a deflection) cashes the other. Distinct from removing-the-defender (which *captures* the sole defender); overloading leaves the piece on the board and *forces a choice*. Moderate misfire risk → start `Medium` confidence, retrospective-only. Listed as W4-impl wave 6; this note records the user's specific interest + the chess.com-parity motivation. See memory `project_overloaded_detector`.

3. **Flank-classified attack signal (needs design discussion).** Distinguish *which wing* an attack is on — kingside (files e–h) vs queenside (a–d), board *halves*, independent of where the king sits. Our existing `kingDanger` is **king-centric** (pressure on the enemy king's ring wherever it is): it tells us *whether* there's an attack, not *which flank*. SF11 has a middle ground worth pulling when we revisit: `KING_FLANK` + the `flank_attacks` king-safety sub-term measure attacks on *the king's own flank*, but there's no free-standing "queenside attack while the king is elsewhere" (e.g. minority attack). lichess's `side_attack` is wing-aware but ties the wing to the king's corner. **What we'd build:** a wing-classified pressure metric (attackers / open & half-open files / space bucketed by board-third) decoupled from king location. Deferred conversation — design not settled. See memory `project_flank_attack_classification`.

4. **Named-endgame teaching library (separate from the lichess audit).** A trap-library-shaped surface — curated, named endgames with rule-of-thumb text — built on our **existing** endgame specialists in [`core/engine/src/endgame/`](core/engine/src/endgame/). Those specialists already *encode* the technique (the `PUSH_TO_EDGES` / `PUSH_CLOSE` / `PUSH_TO_CORNERS` gradients; the KPK bitbase that implicitly knows opposition and wrong-rook-pawn draws), but the knowledge is used only to *play*, never to *explain*. The teaching gap: KPK opposition ("king in front of the pawn"), KBNK right-colour-corner mate, Lucena/Philidor in rook endings, stalemate-avoidance rules of thumb. Note the bitbase stores win/draw, **not the reason** — so rule text is *attached* per recognized endgame, not derived; that's exactly the named-trap pattern. Distinct from lichess's endgame *tags* (`pawnEndgame`/`rookEndgame`/…), which are pure material-bucket metadata we correctly skip. See memory `project_endgame_teaching_library`.

---

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
- **Space cards** drop their per-side threshold from 15 cp (`SPACE_DELTA_THRESHOLD_CP`) to 1 cp, so the +14 cp shifts that happen when a single new reinforced square appears at full piece count show up. Each side gets its own card when its delta crosses threshold (no "dominant side wins" rule).
- **"Other shifts"** drops its 50%-coverage `cumulative_prefix` filter and lists every non-zero residual term.

Toggle lives in the retrospective panel header (`desktop/src/draw/side_panel.rs`); emits `Event::ToggleShowAllSignals`. Sticky for the session, no disk persistence yet. The flag flows into `build_retrospective_view(pre, &analyses, user_move, show_all)`.

### Board overlays (2026-05-21)

Six toggleable, always-on overlays that paint structured highlights on the live (or historically-viewed) position, independent of any retrospective card. UI lives in a collapsible "Board overlays" section above the retrospective panel; each checkbox emits `Event::ToggleOverlay(OverlayKind)`. The toggle set is sticky across moves, not persisted to disk.

Overlays available:
- **My space / Opponent's space** — two-tier (front + reinforced) tints, teal/blue for ours, amber/orange for theirs. Both can be on at once.
- **Mobility area (excluded)** — paints the squares NOT in `Evaluator::mobility_area[us]`. Muted grey. Shows what the engine considers "dead" for mobility-counting purposes.
- **King rings** — both kings' 3×3 boxes (clamped to b2..g7 interior). Reuses `AnnotationKind::KingRing`.
- **Pins** — pieces in `Position::blockers_for_king(us)` for both sides. Magenta tint via new `AnnotationKind::Pin`.
- **Attack heatmap** — per-square net-attacker tint. Green for our advantage, red for theirs; intensity steps at |net| = 1 vs ≥ 2 via `HeatOurs1/2` + `HeatTheirs1/2` kinds.

Data flow:

```
chess_tutor_engine::analysis::compute_overlays(&Position) → OverlayData
                                                            (12 bitboards: space/mobility-excluded/king-ring/pinned × both colours; heat × 4 tiers)
                                            │
                                            ▼
core/ui/src/overlays_view.rs::push_overlay_annotations
            (&mut Vec<BoardAnnotation>, &OverlayData, us: Color, &HashSet<OverlayKind>)
                                            │
                                            ▼
Session::collect_board_annotations    (calls compute_overlays(viewed_pos) once
                                       and routes per-overlay-kind dispatch
                                       through overlays_view)
```

POV-flip: `Session::user_color()` returns `!engine_side` when engine plays one colour, else `viewed_pos.side_to_move()`. Overlays use that as `us`. So "My space" stays on white when you're playing white, regardless of whose move it is.

Engine cost: one full `Evaluator` priming per frame (initialize × 2 + pieces::evaluate × 2) plus a 64-square `attackers_to` walk for the heatmap. Tens of µs in release. Skipped entirely when `active_overlays.is_empty()` — no overhead when off.

What an overlay needs to add later (per signal): a bitboard on `OverlayData`, a new `OverlayKind` variant + label/description, a `match` arm in `overlays_view::push_overlay_annotations`, and (if a new colour is needed) an `AnnotationKind` variant + entry in `desktop/src/draw/board.rs::annotation_square_colors`.

#### Trapped-piece overlay (landed 2026-05-27)

The flagship: a 1200 can't *see* when a piece — especially a deep enemy queen — has run out of safe squares. **Shipped end-to-end** as `OverlayKind::TrappedPieces`: `BadPiece` tint on each doomed piece + muted-red `TrappedEscape` tint on the cage of dead escape squares closing in. Both sides painted under one toggle (mirrors `KingRings` / `Pins`). Engine helper `trapped_cages(pos, colour)` is still around for any future arrow-from-attacker surface; the bitboard pair on `OverlayData` is the cheap rendering path.

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
| Mobility        | `SquareHighlight { GoodPiece / BadPiece }` on the **specific** piece(s) whose per-square mobility delta aligns with the card, **plus** `SquareHighlight { NewMobility / LostMobility }` on each square that piece newly attacks (or no longer attacks). Uses `Evaluator::per_piece_mobility` opt-in tracker, which carries the per-piece `attacks & mobility_area` bitboard; the view builder diffs pre/post bitboards per highlighted piece. | ✅ Solid.    |
| Pawn Structure  | None (text-only).                                                                                                         | Needs work. |
| Passed Pawns    | None (Score-driven, no square list).                                                                                      | Needs work. |
| Piece Placement | None yet — one card per `PiecesBreakdown` sub-signal × side (outpost claimed, rook on open file, bishop blocked by own pawns, etc.) above 20 cp; bishop_pawns suppressed when geometry unchanged. Each card knows its target square type but doesn't yet emit highlights. | Needs spatial work; cards themselves are honest. |
| Space           | Up to two cards per move (one per side, fired independently when each side's delta crosses threshold), each painting only that side's post-move space: `SquareHighlight { SpaceFront }` for the safe c-f × 3-rank box squares, `SquareHighlight { SpaceReinforced }` for the subset on/behind own pawns unattacked by any enemy piece. Driven by `SpaceOutcome.{ours,theirs}_{safe,reinforced}_post` bitboards exposed by `eval::space::space_bitboards`. | ✅ Solid.    |
| Secondary       | None — it's the fallback "Helped / Hurt" list, not spatial.                                                              | OK as-is.   |

### How the mobility per-piece tracker works (engine-side)

A real example of the trick we used to disambiguate "which bishop?" when an aggregate breakdown collapses per-piece detail.

- **`Evaluator::per_piece_mobility: Option<Vec<(Square, Color, PieceType, Score, Bitboard)>>`** in [`core/engine/src/eval/mod.rs`](core/engine/src/eval/mod.rs). The trailing `Bitboard` is `attacks & mobility_area` — the precise set of squares that counted toward the popcount.
- Default `None` — `pieces::evaluate`'s mobility loop checks `if let Some(vec) = e.per_piece_mobility.as_mut()` and pushes only when populated. Single tagged-union test, branch-predicts to skip; bench unchanged (≈2.4 Mnps single-thread depth-13).
- `compute_mobility_outcome` sets it to `Some(Vec::new())` for the analytical snapshot, then reads back per-piece records into `MobilityOutcome.ours_per_piece_pre/post` and `theirs_per_piece_pre/post`. Each `PieceMobility` carries `square`, `piece`, `mg`, and `mobility_squares: Bitboard`.
- View builder (`highlight_specific_pieces` in [`core/ui/src/retrospective_view.rs`](core/ui/src/retrospective_view.rs)) keys by square: same-square pre/post → per-square delta; post-only (the moved piece) → full post score. Filters to deltas aligned with the card's sentiment + above 15 cp threshold; falls back to the largest aligned contributor if nothing crosses. Each picked piece also emits per-square `NewMobility` / `LostMobility` highlights from the pre/post `mobility_squares` diff (same-square piece) or the full post bitboard (moved piece, positive sentiment only — its from-square footprint isn't recoverable without re-running attacks against the pre position).

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
4. **Wire Initiative / Blocked Center / Castling cards.** The narration crate has these; the view builder doesn't yet build cards for them. Compute functions exist (`compute_initiative_outcome`, etc.); copy the pattern from the existing builders. (Space landed 2026-05-21 — see the table above for the two-tier highlight pattern; that's the template for any other category whose primary teaching surface is "where on the board.")
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
