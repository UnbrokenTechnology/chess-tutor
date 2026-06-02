# Case study: the discovered-attack alignment after `…Qxe6`

A position from a real chess.com game between the user (~1200 ELO) and a 1400 ELO bot. After the user played a strong tactical exchange (`Bxe6`), the opponent recaptured with `…Qxe6` and the user — needing to move the queen out of attack — played the natural-looking `Qc5+`. The eval bar went from +3.9 to −1.5 (chess.com's numbers; our engine sees the swing as roughly +6.1 → −3.2, i.e. **a nine-pawn collapse**).

Like the [positional-punish-after-qe6](positional-punish-after-qe6.md) position, this is a case where chess.com couldn't articulate *why* a move was bad and left the user to debug it themselves. The two positions share an architectural pattern: **the user missed a tactic the opponent had already set up.** In the earlier case the opponent's tactic was *remove-the-defender* (`…Nxe4` against `Nf5`); here it's a *discovered attack* (`…Bxh2+` revealing `…Qxe1`).

Date analysed: 2026-05-29.

## The position

FEN (White to move, after `…Qxe6` recaptured the bishop):
`1r4nr/p3k3/4qpp1/4b2p/2Q5/8/PPPP1PPP/R1B1R1K1`

```
8 . r . . . . n r
7 p . . . k . . .
6 . . . . q p p .
5 . . . . b . . p
4 . . Q . . . . .
3 . . . . . . . .
2 P P P P . P P P
1 R . B . R . K .
  a b c d e f g h
```

Material is roughly even: White has Q + 2R + B + 7P = 29; Black has Q + 2R + B + N + 4P = 29. White's queen on c4 is under attack from Black's queen on e6 (along the c4-d5-e6 diagonal, with d5 empty), so White must move the queen on this move.

## What happened in the game

The user reasoned about `Qc5+`:

> step the queen out of danger, put the king in check, move to a more active square, and create pressure against their bishop

Each clause is locally true. None of them are *forcing*, and that turns out to be everything.

## The hidden alignment

Look at the e-file in the position above. From bottom to top: `Re1`, then three empty squares, then `Be5` (black), then `Qe6` (black). That's:

```
e8 .   (empty)
e7 k   black king nearby
e6 q   BLACK QUEEN (the attacker)
e5 b   black bishop (the blocker)
e4 .
e3 .
e2 .
e1 R   WHITE ROOK (the target)
```

This is a textbook **discovered-attack alignment**. Queen and rook on the same file, with one of Black's own pieces between them. Any time the bishop moves with sufficient force to make White respond (a check or another forcing threat), the queen fires down the e-file at the rook.

The piece that makes this dangerous is the queen on e6 — the *discoverer*. The piece that's blocking it is the bishop — the *discovery vehicle*. The target is the rook on e1. As long as all three are in place, Black is sitting on a latent tactic.

## Why `Qxe6+` defuses it (and `Qc5+` doesn't)

`Qxe6+` is the engine's top move at **+6.09** — almost a full pawn ahead of the next-best (`Qe4`, +4.15). It does three things simultaneously, and the first one is the load-bearing one:

1. **It captures the discoverer.** Once Black's queen is off the board, the bishop on e5 is just a bishop again. Moving it no longer reveals any attack. The whole tactical motif evaporates because the alignment needs three pieces (attacker / blocker / target) and we've removed the attacker.
2. **It forces `…Kxe6`.** Black's king is the only thing that can recapture (everything else is too far or wrong-coloured), so the king is dragged to the centre.
3. **It sets up the pin.** With the king on e6 and the rook on e1, the bishop on e5 is now **hard-pinned**: it physically cannot move because doing so would expose the king to the rook. The bishop's only defender is the f6 pawn, and `d4` next attacks it from a second direction the pawn can't cover.

The engine line continues:

```
Qxe6+ Kxe6 d4 Ne7 dxe5 fxe5
```

Material accounting through the forced sequence:

| Move | What was captured | Material delta (white POV) |
|---|---|---|
| Qxe6+ | Black queen | +9 |
| Kxe6 | White queen | 0 |
| d4 | — | 0 |
| Ne7 | — | 0 |
| dxe5 | Black bishop | +3 |
| fxe5 | White pawn | +2 |

Net: **+2 material for White, plus a wide-open Black king on e6 in a position where White has both rooks and a bishop versus Black's two rooks and a knight.** Engine settles around +5.75 to +6 depending on depth.

`Qc5+` does none of these things. It moves the queen to a square that *looks* useful but isn't forcing:

- **Doesn't capture the discoverer.** Black's queen stays on e6. The e-file alignment is preserved intact.
- **Doesn't force anything specific.** Black has multiple legal responses to the check (`…Bd6`, `…Qd6`, several king moves). The engine picks `…Kf7` — the simplest one, just sidestepping. Black has spent zero pieces and zero structure.
- **Doesn't create real pressure.** `Qc5` attacks `Be5` along the 5th rank, but `Be5` is defended by `Qe6` *and* by the f6 pawn (two defenders against one attacker). The "pressure" is illusory.

After `Qc5+ Kf7`, Black has *gained a tempo* and the discovered attack is still loaded.

## How the discovered attack actually fires

The engine's line after `Qc5+` is:

```
Qc5+ Kf7 b3 Bxh2+ Kf1 Qd7 …
```

Black's `…Bxh2+` is the bishop sacrifice that uses the alignment. Three things happen in one move:

1. The bishop captures the h2 pawn (small material gain).
2. The bishop gives check (forces White's response).
3. **The bishop vacates e5, unblocking the e-file for `Qe6 → Qxe1`** (the discovered threat).

White is now stuck choosing between two bad options:

**Option A: `Kxh2`** — the natural "take the free bishop" move. This loses cleanly:
- After `Kxh2`, Black plays `…Qxe1`. The rook is gone; White's king is on h2 with no defenders nearby.
- Material flow: White loses h2 (−1) and Re1 (−5), gains the bishop (+3). Net **−3 material for White**.
- That's not even counting the disaster of the king on h2 in an open position.

**Option B: `Kf1`** — sidestep the check without taking. The engine's pick:
- The bishop on h2 survives but is stuck deep in White's position.
- White doesn't immediately lose more material, because `…Qxe1+ Kxe1` would let White recapture the queen with the king (rook + pawn = 6, less than the queen at 9, so Black would be down material if they cashed in directly).
- But White's king has voluntarily walked to f1, the bishop on h2 hangs over the kingside permanently, and Black has gained a free pawn plus a long-term attacking initiative.

That second-option positional damage is what the engine's −3.16 eval is paying for. The discovered attack didn't *directly* win material in the main line — it functioned as a **threat that forced White to choose between losing material (Kxh2) or accepting permanent positional damage (Kf1)**. There was no third option that kept everything together.

## Why the user's reasoning sounded right but wasn't

Mapping each clause of the user's stated reasoning against what was actually true:

- **"Step the queen out of danger"** — true, but several other queen moves also do this. `Qxe6+` (the right move) also gets the queen out of danger, *and* removes the threat against it permanently by trading.
- **"Put the king in check"** — true, but the check doesn't force anything specific. The king has at least five legal escape squares (`Kf7`, `Kf8`, `Ke8`, `Kd8`, `Kd7`), plus two legal blocks (`Bd6`, `Qd6`). A check that gives the opponent that many options is just a tempo gift.
- **"Move to a more active square"** — c5 *looks* more central than c4, but it's not actually more active. The attacked target (`Be5`) has two defenders to White's one attacker. Compare to `Qxe6+`, which physically occupies the most active square on the board (with check).
- **"Create pressure against their bishop"** — there is no pressure. With `Qe6` and the f6 pawn defending `Be5`, and only `Qc5` attacking it, the count is 2:1 in Black's favour. The bishop is *more* defended than attacked.

The pattern across all four: each clause is a *true static statement* about the move, but none of them describe a *forcing mechanism*. In a position where the opponent has a loaded discovered attack, only forcing moves are good. Soft pressure gives the opponent the tempo they need to fire.

## The structural parallel to the missed-desperado case

This is the second example we've found in real games of the same architectural failure mode:

| | [positional-punish-after-qe6](positional-punish-after-qe6.md) | this case |
|---|---|---|
| Opponent's setup move | `…Qe6` (kept the *standing* `…Nxe4` threat alive) | `…Qxe6` (created the e-file alignment) |
| Opponent's tactical pattern | Remove-the-defender (`…Nxe4` removes the pawn defending `Nf5`) | Discovered attack (`…Bxh2+` unblocks `…Qxe1`) |
| White's correct response | `Ne3` (evacuate the target) | `Qxe6+` (capture the discoverer) |
| White's actual response | `O-O` (treated opponent move as harmless) | `Qc5+` (treated opponent move as a normal threat) |
| What was missed | A standing tactic the opponent could have played | A standing tactic the opponent *will* play |

Both errors share the same mental motion: **the user evaluated their own move's positive properties without checking whether it disrupts the opponent's pre-existing tactical alignment**. That's the 1200 → 1600 habit gap. A 1600 instinctively asks "what does my opponent have set up that I need to defuse?" before they ask "what does my move accomplish?". A 1200 asks only the second question.

## What this would take to teach automatically

The engine already has a `DiscoveredAttack` detector in [`core/engine/src/analysis/tactic_outcome/`](../core/engine/src/analysis/tactic_outcome/). The gap exposed by this position is **where in the pipeline we run it**:

1. **Currently:** detectors run on *our* moves (does my move create / enable a tatcic?).
2. **Needed:** detectors that scan *standing opponent tactics* before our move (is the opponent already sitting on a discovered-attack pattern that any non-defusing move from me will let them execute?).

For this position the detector would have flagged: *latent threat — Black has a DiscoveredAttack. Move candidates that don't address this (`Qc5+`, `Qe4`, `Qe2`, etc.) leave it on the table. The only candidates that removes the alignment are `Qxe6+` (captures the discoverer) or `Qe4` (interposes between discoverer and target).*

This is the same architectural item I noted in the missed-desperado writeup ("an existing threat is about to expire / fire"). It's not a *move-to-move diff* — neither side's current move creates the alignment; it was created earlier and is patiently waiting. It needs a *static board scan* layer that runs before each user move.

## Regression target

If a future iteration of the coaching / retrospective panel, fed this position with `Qc5+` as the candidate move, doesn't produce a card saying roughly *"this move doesn't address Black's standing discovered-attack threat: `…Bxh2+` reveals `…Qxe1`."*, the teaching layer isn't doing its job yet.

Keep this position paired with [positional-punish-after-qe6](positional-punish-after-qe6.md) when designing the latent-opponent-threat detector — together they exercise both the *defensive* version (opponent's tactic equalises material) and the *offensive* version (opponent's tactic wins material), so the detector needs to be general enough to handle both.
