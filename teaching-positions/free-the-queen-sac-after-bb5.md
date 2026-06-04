# Case study: the rook sac that frees a cramped queen (`…Bb5` edition)

A position from a real chess.com game between the user (~1200 ELO) and a bot. White is up a clean exchange-plus (Q + 2R vs R + 2B, +6 material) but the heavy pieces are jammed: the queen sits dead in the h8 corner, the king on e8 hides behind a pinned bishop, and there's no obvious way in. There are two lessons here, and the more valuable one is not the obvious one:

1. **(Verdict-labeling)** chess.com is *right* that the rook sac `Rxe7+` is the best move — but in Position 1 it's right for reasons no human and not even our own depth-20 engine can see. Stamping a "miss" on the user's genuinely excellent reply punishes them for not finding the unfindable. The severity of a verdict should track **findability**, not raw centipawn swing.
2. **(The real teaching gold)** Even *after* the sac wins back a bishop, White is still **down a point of material** (rook for bishop-plus-pawn). The move is correct anyway because the **position** — a bare enemy king under heavy-piece fire, plus an enemy rook frozen by the queen on the back rank — is worth more than the missing point. Teaching *that* valuation — "yes, this loses material, but the position you get is worth it" — is the single highest-value thing this tool can do for a player stuck at 1200.

Date analysed: 2026-06-03.

## The three positions

This game produced a sequence worth tracking as a unit:

**Position 1** — the original, queen cornered (White to move):
`4kb1Q/rp1bpp2/p2p2p1/8/8/3P4/PP3K1P/4R2R w - - 0 1`

**Position 2** — after `Rhg1` (user) and `…Bb5` (opponent), White to move:
`4kb1Q/rp2pp2/p2p2p1/1b6/8/3P4/PP3K1P/4R1R1 w - - 0 1`

**Position 3** — after the sac fires, `Rxe7+ Kxe7 Re1+ Kd7 Qxf8`, Black to move:
`5Q2/rp1k1p2/p2p2p1/1b6/8/3P4/PP3K1P/4R3 b - - 0 1`

```
Position 1                Position 2                Position 3
8 . . . . k b . Q        8 . . . . k b . Q        8 . . . . . Q . .
7 r p . b p p . .        7 r p . . p p . .        7 r p . k . p . .
6 p . . p . . p .        6 p . . p . . p .        6 p . . p . . p .
5 . . . . . . . .        5 . b . . . . . .        5 . b . . . . . .
4 . . . . . . . .        4 . . . . . . . .        4 . . . . . . . .
3 . . . P . . . .        3 . . . P . . . .        3 . . . P . . . .
2 P P . . . K . P        2 P P . . . K . P        2 P P . . . K . P
1 . . . . R . . R        1 . . . . R . R .        1 . . . . R . . .
  a b c d e f g h          a b c d e f g h          a b c d e f g h
```

## The chess.com saga: three "misses," and why the *label* is the bug

Over three consecutive moves chess.com flagged the user for a "miss," recommending the rook sacrifice `Rxe7+` **every single time**. Here's the crucial nuance, which is *not* about win% saturation: in every case `Rxe7+` genuinely **is** the best move. chess.com isn't wrong about the move. It's wrong about the **severity label** — because whether `Rxe7+` was *findable* changes completely from move to move, while the "miss" stamp does not.

| Move | User played | chess.com | Is `Rxe7+` actually best? | Findable? | Fair verdict |
|---|---|---|---|---|---|
| Pos 1 | `Rhg1` | "miss" | **Yes** (chess.com is right) | **No** — beyond human *and* beyond our d20 engine (the `…Be6` block hides the payoff) | **"Excellent."** `Rhg1` is our engine's own top move at d20 (+8.52) |
| — | `…Bb5` | "mistake" (no reason given) | — | — | genuine mistake — removes the `…Be6` defender (see below) |
| Pos 2 | `d4` | "miss" | **Yes** | **Yes** — d12-visible (`Rxe7+` +11.06 vs `d4` +7.95) | **"Miss" is fair** — a real, reachable improvement |
| — | `…a5` | — | — | — | — |
| Pos 3-ish | `a3` | "miss" | likely yes | likely yes (sac stays on) | probably fair |

The lesson for *our* product is sharper than "saturation noise," and it's a verdict-labeling principle:

> **A move's verdict should scale with how findable the better move was, not with the raw centipawn swing.** A −739cp "improvement" that requires NNUE-at-depth-24 to even *prove* is not a mistake a human made — it's a discovery a human couldn't make. Labeling it a "miss" punishes the impossible and teaches distrust.

And here's the happy accident: **our own engine, precisely because it's depth-20 non-NNUE, gets Position 1 right.** It can't see past the `…Be6` block either, so it ranks `Rhg1` as the *best* move and would never flag it as a miss. In other words, judging verdicts against an engine pitched at *human-reachable* strength is not a limitation to apologize for — it's the **humane and correct** behavior. We should lean into it: our verdict labels should be generated against a search whose ceiling is "what a strong human could find," and reserve harsh labels for genuinely reachable improvements like `d4` in Position 2.

(A note on win% saturation, since it's tempting to reach for here and would be **wrong**: saturation is a trap that *hides* lessons — the cp→win% sigmoid flattens in lopsided positions, so a real blunder can barely move the win%. The established rule is to **back win% with an absolute-cp gate so tactics are NOT suppressed**, never to use "the position is already decided" as a reason to stay silent. That would be the chess.com-"brilliant" failure mode — praising/excusing a move by its absolute resulting level instead of the eval delta it caused. Position 1 is not a saturation case at all: `Rhg1` isn't a miss because **our own engine rates it best**, i.e. zero positive delta — a pure eval-comparison, nothing to do with the position being won.)

## The horizon edge-case: how `…Bb5` flipped the sac from depth-24 to depth-12

This is the interesting part and the reason the position is worth keeping.

**In Position 1, our engine cannot see that `Rxe7+` works** — not at d12, not at d20. It ranks it 2nd, −739cp behind quiet play. The reason is a single defensive resource:

```
Rxe7+ Kxe7 Re1+ Be6   ← the d7 bishop interposes on e6
```

`Be6` blocks the check with a piece. That piece then has to be won by a *slow, quiet* pin-overload (`d4–d5`, maneuver, capture) — a non-forcing subtree that alpha-beta has to grind through ply by ply. It eats depth. When we hand the engine the post-`Be6` position directly (`5b1Q/rp2kp2/p2pb1p1/8/8/3P4/PP3K1P/4R3 w`), it finds `d4!` and climbs to **+5.97 at d22** — but from the root at d20 the payoff is over the horizon.

**In Position 2, the bishop has moved to b5 — and that single displacement removes the block entirely.** From b5 the bishop *cannot reach the e-file*: the `b5–c4–d3` diagonal is blocked by White's own d3 pawn, and e4/e6 aren't on its diagonals at all. So after `Rxe7+ Kxe7 Re1+`, Black has **king moves only**, and every one walks into `Qxf8` winning the dark-squared bishop with a raging attack. Every branch is now a check or a capture — forcing moves that search extensions tear through almost for free.

> **The key insight:** `…Bb5` didn't just lose a tempo. It changed the *search-theoretic shape* of the position — from "bushy, with a quiet defensive interposition" to "a narrow forcing corridor." That is **the same fact** as "the engine couldn't see it before but can now" and "it was a genuine mistake," viewed three ways. It's also the concrete, nameable "why" chess.com waved at but couldn't articulate: **`…Bb5` removed the only defender of the `e6` interposition square.**

## What the static eval shows about "freeing the queen"

The user's intuition: even without seeing the mate, the sac is worth it because it (a) wins a pawn immediately, (b) has a short forcing follow-up to win a bishop (R for P+B = −1 material), and (c) **liberates a dead queen and throws her + a rook at a bare king.** Does the static eval confirm that the *positional* payoff outweighs the material cost?

Comparing the static `eval` of Position 2 (before the sac) and Position 3 (after `Qxf8`):

| | Static eval (white-POV) | Material | Phase |
|---|---|---|---|
| Position 2 (before) | **+9.58** | White +6 | 46/128 |
| Position 3 (after `Qxf8`) | **+11.69** | White +5 | 22/128 |

**A +2.1 pawn swing in White's favor — while spending a point of material.** So the position is worth ~+3 pawns of pure positional compensation for a 1-pawn outlay. The user's ">+1.0 worth it" hypothesis is confirmed, roughly tripled.

Where the swing lives (net terms, White POV, engine-cp at PawnEG=213):

| Term (net) | Before mg / eg | After mg / eg | Read |
|---|---|---|---|
| **King — danger** | +286 / +67 | **+3211 / +226** | The whole story. Order-of-magnitude jump. |
| King — flank-attacks | +56 / 0 | +80 / 0 | More attackers in the king zone |
| King — pawn-shield | −125 / 0 | −14 / 0 | His king lost its cover |
| Queen mobility | +48 / +92 | +60 / +113 | Freed queen — but only a *modest* tick |
| Rook mobility | +103 / +348 | +73 / +184 | Down a notch (a rook was traded off) |
| Threats (net) | −93 / −45 | +1 / −25 | Black's counter-threats gone |
| Trapped rook | 0 / 0 | +104 / +20 | **Queen on f8 freezes Black's a7 rook** — it controls the whole 8th rank, so `…Ra8` hangs and `b7` blocks the lateral escape. A whole enemy rook taken out of the game. |

Two subtle, important readings:

1. **The signal is "King danger," not "Queen mobility."** This corrects the natural framing. Queen mobility barely moves (+48 → +60). The freed queen's real value is that she becomes an *attacker adjacent to a bare king* — which the engine books under **King danger / flank-attacks**, not mobility. Mobility counts *squares*; the king-attack terms count *squares that matter*. The right mental model isn't "I freed my queen to roam," it's **"I added a second attacker to the king's zone."**

2. **Static eval *understates* this position, and the phase number tells you why.** Phase drops 46 → 22 (lots of material off → endgame-weighted), which heavily discounts the +3211 *middlegame* king-danger term — that's why the headline only moves +2.1 instead of +15. The engine is hedging: a bare board gives the king room to run, so it won't credit a mate it can't prove. And it's right to — the *search* reaches **+14.67**, well above the static +11.69, because it converts the attack into material the static eval can't foresee. **Takeaway for intuition-building: static eval will tell you a heavy-piece king hunt is strong, but it is the conservative floor — once material thins it systematically under-credits the attack.**

## Can static eval explain why `…Bb5` was bad? No — and *that's the lesson*

The natural follow-up: if static eval tells the story of the *sacrifice* so well, does it also explain why Black's `…Bb5` (the move that made the sac work) was a mistake? **It does not — and the fact that it can't is itself the most important finding here.**

First, isolate `…Bb5` cleanly. Position 1 → Position 2 spans *two* half-moves (`Rhg1` + `…Bb5`), so compare the position *after `Rhg1`, bishop still on d7* against *after `…Bb5`*:

| Position | Static eval (white-POV) |
|---|---|
| After `Rhg1`, bishop on **d7** | **+9.27** |
| After `…Bb5`, bishop on **b5** | **+9.58** |

`…Bb5` moved White's static eval by **+0.31 pawns** — a rounding error in a +9 position. Statically the move is *fine*. Its only fingerprints, all tiny (net engine-cp, White POV):

| Term | d7 → b5 | What it caught |
|---|---|---|
| King-protector | +14 / +16 | bishop walked **away from its own king** |
| Minor-behind-pawn | +18 / +3 | bishop **left its sheltered post** (it sat behind the d6 pawn) |
| Bishop mobility | +25 / +41 | the b5 bishop actually has *fewer* safe squares (own d3 pawn blocks its diagonal) |

That's all static eval can say: *"the bishop left the king's side and a sheltered square,"* worth ~0.3 pawns. **No human weights a 14cp king-protector shift as a reason not to develop a bishop.** The real cost — removing the lone `…e6` interposition that refutes the rook sac — casts **zero static shadow**. It is purely tactical, purely search-visible.

> **The orthogonality principle.** `…Bb5` is **statically sound but tactically losing** — the exact *mirror* of the sacrifice, which is **statically (materially) losing but tactically winning.** Static soundness and tactical soundness are independent axes. A teaching tool must never let one masquerade as the other: not every static-neutral move is safe, and not every material-losing move is bad.

### What our own retro currently says about `…Bb5` (and why it's right to flag)

Running `critique` on `…Bb5` today, our engine fires an **"ALLOWED, NOT MISSED"** banner: a 1.8-pawn swing (+7.89 → +9.69), names the refuting line (`Rxe7+ Kxe7 Re1+ Kd7 Qxf8`), and points at the `danger:` pins. That's *better than chess.com* — we give a concrete, named reason instead of "bad to worse" — **and, contrary to an earlier draft of this doc, it is the correct call. `…Bb5` IS a real error.**

Two anti-patterns to explicitly reject here, because the first draft fell into both:

- **Do NOT excuse it as "the game was already decided."** That is absolute-level reasoning, and it's exactly the failure mode that teaches people to blunder. The position being +7.89 for the *opponent* says nothing about whether *this move* was an error. A move is judged by the **eval delta it causes**, never by the absolute level it lands at. (A rook blundered into a fork while up a queen still reads ~99% win — and is still a blunder we must show. Same principle.) `…Bb5` gave up ~1.8 pawns at d12, and far more at d20 — that is a real, our-engine-visible delta.
- **Do NOT excuse it as "statically fine."** By the orthogonality principle one paragraph up, static soundness and tactical soundness are independent. `…Bb5` is the poster child for *statically sound, tactically losing*; "it costs nothing static" is therefore irrelevant to whether the move was an error. It was.

The one *legitimate* reason we would NOT flag a move like this: if the played move's eval were **similar to or better than** the alternative — then it's an *alternative*, not an error (and sometimes the "tactic" you skipped was actually worse, e.g. it blundered mate). That is a pure **eval-delta** test against the best move *our own (human-pitched) engine* sees. `…Bb5` fails that test (the available `Ra8` was ~1.8 pawns better), so it correctly flags.

The richest treatment is then a **perspective** one, not a suppression one: because the user was White, `…Bb5` is best surfaced as an **opportunity trigger** — "your opponent just left the square that was holding your sac together; *now* `Rxe7+` crushes, and here's the static reason." Same move, rendered from `Perspective::Opponent`. We flag it; we just frame it as White's gift rather than scolding the bot.

## The pre-`…Bb5` sacrifice: static eval loves it *despite* the defense

One more position, the counterfactual where White sacs *with the bishop still able to defend* (here on c6, after `…Kd7`, able to block a future `Re7+`/`Qe7+` with `…Bd7`): `5Q2/rp1k1p2/p1bp2p1/8/8/3P4/PP3K1P/4R3 b`.

> **Static eval: +11.68 white-POV** — essentially identical to Position 3's +11.69, where the bishop *can't* defend.

Same crushing trio: **King danger +3153**, **Trapped rook +104** (queen on f8 freezes the a7 rook), **Queen mobility +60 / +113**. The defensive resource that ties our *search* in knots (the deep `…Bd7` block lets Black consolidate to "only" +5–6) is **invisible to static eval** — which simply reports the geometry it sees: bare king, dominant heavy pieces, frozen rook, and rates it ~+4 pawns above the bare material count.

This is the load-bearing insight for the teaching layer:

> **For valuing a sacrifice, our static eval is a *better* teaching signal than our depth-20 search.** The search gets dragged back toward material by defenses a human also cannot calculate (`…Be6`, `…Bd7`). The static eval ignores those and reports the positional gestalt — which is *exactly* the instinct a strong human feels and a 1200 needs to learn. **Drive the sacrifice-justification card off the static-eval term diff, not the search verdict** — the search number paradoxically under-sells the very thing we want to teach.

## What the ideal engine does with this whole game

This game is a near-complete spec for what chess-tutor should *be*, distinct from chess.com's black box. The target behavior, move by move:

- **`Rxe7+` → "strong"** — declared strong because of the **static position it creates** (a bare king under Q+R, a frozen rook), *not* because of a deep search PV. We can say *why*; chess.com can only say *that*.
- **`Rhg1` → "strong/excellent"** — our d12 engine liked it and our d20 engine liked it *most*. A good move is a good move; the existence of a NNUE-only alternative doesn't demote it.
- **`d4` → a real "miss"** — the one move where the better option (`Rxe7+`) was both **reachable** (d12-visible to our own engine) and **genuinely better** (a large eval delta vs. what was played). This is what "miss" is for.
- **`…Bb5` → a real (small) error, surfaced as the opponent's gift** — *not* "fine." It gave up ~1.8 pawns at d12 by allowing `Rxe7+`, and the available `Ra8` was strictly better. "Statically sound" does not rescue it (orthogonality), and "already decided" must not excuse it (absolute-level reasoning trains blunders). Because the user is White, render it from `Perspective::Opponent` as *"your opponent opened the door — here's why `Rxe7+` now wins."*

The unifying definition the game forces on us — purely **eval-delta**, never absolute level:

> **A "miss" is failing to play a move that our own (human-pitched) engine rates *meaningfully better* than what was played.** Two clean consequences: (1) if the played move is *similar-or-better* than the "tactic," it's an **alternative**, not a miss — and the tactic may even be worse (it blundered mate). (2) "Meaningfully better" is judged in **cp / material delta** (with win% only as an *additional* trigger, never the sole gate — the sigmoid saturates and would hide real lessons in lopsided positions). What makes a god-engine-only line like `Rxe7+`-from-Position-1 *not* a miss isn't a special "findability gate" — it's simply that **our engine doesn't rate it best either**, so the delta is zero-or-negative. Diff against the engine a strong human could be, and findability falls out for free.

And the mission statement this case crystallizes:

> **The goal is not to teach players to calculate tactics 10 moves out. It is to teach them to recognize when the board has created the *circumstances* that make 10-move tactics exist** — "a rook is aimed at the enemy king," "the enemy queen is cramped behind her own pieces," "the king has lost its defenders," "a defender just left the square it was guarding." These are the instincts strong players accrete over years. We have the tools to name them on demand — the **decomposed static eval** and the **tactic detectors** — which a NNUE black box structurally cannot. The entire engineering challenge is heuristic discipline: surfacing these signals **without false positives, misleading phrasing, or non-lessons.** This game is a stress test for exactly that discipline — it contains a real miss (`d4`), one move wrongly stamped a mistake that is actually engine-best (`Rhg1`), one small-but-real error best surfaced as the opponent's gift (`…Bb5`), and a positional-sacrifice valuation worth more than the material it spends. The discipline is in telling those four apart by **eval-delta against our own engine** — not by the absolute level of the position.

## Depth is the wrong lever (don't lower the retro search)

Tempting fix: if `Rxe7+`-style lines are "too deep to be human," run the retro at a shallower depth so only human-findable moves surface. **Tested at depth 8 — it fails in both directions at once:**

| Position | Move played | d8 result | the problem |
|---|---|---|---|
| Pos 1 (before `Rhg1`) | `Rhg1` | 2nd, −58cp behind `Qd4` → clean | (fine either way) |
| After `Rhg1` | `…Bb5` | **still flagged** "ALLOWED", 1.1pp | the move we're unsure about *still flags* |
| Pos 2 (after `…Bb5`) | `d4` | reads **tied-for-best**; `Rxe7+` only +0.38 and unstable | the genuine lesson **vanishes** |

The sac's measured value *grows with depth* (`Rxe7+`: **d8 +0.38 → d12 +3.1 → d20 +14.67**) as the engine sees more of the king hunt. At d8 it's a flicker; at d12 it's stable and decisive. So **d12 is the shallowest depth where the real lesson is both visible and stable** — lower it and you go blind to exactly the tactic most worth teaching, while *not* silencing the borderline `…Bb5` flag. Depth 8 is strictly worse. **The lever is better heuristics, not a shallower search.**

## Surfacing the lesson: walk the PVs, not just the best move

The hard part is recovering the *human-teachable* "why" for a best move whose payoff isn't at move+1. Two cases share the signature **static-delta ≈ 0 at move+1, large search-delta** — and they are NOT the same lesson, so you cannot collapse them into one rule:

1. **Deferred own-tactic** — the best move's value materializes deeper in *its own* PV (e.g. `Rxe7+`: the king-danger explosion lands only after `…Kxe7 Re1+ … Qxf8`).
2. **Prophylaxis** — the best move removes an *opponent's* tactic (`Ra8`: it never improves the static eval; it just makes `Rxe7+ … Qxf8` fail to `…Rxf8`).

A single static-eval reading at move+1 can't tell these apart, and — critically — **you cannot teach prophylaxis without showing what it prevented.** A plain best-move search discards that: "what you prevented" lives in the *pruned* leaves of branches that never happened. **The retrospective is special because it keeps the lines a forward search throws away** — the user's actually-played move, the engine-best move, and (with MultiPV-2/3) often a third. That gives us material to diff.

The method this position argues for:

1. **Walk the user's actual PV ply-by-ply, computing static eval at each step.** Find the **explosion point** — where the static eval lurches against them. The *opponent move* at that point is "the thing you were supposed to prevent." For `…Bb5`: walk `Bb5 Rxe7+ Kxe7 Re1+ Kd7 Qxf8` → the static eval erupts at `Qxf8` (king danger, freed queen) → **that** is the lesson, expressed in static terms a human can feel.
2. **Read the static terms at the explosion**, not just the number — king-danger / trapped-rook are the human-legible "why," exactly as in the constructive sacrifice case.
3. **Confirm prophylaxis with a replay test** (the user's idea): after the *best* move, replay the punishing tactic. If `Rxe7+` now *fails* (because `…Rxf8` wins the queen), the best move's value was **removing that tactic** — that's the distinguisher between prophylaxis and a deferred own-tactic, and it gives the exact sentence to show: *"`Ra8` doesn't build anything — it makes `Rxe7+` stop working."*

This reframes the whole "what would the best move have bought you?" idea: for a **constructive** best move you read the static eval at the climax of *its* PV; for a **prophylactic** best move you read the static eval at the explosion of *the user's* PV (the disaster avoided) and confirm with the replay. Same tool (static eval along a PV), pointed at different lines. It stays a design sketch, not an implemented plan — but the retro's possession of both PVs is what makes it tractable at all.

## The teaching lesson — and why it's the whole point of this tool

The headline lesson is **not** "`…Bb5` moved the defender so we could win the dark-squared bishop." That's just the *mechanism* that made the line computable. The substance is one level deeper, and it's the thing chess-tutor exists to teach:

> **You can be down material and still be winning, because the *position* is worth more than the missing piece.** After the whole forcing sequence White is still **−1 in material** (rook for bishop-plus-pawn). The sac is correct anyway — the compensation is entirely positional: a bare enemy king with two heavy pieces crashing in (King danger +286 → **+3211**), and an enemy rook frozen out of the game by the queen on the back rank (Trapped rook 0 → **+104**). The static eval *swings +2.1 in White's favor while material goes down by 1.*

This is the 1200 → 1600 intuition gap in its purest form. A 1200 counts material and stops. A 1600 asks "what is this position *worth* — in king safety, in activity, in pieces I've taken out of play — and is that worth more than the point I'd spend?" The generalizable rule:

> **When your pieces are cramped or blocked out of the attack, spending 1–3 points of material to pry open lines and throw heavy pieces at the enemy king is often worth it — even when you never reclaim the material.** The payment is in pawns; the return is in king-danger and activity. Learn to price that return.

If we can build a surface that makes a student *feel* "yes, this loses a piece, but look how much the position is worth" — with the concrete terms (king danger, trapped rook) shown as the receipt — that is the single highest-value thing this tool can deliver. It's the entire reason the engine exposes a decomposed eval instead of a black-box number.

And the human-tractable *calculation* kernel is only ~5 plies, not 12:

1. `Rxe7+` drags the king out (declining via `…Kd8` runs into `Qxf8+` anyway).
2. `…Kxe7 Re1+` — the second rook checks on the open file.
3. **The one real observation:** can the bishop interpose? (In Pos 1: yes, `…Be6` — sac unclear. In Pos 2: no, it's on b5 — sac crushes.)
4. `…K-moves Qxf8` collects the bishop; the freed queen leads the hunt.

## What this would take to teach automatically

The user's own proposed mechanism, recorded here because it's the right shape:

> Any time there's a **sacrifice in the PV** (a move whose immediate SEE is materially negative), walk each term of the eval trace *before vs. after the forcing sequence* and surface the term that explains the compensation — **specifically one that does NOT depend on reclaiming the material**, but on a purely positional gain (here: King danger +286 → +3211).

Concretely this is a new analysis surface — call it a **sacrifice-justification card**:

1. Detect a material-losing move that the search nonetheless ranks at/near the top (PV head with negative SEE).
2. Diff the static eval trace of the pre-sac position against the post-forcing-sequence position.
3. Find the dominant *non-material* term that flipped in our favor (king danger, mobility, file control…).
4. Phrase it: *"This gives up R for P, but it's worth it: it frees your queen onto an exposed king — king-safety pressure jumps from X to Y."*

The hard parts, flagged honestly:
- **Defining the comparison endpoints.** The "after" position is the end of the forcing tail, not the move+1. Needs the PV walked to where it quiesces.
- **Not double-counting material recapture.** The card is only honest if the justification survives *excluding* eventual material regain — otherwise it's just "you win material," which the existing surfaces already say. The filter must isolate the purely-positional delta.
- **Phase distortion.** As shown above, the most dramatic term (mg king-danger) may be the one the final blend discounts. The card should report the term that moved, but the *headline* number should stay the phase-correct eval, or it'll over-promise.
- **This overlaps the [tactical-vs-positional mode](../HANDOFF.md) switch.** The moment a position collapses from "quiet defense available" to "all-forcing" (the `…Bb5` moment) is exactly when tactics go live — a sacrifice-justification card and a "tactics are now live here" flag are two faces of the same detector.

## Regression target

Three things this position should exercise once the relevant surfaces exist:

1. **Findability-scaled verdicts (defensive).** Fed Position 1 with `Rhg1` as the played move, the coaching/retrospective layer must **not** produce a "miss" card — even though a god-mode engine prefers `Rxe7+`. The reason here is *not* saturation; it's that `Rhg1` is our own engine's top move at d20, so the better alternative is unreachable. Verdict severity must be generated against a human-reachable search, so unreachable improvements read as "excellent," not "miss." (Contrast: `d4` in Position 2 *should* draw a real tactical-miss card — there `Rxe7+` is d12-visible and genuinely reachable.)
2. **Sacrifice justification (offensive) — the headline.** Fed Position 2, a future sacrifice-justification surface should explain `Rxe7+` as *"this stays down a point of material, but it's worth it: a bare enemy king under your queen + rook, and his a7 rook frozen by your queen on the back rank"* — leaning on the **King-danger** and **Trapped-rook** terms — and explicitly **not** on the eventual material recapture (because there isn't a net one; White ends −1).
3. **Positional-compensation literacy.** The student-facing takeaway must land the *"down material, still winning, because the position is worth it"* intuition — with the eval terms shown as the receipt. This is the gold; items 1 and 2 are plumbing in service of it.

Keep this paired conceptually with the [discovered-attack](discovered-attack-after-qxe6.md) and [positional-punish](positional-punish-after-qe6.md) cases: those are *"you missed the opponent's tactic"*; this is two mirrors at once — *"you were dinged for declining a sac no one could find,"* and *"the sac, once findable, was a positional sacrifice you stay down material on."*
