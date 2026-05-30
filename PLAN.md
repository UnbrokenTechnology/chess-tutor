# Plan: surface tactics in the CLI for static position analysis

## What this is

A forward-looking design note for a CLI enhancement that exposes the engine's existing tactical detectors as a human- and AI-readable surface for arbitrary positions. Today the detectors fire only inside the teaching-layer pipeline (`compute_tactic_outcome`, `find_tactic_in_line`, `find_best_tactic_in_position`) and their output is consumed by retrospective / coaching cards in the desktop UI. There is no way, from the CLI, to ask *"for this FEN, what tactical patterns exist for either side?"* — which has become the limiting factor in agent-assisted position analysis (see motivation below).

Read this before starting work on the CLI tactical surface. [`HANDOFF.md`](HANDOFF.md) is the project-wide state; this file is one specific design problem to be picked up in a fresh context.

## Motivation

While the user was working through chess.com game-review screenshots with an AI agent (see [`teaching-positions/`](teaching-positions/)), a pattern emerged: the agent could run `chess-tutor search` to get engine scores and PVs, but it had **no surface for the actual tactical content of a position**. To figure out *why* a move was good or bad, the agent had to:

- Reason about piece geometries by hand (often incorrectly).
- Reach for SF11-style threat formulas it half-remembered.
- Reconstruct discovered-attack alignments mentally, which it repeatedly got wrong.

The user — a ~1200 player — kept catching these mistakes and steering the agent back. That's a useful debugging exercise for the teaching system, but it also revealed a real gap: the engine *already knows* about discovered attacks, pins, removed defenders, overloaded pieces, and so on. None of that knowledge is reachable from the CLI.

Two concrete positions surfaced this:

1. [`missed-desperado-after-qe6`](teaching-positions/missed-desperado-after-qe6.md) — agent missed Black's standing remove-the-defender tactic (`…Nxe4` against `Nf5`), and missed White's desperado response (`Nxg7+`). Engine has both patterns; agent had no way to query them.
2. [`discovered-attack-after-qxe6`](teaching-positions/discovered-attack-after-qxe6.md) — agent missed Black's discovered-attack alignment on the e-file (`Qe6 / Be5 / Re1`). Engine could detect this pattern but had no surface to expose the standing alignment, and the agent reasoned about the geometry wrong twice before the user spotted it.

**The goal:** make `chess-tutor` (the CLI) capable of saying, for any FEN, what tactical patterns are in play for both sides, so any consumer — human user at a CLI, AI agent doing position analysis, future test harness — can see the engine's tactical view of the position without having to reconstruct it from search PVs.

## What we already have

All in [`core/engine/src/analysis/`](core/engine/src/analysis/). The summary from [`HANDOFF.md`](HANDOFF.md) §"Engine-available tactic surface":

- **`find_best_tactic_in_position(pos, mover, prior_move) -> Option<TacticHit>`** — static fork-shape scan over every legal move for `mover`. Predicate-based, no search. The detector chain runs by pattern severity (Fork beats TrappedPiece etc.).
- **`find_tactic_in_line(pre, line, mover, prior_move) -> Option<TacticHit>`** — single-line variant for analysing a specific move sequence.
- **`compute_tactic_outcome(best_ma, user_ma, pre_pos, root_stm, prior_move) -> TacticsOutcome`** — three-slot outcome (`user_played_tactic`, `user_missed_tactic`, `user_walked_into`) used by the retrospective.
- **`find_overloaded(pos, victim) -> Vec<OverloadedPiece>`** — strict sole-defender-of-≥2 scan. Pre-move analytical surface.
- **`TacticPattern`** — Fork, HangingCapture, RemovingDefender, TrappedPiece, Pin, Skewer, DiscoveredAttack, DiscoveredCheck, DoubleCheck, Sacrifice, Intermezzo, Deflection, Attraction, Interference, Clearance, XRay, AttackingF2F7, UnderPromotion, Checkmate.
- **`MatePattern`** — named mate patterns alongside Checkmate.
- **`TacticHit`** — the full tactic descriptor: `pattern`, optional `mate_pattern`, `sacrifice` flag, `primary_piece`, `targets`, `material_gain`, `confidence`, `pv_ply`.

These are all already exposed as `pub` from `chess_tutor_engine::analysis`. The CLI crate ([`core/cli/`](core/cli/)) can import them directly.

## What's missing

Two distinct things:

### 1. CLI surface for the existing detectors (mechanical)

A subcommand or flag that runs `find_best_tactic_in_position` for both colours on a given FEN and pretty-prints the results. Output should be human-readable but also AI-parseable (line-oriented, structured).

Sketch of what the output might look like:

```
$ chess-tutor tactics "1r4nr/p3k3/4qpp1/4b2p/2Q5/8/PPPP1PPP/R1B1R1K1 w - - 0 1"

White (to move):
  no tactic detected by static scan

Black (one-ply ahead):
  DiscoveredCheck via …Bxh2+
    discoverer:  Qe6
    blocker:     Be5
    target:      Re1
    material:    +5 (rook for bishop sac)
    confidence:  High

Overloaded pieces:
  none detected on either side
```

Open questions for the surface design — these need a decision before implementation:

- **Subcommand vs. flag on existing commands?** Could be `chess-tutor tactics <FEN>`, or could be `chess-tutor eval --tactics`, or could be a new section appended to `chess-tutor search` output. The first is most discoverable for AI agents; the second integrates more cleanly with existing eval workflow.
- **`prior_move` handling.** `find_best_tactic_in_position` takes an `Option<PriorMove>` for the recapture guard. From a bare FEN we don't have one. Options: (a) accept `--prior-move <SAN|UCI>` as a CLI flag, (b) pass `None` and document that recapture-guard tactics may be reported incorrectly, (c) require the user to provide a sequence (not just a FEN) so we can synthesise the prior move. Default to (b) with the flag from (a) available — simplest UX, most honest about the limitation.
- **One-ply ahead for the opponent.** The detectors look at moves the *side to move* can play. To show what Black can play *after* White's move (so we can see Black's responses), we need either a null-move trick (flip side to move, run detector) or run the detector after each candidate White move. The null-move version is cheap and correct for "standing threats"; the per-candidate version is the proper way to populate something like `user_walked_into` but is much more expensive.
- **Output verbosity.** Per `TacticHit` we have ~7 fields. For an AI agent, all of them are useful. For a human at a CLI, the pattern name + primary piece + targets is probably enough. Suggest a `--verbose` flag, default to short form.
- **Format.** JSON output mode (`--json`) for machine consumption would help — agents currently parse line-oriented output with regex. JSON would mirror what an FFI consumer would want anyway. Worth defining the schema in the engine crate so CLI / future FFI share it.

### 2. Latent-threat detector for opponent's standing alignments (new feature)

This is the architecturally new piece. The existing detectors find tactics that the side to move *can play right now*. They do not find tactics that the *opponent* has **pre-loaded** and is waiting for a free tempo to execute. That gap is what both of the [`teaching-positions/`](teaching-positions/) case studies expose.

Concrete shape of what's missing:

- **Standing remove-the-defender:** opponent has a piece attacking one of our pieces' defenders, such that if they get a free move they win our piece. Static input: enemy attacks on the defenders of our hanging-after-defender-loss pieces.
- **Standing discovered attack:** enemy slider + enemy blocker + our valuable piece, all on the same line. Any forcing move by the opponent that moves the blocker executes the discovery. Static input: ray-walking from each enemy slider through one friendly blocker to a more valuable enemy target. Documented in [`discovered-attack-after-qxe6`](teaching-positions/discovered-attack-after-qxe6.md) §"What this would take to teach automatically".
- **Standing pin / skewer threats** — same shape, slightly different geometry. Probably falls out of the same ray-walking infra.

Architecturally this is a **static board scan that runs in `current side's turn`**, looking at *enemy* geometric alignments. Output should be a list of `LatentThreat { pattern, discoverer, vehicle, target, trigger_move_shape }` records. Not the same type as `TacticHit` (which describes a tactic the *mover* is playing) — this describes a tactic the *opponent* has on standby.

Pipeline consumers:

- The CLI surface from item 1 above (show latent threats for both sides).
- The pre-move coaching surface in `core/ui/` (currently `coaching_view::overloaded_card`, etc.) — a `latent_threat_card` would be the natural addition. This is the surface that would have caught the `Qc5+` blunder in real-time.
- The retrospective / `compute_tactic_outcome` pipeline — when the user's move *fails to address* a standing latent threat, that becomes `user_walked_into` content. (Today `user_walked_into` requires the opponent to play the tactic; with latent-threat detection we can fire it pre-emptively against any user move that doesn't disrupt the alignment.)

Open design questions for the latent-threat detector:

- **Where does it live?** Either as a new module under [`analysis/`](core/engine/src/analysis/) (e.g. `analysis/latent_threats.rs`) alongside `overloading.rs`, or as a sibling to the existing tactic_outcome module. The first feels right — it's a *static pre-move scan over the opponent's setup*, parallel to `find_overloaded` which is also a pre-move static scan.
- **What's the trigger predicate?** For discovered attacks: "for each enemy slider, walk each ray; if first hit is a friendly blocker, continue past it; if next hit is our piece of higher value than the blocker, record." For remove-the-defender: "for each of our pieces that would be hanging without its defenders, check whether any enemy piece attacks any of those defenders." Both are O(pieces × rays) — very cheap.
- **Confidence / filtering.** Static detection of *latent* threats will produce false positives (e.g. the opponent's bishop could move along the line, technically discovering an attack, but our piece is actually defended). The detector should run a one-ply SEE-style check on the discovered-attack outcome before reporting. Set a threshold on `material_gain` — e.g. only report if the opponent can win ≥1 minor piece from executing the threat.
- **Recursion limit.** Don't recursively look at what the opponent's response to our defusal would be. This is a static scan, not a search. The detector reports "they have this loaded"; the search / `compute_tactic_outcome` chain handles the dynamic question of whether it actually wins.

### 3. (Stretch) integration into existing analysis pipeline

Once items 1 and 2 land, the natural follow-on is to wire latent-threat detection into `compute_tactic_outcome` so the retrospective can say "you played `Qc5+` and walked into Black's discovered-attack alignment." Today the pipeline runs after-the-fact and detects what the opponent's response *was*; with latent detection it can detect what was *already loaded*.

Out of scope for the initial PLAN — let items 1 and 2 land first, then revisit.

## Suggested implementation order

1. **Item 1, minimal:** add `chess-tutor tactics <FEN>` subcommand that calls `find_best_tactic_in_position` for both sides (with `prior_move = None`) and `find_overloaded` for both sides. Output as plain text, line-oriented. Existing CLI args style (see [`core/cli/src/cli_args.rs`](core/cli/src/cli_args.rs)) and existing pretty-printer style (see [`core/cli/src/analysis_report.rs`](core/cli/src/analysis_report.rs)). Tests in `core/cli/src/tactics_report_tests.rs` (sibling style per CLAUDE.md).
2. **Item 1, polish:** add `--prior-move`, `--json`, `--verbose` flags as decided. The JSON schema lives in the engine crate so the FFI work later can reuse it.
3. **Item 2, scoped to discovered attacks first.** Land a single-pattern detector (`analysis/latent_threats.rs::find_latent_discoveries`) and surface it through the CLI. Tests against both case-study FENs.
4. **Item 2, broaden to other latent patterns.** Remove-the-defender, then standing pins / skewers. Each as its own detector function, composed in a top-level `find_latent_threats` aggregator.
5. **Item 2, retrospective wiring.** Plumb latent-threat detection into `compute_tactic_outcome`'s `user_walked_into` slot for moves that fail to disrupt a standing alignment. UI changes deferred to the desktop / ui crate work tracked in [`HANDOFF-ux.md`](HANDOFF-ux.md).

## Pointers to relevant code

- Engine tactic detectors: [`core/engine/src/analysis/tactic_outcome/`](core/engine/src/analysis/tactic_outcome/) — `mod.rs` (`compute_tactic_outcome`, `find_best_tactic_in_position`), `detectors.rs` (per-pattern chain).
- Engine overloaded detector: [`core/engine/src/analysis/overloading.rs`](core/engine/src/analysis/overloading.rs).
- Engine analysis module entry: [`core/engine/src/analysis/mod.rs`](core/engine/src/analysis/mod.rs) `pub use` list — confirms what's already exposed.
- CLI subcommand pattern: [`core/cli/src/cli_args.rs`](core/cli/src/cli_args.rs).
- CLI pretty-printer style: [`core/cli/src/analysis_report.rs`](core/cli/src/analysis_report.rs) — top-percent term breakdown is the model for what concise tactical output should look like.
- Existing CLI tactic-adjacent surfaces: search the cli crate for any current consumers of `TacticPattern` (there shouldn't be any; this is greenfield for the CLI).

## Done criteria

For a fresh context picking this up:

- **Item 1 done** when `chess-tutor tactics <FEN>` runs against the two case-study FENs in [`teaching-positions/`](teaching-positions/) and produces output identifying the relevant patterns (`RemovingDefender` against `Nf5` in the desperado case; the `Be5` overload / pinned-bishop content in the discovered-attack case).
- **Item 2 done** when the same CLI command, run against the `discovered-attack-after-qxe6` FEN with White to move, reports the standing `Qe6 / Be5 / Re1` discovered-attack alignment as a latent Black threat — *without White having moved yet*. This is the regression target named in that doc.

If both done criteria are met, an AI agent given an unfamiliar FEN can call one CLI command and get the tactical landscape without having to reason about geometries by hand. That's the unlock.
