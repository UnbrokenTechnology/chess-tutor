# Case study: the missed desperado after `…Qe6`

A position from a real chess.com game between the user (~1200 ELO) and a 1400 bot, surfaced as a chess.com "game review" finding that the app's NN-driven narrator could not articulate. The post-mortem here is what the chess-tutor teaching layer **should** be able to produce automatically once the pieces are wired up. This file exists so we have a reproducible reference position for that wiring.

Date analysed: 2026-05-29.

## The position

```
8 r . b . k b . r
7 . p . . . p p p
6 p . . p q n . .
5 . . . . p N B .
4 . . . . P . . .
3 . . N . . . . .
2 P P P . . P P P
1 R . . Q K . . R
  a b c d e f g h
```

FEN (White to move, after `8…Qe6`):
`r1b1kb1r/1p3ppp/p2pqn2/4pNB1/4P3/2N5/PPP2PPP/R2QK2R w KQkq - 1 9`

The position one ply earlier (Black to move, with the queen still on d7):
`r1b1kb1r/1p1q1ppp/p2p1n2/4pNB1/4P3/2N5/PPP2PPP/R2QK2R b KQkq - 0 9`

## What happened in the game

| Side | Move | Why |
|---|---|---|
| Black | `8…Qe6` | Wanted to "develop the queen + put more pressure on f5." |
| White | `9.O-O` | Standard development. "It's been a long time since I castled; let's get the king tucked away." |
| chess.com | "you missed an opportunity to punish your opponent's mistake. Best was `Ne3`." | Could state *that* the eval shifted (+1.59 → +0.60), couldn't say *what* the opportunity was. |

The user's read of `Ne3` after the fact, on the surface:

- Doesn't create a tactic.
- Doesn't win material.
- Moves a knight from an aggressive square to a less aggressive one.
- Gives up space.
- Backs off the kingside attack.

All of those observations are **true**, and that's the trap. `Ne3` is, by every static positional yardstick, a slightly *worse* move than the natural alternatives. The reason it's still the engine's pick has nothing to do with static positional features and everything to do with what the alternatives lose to.

## The hidden refutation: `…Nxe4` with the f5-knight desperado

Black's `Qe6` and the earlier `Qd7` *both* attack `Nf5` along the diagonal. White's `Nf5` is defended **only** by the `e4` pawn. That makes it a classic *remove-the-defender* shape — and Black has the tool to execute it: `…Nxe4`.

The naïve refutation Black would *play* — the one a 1200 sees and discards — is the direct recapture:

```
1…Nxe4
2.Nxe4   (white Nc3 recaptures)
2…Qxf5   (Black wins the knight because nothing defends f5 anymore)
```

Material flow:

| Move | What was captured | Running material delta (white POV) |
|---|---|---|
| `1…Nxe4` | white e4 pawn | −1 |
| `2.Nxe4` | black f6 knight | −1 + 3 = +2 |
| `2…Qxf5` | white f5 knight | +2 − 3 = **−1** |

So the direct recapture line nets **white down one pawn**. That's bad, but in isolation it's "annoying," not "ruinous" — and the user's intuition that "I have a knight on c3 to recapture" was tracking this line. Crucially, this is *not* the line the engine cares about.

The engine sees the **desperado**: instead of passively recapturing, white sacrifices the doomed `Nf5` for the `g7` pawn before it dies.

```
1…Nxe4
2.Nxg7+!  (the f5 knight takes the g7 pawn with check — it was going to be lost anyway, so cash in)
2…Bxg7    (Black must recapture)
3.Nxe4    (now white Nc3 mops up the e4 knight)
```

Material flow:

| Move | What was captured | Running material delta (white POV) |
|---|---|---|
| `1…Nxe4` | white e4 pawn | −1 |
| `2.Nxg7+` | black g7 pawn | 0 |
| `2…Bxg7` | white f5 knight | −3 |
| `3.Nxe4` | black f6 knight | 0 |

**Even material**. The desperado is the difference between "Black wins a clean pawn off the tactic" and "Black gets nothing tangible." The same knight that the direct recapture loses for nothing instead dies for a pawn.

## What `…Qe6` actually was

Once you see the desperado, `…Qe6` looks very different. The position before Black's 8th move already contained the same tactical resource — `Qd7` attacks `Nf5` along the same diagonal, the `e4` pawn is the only defender, the desperado works the same way. **Black already had everything they needed to play `…Nxe4`.**

`…Qe6` does not add a new threat. It does not remove a defender. It does not block a defensive resource. What it does is **commit Black's move to something other than the only equalising line**. It's a positional move played in a position whose tactical clock was already ticking, and it lets the clock expire.

Empirically — our engine, depth 18, four threads, MultiPV searching the Black-to-move-Qd7 position:

| Black's move | Engine eval (white-POV pawns) | Note |
|---|---|---|
| **`…Nxe4`** | **+0.33** | the desperado-equalising line |
| `…Ng8` | +2.66 | retreat |
| `…Qe6` | +3.36 | the move played |
| `…Qc6` | +3.49 | similar |

That ~3-pawn jump from `…Nxe4` to `…Qe6` is the eval cost of *forfeiting an option that was about to expire*. It isn't a "wasted tempo" in the small sense (tempo ≈ 0.3 pawns in our eval). It's an option premium being thrown away — the option to extract value out of the doomed `Nf5`.

## Why `Ne3` is positionally a loss and tactically the win

Now we can talk about what `Ne3` is doing. Engine search from the Qe6-position (white to move), depth 18, four threads:

| White's move | Engine score | What the engine thinks happens |
|---|---|---|
| **`Ne3`** | **+3.47** | step the knight to safety, head for d5 outpost |
| `f3` | +2.42 | shore up e4, but slow |
| `Ng3` | +2.08 | same idea, worse square |
| `g4` | +2.06 | kingside pawn storm |
| `Bxf6` | +1.98 | resolve the tension immediately |
| `Nh4` | +0.83 | knight retreat to a passive square |
| `O-O` | −0.03 | walks into the tactic |

`O-O` is the user's actual move. Note: `−0.03` here is **deep search**; it includes the engine playing the best defence (`Nd5 → Nb6 → Nxa8` to win back material). The honest "no defence" eval after `…Nxe4` is much worse — a separate search from that position shows Black mate-distance winning.

Now look at the **static term deltas** (one ply, root vs immediately-after-move, white-POV net per `core/engine/src/analysis/term_delta.rs`):

```
Ne3:                                      O-O:
  king.danger              −1.15            material.psq-positional   +1.41
  threats.slider-on-queen  −0.58            pieces.trapped-rook       +0.51
  mobility.bishop          −0.57            king.flank-attacks        +0.23
  material.psq-positional  −0.30
  threats.by-pawn-push     +0.47
  threats.by-minor         +0.45
  pieces.reachable-outposts +0.31
```

By every loud static signal, `Ne3` looks **worse** than `O-O`. Sum of the visible deltas is negative for `Ne3`, positive for `O-O`. A coach who only reads the static eval would (correctly) say `Ne3` weakens white's position:

- `king.danger −1.15` — the kingside-attack pressure on Black drops from −117 mg of pressure to **zero**, because moving the knight off `f5` removes its attack on `g7`/`h6`/`e7`/`d6`. The user's intuition that "Ne3 backs off the attack" was exactly right.
- `mobility.bishop −0.57` — Bg5 loses `e3` as a reachable square (it's now occupied by own knight), and Black's bishops simultaneously gain a little scope.
- `threats.slider-on-queen −0.58` — Black picks up some pressure on white's queen.
- `pieces.reachable-outposts +0.31` — partial compensation: `d5` is now within knight-hop range.

This is the crux of the lesson and the hard problem for any teaching layer: **the static positional ledger says `Ne3` is a worse move than `O-O`, and that ledger is honest about the board after one ply.** The reason `Ne3` wins is not visible in any one-ply trace. It's visible only in the search: the search sees that `O-O` allows `…Nxe4 Nxg7+ Bxg7 Nxe4 …` and the rest, and `Ne3` doesn't.

The right framing for a student is therefore **not** "Ne3 was positionally great" (false) but:

> Both moves trade something. `O-O` trades a piece for nothing. `Ne3` trades a bit of king pressure and bishop mobility for keeping the piece. The tactical price tag on `O-O` dwarfs the positional price tag on `Ne3`.

## The asymmetry

A natural way to mis-read this position — and the framing chess.com defaulted to — is "you missed an opportunity to punish Black's mistake." That implies Black's `Qe6` was a *gift* that white could *seize*. It wasn't.

The right framing is:

- **There was nothing for white to gain from `Qe6`.** No tactic was created, no piece hung, no structural weakness opened.
- **Black's `Qe6` failed to force white to lose.** The position already contained a tactical threat against white's `Nf5`. Black's correct move was to execute that threat (the `…Nxe4` desperado). By playing `Qe6` instead, Black gave white a free turn — and white's correct use of that free turn was to address the threat, not to develop normally.

`O-O` was the user's mistake precisely because it treated `Qe6` as a *normal* move in a *normal* position. In a normal position, castling is fine. In *this* position, the `Nf5` was already on borrowed time, and any move that doesn't address that fact is a blunder. The student-facing lesson isn't "punish Black's mistake" — it's **"before you treat your opponent's move as harmless, look at what tactical threats *you* still need to defuse from earlier."**

## What this would take to teach automatically

The chess-tutor pieces needed to produce this lesson at game-time:

1. **Pre-move (coaching) detector for "your piece is loose":** before white's 9th move, the coach needs to see that `Nf5` is attacked by `Qe6`, defended only by `e4`, and that `e4` itself is attackable by `…Nxe4`. The shape is **remove-the-defender**, and we already have `TacticPattern::RemovingDefender` in [`analysis/tactic_outcome/`](../core/engine/src/analysis/tactic_outcome/). What's missing is running this detector against the **opponent's** prospective moves, not just our own — "what tactics could Black play *next* move?"
2. **Desperado check for terminal evaluation:** when the engine sees a piece losing, the eval / commentary must account for whether the doomed piece can cash itself in for a pawn before dying. The engine *search* gets this right (it finds `Nxg7+!` in the PV automatically); the *narrator* needs to be able to *say* "this trade goes from −1 pawn to even because of the desperado on g7."
3. **Retrospective surface for "you walked into a tactic":** `compute_tactic_outcome` already has a `user_walked_into` slot. For this position, after `O-O` the retrospective should populate that slot with `TacticPattern::RemovingDefender`, primary piece = `Nf5`, targets = the eventual material loss via `…Nxe4 …Qxf5`. The detector input is fine; we need to make sure it actually fires for *static threats that became tactics on the next ply*, not only for tactics the user's move *created* against themselves.
4. **Honest narration of "the positional ledger says one thing, the tactical ledger says another":** when the static term deltas of the recommended move are net-negative but the search score is decisively positive, the narration must not lie that the move was "positionally strong." The honest framing is "the static eval would tell you to play `O-O`; the search overrules it because `O-O` loses a piece." We don't have this framing in any narration template yet.
5. **Pre-move detector for "an existing threat is about to expire":** the subtlest piece. The opponent's `Qe6` didn't introduce anything new; it failed to execute a threat that was already there. A coach that only diffs board state move-to-move won't see this — it needs to track "tactical opportunities the position *already* contained" across moves and flag when one is about to be forfeited (by either side).

Item 5 is the one the existing pipeline doesn't really model. Everything else is plumbing on top of detectors we already have.

## Why this is an ideal use case for the project

A 1200 player presented with this position, with an evaluation bar, learns nothing — the bar said +1.59 vs. +0.60, but the bar can't say *why*, and inventing a why is exactly the failure mode that calcifies bad habits. ("`Ne3` must have been better because the bar moved" → next position, the student plays an even more passive knight retreat.)

A teaching tool that surfaces:

- "`Nf5` is loose: attacked by `Qe6`, defended only by `e4`."
- "`e4` is attackable by `…Nxe4`."
- "If Black plays it, the *clean* refutation is the desperado `Nxg7+`, which equalises material."
- "Therefore, before any other move, white needs to address this. `Ne3` does that and also wins the d5 outpost. `O-O` ignores it."

…teaches the student a transferable concept: **resolve standing tactical threats before continuing with development**. That's the kind of concept that's the entire reason this project exists. It is the gap between the 1200 player ("I don't hang pieces and I can see hanging enemy pieces") and the 1600 player ("I can see *that my pieces are about to hang* two moves out, and I prioritise that over development"). The engine has all the signals to know this position contains that lesson. The teaching layer's job is to extract it.

Keep this position as a regression target. If a future iteration of the retrospective panel, fed the actual sequence `…Qe6 / O-O`, doesn't produce a card saying roughly "you walked into a remove-the-defender tactic against `Nf5` that Black missed," the teaching layer isn't doing its job yet.
