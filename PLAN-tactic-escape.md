# PLAN: tactic escape-hatch detection

**Status:** Phases 1–3 LANDED. Phase 1: engine `analysis/tactic_escape.rs` + `key_move` on `TacticHit` + CLI wiring (`tactics`, `search --annotate`). Phase 2: `compute_tactic_outcome` now computes escapes for all three slots (paired `user_*_escape: Option<TacticEscape>` companion fields; full-probe, approach (b)). Phase 3: retrospective tactic cards (`core/ui/src/retrospective_view/tactic.rs`) render a per-slot escape sentence — opponent's out for played/missed, the user's own out for walked-into.

**Seed finding (worth keeping):** on the case-study FEN the surfaced "best tactic" is **`Rxe5`** (rook takes the e5 bishop → pins the e6 *queen* to the king, `targets: [e6]`, gain 825) — and its escape is **`Qxe5`** (the pinned queen captures the pinner along the pin line, an `EscapeKind::Zwischenzug`). So the `tactics` command was effectively recommending a move that loses the exchange; the escape annotation now names the refutation. (The `…Bxh2+` resource from the original case study is a *different* thing — a standing threat against White, surfaced by the `danger:` header, not by this tactic's escape.)

---

**Author context:** follows the 2026-05-30 case study `teaching-positions/discovered-attack-after-qxe6.md`, where a *real* pin (`Re1`×`be5`→`qe6`) had a forcing escape (`…Bxh2+`) that the static one-ply detector can't see. See memory `feedback_eval_swing_means_allowed_threat`.

## Goal

For a detected tactic, also report whether the opponent has a **clean escape** — a defensive resource (usually a *forcing* in-between move: a check or a capture) that prevents the tactic from achieving what its geometry promises. Report *that* there is one, *which move* it is, and *what kind* it is.

Pattern-agnostic: covers a pin broken by check, a fork where one move defends both targets, a fork escaped by checking with one forked piece then saving the other, a discovered attack interrupted by an in-between move, etc.

**Not** a goal: refuting our tactic detection. A pin with an escape is still a real, valuable pin (it freezes the piece against every *quiet* move). The escape is an annotation on a true tactic, not a reason to suppress it.

## Why static detection can't do this

The detector chain (`core/engine/src/analysis/tactic_outcome/`) is static by design — "no new search — cheap predicates" (`mod.rs:1-8`). Escapes are exactly the cases one-ply geometry misses, because they are *forcing-move chains*: `…Bxh2+` is only available because it's check; a quiet bishop move loses the queen. We keep the detectors static and add a separate, opt-in layer that checks whether the tactic's expected outcome actually occurs.

## Core model: expected outcome + firing condition (no eval thresholds)

Each tactic asserts a **specific expected board-state change** — a concrete capture, not a cp delta. The tactic *fires* when its **firing condition** is met; it has *fired successfully* iff the expected capture then materialises in the immediate follow-up. If the firing condition is met but the expected capture does **not** happen, the opponent **escaped**, and the **refutation is the move that met the firing condition while dodging the consequence** (the first forcing reply — confirmed sufficient).

This sidesteps the eval-threshold trap entirely. "Won a rook but dropped 150 cp of position" or "swapped down to a minor instead" are not threshold judgments — we ask only "did the *specific* expected capture occur?", a boolean over the line. No `ESCAPE_MARGIN`, no win% gate, no scale/POV normalisation.

Per-pattern spec (squares come straight off `TacticHit`: `primary_piece` + `targets`):

| Pattern | Firing condition | Expected capture = success | Refutation move |
|---|---|---|---|
| Fork | active at ply 0 | owner captures one of `targets` within the window | opponent's reply |
| HangingCapture | active at ply 0 | owner captures the `targets` piece | opponent's reply |
| RemovingDefender | owner captures the defender (ply 0) | owner captures the now-undefended `target` next | opponent's reply |
| Pin (prevents escape/attack) | the pinned piece (`primary_piece`) moves | owner captures the pinned-to `target` behind it | the move of the pinned piece |
| Skewer | the front (valuable) piece moves | owner captures the rear `target` | the move of the front piece |
| DiscoveredAttack | the vehicle moves | the unmasked slider captures `target` | the move of the vehicle |

- **Active** tactics (fork / hanging / removing-defender) fire at ply 0. Escape check is "does the opponent's reply prevent the expected capture?"
- **Constraint** tactics (pin / skewer / discovered) are dormant. They fire only when the constrained/vehicle piece moves — which may never happen, and that's the constraint *working*, not an escape. A dormant pin whose piece has no forcing escape has **no escape and is simply holding** (correct, not a miss).

## Detection (structural, mostly search-free)

- **Active tactics:** look at the opponent's best reply. Does owner still capture an expected `target` in the follow-up (within `MATERIAL_WINDOW_PLIES = 4`)? If not → escape = that reply. (The retrospective path already has this reply in the searched PV; the `tactics` path gets it from a shallow reply check or short search.)
- **Constraint tactics:** enumerate the constrained/vehicle piece's *forcing* moves only (its checks + captures — one piece, cheap and exact). Apply each; ask "is the expected capture still available to owner on the immediate next ply?" If a forcing move denies it → escape = that move (e.g. `…Bxh2+` is check, so `Rxe6` is unavailable). If none → no escape; the constraint holds.

No deep verification search needed in the common case. The constraint-tactic probe is 1–2 ply over a single piece's forcing moves — cheaper and more deterministic than a full search, with nothing to tune.

## Refutation classification (why the expected capture failed)

```
enum EscapeKind {
    ForcingCheck,        // escaping move is check → owner must respond, can't capture (our case)
    Zwischenzug,         // escaping move is a forcing capture inserted before owner can collect
    DefendsBothTargets,  // (forks) the reply adds a defender to the surviving target(s)
    CounterAttack,       // reply makes a threat >= the tactic's value, pulling owner off
    AdequateRetreat,     // the attacked/forked piece simply had a safe square (static over-claimed reach)
}
```

Cheap board queries on `(pre_move_pos, refutation_move, hit.targets)`, reusing `gives_check` / `is_capture` / attacker bitboards in `tactic_util`. Order matters (a checking reply that also defends → `ForcingCheck` leads).

## New types & where things hook

- **New module `core/engine/src/analysis/tactic_escape.rs`** (sibling to `tactic_outcome/`):
  ```
  pub struct TacticEscape {
      pub refutation: Move,         // the first forcing move that dodges the expected capture
      pub kind: EscapeKind,
      pub expected_target: Square,  // what we expected to capture — for the teaching string
  }
  // Active tactics: check the opponent reply. Constraint tactics: probe the constrained piece.
  pub fn find_tactic_escape(pos, hit: &TacticHit, owner: Color, opp_reply: Option<Move>)
      -> Option<TacticEscape>;
  ```
  `opp_reply` is supplied from a searched PV when available (retrospective) and `None` on the `tactics` surface, where the function probes directly. Static detectors are untouched.
- **Add `key_move: Option<Move>` to `TacticHit`** (`mod.rs:392`). Detectors receive `line: &[Move]` + the ply, so `key_move = line.get(ply).copied()`. Needed to identify the owner's tactic move (active tactics) and independently useful — the `tactics` text currently shows only `key sq: e5`, never the move. Small ripple through construction sites in `detectors.rs`/`mate.rs`. ← decision for review (recommend yes).

## Surfaces to update

- `chess-tutor tactics` / `explain`: under `best tactic`, add e.g.
  `escape: opponent breaks it with Bxh2+ (forcing check) — pin still holds vs every quiet move`.
- `chess-tutor search --annotate`: same one-liner on the PV's tactic.
- `danger:` header / latent threats: the mirror case — when the side to move's own piece is the vehicle/constrained piece of an opponent threat, note if *they* have the forcing break.
- Retrospective card (Phase 2): the teaching payload — "the tactic works unless they find …".

## Determinism & performance

- Constraint-tactic escape is a structural forcing-move probe of one piece — no search, fully deterministic.
- Active-tactic escape reuses the retrospective's already-searched reply; on the `tactics` surface it does a structural reply check (or, if a search is used, a cloned engine at fixed depth — depth-budget, never the play engine).
- Runs only on the High-confidence hit actually being surfaced — at most one probe per command.

## Lift estimate

- **Phase 1 — `tactics`/coaching surface (~200–300 LOC + tests, ~1–1.5 days).** New `tactic_escape.rs` with the per-pattern expected-outcome + firing-condition spec, the active-vs-constraint detection split, and the classifier; `key_move` on `TacticHit`; CLI rendering in `tactics_view` / `--annotate`. (Slightly more than the v1 estimate because the per-pattern spec is the substance — but it's deterministic boolean logic, not threshold tuning.)
- **Phase 2 — retrospective reuse (~100 LOC, ~0.5 day).** Feed the searched `opp_reply` from `compute_tactic_outcome`'s slots; add `Option<TacticEscape>` to the outcome.
- **Phase 3 — teaching narration / retrospective UI.** Separate, deferred.

## Risks / open questions (for review)

1. **`key_move` on `TacticHit`** — add it (recommended, also fixes "name the move" in `tactics`) or keep it local to a verified wrapper? ← needs your call.
2. **Per-pattern expected-outcome spec is the correctness surface.** Each row of the table is a small predicate; the risk is getting a pattern's "expected capture" subtly wrong (e.g. pin-prevents-*escape* vs pin-prevents-*attack* expect different captures). Mitigated by per-pattern tests, but this is where review attention pays off — does the table above match your mental model for each pattern?
3. **The window after firing.** For active forks the expected capture may be ply 2–4 (collect the second target); for constraint tactics it should be the *immediate* next ply (else the forcing move bought the escape). Proposal: reuse `MATERIAL_WINDOW_PLIES = 4` for active, "next ply" for constraint. OK?
4. **lichess parity:** `cook.py` has no escape/refutation concept (`defensive_move`/`check_escape` are narrative tags on already-forced mainlines) — nothing to port; we author it. On-thesis.

*(Resolved from v1: cp-swing gating dropped entirely; "which ply is the refutation" answered by the firing-condition model; multi-move escapes → report the first forcing reply only.)*

## Test plan

`core/engine/src/analysis/tactic_escape_tests.rs`:
- **Seed (must pass):** `1r4nr/p3k3/4qpp1/4b2p/2Q5/8/PPPP1PPP/R1B1R1K1` — White's Pin (`be5`→`qe6`) detected **and** flagged with refutation `…Bxh2+`, kind `ForcingCheck`, `expected_target = e6`.
- **No-escape pin:** a pin whose pinned piece has no forcing move → `None` (constraint holds).
- **Fork saver:** a fork where one quiet reply defends both targets → `DefendsBothTargets`.
- **Zwischenzug:** a capture-tactic dodged by an in-between forcing capture → `Zwischenzug`.
- **Adequate retreat:** a "fork" where an attacked piece simply has a safe square → `AdequateRetreat`.
- Determinism: same FEN → identical result across runs.

## Out of scope

- Changing detector confidence based on escapes (escapes annotate, don't suppress).
- Tracing multi-move escape chains beyond naming the first forcing move.
- Any change to the play engine or search internals.
