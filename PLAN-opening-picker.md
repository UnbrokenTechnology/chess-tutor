# PLAN — Desktop opening picker (allowed-openings GUI)

**Status:** LANDED 2026-06-04. Engine/UI logic unit-tested; the egui widget was
iterated interactively against screenshots (see "UX iteration" below). This doc
describes the **final** shipped design — the picker is a standalone window, and
the Any/Only/None mode enum from the first draft was dropped.
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

## Widget (final)

A **standalone, fixed-size (680×470), centered, locked** window, launched by an
`[📖 Openings…]` button in the New Game dialog (with a one-line summary beside
it). It is *not* inline in the dialog — see "UX iteration".

```
┌ Choose openings ──────────────────────────────────── [X] ┐
│  [Select all] [Clear]        → 38 lines allowed           │
│  🔍 search any opening, defense, or system…               │
│ ┌ Opening ─────┬ Queen's Pawn (1.d4) ──┬ Systems ───────┐ │
│ │ ☑ King's Pawn│ ⊟ Indian Defense (52) │ Setups that    │ │
│ │ ⊟ Queen's Pwn│   ☑ London System (3) │ cut across     │ │
│ │ ☐ English    │   ☐ King's Indian (…) │ defenses.      │ │
│ │ ☐ Réti       │ ▸ ☑ London System (18)│ ☑ London System│ │
│ │ …            │ …                     │ ☐ Colle System │ │
│ │ Irregular ↓  │                       │ ☐ Catalan …    │ │
│ └──────────────┴───────────────────────┴────────────────┘ │
└───────────────────────────────────────────────────────────┘
```

- **No mode enum.** The tree is always shown. A fresh game starts with
  **everything selected** (= full book). The user narrows it. An **empty**
  selection *is* "no book" (bot plays from move 1) — `Select all` / `Clear` are
  the bulk ops, and the summary line spells out the empty case. (The first-draft
  `Any | Only | None` radios were removed as redundant — see UX iteration.)
- **Three fixed-width columns** (so nothing stretches the window): Level-1
  openings (click to focus) | the focused opening's Level-2 defenses, each
  expanding to Level-3 variations | the cross-cutting **systems** as a vertical
  scrollable list (not a horizontal chip row).
- **Tri-state checkboxes** (`☑` all / `⊟` some / `☐` none, phosphor glyphs):
  toggling a parent toggles its whole subtree.
- **Sorted by line count descending** at every level (popularity proxy): Sicilian
  before Zukertort, etc. "Irregular / Other" pinned last.
- **Systems**: curated cross-cutting substring selects (`find_ids_matching`);
  toggling adds/removes all matching leaves, lighting up the tree tri-state.
- **Search**: filters to a flat breadcrumb'd list of matching leaves.
- **Behavioral framing**: neutral — "lines the bot is allowed to follow." The
  book only plays the bot's moves, so which selections actually bind depends on
  the bot's color (enabling Sicilian → bot answers your 1.e4 with …c5 as Black,
  follows White's Sicilian theory as White). Not color-aware in the UI.

## UX iteration (why the design diverged from the first draft)

The first cut put the picker **inline** in the New Game dialog as a collapsible.
That broke badly: the New Game modal auto-sizes to its content, so the
width-hungry tree/columns/chip-row had no width to lay out against and ballooned
the modal past 4000px, pushing everything off-screen. Fixes, in order:

1. **Picker → its own window**, launched by a button. Decouples it from the
   auto-sizing modal.
2. **Options block → collapsible** in the New Game dialog, so the dialog is short.
3. **Systems → a vertical third column** (was a horizontal wrap-row that couldn't
   wrap in an unbounded-width parent).
4. **Window made `fixed_size` + center-anchored** — a *resizable* window still
   grew when search results were wide, and egui remembered the grown width. A
   fixed-size window physically can't grow; wide content clips instead.
5. **Columns given fixed widths** (was one fill-to-available column → a wall of
   white space that also dragged the window wide).
6. **Mode enum dropped** → `OpeningSelection { allowed }`, empty = no book. The
   separate "No opening book" toggle was redundant with `Clear`.

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
- Curated system chips (precise phrases): London System, Colle System, King's
  Indian Attack, Catalan, Stonewall, Torre Attack, Trompowsky — each verified to
  return a sane, non-empty, non-over-matching set. (Réti/Zukertort dropped — they
  are Level-1 openings, not cross-cutting systems.)
- **All levels sorted by line count descending** (`sort_by_key(Reverse(len))`),
  "Irregular / Other" pinned last.
- Tests: every entry lands in exactly one leaf; leaf-id union == all entries;
  each system tag non-empty and excludes the obvious false positive; every level
  is count-sorted.

### 2. UI form plumbing — `core/ui/src/session/`

- `types.rs`: add `book: OpeningSelection` to `NewGameForm`, where
  `OpeningSelection { allowed: HashSet<OpeningId> }` (no mode/flag — empty = no
  book). `to_book()`: empty → `None`, else `Allowed(allowed)`. `from_book()`:
  `None` → empty, `Allowed(ids)` → that set. `initial()`/`from_current()` seed it
  (`any()` = full book / derive from `session.opponent.book`).
- `lifecycle.rs`: thread `book` through `try_start_from_form` →
  `start_new_game(.. , book)`, setting `self.opponent.book` instead of letting
  `new_random()`'s full book stand. (`start_game`/CLI path already takes a full
  `OpponentProfile` — unchanged.)

### 3. Desktop widget — new `desktop/src/draw/opening_picker.rs`

- `open(ctx)` / `close(ctx)` toggle a window-open flag in egui temp memory;
  `summary(&OpeningSelection)` → the launch-button caption; `draw_window(ctx,
  &mut OpeningSelection)` renders the standalone fixed/centered window when open.
- `dialog.rs` shows an `[📖 Openings…]` button (+ summary) and calls
  `draw_window` at top level *after* the New Game modal closes (so the picker is
  not nested inside the auto-sizing modal). Dismissing the dialog calls `close`.
- Transient focus/search state lives in egui temp memory; selection lives in
  `OpeningSelection.allowed`.

### Out of scope

- CLI already has `openings allow/deny/reset`; no CLI change.
- Mid-game ⚙ gear: opening is a per-game start choice (like color/FEN), not
  live-changeable — New Game dialog only.
- Mobile: the engine `tree()`/`system_tags()` data is FFI-shareable later;
  Swift/Kotlin re-render only.

## Done when

The New Game dialog's `Openings…` button opens a standalone picker where a user
selects any subset of openings/defenses/variations (or systems, or clears to "no
book"); the choice commits onto `OpponentProfile.book`; the bot honors it via the
existing `BookCursor`; `cargo test` green. ✅ Landed. Final interactive smoke test
(restrict to one opening, confirm the bot follows it) is the user's to run.
