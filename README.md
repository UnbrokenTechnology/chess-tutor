# Chess Tutor

**A chess tutor that shows its work.** It plays at a tunable strength, and for
every move — yours or its own — it can tell you not just *that* a move was good
or bad, but **why**, in terms of named chess concepts a club player can
actually learn from.

It's built for the player stuck around 1200 who doesn't hang pieces, spots
basic tactics, and still loses to 1400 bots with "zero blunders" — because the
position quietly rots until the only moves left are bad ones. That gap from
~1200 to ~1600 is *positional understanding* — space, weak squares, outposts,
pawn structure, initiative, which of three safe-looking moves walks you into
zugzwang. Commercial engines can't teach this, because the thing that makes
them strong is the very thing that makes them silent.

---

## The core idea: a glass-box engine

Modern chess engines are **black boxes**. Since Stockfish 12 (September 2020),
the world's strongest engine evaluates positions with **NNUE** — a neural
network that takes a board and emits a single number like `+0.4`. It's
phenomenally accurate, but it has no idea how to explain itself: there is no
human-readable breakdown of *why* that position is worth +0.4. When chess.com
or Lichess label your move an "inaccuracy," that verdict comes from a neural
net. They can tell you *that* it was an inaccuracy and show you a better move —
but not *why* yours was worse, in concepts you could study and generalize.

This project takes the opposite bet. Instead of a neural net, it uses a
**classical, hand-crafted evaluation** — a hand-port of **Stockfish 11**, the
last major Stockfish version before NNUE. A classical evaluation isn't a black
box: the score is a **sum of named, weighted terms**, each one a concept a
human can read:

> Material · Imbalance · Mobility · Threats · Passed pawns · Space ·
> Initiative · King safety · Pawn structure · and per-piece positional terms
> for Knights, Bishops, Rooks, and Queens

…and beneath those, the finer ideas every coach teaches — doubled and isolated
pawns, outposts, the bishop pair, a rook on an open file, a knight with no good
squares. Because the evaluation is **decomposable**, the engine can hand the
teaching layer a full before/after breakdown of *which concepts changed* when
you made your move: "this move dropped King Safety from +8 to −24 and gained
nothing in return." That sentence is impossible to produce from an NNUE score.
It's nearly free here, because the decomposition is how the engine thinks in
the first place.

We are explicit about the trade-off. NNUE is genuinely **stronger** than
classical evaluation (~80–90 Elo in isolation; Stockfish 12 was ~130 Elo over
Stockfish 11 overall). chess.com's eval bar is more accurate than ours, and
that's fine — **the goal is never to be "more right" than chess.com.** The goal
is to be *transparent* where they are *opaque*. We only need to be strong
enough (~2000 Elo) to pose instructive positions; the product is the
explanation, not the rating.

### What that buys the student

The engine exposes a set of analysis surfaces (used by the app, and directly by
the CLI) that all trace back to concrete evaluation signals — never vibes:

- **Per-term eval trace** — the full named breakdown of a position's score.
- **Move critique** — score a move you played against the engine's best line and
  explain the swing: what it *allowed* the opponent to do, not just a number.
- **Threats / tactics / alignments** — hanging pieces, pins, forks, skewers,
  discovered attacks, overloaded defenders, trapped pieces — for *both* sides,
  with the geometry named.
- **Tactics resolve before strategy.** Chess is two modes: positional advice is
  only meaningful in a quiet position. The tutor checks for live tactics first
  and only talks structure once the position is calm — the same discipline a
  strong human applies.

---

## Believable opponents: the "perception" model

A teaching opponent has to be *beatable in a realistic way*. A 1000-rated bot
should feel like a 1000-rated human — making the kinds of mistakes a 1000 makes
— not like a grandmaster who occasionally and randomly throws a piece into the
void. This turns out to be the hardest unsolved problem in computer chess, and
it's where the project does something genuinely new.

### How everybody else makes a weaker bot

There are basically four techniques in the wild, and they share a failure mode.

1. **Depth limiting** — let the engine search only a few moves ahead. Simple,
   but it produces *"thin engine moves,"* not human moves. The Maia research
   team (KDD 2020) measured this directly: a depth-limited Stockfish matches the
   moves of *strong* humans better than *weak* ones — the exact opposite of what
   you want from a beginner bot. Throttling depth changes how *strong* the
   engine is without changing the *character* of its mistakes.

2. **Move-quality degradation** — Stockfish's "Skill Level" and `UCI_Elo`
   settings compute several candidate moves and then deliberately pick a worse
   one with some probability. It works down to a point, but `UCI_Elo` **floors
   at 1320** and is calibrated against an *engine-vs-engine* rating list, not a
   human scale — it literally cannot represent the 600–1200 players this tutor
   is built for.

3. **Evaluation noise** — add randomness to the scores so the engine sometimes
   prefers a worse move. This one famously *doesn't work*: the **Beal effect**
   (Beal & Smith, 1994) showed that even **random** leaf evaluations, run
   through a normal-depth search, still play surprisingly strong chess — the
   search "launders" the noise back into competence. Noise on top of search is
   not a reliable weakness dial.

4. **Random blunder injection** — every so often, force the engine to drop a
   piece. This is the source of the universal complaint about weak bots: they
   *"play like a grandmaster for 30 moves, then hang the queen."* The weakness
   is **uncorrelated with the position** — a real 1000 doesn't blunder at random
   intervals; they have *consistent, patterned* blind spots, the same kinds of
   moves they reliably fail to see.

There's also **Maia** — neural nets trained to predict the move a human of a
given rating band actually plays. It's the most human-*like* approach and a
genuine advance, but it has its own limits for a *teaching* product: the models
floor at ~1100, each plays *above* its nominal rating (they reproduce the
*average* move of a band, which skips that band's blunders), and — being a
neural net — they're a black box and largely deterministic. They can imitate a
human move; they can't explain one.

The thread running through all four: they tune *how strong* the bot is, but a
real weak player isn't a strong player with a dimmer switch. **A real player's
mistakes have shape.** They miss the move that's hard to *see*.

### What we do instead

We model the one thing all of the above ignore: **vision**. Before the engine
searches a position, every candidate move is scored for **how hard it is for a
human to actually notice** — a single "perception" dial (0–1) then decides
whether the bot even *considers* it. Moves the bot doesn't perceive are pruned
*before* the search ever sees them, so the engine genuinely cannot use a move it
"didn't see" — which sidesteps the Beal effect entirely (you can't launder a
move that was never in the tree).

A move's visibility is a product of human-plausible factors, all cheap to
compute from the board:

- **Direction** — backward and sideways moves are notoriously harder to spot
  than forward ones (the classic "I'd never have found Qe1" blind spot).
- **Knight moves** — the hardest piece to visualize; forks land off the lines
  the eye scans.
- **Occlusion** — a move whose point rides a screened diagonal, or threads a
  "pinch point" between two pawns, is easy to miss.
- **Attention** — moves far from where the action just happened (the opponent's
  last move) get less notice; a cluttered board hides more.
- **Special-rule moves** — en passant and underpromotion are genuinely
  invisible to most players.

Two consequences make the bots feel *human* in a way the standard techniques
don't:

- **Blind spots are patterned and stable.** The visibility roll is keyed to the
  position, so a bot misses *the same* hard-to-see diagonal all game long — a
  human "bad day," not a random dice roll. Its blunders are explainable after
  the fact: it didn't lose the queen at random, it never saw the backward-knight
  capture that won it.
- **It models *Hope Chess*.** The filter is harsher on the *opponent's* replies
  — exactly how real players blunder, by making a move without seeing the
  refutation. The bot overvalues its own plan because it can't see your answer,
  and walks into trouble organically. The *severity* of the resulting mistake
  emerges from what the unseen reply actually wins — it isn't dialed in.

To our knowledge, no other engine weakens itself by modeling geometric move
*visibility*. It's a small idea with an outsized payoff: weak bots that miss the
moves a human would miss, deterministically and explainably — which is exactly
what a teaching tool needs, because the bot's blind spots become *lessons*.

The single "opponent strength" slider in the app is calibrated against the Maia
human-rating ladder (offline, with the [`calibration/`](calibration/) harness)
so that a target number plays like a human of roughly that strength — and it's
been feel-validated to line up with chess.com ratings directly.

---

## What this project is *not*

- **Not a neural net.** NNUE is deliberately banned — the whole point is that
  evaluations must be human-decomposable. (We still use the *strong* engines as
  measuring sticks; we just don't ship one.)
- **Not trying to beat Stockfish.** ~2000 Elo is plenty to pose interesting
  positions. Strength is a means; explanation is the end.
- **Not an online service.** Fully offline, on-device — no network, no account.
- **Not a subscription.** Positioned as a one-time purchase: an interactive,
  adaptive answer to *How to Reassess Your Chess*, with a real opponent.

---

## Layout & status

A Rust workspace: the engine is a pure library; the teaching layer, shared UI
view-models, a CLI, and an egui desktop app sit on top; mobile shells are
planned behind an FFI boundary.

```
core/engine    classical SF11-style evaluation + search (the teaching core)
core/teaching  the single prose translator (Claim IR → "why this move")
core/ui        renderer-neutral view models, overlays, events
core/cli       the agent- and human-facing `chess-tutor` CLI
desktop        egui desktop app (Windows primary)
calibration    offline strength-calibration harness (Python; not shipped)
```

The engine and teaching layers are built; current work is strength calibration
(the opponent-Elo slider) and teaching-UX polish. For the working state, start
with [`HANDOFF.md`](HANDOFF.md); for project mission, licensing, and ground
rules, see [`CLAUDE.md`](CLAUDE.md).

---

## Sources for the claims above

- NNUE became Stockfish's default in v12: <https://stockfishchess.org/blog/2020/stockfish-12/>
- Stockfish 11's classical per-term evaluation trace: [`evaluate.cpp` @ `sf_11`](https://github.com/official-stockfish/Stockfish/blob/sf_11/src/evaluate.cpp)
- Depth-limited engines don't play like weak humans (Maia, KDD 2020): <https://arxiv.org/abs/2006.01855>
- The Beal effect (random evaluations still play strongly through search): Beal & Smith, *Random Evaluations in Chess*, ICCA Journal, 1994.
