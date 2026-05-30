# Case study: the two-step fork after `…Qd8` — look one ply past the check

A position from a real chess.com game between the user (~1200 ELO) and a 1600 ELO bot. The game went through five consecutive moves (`10…Qd8 11.O-O-O 11…Bg7 12.f4 12…Qd6`) with eval bar swings of 1.5–4.5 pawns on every single ply, alternating between the two sides. Neither side was making moves that looked obviously bad to the user; both sides were missing the same latent tactic, repeatedly. The narrator (chess.com's NN) labelled every swing as "you missed a chance to punish your opponent's mistake" without explaining what.

What made every move's eval swing? **The c5 knight had a two-step forcing sequence — `…Nd3+` (check) followed by `…Nf2` (fork) — that was loaded against White from the moment Black's queen left e7.** Every move from then on either ran the sequence, prevented it, or failed at both. None of the players (including the 1600 bot) saw it.

This file documents the position because it sits on the **border between teachable and not-teachable**: the mechanism is multi-step and forcing (which usually puts it in "deep tactical, don't claim to teach" territory like [`silent-sequencing-after-qc8`](silent-sequencing-after-qc8.md)) but the look-ahead required is only *one ply past a check*, which is the kind of discipline a 1200→1600 player can actually develop. The lesson isn't "see the full mate"; it's "**after your opponent's most forcing reply, check whether *another* forcing move is on the table**." That's humanly findable, and it's exactly what was missed here.

Date analysed: 2026-05-29.

## The position (move 10, Black to move)

FEN: `r1b1kbnr/pp2qp1p/2p3p1/2n1p3/2P1P3/1P3P2/PBQPN1PP/R3KBNR b KQkq - 0 10`

```
8 r . b . k b n r
7 p p . . q p . p
6 . . p . . . p .
5 . . n . p . . .
4 . . P . P . . .
3 . P . . . P . .
2 P B Q P N . P P
1 R . . . K B N R
  a b c d e f g h
```

Material is roughly even: White is up one pawn (8 vs 7), Black has all minor pieces, both queens on the board, all four rooks on home squares. Black to move. Engine eval: **−0.87** in Black's POV — White has a small edge (~half a pawn).

This is the calm-looking pre-blunder position. Everything that follows comes from here.

## What happened in the game (with engine evals)

Each row shows the move played, our engine's eval *after* the move (side-to-move POV), and what the engine thought was best instead.

| Ply | Move played | Engine eval after | Engine's preferred move | Reason for swing |
|---|---|---|---|---|
| 10… | `…Qd8` | +1.80 (White) | `…Nf6` (−0.87) | Passive retreat; ignores own tactic |
| 11 | `O-O-O` | −0.78 (Black) | `d4` (+4.79) | Misses `d4` |
| 11… | `…Bg7` | +2.55 (White) | `…Nd3+` (+0.84) | Misses `…Nd3+` |
| 12 | `f4` | −1.85 (Black) | `d4` (+6.57) | Misses `d4` again |
| 12… | `…Qd6` | +2.26 (White) | `…Nd3+` (+0.52) | Misses `…Nd3+` again |

Three of those five moves (every White move and every Black move except the first one) **either fail to execute a winning tactic that's available, or fail to defuse one that's about to be executed**. The eval swings aren't about positional / static features changing — they're entirely about which side is going to convert the c5 knight's tactical resource. It alternates because each player misses their turn.

## The mechanism: `…Nd3+ → …Nf2`

Look at what the c5 knight can reach in one and two hops:

```
Nc5's one-hop squares:  a4, a6, b3, b7, d3*, d7, e4, e6
Knight on d3's one-hop squares:  b2, b4, c1, c5, e1, e5, f2*, f4

*'d squares are where things happen.
```

`…Nd3+` is the trigger. It's a check on any white king on e1 (start) or c1 (after O-O-O). After the check, the knight reaches f2, and **f2 is the key weak square** — the white pawn on f3 (instead of f2) left it permanently unguarded by anything except the king. Once the king moves off its starting square in response to the check, f2 is undefended and the knight lands there to fork rooks.

What makes the sequence forcing is that White's response options are very narrow in both setups:

**Pre-castle (King on e1), after `…Nd3+`:**
- `Kxd3` — illegal. `Qd8` attacks down the d-file; with the knight removed there's no blocker between d8 and d3.
- `Kd2`, `Ke2`, `Kf1` — illegal. All occupied by own pieces (pawn / knight / bishop respectively).
- `Kf2` — illegal. Attacked by the knight on d3.
- `Kd1` — *only* legal king move.
- `Qxd3` — block-by-capture. Legal but loses the queen: `…Qxd3` recaptures (now the d-file is clear because the white queen sits on d3 itself, not blocking from c2).
- `Nxd3`, `Bxd3` — neither white piece reaches d3 (Ne2 isn't a knight move away; Bf1's diagonal to d3 is blocked by Ne2).

So White's choices are **`Kd1` or `Qxd3` (queen sac)**.

**Post-castle (King on c1), after `…Nd3+`:**
- `Kxd3` — illegal (same Qd8 attack).
- `Kc2`, `Kd2` — illegal (occupied by own pieces).
- `Kxd1` — illegal (own rook).
- `Kb1` — *only* legal king move.
- `Qxd3` — same as before, loses queen.

So White's choices are **`Kb1` or `Qxd3` (queen sac)**.

In both setups, White has *one* legal king move and one losing capture. Then `…Nf2` follows:

**Pre-castle, after `Kd1`:** `…Nf2+` forks the king on d1 and the rook on h1. White moves the king, Black takes the rook. Net: **knight for rook = +2 for Black**.

**Post-castle, after `Kb1`:** `…Nf2` forks the rook on d1 and the rook on h1. White saves one, Black takes the other. Net: **knight for rook = +2 for Black**.

**Same material outcome.** The difference between pre-castle and post-castle is purely cosmetic: pre-castle the second fork is "king + rook"; post-castle it's "rook + rook" because the king's only safe square (Kb1) happens to be one that f2 doesn't attack, and the square the rook *now* occupies (d1) happens to be one that f2 *does* attack. **Pure coincidence of geometry** — the d-rook lands on exactly the square the pre-castle king was being driven to.

## Why O-O-O isn't the lesson

This is the part that's easy to misread (I did it). The temptation is to say "O-O-O created the fork by putting a rook on d1." That's wrong in two ways:

1. **The fork existed pre-castle.** With the king still on e1 / forced to d1, `…Nf2+` already won the exchange against the Rh1. Castling didn't *create* anything tactical.

2. **The post-castle "improvement" for Black is coincidental.** Yes, the second fork now hits two rooks instead of king+rook, but the material gain is identical (knight for rook in both cases). What looks like O-O-O "creating" the fork is really just the same fork with different target labels.

The thing O-O-O *did* do is reduce White's situational understanding of the danger — it felt like a routine king-safety move and didn't visibly change the tactical landscape, so White didn't pause to recalculate. But analytically, **O-O-O was approximately tactically neutral**. The blunder, if you locate it precisely, is `O-O-O` *instead of* `d4` — i.e. failing to address the standing threat — not `O-O-O` *because of what it created*.

This matters for the teaching layer: if the system tries to surface "you castled and that created a tactic," it'll be making a story up. The honest story is "you castled while a standing tactic was on the board, and the move didn't address it."

## Why `d4` works (and the rook angle is real, just not load-bearing)

The engine recommends `d4` at every decision point in the sequence. It does two things:

1. **Pawn fork on Nc5 and e5.** The c5 knight has to move, and once it moves to a non-c5 square (a6 / b7 / e6 etc.), the `…Nd3+` sequence is no longer one-hop. The tactic dissolves.

2. **(Post-castle only) opens Rd1's view of d3.** After d4, the d-file is clear from d1 up to the new pawn on d4. If Black still tried `…Nd3+` somehow, White could play `Rxd3` (the rook captures, no queen risk because the rook isn't sacrificing material). Pre-castle this effect doesn't exist (no rook on d1), so it's not d4's primary purpose. Post-castle it's an additional benefit.

Reason 1 is the load-bearing one — `d4` works pre-castle *and* post-castle, and it works because **it removes the knight from c5**, period. The user's earlier framing was right: f2 is the structural weakness, the c5 knight is the engine that exploits it, and `d4` neutralises the engine.

## The teachable principle (the user's "look one ply past the check" rule)

The thing that *would* have caught this for a human, without requiring deep search:

> When the opponent has a check available, don't ask only "*can I respond to it calmly?*" — also ask "*what do they have **after** I respond?*" Most checks die out after the response. The ones that don't — where another forcing move (another check, a fork, a discovered attack, a capture with check) is waiting one ply past the obvious response — are the ones you have to defuse.

In this position, the check is `…Nd3+`, the obvious response is `Kb1` (or `Kd1` pre-castle), and **the follow-up forcing move is `…Nf2` forking rooks**. The user's calculation would go:

1. *"Black has `…Nd3+`."* — Yes.
2. *"Can I respond safely?"* — Yes (one legal king square: Kb1).
3. **(One ply further)** *"What do they have after `Kb1`?"* — The knight can hop to f2. Where does that hit? Both my rooks.
4. *"Therefore `…Nd3+` is the setup move, not a stall. I need to defuse it before doing anything else."*

The chain stops at step 3. The user doesn't need to calculate further or see the full material count — just *notice that another forcing move exists* after the obvious king response. The rest follows.

This is the operational form of the discipline. It is **one ply deeper than what a 1200 typically calculates**, and **two plies shallower than what's required to see a full mating net**. It's exactly the level of look-ahead that distinguishes 1200 from 1600 in tactical positions.

## How this differs from the other case studies

| Case | Mechanism depth | Detectable pattern? | Teaching action |
|---|---|---|---|
| [`missed-desperado-after-qe6`](missed-desperado-after-qe6.md) | shallow | RemovingDefender + Desperado | Surface as named tactic |
| [`discovered-attack-after-qxe6`](discovered-attack-after-qxe6.md) | shallow (latent) | DiscoveredAttack | Surface as latent threat (needs scanner from `PLAN.md`) |
| [`silent-sequencing-after-qc8`](silent-sequencing-after-qc8.md) | deep (≥ 8 ply) | none — no pattern | Suppress blunder framing entirely |
| [`mating-net-after-ng5`](mating-net-after-ng5.md) | deep (king hunt, ≥ 7 ply) | structural cluster, no named pattern | Open question; soft warning at best |
| **`double-fork-after-qd8`** (this file) | **shallow forcing line (2 ply past check)** | **Fork — but only after a one-ply check simulation** | **Extend latent-threat scanner to "run detectors after each enemy check"** |

The new case fits cleanly between the "shallow tactic, just surface it" cases and the "too deep, stay humble" cases. The mechanism is multi-step but each step is short enough that **a one-ply extension of the existing latent-threat scanner catches it**.

## Implementation implications (modest extension to `PLAN.md`'s item 2)

The latent-threat scanner sketched in [`PLAN.md`](../PLAN.md) currently looks at static alignments — "enemy slider lined up with our king, with one of their pieces blocking." For this position, the relevant scanner extension is:

```
latent_check_followup(pos, mover):
    for each check the OPPONENT (not-mover) can play:
        for each legal response we have:
            post = pos after check then response
            for each forcing move the opponent has in post:
                if tactic_detector_chain(post, opponent_forcing_move) fires:
                    return LatentMultiStepThreat {
                        check: <opponent check>,
                        our_response: <our forced reply>,
                        followup_tactic: <the detector hit>,
                    }
```

This is a small wrapper over the existing tactic detector chain. The cost is bounded: opponents typically have few checks (often 0–2) and we often have very few legal responses to a check (1–3), so the inner detector chain runs ≤ ~6 times per board scan. That's well within the per-move budget.

What it surfaces, in narration form: *"After `…Nd3+`, your only legal king move is `Kb1`. From there, Black plays `…Nf2`, which forks your two rooks. The check isn't a stall — it's the first half of a fork. Consider `d4` to chase the c5 knight before this fires."*

That's the right level of explanation. It's specific (names the tactic), it's one ply past the check (not deeper), it surfaces the structural feature (the c5 knight), and it tells the user what to do (`d4`). Nothing in this narration requires us to see the full mate equivalent or to invoke depth-greater calculation.

## The over-tuning concern

The user flagged something worth being explicit about: **this case has the same "tactically rich, eval swings every move" surface signature as [`silent-sequencing-after-qc8`](silent-sequencing-after-qc8.md)**, but the prescription is different (here we *should* surface a teaching card; there we should not). The distinguishing signal between "teachable tactic position" and "untouchable deep sequencing" is **whether the tactic detector chain fires when run after a one-ply forcing simulation**. If it does, the mechanism is shallow and nameable. If it doesn't, the depth is real and we should stay humble.

Concretely, the rule for the teaching layer:

- **Surface a tactic card when:** running the detector chain on the position-after-each-opponent-check produces a hit. The card names the latent tactic, the check, and the user's response — exactly as in the existing fork / discovery / etc. detector narrations.
- **Suppress and stay quiet when:** no detector fires anywhere in the forcing scan, but the eval swing is large. That's the silent-sequencing case. Don't manufacture an explanation.

The two cases look identical on the eval bar; they're distinguished entirely by whether a static-pattern detector can find a hit after a one-ply forcing simulation. That's a clean implementation criterion, and it keeps us from over-tuning ("call everything teachable" — wrong, would invent mechanisms) while also not under-tuning ("call everything unteachable" — wrong, would miss real lessons like this one).

## Regression target

If a future iteration of the latent-threat scanner is fed the FEN at move 10 (Black to move) and run for both sides, the expected output is approximately:

- *"Black has a latent two-step fork: `…Nd3+` (forces White's only legal king move) followed by `…Nf2` (Fork — targets White's rooks). The current standing weak square is f2. Recommended defuse: `d4` to displace the c5 knight, or move a piece to defend f2."*

Symmetrically (in the position-after-`…Qd8`), the scanner should flag the same threat for White's consideration:

- *"Standing latent threat: Black's c5 knight has `…Nd3+ → …Nf2` (Fork). Your move should address it. Top candidate: `d4` (also opens d-file for your rook after castling)."*

If we ever reach a point where running this scanner on this position produces something like that — and on the [`silent-sequencing-after-qc8`](silent-sequencing-after-qc8.md) position produces nothing (correctly, because no detector fires there) — we've calibrated the system at the right resolution. Those two positions together are the bookend regression tests for the latent-threat layer.
