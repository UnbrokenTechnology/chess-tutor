# Case study: the racing-checks trap after `Ng5`

A position from the same user's chess.com game pool. White (the user, ~1200 rated) had a position so winning the engine reports +9.78 for the best move. Looking at the kingside, the user saw their queen on h6 next to an exposed black king with no pawn shield, and noticed that a single knight hop to g5 would create `Qh7#` mate-in-1 — the knight defends h7, the queen attacks h7 with check, and Black's king has no escape squares.

The user played `Ng5`. Chess.com flagged it as a blunder, attributing it to "you permitted the opponent to win material through a fork." Our engine agrees it's a blunder but for a much harsher reason: **after `Ng5`, Black has forced mate against the *white* king in 7 plies**, swinging the eval from +9.78 to roughly −∞. The eval bar tells the truth; chess.com's narrator picks an irrelevant cosmetic feature (a real-but-non-load-bearing fork) and labels with that instead of admitting the actual mechanism is beyond what its narration can express.

This is the fourth case study in the [`teaching-positions/`](.) folder, and the one we have least clarity on what to *do* with. It's filed here as a complexity benchmark, not as a regression target for a specific detector.

Date analysed: 2026-05-29.

## The position

FEN (White to move):
`5rk1/ppp1qp2/4b1nQ/4p3/3p4/2P2N2/P1P3PP/2KR1B1R`

```
8 . . . . . r k .
7 p p p . q p . .
6 . . . . b . n Q
5 . . . . p . . .
4 . . . p . . . .
3 . . P . . N . .
2 P . P . . . P P
1 . . K R . B . R
  a b c d e f g h
```

Material: White is up a clear rook (two rooks + bishop + knight + 5 pawns vs one rook + bishop + knight + 6 pawns). Engine eval with best play: **+9.78** in white's favour. This is a "convert and win" position by every measure except the specific way the move is chosen.

## What chess.com said

> You permitted the opponent to win material through a fork.

The "fork" exists: `…Qxg5+` captures the white knight, attacks the white queen on h6 along the g5-h6 diagonal, *and* checks the white king on c1 along the g5-f4-e3-d2-c1 diagonal. Three simultaneous attacks from one move. That is the textbook geometric definition of a fork. **And it's irrelevant to why Ng5 is bad.** If Black actually played `…Qxg5+`, White recaptures with `Qxg5` — Black has paid a queen for a knight, a net loss of six points of material. Black wouldn't voluntarily do this.

So chess.com is correctly identifying the move as a blunder (the eval swings by ~15+ pawns; their engine is right about that) but the *attribution* it offers — pick a real-looking tactical pattern that exists on the board and stamp it onto the narration — doesn't describe the actual mechanism. The user reading "you permitted a fork" can correctly verify that the fork doesn't lose material if executed, conclude chess.com is being silly, and *miss the actual reason their move was bad*.

This pattern (NN narrator picks a plausible-but-non-load-bearing feature when the real mechanism is beyond its expression range) has shown up in every one of the four case studies in this folder. Filing the observation again here for the cumulative record.

## What's actually happening

Engine PV after `Ng5`:

```
Ng5 Qa3+ Kd2 Qxc3+ Kc1 Qa3+ Kb1 Bxa2+ Ka1 [mate]
```

Each check forces White's king onto the next bad square, and the next check is always available. The sequence is:

1. **`…Qa3+`** — Black's queen swings to a3 via the long diagonal `e7-d6-c5-b4-a3`, which has been open all game. The queen attacks Kc1 along `a3-b2-c1`. White must move the king. (Note that there's no defender available to block on b2 — Nf3, despite my initial confused claim, never had a tactical role here; even if it had stayed home, it couldn't have stepped to b2 or interrupted this diagonal.)

2. White picks one of the legal escapes (`Kd2` per engine PV; `Kb1` is also legal). Either choice runs into the next check.

3. **`…Qxc3+`** if `Kd2` was chosen — the queen captures the c3 pawn and checks along the c3-d2 diagonal. `Kxc3` looks like it should work but is illegal: the d4 pawn covers c3.

4. White retreats `Kc1`. Black plays `…Qa3+` *again*, since the a3-c1 diagonal is now empty.

5. White's only safe square is `Kb1` (Kd2 now loses to `…Qe3#` — the c3 pawn that previously blocked the queen's path from a3 to e3 is gone, and the d4 pawn covers c3 so `Kxe3` is illegal). Note that earlier in the sequence Kd2 was a fine escape; the *same square* is now mate because one piece (the c3 pawn) was removed from the board two moves prior.

6. **`…Bxa2+`** — the bishop captures the a2 pawn. Not a sacrifice: the queen on a3 defends a2, so `Kxa2` is illegal. Bishop arrives for free, peels the last pawn, and forces `Ka1` with no choice.

7. Mate follows.

## The d4 pawn is doing all the work

Look at what the d4 pawn does in this sequence:

- Covers c3, so `Kxc3` is illegal after `…Qxc3+`.
- Covers e3, so `Kxe3` is illegal after `…Qe3#`.

Without the d4 pawn, the entire mating net falls apart — *every* king move that looks blocked has a king-takes-checker escape that the d4 pawn closes. **The d4 pawn isn't restricting any of White's *immediate* escape squares (b1, b2, d2 are not covered by it). It's restricting the squares the king would be driven to *during* the check sequence.** That's why it's not visible if you look only at "where can my king move right now?" — its role only becomes apparent if you walk the check sequence forward and ask "where does the queen want to land, and what makes the king unable to capture it there?"

The defensive moves the engine prefers all touch this:

- **`Rd3` (+9.78):** defends c3 along the file. After `…Qa3+ Kd1 …Qxc3` is no longer free — the rook recaptures.
- **`Re1` (+9.03):** covers e3 along the file. `…Qe3#` ceases to be mate because `Rxe3` exists.
- **`cxd4` (+8.57):** removes the keystone pawn directly. After `…exd4` Black has rebuilt a pawn on d4, but the path now has only one cover (not two), and `Rxd4` is no longer hanging a rook.
- **`Kd2` (+8.44):** doesn't *prevent* the loss of a pawn — Black plays `…dxc3+` and White retreats `Ke1`, losing a pawn — but the trade is favourable because both d4 and c3 come off the board simultaneously, dismantling the mating geometry. White trades a pawn for total king safety.

All four winning moves attack the same problem: **break the d4 pawn's grip on the king's escape squares**, either by removing it (cxd4, Kd2's invitation), covering one of its squares with another piece (Re1 on e3, Rd3 on c3), or stepping the king out of the corridor entirely (Kd2 sidestep).

`Ng5` does none of those things. It commits a piece to a kingside attack that needs *one full tempo* to execute (Qh7# can't fire until the move after Ng5), and Black uses that tempo to start an unbreakable check sequence on the other wing.

## What makes this teachable (and what doesn't)

We don't yet know the right UI for surfacing this kind of position. Some things we can say:

**What's not the right framing:**
- "Nf3 was a load-bearing defender." It wasn't. The square it occupied didn't matter for the mating net.
- "You walked into a fork." There is a fork but it's not what loses the game.
- "Address every opponent check." Way too strong. Most checks are stalls; players who freeze at every check don't make progress.
- "See the mate-in-7." Not human-findable. Filing this position as a *teachable blunder in the same sense the desperado position was teachable* would be dishonest.

**What does seem to be the right framing — a user-articulated rule of thumb:**

> If the opponent has a check, and the position after the check has *another* check available, and that one *also* has another check available — i.e. the forcing line is at least three checks deep — that's a signal to stop and reconsider, regardless of whether you can actually see the mate.

The user articulated this after working through the position. The intuition is that a check sequence of length three or more carries a non-trivial probability of either looping (perpetual / draw by repetition, sometimes acceptable but worth being deliberate about) or closing in on a mating net (catastrophic). A human can validate this *without* calculating to the mate — they just walk three checks forward and notice "there's still another check available, and the king is being herded toward a smaller area." That's enough information to reject the candidate move and look for something more defensive.

This isn't a chess engine detector. It's a *human discipline*: when committing to a non-forcing plan in a position where the opponent has visible checks, walk the forcing line forward ~3 plies and see whether the checks naturally die out or whether they're self-replenishing. If they self-replenish, treat the candidate move as suspect.

**What this means for the teaching layer (provisional):**

We already have the engine machinery this would touch:
- The tactic detector chain catches the `…Qxg5+` fork (real, present, not the mechanism).
- The king-danger evaluation term is sensitive to king exposure (presumably scores Kc1 in this position as dangerous, even if it can't articulate why).
- The search itself sees the mate at modest depth (the −#7 verdict is at depth 18 in our engine; chess.com's NNUE presumably sees it shallower).

What we *don't* have is a way to translate "the search sees a mate at depth ≥ 7 but no named tactic detector fires on the user's move" into a narration that's honest about what's happening — i.e. *"your opponent has a long forcing sequence available that ends badly for you; the specific mechanism is too deep to articulate"*. Building such a narration would risk the false-explanation failure mode (the very thing we keep catching chess.com doing). The honest version might just be: *"this move leaves your king under sustained attack; the engine sees a forced sequence beyond practical calculation depth — consider a king-safety move instead"* — without naming the mechanism.

Filing this here because the position is a clean example of the failure mode, not because we know what to build for it. Future UX iterations should be able to point at this case as a regression check: **do not narrate a fake mechanism when the real one is depth-out-of-reach, but also do not silently call it a blunder with no explanation**. There's a middle ground — "your king position is fragile here and the engine sees a long forced sequence we can't summarise" — and finding it is the design problem this position illustrates.

## How this fits with the other three case studies

The four case studies together bracket what the teaching layer should and shouldn't claim:

| Case | Mechanism | Visibility | Teaching action |
|---|---|---|---|
| [`positional-punish-after-qe6`](positional-punish-after-qe6.md) | RemovingDefender + Desperado | static, detectable | Surface as missed tactic, name the pattern |
| [`discovered-attack-after-qxe6`](discovered-attack-after-qxe6.md) | DiscoveredAttack (latent) | static, detectable (needs latent-threat detector) | Surface as latent opponent threat, name the pattern |
| [`silent-sequencing-after-qc8`](silent-sequencing-after-qc8.md) | Deep positional sequencing | invisible below depth 8 | Suppress blunder framing entirely — no honest mechanism to teach |
| **`mating-net-after-ng5` (this file)** | King hunt via continuous checks | mechanism depth-out-of-reach; warning signals exist but no single one is decisive | **Open question** — neither "surface as named tactic" nor "stay silent" is right |

The fourth case is the genuinely hard one. The mechanism is real (it's a forced mate, not a sequencing accident like the Qc8 case), but it's not a named pattern (no fork / discovered attack / pin describes it cleanly), and the user-actionable signal (king under sustained attack with restricted escape) isn't crisp enough to fire a single detector reliably. The right teaching-layer response is probably *some kind of structural warning that doesn't try to name a mechanism* — but designing that without falling into the chess.com false-explanation failure mode is exactly the UX problem we don't have a clear answer to yet.

Keeping the position here as a worked example for when we revisit the question.
