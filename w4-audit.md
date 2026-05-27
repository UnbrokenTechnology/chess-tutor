# W4 — Broader lichess feature audit

> **Workflow 4 of [`ROADMAP.md`](ROADMAP.md).** A *research* pass, not an
> implementation pass. Deliverable: a port / reference / skip verdict on every
> lichess-puzzler component and every `cook.py` tag, split by **teaching value vs.
> mere puzzle-bucketing plumbing**, plus a concrete plan for the flagship
> **trapped-piece** feature (engine port + visual overlay). Implementation of the
> "port" verdicts is sequenced at the end and happens *after* this audit, per the
> 2026-05-26 user directive (UI surfacing is a still-later layer).
>
> Sources read: `tagger/cook.py` (30 tags + named-mate sub-detectors),
> `tagger/util.py` (predicate primitives), `tagger/zugzwang.py`,
> `generator/generator.py`, `generator/util.py`, `validator/` (TS web app).

---

## TL;DR verdicts

- **Already in the engine (W3 Ship 1):** Fork, Hanging-piece capture, Removing-the-defender, plus all the `util.py` primitives (`is_defended`/`is_hanging` w/ ray-defense, `can_be_taken_by_lower_piece`, `is_in_bad_spot`, `attacked_opponent_squares`, `king_values`). See [`analysis/tactic_outcome.rs`](core/engine/src/analysis/tactic_outcome.rs).
- **Port to engine (genuine teaching patterns), in priority order:**
  1. **Trapped piece** (FLAGSHIP — port `is_trapped` *and* ship a visual overlay).
  2. Pin (both flavours), Skewer, Discovered attack, Discovered check, Double check.
  3. Sacrifice classification (also fixes the long-standing one-ply-guarantee misfire).
  4. Deflection, Attraction, Interference (self + player), Intermezzo (zwischenzug), Clearance, X-ray.
  5. Back-rank mate + Smothered mate; remaining named mates engine-available but low-priority.
  6. Overloading — **lichess's `cook.py:overloading` is a `return False` stub**; we build our own from scratch (pre-move scan) or drop it.
  7. attackingF2F7 — a real 1200-relevant motif; cheap; nice-to-have.
- **Port one shared *utility*:** `win_chances` (the lila-tuned cp→win% sigmoid). Small, broadly useful (sacrifice/zugzwang thresholds, "how winning is this" teaching surface, solution-uniqueness gate). See [Utilities](#shared-utilities-worth-porting).
- **Port as an analytical-only detector (not per-node):** Zugzwang. Feasible — we have `do_null_move`/`undo_null_move` — but it costs a null-move eval-compare, so it runs only on the analytical path, never in search. Defer unless a position demands it.
- **Skip — puzzle-bucketing / scoreboard plumbing with no teaching payload:** the cp buckets (crushing/advantage/equality), mate-distance buckets (mateIn1..5), puzzle-length tags (oneMove/short/long/veryLong), endgame-type tags (pawn/queen/rook/bishop/knight/queenRook endgame), kingside/queenside-attack scoring heuristic, castling/promotion move-type tags, and the composer themes (quietMove, defensiveMove, checkEscape, exposedKing, advancedPawn, collinearMove). We already have eval / mate scores / phase / material that subsume the scored ones; the rest only exist to theme a puzzle database we don't have.
- **Skip — out of scope:** generator's PGN-mining pipeline (we generate from live play, not a game DB), tablebase prober (`tb.py` — we don't ship tablebases), validator web app (human review UI). All three are **reference-only** at most.
- **Reference (read, don't port):** generator's *uniqueness / "is this clearly the only move"* gate and its "already winning / already up material" suppression gates — directly applicable to **not nagging the student** when our retrospective claims "you missed THE move," but they're a design pattern, not portable code.

---

## Method: the teaching-value vs. plumbing split

The 2026-05-26 directive asks us to separate tags that are *pedagogically meaningful patterns* from tags that *only exist to bucket puzzles or compute difficulty*. The tell:

- **Teaching pattern** — describes a *reason a move works* that a 1200 student can learn and reuse ("your knight forks the king and rook"; "that queen has no safe square"). These must end up **engine-available** (the W4 deliverable). UI surfacing is later.
- **Plumbing** — exists so lichess can sort/label/difficulty-rate its puzzle DB: cp magnitude buckets, mate-distance buckets, solution-length, endgame-material type, "which corner the attack points at." A student learns nothing from "this is a "long" puzzle." We either already compute the underlying signal (eval, mate score, phase, material) or it's meaningless without a puzzle database.

Note the order `cook()` itself uses is *not* a priority order — it's just append-order into a tag list. Our priority is pedagogical load + misfire risk + 1200-relevance.

---

## Full `cook.py` tag verdict table

`pov` = the side to be taught (the solver). lichess walks `mainline[1::2]` (pov's moves at odd indices); our framing walks `MoveAnalysis.pv` with `pv[0]` played by `root_stm` — every ported predicate adapts to that (already established in `tactic_outcome.rs`).

| Tag (`cook.py`) | Kind | Verdict | Teaching value & notes |
|---|---|---|---|
| `fork` | tactic | **DONE** | Ship 1. `detect_fork`. |
| `hanging_piece` | tactic | **DONE** | Ship 1. `detect_hanging_capture` + `PriorMove`/`op_capture` recapture guard. |
| `capturing_defender` | tactic | **DONE** | Ship 1, exposed as `RemovingDefender`. |
| `trapped_piece` | tactic | **PORT — FLAGSHIP** | See [dedicated section](#flagship-trapped-piece). Port `util.is_trapped` + a board overlay. The standout: a 1200 can't *see* a deep enemy queen run out of squares. |
| `pin_prevents_attack` / `pin_prevents_escape` | tactic | **PORT (P2)** | Pin — one of the core-8. Derivable from `blockers_for_king` + ray geometry; two flavours (pinned piece can't *attack* a target / can't *escape* its own attacker). Both worth it; they teach distinct ideas. |
| `skewer` | tactic | **PORT (P2)** | Core-8. Ray piece captures through a higher-value front piece to a lower-value one behind, front piece sits in a bad spot. Needs `between_bb` (have it) + `ray_piece_types`. |
| `discovered_attack` / `discovered_check` | tactic | **PORT (P2)** | Core-8. Mover vacates a square on the line between a friendly ray piece and an enemy target/king. `discovered_check` is the cheap special case (checker ≠ the moved piece). |
| `double_check` | tactic | **PORT (P2)** | Trivial (`checkers().popcount() > 1` after the move) and very instructive — double check forces a king move. Bundle with discovered. |
| `sacrifice` | classification | **PORT (P3)** | Material down ≥ 2 after pov's *second* move, no promotion. This is the **fix for the one-ply-guarantee misfire** (memory `project_threat_signal_revisit`): a user move that loses material at ply 0 but wins by ply 4 is a *played* tactic (`Sacrifice` flag), not a missed one. Needs a flag on `TacticHit` + reads `MaterialOutcome` we already compute. |
| `deflection` | tactic | **PORT (P4)** | Distinct from removing-the-defender: lure a defender *off* a duty square. Geometric predicate (defender's attack-set, prior-move squares). Validated heuristic; port faithfully. |
| `attraction` | tactic | **PORT (P4)** | Lure K/Q/R onto a square, then attack/capture it. 4-step PV pattern; needs ≥ 4-ply line. Surface `Medium` confidence (deep). |
| `self_interference` / `interference` | tactic | **PORT (P4)** | Block the defender's ray to a hanging piece (by opponent's own piece, or by ours). Needs `between_bb` + ray-piece defender detection. One `Interference` pattern, two sub-cases (matches `cook()` OR-ing them into one tag). |
| `intermezzo` | tactic | **PORT (P4)** | Zwischenzug / in-between move. The capture happens via a move that wasn't the attacker of the capture square one ply earlier; the delayed capture was legal a ply before. High teaching value for 1200s ("don't auto-recapture"). |
| `clearance` | tactic | **PORT (P4)** | Move a ray piece off a square (without capturing) to clear a line, where the cleared-from square enables the tactic. Lower frequency; port with the P4 batch. |
| `x_ray` | tactic | **PORT (P4)** | X-ray / battery: capture on a square where a friendly piece behind the mover (on the same line, between from/to) re-captures. Needs `squares_are_collinear` (trivial) + `between_bb`. |
| `back_rank_mate` | mate pattern | **PORT (P5)** | Core-8 ("back-rank mate / mating net"). Detected at the mate position: king on back rank, escape squares blocked by own pieces / covered, checker on back rank. We get *the mate itself* free from search; this names *why*. |
| `smothered_mate` | mate pattern | **PORT (P5)** | Iconic, worth teaching: knight checkmate, all king-escape squares blocked by own pieces. Cheap at the terminal node. |
| `anastasia_mate`, `hook_mate`, `arabian_mate`, `boden_or_double_bishop_mate`, `dovetail_mate` | mate patterns | **PORT-available (P5, low UI priority)** | Per the 2026-05-26 directive "the mate patterns" should be **engine-available**; this supersedes the older HANDOFF-ux "deferred indefinitely (1200 doesn't need them)" note. Resolution: port the detectors (cheap, terminal-node-only) so they're *available*, but **do not** surface them in the 1200 student UI by default. They're a named-pattern library for later / for stronger users. |
| `mate_in` (→ mateIn1..5, "mate") | score/meta | **SKIP** | Mate distance is a *search output* (mate scores), not a pattern. The *fact* of a forced mate is hugely teaching-relevant and we already have it; the 1..5 bucketing is puzzle metadata. |
| `crushing` / `advantage` / `equality` | score bucket | **SKIP** | Pure cp-magnitude buckets. We have the eval; bucketing adds nothing. |
| `overloading` | tactic | **PORT-from-scratch or DROP** | ⚠️ `cook.py:overloading` is literally `return False` — lichess **never implemented it**. There is nothing to port. If we want it, we design our own pre-move scan ("a defender guarding ≥ 2 targets that can't cover both once forced"). Reasonable P4 candidate but flag that we're building, not transliterating. |
| `advanced_pawn` (advancedPawn) | theme | **SKIP** | "Solution contains a far-advanced pawn push." Composer theme; a passed/advanced pawn is already an eval term and a retrospective surface. |
| `double_check` | (listed above) | — | — |
| `quiet_move` | theme | **SKIP** | "A non-forcing, non-capturing waiting move appears in the solution." Puzzle-composition flavour; not a tactic a student *spots*. |
| `defensive_move` / `check_escape` | theme | **SKIP** | "Last move is a quiet defensive resource." Composer theme. |
| `attraction` | (listed above) | — | — |
| `exposed_king` | theme | **SKIP** | "Enemy king on an advanced exposed rank and a check happens." Heuristic theme; our king-safety eval + overlay already shows king exposure far better. |
| `collinear` (collinearMove) | theme | **SKIP** | Player keeps a ray piece on a line instead of capturing along it. Niche composer theme; not a teachable named tactic. |
| `attacking_f2_f7` | motif | **PORT (P6, optional)** | The classic beginner f2/f7 weak-square hit. Genuinely 1200-relevant and cheap (capture lands on f2/f7 next to the enemy king). Low risk, nice teaching note. Optional. |
| `kingside_attack` / `queenside_attack` | scoring theme | **SKIP** | `side_attack` is a hand-tuned *score* (checks + captures near the corner) for theming. Not a discrete pattern; our space / king-ring / heatmap overlays already convey "attack is pointed kingside." |
| `en_passant` | move-type | **SKIP (already surfaced)** | We already detect e.p. and the coaching panel surfaces available e.p. captures. A move-type flag, not a tactic. |
| `castling` | move-type | **SKIP** | Trivially known from the move; no teaching payload as a "tactic." |
| `promotion` / `under_promotion` | move-type | **SKIP (underPromotion: maybe later)** | Promotion is obvious from the move. Under-promotion (esp. knight-promotion mate / =N) is mildly instructive but niche; revisit only if real play surfaces it. |
| `piece_endgame(·)` / `queen_rook_endgame` | material/phase meta | **SKIP** | Endgame-type bucketing for the puzzle DB. We have phase + material; this is metadata. |
| `oneMove` / `short` / `long` / `veryLong` | length meta | **SKIP** | Solution length. Pure puzzle metadata. |

### `util.py` primitive inventory

These are the building blocks the predicates call. Status against our port:

| `util.py` | Status | Notes |
|---|---|---|
| `material_count` / `material_diff` | have equivalent | `MaterialOutcome` + `Value::mg_of_piece`. |
| `values` / `king_values` / `ray_piece_types` | DONE | `king_value()` in `tactic_outcome.rs`; ray-piece set is R/B/Q. |
| `is_defended` (incl. ray-defense) / `is_hanging` | DONE | Ported with the hidden-slider ray-defense case. |
| `can_be_taken_by_lower_piece` / `is_in_bad_spot` | DONE | Ported. |
| `attacked_opponent_squares` / `attacked_opponent_pieces` | DONE | `attacked_opponent_squares()`. |
| `is_capture` / `moved_piece_type` / `is_castling` / `is_king_move` | trivial / have | `Position::is_capture`, `Move::kind`, etc. |
| `is_advanced_pawn_move` / `is_very_advanced_pawn_move` | trivial if needed | Only used by the SKIP themes; port on demand. |
| `squares_are_collinear` | small port | Needed by x-ray & collinear; 4-line rank/file/diagonal test. |
| `attacker_pieces` | small port | Needed by boden-mate (P5). |
| **`is_trapped`** | **NOT PORTED — FLAGSHIP** | See below. The one load-bearing primitive still missing. |

---

## Flagship: trapped piece

**Why it's the priority** (memory `project_trapped_piece_visual_goal`): the user — and 1200 students generally — *cannot visually spot* when a piece, especially a deep enemy queen, has run out of safe squares. Hanging pieces are easy to see; a piece that is *currently fine but has no safe move* is invisible without calculation. Closing that gap with a visual overlay (the way space and king-safety are already overlaid) is a standout teaching feature.

### What `util.is_trapped(board, square)` actually computes

```
is_trapped(board, sq):
  if board.is_check() or board.is_pinned(turn, sq):   return False   # not "trapped", different problem
  piece = board[sq]
  if piece in {PAWN, KING}:                            return False
  if not is_in_bad_spot(board, sq):                    return False   # must already be attacked & (hanging or takeable by lower)
  for each legal move m FROM sq:
      cap = board[m.to]
      if cap and value[cap] >= value[piece]:           return False   # it can trade out at no loss → not trapped
      board.push(m)
      if not is_in_bad_spot(board, m.to):              return False   # it found a safe square → not trapped
      board.pop()
  return True                                                          # every escape still lands in a bad spot
```

The **intermediate data** that makes this visual: as the loop runs, every legal destination of the piece is classified "still a bad spot" or "safe." A trapped piece is one where *every* destination is bad (and no favourable trade exists). That per-square classification *is the cage* — it's exactly what we want to paint.

### Engine port (analytical / overlay path only — never in search)

1. Port `is_trapped(&Position, Square) -> bool` into the engine, reusing the already-ported `is_in_bad_spot` / `is_hanging` / `can_be_taken_by_lower_piece`. The check/pin guards map to `Position::checkers()` and `blockers_for_king`. The escape loop uses `legal_moves_vec` filtered to `from == square`, with `do_move`/`undo_move`. Cost: one legal-move-gen + a handful of make/unmake per candidate piece — fine for an analytical or per-frame-UI path (same budget as the existing overlays; **must not** touch the search hot path).
2. Wire it into the tactic library as `TacticPattern::TrappedPiece`. cook.py's `trapped_piece` checks the piece *before* the opponent's capture (it walks back one ply if the capture lands on the trapped piece's square). Our PV adaptation: on the line, when pov captures a non-pawn on `sq`, test `is_trapped` on the position before the piece had to commit.

### Visual overlay (the flagship deliverable)

Extend the existing overlay machinery (`analysis/overlays.rs` → `OverlayData`, the pattern documented in HANDOFF-ux "Board overlays"):

- New `OverlayData` fields: `white_trapped` / `black_trapped` (bitboard of trapped pieces of each colour), and a per-trapped-piece "dead escape squares" set so we can paint the cage. Because the escape set is per-piece, expose it as `Vec<(Square /*piece*/, Bitboard /*escape squares that stay bad*/)>` rather than a single board (this is the same "opt-in `Vec` for per-piece detail" pattern used by the mobility per-piece tracker).
- New `OverlayKind::TrappedPieces` + an `AnnotationKind` for the cage (reuse `BadPiece` tint for the piece, a new muted-red tint for the dead escape squares). View dispatch in `overlays_view::push_overlay_annotations`.
- Render: trapped piece highlighted, each square it *could* move to but which is *also* attacked highlighted as "dead," and (optionally) `Arrow { Attacker }` from the attacker(s) covering the cage. The student sees the box close around the queen.

This is the concrete answer to directive 4 ("investigate what intermediate data lichess computes and whether it can be surfaced visually"): **yes** — the per-escape-square bad/safe classification is the intermediate data, and it maps cleanly onto our overlay pattern.

### Misfire gates (carry over from the tactic-library brief)

- Don't flag a piece as trapped if it's only "trapped" by a line the opponent can't actually realise within the student's calculation horizon — gate the *tactic* (retrospective) surface on material actually changing in the PV; the *overlay* can be more liberal (it's "notice this," not "you blundered").
- `is_trapped` already excludes in-check and pinned pieces (those are different lessons) and pieces that can trade out evenly — keep those exclusions exactly.

---

## Shared utilities worth porting

### `win_chances` (cp → win probability)

From `tagger/zugzwang.py` / `generator/util.py`, tuned in lila PR #11148:

```
win_chances(cp) = 2 / (1 + exp(-0.00368208 * cp)) - 1     ∈ [-1, 1]   (±1 for mate)
```

**Verdict: PORT** as a small engine/teaching utility. It's a numerical fact (a fitted sigmoid), broadly useful, and several "port" items below want it:
- Sacrifice & zugzwang thresholds are naturally expressed in win-chance deltas (lichess uses 0.3 / 0.6 / 0.7 win-chance gaps, not raw cp).
- Teaching surface: "this move drops your winning chances from 78% to 52%" is far more legible to a 1200 than "−180 cp."
- Solution-uniqueness gate (below) is defined in win-chance gap.

### Generator's suppression / uniqueness gates (reference, not port)

`generator.analyze_position` + `is_valid_attack` encode three judgments we should *mirror in spirit* in the retrospective, so we don't nag:

1. **"Too winning to start with / already up material"** → don't flag a tactic. We shouldn't tell a student "you missed a fork" when they were already +5; the lesson is noise.
2. **Solution uniqueness** (`win_chances(best) > win_chances(second) + 0.7`, or `second is None`) → only call it a *missed* tactic when the best move is *clearly, uniquely* best. If two moves are within a coin-flip, "you missed THE move" is a lie. This directly hardens the `user_missed_tactic` slot.
3. **Minimum advantage** (`score >= Cp(200)` and a real win-chance jump) before a position counts as containing a tactic at all.

These are a **design reference** — the code is a PGN-mining loop, but the *gating philosophy* should inform `compute_tactic_outcome`'s thresholds (and overlaps with the existing "don't generic-miss" anti-pattern in the tactic-library brief).

---

## Zugzwang (`tagger/zugzwang.py`)

**Verdict: PORT as an analytical-only detector; defer build.** The predicate: for a pov move in the solution, if the position isn't check and has ≤ 15 legal moves, compare the eval of the real position against the eval after a *null move* (giving the opponent a free tempo); if the side-to-move's win-chance is ≥ 0.3 *lower* than after the null move, they're in zugzwang (any move they make hurts them).

Feasibility: **yes** — we have `do_null_move`/`undo_null_move` and an evaluator. But it costs a full analytical search per probe, so it runs only on the analytical path (on-demand / retrospective), **never in search** — same invariant as every other analytical surface (and it must use the no-profile analytical engine). Given cost and that zugzwang is rare in 1200-level play, build it last (or only if a real position motivates it). This matches the older "zugzwang deferred indefinitely / too search-expensive for live use" note while satisfying the directive's "engine-available" goal: the detector design is specified here and is implementable when wanted.

---

## Broader components (generator / validator / other repos)

| Component | Verdict | Rationale |
|---|---|---|
| `generator/generator.py` (PGN→puzzle mining) | **SKIP (reference the gates)** | We don't mine a game database — tactics arise in live play and we already gate on `MoveVerdict` + PV. Borrow the *suppression/uniqueness gating philosophy* (above); skip the pipeline. |
| `generator/tb.py` (tablebase prober) | **SKIP** | We don't ship tablebases (CLAUDE.md). Out of scope. |
| `generator/server.py`, `model.py` | **SKIP** | Dedup server + puzzle data model for the lichess pipeline. Irrelevant to us. |
| `generator/diskettes/*.py` | **SKIP** | Snapshotted prior versions of the generator; no value. |
| `validator/` (TS `front`+`back` web app) | **REFERENCE only** | A human-in-the-loop review UI for puzzle candidates (MongoDB + OAuth + a Svelte-ish front). The *automated* validation actually lives in the generator's `is_valid_attack` (uniqueness). Nothing portable; at most a UX reference for "how do you let a human confirm an annotation." |
| `tagger/tagger.py`, `model.py`, `test.py` | **REFERENCE** | Orchestration + data model + a small fixture test set. `test.py` is a useful source of *known-tagged positions* to seed our own detector fixtures when we implement each pattern. |
| **lichess-org/lila** (main server, Scala) | **REFERENCE** | Nothing directly portable (Scala, server). The analysis-board "tactical motif" annotation UX is the gold-standard reference for *how to present* what we detect — worth a look when we build the UI layer (later). |
| **lichess-org/scalachess** | **SKIP** | Chess logic; overlaps our SF11 port. No reason to read. |
| **lichess-org/lila-tablebase** | **SKIP** | Endgame tablebase server; we don't ship tablebases. |
| Opening explorer (lila feature) | **SKIP (for now)** | A master/online-game position DB. Out of scope unless we add an openings trainer; not a teaching-of-*reasoning* feature. |

---

## Proposed engine-availability sequence (the follow-on plan)

W4 is research; this is the implementation plan it produces. Each wave is engine-only (detectors + tests + fixtures); **all UI surfacing is a separate, later layer** per the directive. Land detectors one batch at a time with a small fixture of known-tagged positions (seed from `tagger/test.py`).

- **W4-impl 1 — Flagship: Trapped piece. ✅ ENGINE SIDE LANDED (2026-05-27).** `is_trapped` ported into `analysis/tactic_util.rs` (shared lichess-util primitives extracted there from `tactic_outcome.rs`); `TacticPattern::TrappedPiece` detector in the priority chain; overlay engine side = `OverlayData.{white,black}_trapped` bitboards + `analysis::trapped_cages(pos, colour)` (per-piece dead-escape "cage"). Turn-flip (user-approved): `compute_overlays` null-move-flips the turn so a trapped *enemy* piece shows on *your* move (skipped when the side to move is in check). 770 engine tests pass, clippy clean. **Deferred UI** (per directive 3): `OverlayKind::TrappedPieces` + `overlays_view` dispatch + cage `AnnotationKind`/palette + `RetrospectiveCategory::Tactic` card.
- **W4-impl 2 — Core-8 completion: Pin, Skewer, Discovered attack, Discovered check, Double check. ✅ ENGINE SIDE LANDED (2026-05-27).** `tactic_outcome.rs` split into a `analysis/tactic_outcome/` directory (`mod.rs` types + API + material accounting; `detectors.rs` `detect_line_tactic` + all `detect_*`; `tests.rs`). Five new detectors appended to the priority chain, ports of `cook.py` adapted to single-move framing: DoubleCheck (`checkers > 1`), DiscoveredCheck (checker ≠ moved piece), Skewer (x-ray the piece behind a forced-to-move front piece), DiscoveredAttack (moved piece's vacated square is `between_bb` a friendly slider and an enemy target; revealed attacker must be safe), Pin (`blockers_for_king` + cheaper-attacker / prevents-attack arms). All over existing primitives (`blockers_for_king`, `between_bb`/`line_bb`, `attackers_to`, `checkers`, `attacks_bb`). 777 engine tests pass, clippy clean.
- **W4-impl 3 — Sacrifice classification + `win_chances` utility.** Add the `Sacrifice` flag to `TacticHit`; port `win_chances` (user-endorsed near-term — primary use is a **threshold to gate which retrospective cards show**, plus expressing blunder/missed-tactic thresholds in win-probability lost; **normalize our cp (PawnEG≈213) to pawn=100 first** — see memory `project_win_chances_adoption`); use it to fix the one-ply-guarantee misfire (memory `project_threat_signal_revisit`). This one has a correctness payoff beyond new patterns.
- **W4-impl 4 — Second-wave patterns:** Deflection, Attraction, Interference (self+player), Intermezzo, Clearance, X-ray. Plus the suppression/uniqueness gates ported in spirit into `compute_tactic_outcome`.
- **W4-impl 5 — Mate-pattern library (engine-available, low UI priority):** Back-rank, smothered (surface for 1200s); anastasia/hook/arabian/boden/double-bishop/dovetail (available, not surfaced by default). Terminal-node detectors.
- **W4-impl 6 — Optional / deferred:** attackingF2F7 motif; **overloading** (build-from-scratch since lichess stubbed it — a chess.com-parity target the user explicitly wants; see memory `project_overloaded_detector`); under-promotion; zugzwang (analytical-only, build last).

> **Non-tactic teaching surfaces** the user flagged for the roadmap (flank-classified attack signal; named-endgame teaching library) are *not* lichess-tactic items and live in **HANDOFF-ux.md "Backlog: future teaching surfaces"** with their own memories (`project_flank_attack_classification`, `project_endgame_teaching_library`).
- **THEN — UI layer (the parked teaching-UX resumes):** `RetrospectiveCategory::Tactic` card, `user_walked_into` into forced-consequences, coaching-panel pattern names (Cβ), and the trapped-piece overlay desktop wiring. Per directive 3, none of this happens until the engine-availability waves above land.

---

## Done-criteria check (ROADMAP W4)

- ✅ **Documented decision on every major lichess component:** port / reference / skip — tables above cover all 30 `cook.py` tags, the named-mate sub-detectors, every `util.py` primitive, the zugzwang prober, the generator, the validator, and the sibling lichess repos.
- ✅ **New workflow plan for the "port" items:** the 6-wave engine-availability sequence above, flagship-first.
- ✅ **Teaching-value vs. plumbing split made explicit** (the core directive): genuine patterns are PORT; cp/mate/length/endgame/side-attack buckets and composer themes are SKIP, with the reason (we already compute the signal, or it's puzzle-DB metadata).
- ✅ **Flagship trapped-piece**: engine port + the *intermediate data → visual overlay* investigation answered (per-escape-square bad/safe classification = the cage).

> Per ROADMAP, delete `ROADMAP.md` once W4 is complete. This file (`w4-audit.md`)
> is the durable record of the verdicts; the implementation sequence above is the
> forward-looking plan that moves into HANDOFF once W4-impl work begins.
