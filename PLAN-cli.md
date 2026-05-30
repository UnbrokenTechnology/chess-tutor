# Plan: turn the CLI into an agent-facing chess-analysis tool

## What this is

A design note for redesigning `chess-tutor` (the CLI) as a **tool for AI agents** to explore and explain chess positions without making the basic mistakes that came up during the [`teaching-positions/`](teaching-positions/) post-mortems. The desktop / mobile GUIs are for humans; the CLI is for Claude.

This subsumes the earlier [`PLAN.md`](PLAN.md) — its items 1 and 2 are Phases C and D here. Read [`PLAN.md`](PLAN.md) for the original framing of the tactics-surface gap; read this for the broader plan.

[`HANDOFF.md`](HANDOFF.md) is project-wide state; this file is one specific design problem to be picked up in a fresh context.

## Motivation: how the case studies were debugged

While working through [`teaching-positions/`](teaching-positions/) with an agent on a 1200-ELO chess.com game, the agent repeatedly made mistakes about:

- **What tactics were available** (missed the standing `…Nxe4` remove-the-defender in `missed-desperado-after-qe6`; missed the standing `…Bxh2+` discovered attack in `discovered-attack-after-qxe6`).
- **What pieces threatened what** (mis-stated which bishop defended e5; mis-stated the pin geometry along the e-file).
- **How the eval related to chess.com's numbers** (our engine reports side-to-move POV; chess.com reports white POV; numbers that looked similar meant opposite things).
- **What the eval-term breakdown was telling them** (`king.danger −1.15` parsed as "the king is in danger by 1.15 pawns" instead of "the pressure WE were applying to the enemy king dropped by 117 mg of attack score").
- **What the eval scale was** (the engine prints raw cp with PAWN_EG = 213; a `+150` reading is ~0.7 pawns, not ~1.5).

The 1200-rated user kept catching these — useful for the teaching system, embarrassing for the CLI. The engine already knows everything the agent needed; the CLI just wasn't exposing it in a form an agent could consume without reasoning about geometry by hand.

**The goal:** make `chess-tutor` capable of answering, for any FEN, every static question an agent would otherwise reconstruct by hand — *what attacks this square, what's pinned, what tactics exist for either side, what threats are standing, what's the eval and in what units* — with no ambiguity about POV, scale, or terminology.

## Design principles

1. **Agent-first, human-tolerable.** Default output is line-oriented and self-describing. Humans can read it; humans aren't the priority. `--json` for strict machine parsing.
2. **Every number is labelled.** Units (pawns / cp), POV (white / side-to-move), and source (static eval / search / detector) appear on every value. No bare `+1.59` ever.
3. **White-POV by default.** Matches chess.com, matches the user's intuition, matches what an agent's training mostly saw. `--stm` to switch back to side-to-move.
4. **Pawns primary, engine-cp parenthetical.** PAWN_EG = 213 is an engine internal; the agent should see `+1.85 pawns (395 cp)`, never `+395` alone.
5. **Geometry is a first-class query.** "What attacks e5?" is one command. Pin / skewer / discovered-attack alignments are queryable primitives, not things to reconstruct from `moves` output.
6. **Self-describing summary header on every FEN-taking command.** The agent must never be confused about which side is to move, what the score is, whether it's check, or what opening it's in.
7. **JSON schema lives in `chess_tutor_engine`**, not the CLI — same structs feed the future FFI.

## The position-summary header

Front-matter printed before any command's main output, for every FEN-taking command:

```
position: 1r4nr/p3k3/4qpp1/4b2p/2Q5/8/PPPP1PPP/R1B1R1K1 w - - 0 1
to move:  White
in check: no
material: even   (W: Q+2R+B+7P = 29  vs  B: Q+2R+B+N+4P = 29)
score:    +6.09 pawns white-POV  (1304 cp engine; ~92% win)  [search d=12]
opening:  (none matched)
legal:    37 moves
```

This single change removes the largest single class of agent confusion from the case studies. With it in place, no command output can be misread for the wrong side or wrong scale.

`score:` line: when the command runs a search (`search`, `explain`), this is the search score. When it doesn't (`board`, `moves`, `eval`, `tactics`, `square`, `attacks`, `alignments`, `threats`, `forcing`), it's the static eval — labelled `[static]` instead of `[search d=N]`.

## Output policy

| Aspect | Default | Override |
|---|---|---|
| POV | white | `--stm` |
| Score units | pawns (with cp in parens) | `--cp` (engine-cp primary) |
| Output format | human-readable text | `--json` |
| Term labels | named + one-line gloss | `--no-gloss` (compact); `--glossary` (dump full table) |
| Search depth (`search` / `explain`) | 12 (matches retrospective) | `--depth N` |

The `--json` schema for every command is defined in `chess_tutor_engine::cli_schema` (a new module that holds the serializable structs); the CLI just renders them. This makes the JSON also the FFI surface later.

## New commands

```
chess-tutor tactics    <FEN> [--latent] [--check-followups] [--prior-move M] [--json]
chess-tutor square     <SQ>  <FEN> [--json]
chess-tutor attacks    <FEN> [--by COLOR] [--json]
chess-tutor alignments <FEN> [--json]
chess-tutor threats    <FEN> [--json]
chess-tutor forcing    <FEN> [--by COLOR] [--json]
chess-tutor explain    <FEN> [--depth N] [--json]
```

### `tactics <FEN>`

Runs `find_best_tactic_in_position` for both sides and `find_overloaded` for both sides. With `--latent`, adds the new opponent-standing-threat scan (Phase D). With `--check-followups`, adds the one-ply-check simulation that catches the `double-fork-after-qd8` case. Sketch:

```
$ chess-tutor tactics "1r4nr/p3k3/4qpp1/4b2p/2Q5/8/PPPP1PPP/R1B1R1K1 w - - 0 1" --latent

White (to move) — best tactic this move:
  RemovingDefender  via  Qxe6+
    primary:  Qc4
    targets:  Qe6 (captures, +9 material), then Be5 falls (pinned)
    line:     Qxe6+ Kxe6 d4 Ne7 dxe5
    gain:     +2.0 pawns net
    confidence: High

Black — best tactic if granted a free move:
  DiscoveredAttack  via  …Bxh2+
    discoverer: Qe6
    vehicle:    Be5  (moves with check, unblocks the e-file)
    target:     Re1
    gain:       +3.0 pawns minimum (Kxh2 loses rook; Kf1 keeps it but worse)
    confidence: High

Overloaded pieces:
  none on either side.

Standing (latent) threats — what the opponent has loaded:
  Against White:
    DiscoveredAttack (Qe6 + Be5 → Re1) — fires on any forcing Bishop move; only `Qxe6+` or `Qe4` defuses.
  Against Black:
    (none detected)
```

JSON form returns one `TacticReport` per side plus a `latent_threats` array, each entry the full `TacticHit` / `LatentThreat` record. `--prior-move` feeds the recapture guard the same way `compute_tactic_outcome` consumes it.

### `square <SQ> <FEN>`

The agent's foundational query. The thing the agent reconstructed (wrongly) twice in the case studies. For square `e5` in the discovered-attack position:

```
e5: black bishop
  attacked by:  white Qc4 (along c4-d5-e5 not — empty; reachable? no, blocked by nothing — but Q on c4 doesn't see e5)
                white nothing else
  defended by:  black Qe6, black f6 pawn   (count: 2 defenders vs 0 attackers → safe)
  pinned:       no
  is discoverer for:  black Qe6 along e-file targeting white Re1   ← KEY
    moving the bishop with a check or forcing threat fires Qxe1
  outpost:      no
  mobility:     7 squares (b8, c7, d6, f4, g3, h2, d4)
  is trapped:   no
```

Lists are exact; no judgement calls. The agent reads attackers / defenders / pins / discoverer status straight off the engine's bitboard view of the position. The "count: 2 vs 0 → safe" framing is a static SEE-style readout, not a search.

### `attacks <FEN>`

Every `(attacker, target)` pair on the board, with target piece, SEE-style net material, defender count. Sortable in JSON by SEE / target value. The full geometric ledger — the agent's "have I noticed every threat" enumeration.

```
$ chess-tutor attacks "..." --by black

Black attacks on White pieces:
  Qe6 → Qc4   (attacker P=9, target P=9, SEE 0, defenders 0)   FORCES queen move
  Bb6 → ...
  ...

Black attacks on empty squares:  (only forcing ones — checks and skewer set-ups)
  Bxh2+ → h2 pawn (capture, but also: discovered attack on Re1)
  ...
```

### `alignments <FEN>`

Pure geometry: for each slider, ray-walk through (a) a same-color blocker to an opposite-color target (discovered-attack candidate), (b) an opposite-color blocker to a same-color or opposite-color target (pin/skewer candidate). Reports every alignment without judging whether the move actually fires — that's `tactics --latent`'s job. This is the primitive `tactics --latent` is built on; exposed separately because it's the thing the agent kept guessing wrong.

```
$ chess-tutor alignments "1r4nr/p3k3/4qpp1/4b2p/2Q5/8/PPPP1PPP/R1B1R1K1 w - - 0 1"

Discovered-attack ray candidates:
  Black: Qe6 → Be5 (own blocker) → Re1   (e-file)
  White: (none)

Pin / skewer ray candidates:
  Black: Bb8 → empty → empty → Rh2?     no, ray ends in empty squares
  White: Re1 → Be5 (enemy blocker) → Qe6   (e-file; potential pin once Bishop moves)
  ...
```

### `threats <FEN>`

Unified "what's vulnerable" dump for both sides — the one-stop pre-move audit. Combines:

- Hanging pieces (undefended + attacked).
- Loose pieces (attacked more times than defended, net material lost).
- Pinned pieces (absolute + relative).
- Overloaded defenders (`find_overloaded`).
- Trapped pieces (existing `trapped_cages` data).

Maps 1:1 to the desktop coaching cards so the agent sees what the human would.

### `forcing <FEN>`

Every check, every capture, every promotion, every move that creates mate-in-1 threat. The "look at all forcing moves first" discipline from the double-fork case study, exposed as a query. Used together with `--check-followups` on `tactics` to find the multi-step tactics whose first ply is a check.

```
$ chess-tutor forcing "<double-fork FEN>" --by black

Black forcing moves:
  Nd3+   check (knight to d3)
    only legal White responses: Kb1, Qxd3
    after Kb1 (engine pick):
      Nf2 forks Rd1 + Rh1  ← multi-step Fork, see `tactics --check-followups`
  Nxe4   capture: pawn
  ...
```

### `explain <FEN>`

Aggregator. Runs the position summary, `threats`, `tactics --latent --check-followups`, a depth-N `search` (default 12), and emits the lot in one block. The agent's "give me everything you've got on this position" entry point — one CLI call gives the same context that a full multi-call workflow would.

## Modifications to existing commands

| Command | Change |
|---|---|
| `eval` | Headline trio at top (`+0.85 pawns (white-POV); 182 cp engine (stm)`); one-line gloss per term row; `--glossary` flag for the standalone dump. |
| `search` | Default `--white-pov` and pawns-primary; new `--annotate` flag runs the tactic detector on each PV's first move and appends `(RemovingDefender, gains 2.0p)` etc. |
| `moves` | Annotate each row: `(check)`, `(capture: knight, SEE 0)`, `(promotion)`, `(en passant)`. |
| `board` | No changes — already correct and useful. |
| `opening` | Already correct. |
| `play` | No changes — interactive game loop, not analytical. |
| `bench` / `noise-bench` | No changes — perf tools, not for agent consumption. |

## Term-name glossary

A central data file (`core/engine/src/analysis/term_glossary.rs`) maps each `TermId` (or sub-term path like `king.danger`) to a one-line agent-friendly description. Used by:

- `eval` row annotations.
- `chess-tutor eval --glossary` standalone dump.
- JSON output's `description` field on every term.

Sample entries:

```
king.danger        — "Pressure on the enemy king square: attackers near king,
                     weak shield squares, etc. Positive = WE are pressuring."
king.flank-attacks — "Number of enemy attacks on squares near our king's flank.
                     Negative = THEY are attacking near our king."
threats.by-minor   — "Material at risk to our minor-piece attacks on enemy pieces
                     (net of defenders)."
threats.slider-on-queen — "Bonus when our sliders X-ray the enemy queen, even
                          through one piece. A pin / discovered-attack pre-cursor."
pieces.trapped-rook — "Penalty when our rook can only move along 1-2 squares
                     because own king is in its way and we've lost castling rights."
mobility.knight    — "Net knight mobility advantage, weighted by Stockfish 11's
                     mobility tables. Positive = WE have more knight squares."
```

The "positive = WE" / "negative = THEY" annotation is the single most important thing the gloss adds — the case studies showed the agent confusing direction repeatedly.

## Engine work needed

**Phases A–C** reuse existing engine APIs (no engine changes). **Phases D–E** need:

### Phase D — `analysis/latent_threats.rs` (PLAN.md item 2)

New module alongside `analysis/overloading.rs`. Public:

```rust
pub struct LatentThreat {
    pub pattern: TacticPattern,         // DiscoveredAttack, RemovingDefender, Pin, Skewer
    pub discoverer: Square,             // the piece that "fires" when triggered
    pub vehicle: Option<Square>,        // the blocker that has to move (None for RemovingDefender)
    pub target: Square,                 // what gets attacked when fired
    pub min_gain: Value,                // SEE-style minimum material gain (cp)
    pub confidence: Confidence,
    pub trigger_shape: TriggerShape,    // "any forcing move by `vehicle`" / "any move by enemy attacker of defender X" etc.
}

pub fn find_latent_threats(pos: &Position, defender_color: Color) -> Vec<LatentThreat>;
```

Per PLAN.md §"Open design questions": detector lives alongside `overloading.rs`; predicate is "for each enemy slider, walk each ray; if first hit is a same-color blocker, continue past it; if next hit is a higher-value target, record." SEE-style check on the result. Threshold: report only if `min_gain ≥ 1 minor piece`.

Tests against the two `teaching-positions/` FENs as regression targets (the done-criteria from PLAN.md).

### Phase E — `analysis/check_followups.rs`

New module for the `double-fork-after-qd8` mechanism. Public:

```rust
pub struct CheckFollowup {
    pub check_move: Move,                 // the opponent's check (e.g. …Nd3+)
    pub our_forced_replies: Vec<Move>,    // the (usually 1–2) legal responses
    pub followup: TacticHit,              // tactic detected after each reply
}

pub fn find_check_followups(pos: &Position, mover: Color, prior_move: Option<PriorMove>) -> Vec<CheckFollowup>;
```

Logic per `double-fork-after-qd8.md` §"Implementation implications": iterate enemy checks (usually 0–2), iterate our legal responses (usually 1–3), run `find_best_tactic_in_position` on each resulting position. Inner detector runs ≤ ~6 times per scan — well within per-move budget.

Wired into `chess-tutor tactics --check-followups`.

## Implementation phases

Each phase is shippable on its own; ordering matches dependency chain.

### Phase A — output hygiene (no engine changes)

1. Add `--json` and the JSON schema module in `chess_tutor_engine`.
2. Position-summary header on every FEN-taking command.
3. Default `--white-pov`; pawns-primary scoring; explicit `(stm)` / `(white-POV)` labels.
4. `term_glossary.rs` + `eval` row annotations + `--glossary` flag.

**Done when:** all the case-study FENs produce a CLI output an agent can read without misreading POV or scale.

### Phase B — geometric primitives

5. `square <SQ> <FEN>` — uses existing attacker/defender helpers.
6. `attacks <FEN>` — full attack ledger.
7. `alignments <FEN>` — pure ray-walking; no engine API changes needed.
8. `threats <FEN>` — composes existing hanging / loose / pinned / overloaded / trapped queries.
9. `forcing <FEN>` — composes existing legal-move generation + check/capture/promotion classification.
10. `--annotate` flag on `search` — runs detector chain on PV[0].
11. `moves` annotations — check / capture / promotion / EP.

**Done when:** the agent can answer "what attacks e5", "what's pinned", "what's hanging", "what are all forcing moves" without reading any other engine output.

### Phase C — tactics surface (PLAN.md item 1, mechanical)

12. `tactics <FEN>` (no `--latent`, no `--check-followups`) — calls `find_best_tactic_in_position` for both sides + `find_overloaded` for both sides.
13. `--prior-move` flag for the recapture guard.

**Done when:** running `chess-tutor tactics` on the `missed-desperado-after-qe6` FEN reports the `RemovingDefender` pattern against `Nf5` (PLAN.md's done-criterion 1).

### Phase D — latent threats (PLAN.md item 2, new engine module)

14. `analysis/latent_threats.rs` with `find_latent_threats(pos, defender_color)`.
15. Tests against both two-case-study FENs as regression targets.
16. CLI: `tactics --latent` wires it into the existing report.

**Done when:** running `chess-tutor tactics --latent` on the `discovered-attack-after-qxe6` FEN reports the standing `Qe6 / Be5 / Re1` alignment (PLAN.md's done-criterion 2).

### Phase E — check-followups + `explain`

17. `analysis/check_followups.rs` with `find_check_followups(pos, mover, prior_move)`.
18. Tests against the `double-fork-after-qd8` FEN.
19. CLI: `tactics --check-followups` wires it in.
20. `explain <FEN>` aggregator command.

**Done when:** running `chess-tutor tactics --check-followups` on the `double-fork-after-qd8` FEN reports the `…Nd3+ → …Nf2` Fork sequence, and `chess-tutor explain <FEN>` on any of the four case-study FENs returns one block of output covering summary / threats / tactics / latent threats / search.

## Out of scope (deferred)

- **Retrospective wiring of latent threats.** Once `find_latent_threats` exists, the natural follow-on is to plumb it into `compute_tactic_outcome`'s `user_walked_into` slot for moves that fail to disrupt a standing alignment. See PLAN.md §3 "Stretch". Tracked separately under teaching UX, not in this plan.
- **REPL mode.** Considered and rejected; the one-shot model composes fine with bash and the agent doesn't need persistent state.
- **Trap library exposure.** [`core/engine/src/traps/`](core/engine/src/traps/) has its own structured outputs separate from the tactic chain. If the agent needs them, a separate `chess-tutor trap <FEN>` command is a follow-on, not part of this plan.

## Open design questions

These need a call when each phase starts:

- **`square` square syntax.** UCI (`e5`) only, or also algebraic shorthand (`Ne5` = "the knight on e5")? UCI is unambiguous; algebraic is what the agent more naturally writes.
- **`tactics` output ordering when both sides have a tactic.** Side-to-move first (current side's best move), or higher-confidence first? Recommend side-to-move first by default with a `--sort` flag.
- **`alignments` confidence filtering.** Pure geometric output is noisy (every long-diagonal bishop has dozens of ray endpoints). Recommend default-filter to "at least one piece on the ray + target is higher value than blocker", then a `--all` flag for the unfiltered view.
- **JSON schema versioning.** Add a `"schema_version": 1` field to all JSON output now, so consumers can evolve. Phase A decision.
- **Glossary location.** A single Rust file (`term_glossary.rs`) keeps it next to the eval terms but bloats the source. Alternative: a TOML file checked in alongside, parsed at build time. Recommend the Rust file for now — under 1KLOC, easy to test, no build-time dependency.
- **Deep `--annotate` on `search`.** Per-PV-move detector runs are cheap but adding tactic flags to every PV move makes output dense. Recommend: only annotate PV[0] by default; `--annotate-all` for full PV annotation.

## Pointers to relevant code

- Existing CLI args: [`core/cli/src/cli_args.rs`](core/cli/src/cli_args.rs)
- Existing CLI entry: [`core/cli/src/main.rs`](core/cli/src/main.rs)
- Existing eval rendering: [`core/cli/src/eval_report.rs`](core/cli/src/eval_report.rs) (sub-term names + structure)
- Existing search rendering: [`core/cli/src/search_report.rs`](core/cli/src/search_report.rs) (score format, settled-ply suffix)
- Tactic detectors: [`core/engine/src/analysis/tactic_outcome/`](core/engine/src/analysis/tactic_outcome/)
- Overloaded detector: [`core/engine/src/analysis/overloading.rs`](core/engine/src/analysis/overloading.rs)
- Trapped-piece cage data: `analysis::trapped_cages` (in [`core/engine/src/analysis/mod.rs`](core/engine/src/analysis/mod.rs))
- Win-chance normalisation (for "≈ N% win" in summary header): [`core/engine/src/analysis/win_chances.rs`](core/engine/src/analysis/win_chances.rs)
- The original gap statement: [`PLAN.md`](PLAN.md) — items 1 + 2 become Phases C + D here.
- The four motivating case studies: [`teaching-positions/`](teaching-positions/) — used as regression FENs for Phases C / D / E done-criteria.
