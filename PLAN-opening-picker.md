# PLAN — Desktop opening picker (allowed-openings GUI)

**Status:** LANDED (commit `6aef577`, 2026-06-04). Remaining: interactive GUI
smoke test (open New Game → Openings → Only these → restrict to one opening →
confirm the bot follows it). Engine/UI logic unit-tested; the egui widget can't
be verified headlessly.
**Date:** 2026-06-04.

## Why

The engine + CLI already support restricting which openings the bot may play
(`BookSelection::Allowed(Vec<OpeningId>)`, per-ply matching in `book.rs`, CLI
`openings allow/deny/reset`). The **desktop GUI has no surface to set it** —
`start_new_game` hard-codes `OpponentProfile::new_random()` (full book) and only
overrides `noise` + `eval_mask` from the dialog. This plan adds the missing GUI
picker. No engine *behavior* changes — only new read-only derived data + UI.

## What the data supports (verified against the bundled TSVs)

3,695 rows, `eco \t name \t pgn`. The hierarchy comes from **two** fields:

- **Level 1 — Opening = White's first move** (`entry.line[0]`, NOT the name):
  `1.e4` (1997) · `1.d4` (1219) · `1.c4` English (181) · `1.Nf3` Réti (105) ·
  then a tail (`1.Nc3`, `1.f4`, `1.b4`, `1.b3`, `1.g3`, …).
- **Level 2 — Defense / system = name prefix before `:`**: Sicilian Defense
  (380), Ruy Lopez (233), French (211), Italian (181), QGD (171), KID (119)…
  These nest under their row's actual first move.
- **Level 3 — Variation = first segment after `:`** (split on `,`, take lead):
  Sicilian's 326 full strings roll up to **86 named variations** (Najdorf,
  Dragon, Richter-Rauzer, …). Each leaf aggregates all `OpeningId`s whose
  variation starts with that segment — so toggling "Najdorf" drops every
  Najdorf sub-line.

### The system insight (why the tree alone isn't enough)

Lichess names a position with **one** combined label, e.g.
`Indian Defense: London System` = "Black went Indian, White went London." So a
White **system** like the London smears across many Level-2 branches keyed by
Black's reply (`London System` bare, `Indian Defense: London System`,
`Indian Defense: Accelerated London System`, …). It is **not one tree node**.
→ Solve with orthogonal **system chips**: each chip is a precise substring select
(`find_ids_matching("London System")`, verified to exclude "London Defense").

### Recognition vs selection (answered for the record)

`identify()` returns **exactly one** name per position (`by_epd:
HashMap<EPD, OpeningId>`, first-writer-wins A→E on transposition collisions).
The picker is built on the **forward** index (every row independently
selectable), so the "two names at once" question never bites it.

## Widget

```
┌ Openings ─────────────────────────────────────────────────┐
│  ( ) Any   (•) Only these…   ( ) None                      │
│  Systems:  [▣ London] [ Colle] [ Catalan] [ Stonewall]…    │  ← cross-cutting chips
│  [ 🔍 search any opening, defense, or system…       ]      │
│ ┌─ Opening ────────────┬─ under "Queen's Pawn 1.d4" ─────┐ │
│ │ [ ] King's Pawn 1.e4 │  ▾ [▨] Indian Defense           │ │  ← master-detail tree
│ │ [▨] Queen's Pawn 1.d4│      [x] London System          │ │
│ │ [ ] English   1.c4   │      [ ] King's Indian, Fianch. │ │
│ │ …                    │  ▸ [▣] London System  (bare)    │ │
│ └──────────────────────┴────────────────────────────────┘ │
│  → London system · 38 lines allowed                        │
└───────────────────────────────────────────────────────────┘
```

- **Mode** (Any / Only these / None) maps to `BookSelection`:
  Any → `Allowed(all_ids())`, None → `None`, Only these → `Allowed(checked leaves)`.
- **Master-detail**: left = Level-1 openings (tri-state, click to focus);
  right = focused opening's Level-2 families, each expandable to Level-3
  variation leaves.
- **Tri-state checkboxes** (`▣` all / `▨` some / `☐` none): toggling a parent
  toggles its whole subtree; partial selection shows the dashed state.
- **System chips**: curated cross-cutting substring selects; toggling lights up
  the relevant tree nodes tri-state.
- **Search**: filters the tree to matching nodes (same substring machinery as
  the chips, ad-hoc).
- **Behavioral framing**: neutral — "lines the bot is allowed to follow." The
  book only plays the bot's moves, so which selections actually bind depends on
  the bot's color (enabling Sicilian → bot answers your 1.e4 with …c5 as Black,
  follows White's Sicilian theory as White). Not color-aware in the UI.

## Build layers

### 1. Engine — new `core/engine/src/opening_tree.rs` (read-only derived data)

```rust
pub struct OpeningTree { pub openings: Vec<OpeningGroup> }          // Level 1
pub struct OpeningGroup { pub label: String, pub families: Vec<FamilyGroup>, pub ids: Vec<OpeningId> }
pub struct FamilyGroup  { pub name: String,  pub variations: Vec<VariationLeaf>, pub ids: Vec<OpeningId> }
pub struct VariationLeaf{ pub label: String, pub ids: Vec<OpeningId> }
pub struct SystemTag    { pub label: &'static str, pub pattern: &'static str }

pub fn tree() -> &'static OpeningTree;          // OnceLock, built from openings::entries()
pub fn system_tags() -> &'static [SystemTag];   // curated; resolve via openings::find_ids_matching
```

- Level-1 label: format SAN of `line[0]` from startpos, map via a small canonical
  table (`e4`→"King's Pawn (1.e4)", `d4`→"Queen's Pawn (1.d4)", `c4`→"English
  (1.c4)", `Nf3`→"Réti (1.Nf3)", `f4`→"Bird's (1.f4)", `b3`→"Nimzo-Larsen",
  `b4`→"Polish", `Nc3`→"Dunst", `g3`→"King's Fianchetto"); unknown → "Irregular /
  Other".
- Bare-family rows (no `:`, 225 of them) → a single leaf labeled "Main line".
- Each node caches its descendant `ids` so the UI computes tri-state by set
  membership without re-walking strings each frame.
- Curated system chips (precise phrases): London System, Colle System, Catalan,
  King's Indian Attack, Stonewall, Torre Attack, Trompowsky, Réti — each verified
  to return a sane, non-empty, non-over-matching set.
- Tests: every entry lands in exactly one leaf; leaf-id union == all entries;
  each system tag non-empty and excludes the obvious false positive.

### 2. UI form plumbing — `core/ui/src/session/`

- `types.rs`: add `book: OpeningSelection` to `NewGameForm`, where
  `OpeningSelection { mode: OpeningMode, allowed: HashSet<OpeningId> }`
  (`OpeningMode = Any | Only | None`). `initial()` → `Any`; `from_current()` →
  derive from `session.opponent.book`.
- `lifecycle.rs`: thread `book` through `try_start_from_form` →
  `start_new_game(.. , book)`, setting `self.opponent.book` instead of letting
  `new_random()`'s full book stand. (`start_game`/CLI path already takes a full
  `OpponentProfile` — unchanged.)

### 3. Desktop widget — new `desktop/src/draw/opening_picker.rs`

- `pub(crate) fn draw(ui, &mut OpeningSelection)` — mode radios, system chips,
  search box, master-detail tree with tri-state checkboxes + per-node counts.
- Called from `dialog.rs` as a new collapsing "Openings" section next to the
  existing "Eval mask (advanced)" collapsible.
- Holds transient focus/expansion/search UI state locally (egui `Id`-scoped or a
  small struct in the form); selection lives in `OpeningSelection.allowed`.

### Out of scope

- CLI already has `openings allow/deny/reset`; no CLI change.
- Mid-game ⚙ gear: opening is a per-game start choice (like color/FEN), not
  live-changeable — New Game dialog only.
- Mobile: the engine `tree()`/`system_tags()` data is FFI-shareable later;
  Swift/Kotlin re-render only.

## Done when

Desktop New Game dialog lets a user pick Any / specific openings-defenses-
variations / a system / None; the choice commits onto `OpponentProfile.book`;
the bot honors it via the existing `BookCursor`; `cargo test` green; smoke-tested
by starting a game restricted to one opening and confirming the bot follows it.
