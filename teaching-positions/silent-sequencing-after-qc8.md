# Case study: silent sequencing after `…Qc8` — when "blunder" isn't a fair label

A position earlier in the same chess.com game as [`missed-desperado-after-qe6`](missed-desperado-after-qe6.md) and [`discovered-attack-after-qxe6`](discovered-attack-after-qxe6.md). After the user played the strong `Bd5!` (the engine's only winning move), the opponent had earlier played `…Qc8` — which chess.com flagged as the move that swung the eval from −1.2 to +3.9 (≈5 pawns). The user's instinct was that `…Qc8` looked *brilliant* (defends one bishop, threatens another), and the engine still hates it. **Empirically, the engine only sees the problem at depth 8+. At depth 6 — the calculation horizon of a competent human in a tactical middlegame — `…Qc8` and the engine's pick `…Be5` are functionally tied.**

This case study isn't about the chess. It's about the meta-principle: **what should the teaching layer do when its own engine confirms that a position is silently tactical, but the tactic only resolves below human calculation depth?**

Short answer: don't call the move a "blunder." Don't pretend we can teach the mechanism. Be honest that the depth required is beyond what a 1200 — or arguably any human — would find in a real game.

Date analysed: 2026-05-29.

## The position

FEN (Black to move, after White's `15.Re1`):
`1r1q2nr/p3k3/2Bbbpp1/7p/2Q5/8/PPPP1PPP/R1B1R1K1`

Material is dead even (32 vs 32 in piece value; Black up one pawn after the earlier exchanges, White up a knight in piece-quality terms — exactly cancelling).

The relevant geometry:
- `Re1` pins `Be6` against `Ke7` along the e-file (hard pin).
- `Qc4` attacks `Be6` along the c4-d5-e6 diagonal (d5 empty).
- `Be6` is defended only by `Ke7` (since `Bd6` is dark-squared and pawns don't reach).

**Be6 is hanging 2:1.** Black must address this on the next move or lose the bishop.

## What happened in the game

Black played `…Qc8`. The user (White) recognised the position required `Bd5!` (the only move that simultaneously saves Bc6 from `…Qxc6`, keeps Qc4 defended, and adds an attacker to Be6), played it, and went on to convert. Chess.com flagged `…Qc8` as a blunder (−1.2 → +3.9). When the user reviewed the move themselves they couldn't see what was wrong with it — and it took multiple rounds of analysis (corrected from a flat "Qc8 doesn't defend Be6" reading on my part) to land on the right description.

## The static-counting picture says Qc8 is excellent

This is the part that matters for the teaching question. Look at what `…Qc8` actually accomplishes on a one-ply static analysis:

| Effect | Mechanism |
|---|---|
| **Adds a defender to Be6** | Qc8 attacks the c8-d7-e6 diagonal (d7 empty), defending Be6 |
| **Threatens Bc6** | Qc8 attacks the c-file, hitting the white bishop on c6 |

After `…Qc8`, the attacker/defender count on Be6 becomes **2 attackers (Re1 + Qc4) vs 2 defenders (Ke7 + Qc8)** — a clean even contest. If White plays `Rxe6`, Black recaptures `…Qxe6`. If White plays `Qxe6+`, Black recaptures `…Kxe6` (legal because Qc4 has just moved off the c4-d5-e6 diagonal). Either way Black ends up materially fine.

So `…Qc8` is a textbook dual-purpose move: defends a hanging piece *and* creates a threat against an enemy piece. By every static-tactical detector we have or could plausibly write, this is a *good* move. None of the patterns in our library (fork / pin / skewer / discovered attack / removing the defender / overloading) detect anything wrong with it.

## The engine sees the problem only beyond human depth

Here is the empirical depth sweep, run on this exact position with our engine (multi-PV 4, single thread, scores in side-to-move POV):

| Depth | Be5 (engine pick) | Qc8 (played) | Gap (pawns) |
|---|---|---|---|
| 4 | +18.63 | −4.59 | 23 (search nonsense — too shallow to be meaningful) |
| **6** | **+1.56** | **+0.82** | **0.74 (functionally tied)** |
| 8 | +3.37 | −1.18 | 4.55 (gap emerges) |
| 12 | +3.20 | −1.44 | 4.64 (stable) |
| 20 | +3.19 | −5.37 | 8.56 (grown) |

At depth 6 — roughly the calculation horizon of a competent human in a tactical middlegame — **the two moves are essentially tied**, both evaluated as slightly winning for Black. The blunder verdict emerges only at depth 8 and stabilises by depth 12.

The mechanism the engine sees (and that I attempted to articulate in the previous turn): `…Qc8` commits the queen to a square where, after the forced sequence `Bd5 Be5 Bxe6 Qxe6 Qxe6+ Kxe6`, the queen has been traded off without ever doing the disruption work it could have done from d8. In the `…Be5`-first line, the queen stays flexible on d8/d6 and eventually gets to support a bishop sacrifice (`…Bxh2+`) for kingside initiative. In the `…Qc8` line, the trades happen on White's terms and Black ends with the king exposed on e6 and a doomed bishop on e5.

That's a real mechanism. It's also **completely invisible to anyone calculating fewer than seven or eight plies ahead** — which is to say, completely invisible to anyone.

## The principle: silent sequencing isn't a teachable blunder

For the teaching layer, this position exposes a category of "blunder" that we shouldn't be in the business of flagging:

### The diagnostic
A move qualifies as **silent sequencing** when *all* of the following hold:

1. **The MultiPV is tactical.** The top move is strongly positive; the second-best (or the move played) is strongly negative. The position has only one good move.
2. **No tactic detector fires on the played move.** Running our full detector chain (`find_best_tactic_in_position`, `find_overloaded`, the latent-threat detector once it exists) on the position before the player's move and on the move itself produces no named pattern.
3. **The blunder verdict requires depth ≥ ~7–8 plies.** At shallow depths (4–6) the played move and the engine's pick evaluate similarly (within noise, say ≤ 1 pawn of difference).

When all three hold, the "mistake" is not learnable in the sense the teaching layer is designed to surface. There is no transferable concept the student can extract. The honest description is "the engine found a six-move forced sequence that disfavours your move, but it's beyond practical calculation depth in a real game."

### What the teaching layer should *not* do
- **Don't call it a blunder.** Don't use language like "you walked into a tactic" or "you missed an opportunity." Both imply the user could have done better with available cognitive resources, and the empirical depth evidence says they couldn't.
- **Don't manufacture a fake teachable pattern.** This is the failure mode chess.com falls into — slapping a generic narration like "this move weakens your king" onto a position the NN didn't actually evaluate that way. Lying about the mechanism is worse than admitting we can't explain it.
- **Don't suppress the eval bar.** The eval is honest — the position is in fact decisively better for one side after the trade. Hiding that information leaves the user confused.

### What the teaching layer *should* do
- **Surface the move-level facts that are visible.** "This move kept your queen defending the bishop while threatening the opponent's bishop" is a true static-counting statement and *is* teachable.
- **Be explicit about depth honesty.** Something like: "the engine sees this position becoming difficult over the next several moves, but the specific reason requires deep calculation. There's no shorter lesson here." A 1200 reading that has learned something real: *not every bad-eval move has a teachable reason*.
- **Flag the position as one where intervention should not pause play.** Currently our intervention-pause is gated on dominant-eval-term + share threshold + position-not-hopeless. A "silent sequencing" detector is a *fourth* gate: if no tactic pattern fires and the blunder verdict requires high depth, don't trigger an intervention — it has no actionable content.

### The detector shape
Concretely the silent-sequencing detector would be:

```
silent_sequencing(pre_pos, candidate_move, alternative_best_move) -> bool:
    # 1. Both moves resolve to a similar eval at shallow depth
    shallow = run_search(pre_pos, depth=6, candidates=[candidate_move, alternative_best_move])
    if abs(shallow[candidate_move].score - shallow[alternative_best_move].score) > 100 cp:
        return False  # the gap is visible at human depth — it's a real blunder
    
    # 2. The gap is large at full depth
    deep = run_search(pre_pos, depth=14, candidates=[candidate_move, alternative_best_move])
    if abs(deep[candidate_move].score - deep[alternative_best_move].score) < 200 cp:
        return False  # the gap is small even at depth — neither pick is wrong
    
    # 3. No tactic detector fires on either side for the played move
    if find_best_tactic_in_position(pre_pos, mover) or
       find_best_tactic_in_position(after(candidate_move), opponent) or
       find_overloaded(pre_pos, ...):
        return False  # there IS a name-able pattern; surface it normally
    
    return True  # silent sequencing detected — suppress blunder framing
```

This is something the teaching pipeline can actually compute. The two-depth search adds latency but only on moves that already triggered the blunder pipeline, so the cost is bounded.

## How this differs from the other two case studies

The pattern hierarchy that's emerging:

| Case | Detector signal at human depth? | Pattern nameable? | Teaching action |
|---|---|---|---|
| [`missed-desperado-after-qe6`](missed-desperado-after-qe6.md) | yes (Black's `…Nxe4` removes the defender of Nf5; detectable statically) | RemovingDefender + Desperado | Surface as missed tactic with named pattern |
| [`discovered-attack-after-qxe6`](discovered-attack-after-qxe6.md) | yes (e-file queen/bishop/rook alignment is statically detectable) | DiscoveredAttack (latent) | Surface as latent opponent threat with named pattern |
| **silent-sequencing-after-qc8** (this file) | **no — depth-6 verdict is "fine"** | **none — no detector fires** | **Suppress blunder framing; explain only the visible static effects** |

The first two are the positions the teaching layer should *aspire* to handle (and the latent-threat detector in `analysis/latent_threats.rs`, now landed, is the architectural piece that closes that gap). This third one is the case where the teaching layer's *humility* matters more than its capability: it's where the layer should choose not to overclaim.

## Regression target

For a future iteration of the retrospective panel: when fed this position with `…Qc8` as the candidate move, the panel should NOT produce a "you blundered" or "you walked into a tactic" card. The acceptable output is some combination of:

- A neutral note about the move's static-counting properties ("kept your queen defending the bishop while creating pressure on the c-file").
- An honest depth-honesty note ("the engine evaluates this as significantly worse than `…Be5`, but the difference doesn't resolve until ~8 plies of calculation — there isn't a shorter mechanism to teach").
- No "BAD MOVE" stamp, no false pattern attribution, no narration that invents a teachable mechanism that isn't there.

If we ever wire this to the intervention-pause system, the silent-sequencing classification should *prevent* a pause — interrupting play to surface a verdict the student can't act on is worse than just letting play continue.
