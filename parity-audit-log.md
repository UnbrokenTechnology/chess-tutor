# Stockfish 11 Parity Audit Log

Working log for [ROADMAP.md](ROADMAP.md) workflow 1. Tracks the file-by-file side-by-side walk of our Rust port against [`reference/Stockfish-sf_11/src/`](reference/Stockfish-sf_11/src/), every divergence found, and its disposition.

**Read first:** ROADMAP.md "Workflow 1" section for methodology, ground rules (one fix at a time, A/B each against bench, etc.), and done criteria.

---

## Baseline (2026-05-25, pre-audit)

All benches are the 45-position SF11-mirrored set (`./target/release/chess-tutor bench`) compared to SF11's `bench` (46 positions — our set drops the one Chess960 entry from SF11's `Defaults[]`). Stable mode is **warm-TT** (SF11's default): TT persists across positions in a single bench run. Cold-TT (`--new-game-between-positions`) is for stress-testing pruning and is **not** apples-to-apples with SF11.

Hardware: Windows 11. Binaries: `target/release/chess-tutor.exe` (commit `6ba8947`) and `reference/Stockfish-sf_11/src/stockfish.exe`.

| Config | Mode | Ours: nodes | Ours: NPS | SF11: nodes | SF11: NPS | Node ratio |
|---|---|---:|---:|---:|---:|---:|
| `bench 16 1 13` | warm-TT | 7,141,275 | 2.32 M/s | 5,156,767 | 3.58 M/s | **1.38×** |
| `bench 16 1 14` | warm-TT | 13,321,304 | 2.27 M/s | 6,567,129 | 3.42 M/s | **2.03×** |
| `bench 16 1 14` | cold-TT | 11,755,764 | 2.23 M/s | — | — | — |
| `bench 128 1 20` ⭐ | warm-TT | ~225,000,000 | 1.88 M/s | ~68,000,000 | 2.97 M/s | **3.3×** |
| `bench 128 8 20` | warm-TT | 768,775,887 | 8.06 M/s | 278,148,269 | 20.87 M/s | **2.76×** |

⭐ = **canonical iteration config.** All A/B fixes are measured single-threaded (`bench 128 1 20` for the deep gap, `bench 16 1 14` for fast turnaround). The production app ships single-threaded for determinism (multi-thread Lazy-SMP gives non-reproducible "best move" answers, which confuses students). The d=20/1T numbers above are from the user's run; a per-position breakdown run is in progress to find outliers.

**Observation:** node-count ratio grows with depth (1.38× at d=13 → 2.03× at d=14 → 2.76× at d=20). Consistent with "we search subtrees SF11 prunes" — those subtrees grow exponentially, so the ratio worsens with depth. ROADMAP's "~10×" figure does not reproduce on the bench aggregate at the configurations we tested; it may be a per-position outlier or a stale figure from a regressed state. **Real current gap is 2.0–2.8×**, with the worst showing up at d=20.

**NPS gap is independently real:**
- 1T: ours 2.32 M/s vs SF11 3.58 M/s = **0.65× SF11**
- 8T: ours 8.06 M/s vs SF11 20.87 M/s = **0.39× SF11** — SMP scaling is significantly worse than SF11's

Wall-clock at d=20/8T: ours **95.4 s**, SF11 **13.3 s** = **7.2×**. This is node-ratio × NPS-ratio compounded. HANDOFF.md notes "43 s for the full d=20 bench at 8 threads" — current measurement is 2× worse, likely a hardware/build difference vs the recorded number, not a regression we should investigate as part of this audit. Flagging for the user.

**Per-position outliers (d=20, 1T, warm-TT — the canonical config):** top node consumers, in order:

| FEN | Nodes | Note |
|---|---:|---|
| 1 | 21.5M | new — investigate |
| 32 | 15.4M | new |
| 20 | 15.2M | known passed-pawn/check-ext chain ([[feedback_passed_pawn_ext_chain]]) |
| 26 | 14.6M | known check-ext chain ([[project_fen26_check_extension_investigation]]) |
| 12 | 14.0M | new |
| 8 | 13.7M | new |
| 41 | 12.3M | known ([[feedback_lever2_regressed]]) |
| 14 | 11.3M | new |
| 9 | 10.4M | |
| 7 | 8.7M | |

Use these as the targeted A/B set when a fix candidate is in hand — a fix that helps the aggregate but blows up one of these is a net loss (per [[feedback_pruning_bundles]] four-position-quadrant discipline).

**Per-position outliers (d=13 warm-TT):** FEN 41 (758k), FEN 32 (373k), FEN 40 (302k).

**Done-criteria target (from ROADMAP):**
- d=14, TT=16, 1T: node count within ~2× of SF11. **Currently 2.03× — at the boundary.**
- d=20, TT=128, 8T: node count within ~2× of SF11. **Currently 2.76× — over the boundary.**

Both are much closer to done-criteria than the ROADMAP's "10×" framing suggested. The audit is still worth running, but the headline framing should be "tighten an already-close engine" not "close a 10× gap."

---

## Cross-file findings summary

Full walk status: search ✓ · movepick ✓ · evaluate ✓ · movegen ✓ · tt ✓ · bitboard/pawns/material/psqt ✓ · position ✓. **Complete.** Detailed per-file tables below.

**Headline: the engine is a faithful SF11 port.** Across all 9 file-groups only **two true correctness bugs** surfaced (P1, E1). The rest of the node gap is *missing/simplified pruning* — the additive, A/B-able kind of problem.

**The two genuine bugs (fix regardless of perf — they make us match SF):**
- **P1** — en-passant square set on *every* double push instead of only when capturable → diverges `key()` from SF → defeats TT + repetition hits → **correctness AND a node-gap lever**. (+ P13 FEN parse in lockstep.)
- **E1** — king-ring double-pawn removal uses the enemy's pawns instead of our own. King-safety eval error. One line. Symmetric, so mirror tests miss it.

**Most likely node-gap contributors (the perf levers), ranked:**
1. **P1** (above) — TT/repetition key divergence.
2. **Q1** — qsearch futility pruning entirely absent (qsearch dominates deep node counts).
3. **S10** — no LMR on captures (we full-depth every capture).
4. **S7** no NMP verification (zugzwang); **S8** no IID; **Q5** no qsearch evasion pruning; missing LMR adjusters (S13/S14/S15).
5. **MP1/MP2** — capture-ordering differences.

**Strategic infrastructure item (NPS gap + unblocks Q1):**
- **P10** — no cached check info (`set_check_info`). We recompute `checkers()`/blockers on demand every call (NPS cost). Porting SF's cache also provides the `checkSquares`/cached-blockers a real `Position::gives_check` (Q1's prerequisite) needs. One project, two payoffs.

**Works *against* the gap (we search fewer than SF) but are strength/teaching concerns:**
- **MG1/MP3** — no quiet checks generated in qsearch (miss quiet-check tactics).
- **MT5** — deferred endgame scaling (drawish B/R-pawn endings over-valued — exactly the endgames our target student mishandles).

**Verified equivalent (don't re-walk):** TT replacement/aging math, all eval weight tables + king-danger stack, all PSQT/pawn/material/imbalance tables, bitboard/magics, movegen legal-set correctness, SEE swap loop + piece values, make/undo + Zobrist incremental updates.

**Test-suite gap noted:** no perft FEN isolates an en-passant discovered-check pin (MG5) — add one as a regression guard (especially relevant once P1 changes ep handling).

## Per-file walk log

Format per entry:

```
### <SF11 file> ↔ <Rust file>

| # | SF11 location | Rust location | Divergence | Class | Notes |
|---|---|---|---|---|---|
```

`Class` is one of:
- **FIX** — clear bug or missing feature with high confidence it costs us nodes
- **DEFER** — divergence is intentional / explicitly deferred; rationale recorded
- **INVESTIGATE** — could be either; needs A/B before classification
- **OK** — verified equivalent on inspection (logged for completeness so reviewers don't re-walk)

### search.cpp ↔ core/engine/src/search.rs

**`search<NT>` / `negamax` (SF11 `search.cpp:594-1347` ↔ `search.rs:714-1860`).** Walked Steps 1–19. Step 13 shallow-pruning sub-prunes (LMP, countermove-pruning, parent futility, SEE) are all present and structurally match. Divergences below.

| # | SF11 loc | Rust loc | Divergence | Class | Notes |
|---|---|---|---|---|---|
| S1 | Step 1, L600-610 | absent | `has_game_cycle` upcoming-repetition draw detection before the qsearch dive | DEFER | Needs cuckoo-hash subsystem; ~1 Elo; low node impact |
| S2 | Step 4, L710-729 | search.rs:801-811 | TT-cutoff move-ordering updates (quiet-stats bonus when ttValue≥β; penalty when ttMove fails low) | INVESTIGATE | We just `return tt_value`. SF feeds history even on cutoffs → better ordering downstream |
| S3 | Step 4, L731 | search.rs:808 | `rule50_count() < 90` guard before returning ttValue | INVESTIGATE | Avoids trusting TT cutoff near 50-move draw; tiny node effect, correctness nuance |
| S4 | Step 6, L803-806 | search.rs:844-845 | "Can ttValue be used as a better eval?" refinement (`eval = ttValue` when bound allows) | INVESTIGATE | Changes the eval feeding futility/NMP gates → directly affects pruning |
| S5 | Step 6, L810-817 | search.rs:842-848 | `eval += -(ss-1)->statScore/512` bonus; after-null `eval = -(ss-1)->staticEval + 2·Tempo` | INVESTIGATE | Eval tweak affecting pruning gates; we always re-`evaluate()` |
| S6 | Step 7, L822-826 | absent | Razoring (`depth<2 && eval ≤ α−531 → qsearch`) | DEFER | Documented deferred in `//!`; ~1 Elo, cheap, cuts shallow nodes — good easy A/B candidate |
| S7 | Step 9, L869-884 | search.rs:952-959 | NMP **verification search** at depth≥13 + `nmpMinPly`/`nmpColor` machinery | INVESTIGATE→maybe FIX | Zugzwang guard — directly relevant to the student's endgame-loss pattern. Net node effect ambiguous (adds a verify search, prevents bad cutoffs) |
| S8 | Step 11, L931-939 | absent | Internal Iterative Deepening (`depth≥7 && !ttMove` → search at depth−7, re-probe) | INVESTIGATE | Improves ordering when no TT move at high depth → more cutoffs; ~1 Elo, self-contained |
| S9 | Step 14, L1036-1069 | absent | Singular extension + **multi-cut** pruning | DEFER | Documented; SE 3rd attempt regressed ([[feedback_singular_extensions_third_attempt]]). Multi-cut returns a soft-bound cutoff → likely real node cost; revisit after other levers |
| S10 | Step 16, L1117-1124 | search.rs:1534-1539 | **LMR not applied to captures** — ours gates on `!is_capture`. SF reduces captures when `moveCountPruning ∥ staticEval+capturedVal≤α ∥ cutNode ∥ low ttHitAvg` | INVESTIGATE | Likely a meaningful chunk of the node gap — we full-depth every capture |
| S11 | Step 16, L1129-1130 | absent | `r--` when `ttHitAverage` large (and the whole ttHitAverage tracker) | DEFER | Needs ttHitAverage machinery; couples with S10's capture-LMR gate |
| S12 | Step 16, L1137-1138 | absent | `ttPv → r -= 2` | DEFER | Documented regression without prereqs (search.rs:1551-1563) |
| S13 | Step 16, L1141-1142 | absent | `(ss-1)->moveCount > 14 → r--` (opponent move count high) | INVESTIGATE | Cheap, no prereqs. *Loosens* (more nodes locally) but improves quality |
| S14 | Step 16, L1151-1152 | absent | quiets: `ttCapture → r++` | INVESTIGATE | Cheap, no prereqs. *Tightens* → fewer nodes. **Top easy candidate** |
| S15 | Step 16, L1161-1163 | absent | quiets: escape-capture `type==NORMAL && !see_ge(reverse_move) → r -= 2` | INVESTIGATE | Cheap; loosens |
| S16 | Step 16, L1190-1191 | N/A | captures: `depth<8 && moveCount>2 → r++` | tied to S10 | Only relevant once capture-LMR (S10) lands |
| S17 | Step 16/17, L1193+1203 | search.rs:1613,1628 | `d != newDepth` guard on the full-depth re-search | INVESTIGATE | When adjusters drive `r ≤ 0`, `reduced` clamps to `new_depth` and we re-search the *same* depth — wasted nodes. SF skips the re-search in that case |
| S18 | Step 17, L1207-1216 | absent | cont-history bonus feedback after an LMR full-depth re-search (`bonus = ±stat_bonus(newDepth)`, +¼ if killer) | INVESTIGATE | Move-ordering quality signal |

**`qsearch<NT>` (SF11 `search.cpp:1349-1560` ↔ `search.rs:1865-2052`).** Walked fully. Stand-pat, SEE≥0 capture filter, recapture-square chain-bounding (`QS_RECAPTURES`), and mate detection all present and match. Divergences:

| # | SF11 loc | Rust loc | Divergence | Class | Notes |
|---|---|---|---|---|---|
| Q1 | L1471-1492 | absent | **qsearch futility pruning** — `futilityBase = bestValue+154`; skip captures whose `futilityBase + capturedVal ≤ α`, and `futilityBase ≤ α && !see_ge(move,1)` | INVESTIGATE | **Entirely missing.** qsearch is a large fraction of total nodes; this likely contributes materially to the gap. Strong A/B candidate |
| Q2 | L1435-1437, final save | absent (search.rs:1919) | qsearch never writes to TT — SF saves stand-pat-fail-high and best result with `DEPTH_QS_*` | INVESTIGATE | We probe (reading negamax entries) but never cache quiescence results → recompute on revisit. Deliberate-looking ("no raw variant to preserve") but diverges; costs qsearch TT cutoffs |
| Q3 | L1422-1425 | search.rs:1923-1924 | "ttValue as better eval" refinement (same shape as S4) | INVESTIGATE | Consistent with S4 — fix together |
| Q4 | L1428-1430 | search.rs:1925-1927 | after-null stand-pat `-(ss-1)->staticEval + 2·Tempo` (same as S5) | INVESTIGATE | Fix together with S5 |
| Q5 | L1494-1502 | search.rs:1993 | **evasion pruning** — SF prunes negative-SEE *non-capture evasions* when `inCheck && (depth≠0 ∥ moveCount>2)`; ours searches **all** evasions when in check | INVESTIGATE | Node-relevant in check-heavy lines — directly touches the FEN 20/26 check-extension outliers |
| Q6 | L1456-1459 ttDepth | n/a | TT entry `ttDepth` typing (`QS_CHECKS`/`QS_NO_CHECKS`) — moot since we don't write qsearch TT (Q2) | tied to Q2 | — |

**Stat-update helpers** (`update_all_stats`, `update_quiet_stats`, `update_continuation_histories`): present in our move-loop cutoff block (search.rs:1712-1800). Quick comparison done — capture-history bump, killer/countermove update, quiet history ± and cont-hist ± all present. One gap noted as S2 (the *TT-cutoff* call site at SF L710-729 doesn't exist in ours). Detailed line-by-line of bonus magnitudes deferred to a focused pass if a history-ordering fix is pursued.

### movepick.cpp ↔ core/engine/src/movepick.rs

Walked the full stage machine and all three `score<>` paths. Our pipeline mirrors SF11's exactly (it just splits SF's single `REFUTATION` stage into separate `Killer0`/`Killer1`/`CounterMove` stages — equivalent, with correct dedup). Quiet scoring weights (`main + 2·ch[1ply] + 2·ch[2ply] + 2·ch[4ply] + 1·ch[6ply]`), evasion scoring, the `-3000·depth` quiet sort limit, and the recapture-square filter all **match**.

| # | SF11 loc | Rust loc | Divergence | Class | Notes |
|---|---|---|---|---|---|
| MP1 | movepick.cpp:110-111 | movepick.rs:1093-1097 | Main/qs **capture scoring**: SF = `PieceValue[MG][victim]·6 + captureHist`; ours = `victim·6 − attackerMgValue + captureHist`. Ours subtracts the **full attacker MG value** (≈198–2500); SF subtracts nothing | INVESTIGATE | Real capture-ordering difference — ours over-prefers cheap attackers. (SF's only LVA tiebreak is in *evasions*, and it's `− type_of` = 1–6, which ours matches at movepick.rs:1013.) Reorders captures → can shift cutoffs |
| MP2 | movepick.cpp:177 | movepick.rs:1043 | Good/bad capture split threshold: SF = `see_ge(move, −55·value/1024)` (permits slightly-negative SEE for high-scoring captures); ours = `see_ge(move, 0)` (strict) | INVESTIGATE | Ours sends borderline captures to `bad_captures` that SF keeps as "good" → later ordering. Minor |
| MP3 | movepick.cpp:258-266 (QCHECK) | movepick.rs (absent) | qsearch has **no QCHECK stage** — ours never generates quiet checks in quiescence (SF does at `depth==QS_CHECKS`, the first qs ply) | INVESTIGATE | *Reduces* our nodes (works against the gap), but a tactical-accuracy gap: we miss quiet checking moves in qsearch (perpetuals, quiet-check forks). Strength/teaching concern, not a node-gap culprit |

### tt.cpp/h ↔ core/engine/src/tt.rs

Subagent walk (verified the two correctness-critical pieces — replacement condition and generation-aging math — are arithmetically equivalent to SF11). **No FIX-class bugs.** The port is sound; the classic "subtly wrong replacement formula" is *confirmed not the problem*.

| # | SF11 loc | Rust loc | Divergence | Class | Notes |
|---|---|---|---|---|---|
| TT4 | tt.h:38-77 | tt.rs:38-73,170 | **Entry is 16 B vs SF's 10 B** → ~half the effective capacity per MB | DEFER (minor) | Documented intentional (tt.rs:23-28, all-atomic clean impl). **Diagnostic run (d=14 1T): 16MB=13.32M, 32MB=11.01M (−17%), 64MB=12.64M, 128MB=12.63M.** Non-monotonic → capacity is a *minor, noisy* lever, NOT the 2-3× story. Even at its best (32MB, 11.0M) we're still 1.67× SF's 6.57M. Not worth a layout rewrite for the audit; revisit only if a clean 10 B layout is wanted later |
| TT10 | tt.cpp:150-158 | tt.rs:417-431 | `hashfull` also requires `bound!=0`; SF counts current-gen entries regardless of bound | DEFER | Cosmetic (UCI permille). Verify no pruning heuristic consumes `hashfull()`; if diagnostics-only, ignore |
| TT1,2,3,5-9,11,12 | — | — | Replacement condition, move-preservation, age/value formula, probe refresh, gen bump, depth offset, cluster index, concurrency model, allocation/alignment | OK | All verified equivalent to SF11 — logged so they're not re-walked |

### evaluate.cpp ↔ core/engine/src/eval/*

Subagent walk. **Very faithful port** — every weight table (mobility, threats, passed-rank, king-attack weights, safe-check penalties), the ~12-coefficient king-danger accumulator, tapered mg/eg combine, scale factors, lazy-eval gate, and initiative side-capping verified equivalent. **One real bug found.**

| # | SF11 loc | Rust loc | Divergence | Class | Notes |
|---|---|---|---|---|---|
| E1 | evaluate.cpp:223,247 | eval/mod.rs:291-292 | **King-ring double-pawn-attack removal uses the WRONG COLOR.** SF removes from our king-ring the squares *our own* pawns double-attack (`dblAttackByPawn = pawn_double_attacks<Us>(pieces(Us,PAWN)); kingRing[Us] &= ~dblAttackByPawn`). Ours removes squares the *enemy's* pawns double-attack | **FIX (verified)** | ✅ Confirmed by reading both sides. SF intent (comment evaluate.cpp:246): a square our *own* pawns doubly defend is safe → drop it from the king-danger zone. Our code AND its comment (mod.rs:287-289) have the wrong color. `king_attackers_count` is computed before the removal on both sides (order matches), so only the danger-zone ring is affected. **Fix: `ring &= !our_double_pawn;`** (already computed at mod.rs:244). First true bug in the audit; symmetric so the startpos mirror test misses it |
| E3 | evaluate.cpp:784,813 | eval/mod.rs:605, search.rs:2067 | Contempt added pre-taper as a blended `Score` in SF; ours adds a flat ±2cp in the search layer post-eval | OK/INVESTIGATE | Deliberate (deterministic teaching eval; [[project_contempt_status]]). Tiny magnitude. Semantic note: flat cp vs phase-blended; contempt feeds TT-stored evals |
| E4 | evaluate.cpp:26-28 | eval/mod.rs:24-28 | Stale module doc claims king-safety/threats/passed/space/initiative are "stubbed to Score::ZERO" and lazy-eval "skipped" — all are actually implemented | OK (doc fix) | Misleads parity readers; worth correcting the comment |
| E2 | evaluate.cpp:321-332 | eval/pieces.rs:442 | Chess960 cornered-bishop penalty not ported | DEFER | Intentional, documented; standard chess never triggers it |
| E5,E6,E7 | — | — | Zero-piece early-return, WeakQueen blocker test, lazy-eval gating | OK | Verified equivalent (E7 lazy-bail matches SF on the search path; trace path stays exact) |

### movegen.cpp ↔ core/engine/src/movegen.rs

Subagent walk. The port does **not** mirror SF11's templated `GenType` architecture — it has two entry points (`generate_pseudo_legal_moves` = full set; `generate_legal_moves` = full set through a uniform do/undo king-safety filter). Capture/quiet/evasion partitioning happens in movepick instead. **Legal-move-set correctness is sound** (and arguably more robust — every move gets a real make/unmake legality test). Divergences are about *which subsets get generated*, not legality.

| # | SF11 loc | Rust loc | Divergence | Class | Notes |
|---|---|---|---|---|---|
| MG1 | movegen.cpp:282-309 (`QUIET_CHECKS`) | absent | **No `QUIET_CHECKS` generator** — direct/discovered/promotion checks never produced. Confirms MP3: qsearch tries captures only | INVESTIGATE | Invisible to perft (perft only counts the full LEGAL set). *Reduces* our nodes but misses quiet-check tactics in the q-tree. Confirm whether intentional MVP cut |
| MG5 | movegen.cpp:160-176 | movegen.rs:221-232 | En passant generated unconditionally; SF gates by GenType + evasion target. Ours relies on do/undo filter | OK (but highest-risk spot) | Verified correct (do_move removes the EP victim before the king scan). **Recommendation: add a dedicated EP-discovered-check FEN to the perft suite as a regression guard** — no current perft FEN isolates an EP pin |
| MG2,3,4,6-9 | — | — | Full-generate-then-filter vs SF targeted generation; double-check shortcut; promotion bucketing; double-push source set; castling safety inlined | OK | All produce the same legal set (perft-equivalent). Constant-factor NPS cost only (relevant to the separate NPS gap, not node count) |

### bitboard / pawns / material / psqt

Subagent walk. **All faithful, byte-for-byte where it matters, no FIX bugs.** Every weight table (pawn penalties, Connected, ShelterStrength, UnblockedStorm, imbalance QuadraticOurs/Theirs, all 5 piece PSQTs + asymmetric pawn PSQT), phase constants, cache keys (`pawn_key`, `material` direct-mapped), and indexing (file-fold, vertical-flip-negate, magic/PEXT sizing) match. Bitboard/magics validated by the perft suite.

| # | SF11 loc | Rust loc | Divergence | Class | Notes |
|---|---|---|---|---|---|
| MT5 | material.cpp:57-204 | material.rs:124-138 | Specialized endgame **scaling** functions (KBPsK, KQKRPs, KPsK, KPKP) not ported — only KXK wired through `endgame::probe` | DEFER | Documented intentional. **But teaching-relevant:** drawish bishop/rook-pawn endings are over-valued until ported — directly the kind of endgame our 1200 student mishandles. Track for a follow-up. See [[feedback_endgame_evaluator_gradients]] |
| BB*/PW*/PQ* | — | — | Bitboard primitives, magic indexing, full pawn eval + shelter/storm split, all PSQT tables | OK | No divergence. Pawn shelter component-split (teaching) re-sums to SF's aggregate (test-asserted) |

### position.cpp/h ↔ core/engine/src/position/*

Subagent walk + I verified P1 directly. make/undo roundtrip, Zobrist incremental updates, SEE swap loop, slider_blockers/pinners, attackers_to, castling/promotion all faithful. **One real bug (P1), and a strategic infrastructure gap (P10) that ties into Q1.**

| # | SF11 loc | Rust loc | Divergence | Class | Notes |
|---|---|---|---|---|---|
| P1 | position.cpp:792-798 | make_move.rs:143-146,163 | **EP square set unconditionally.** SF sets `epSquare` + XORs the ep key *only if* an enemy pawn can capture (`attacks_from<PAWN>(epsq,us) & pieces(them,PAWN)`). Ours sets `en_passant` on **every** double push and always XORs the ep key | **FIX (verified)** | ✅ Confirmed by reading both. Our `key()` diverges from SF for any double push with no capturer. **Repetition (search.rs:2116) and TT both key off `key()`** → two identical positions reached differently get different keys → missed transposition/repetition hits → extra nodes. Correctness + node-gap lever. Fix: gate on `pawn_attacks(us, epsq) & their_pawns`. **Fix P13 (FEN parse) in lockstep** |
| P10 | position.cpp:315-341 (`set_check_info`) | absent | **No cached check info** — no `checkSquares[]`, no cached `blockersForKing`/`pinners`, no `checkersBB`. Ours recomputes `checkers()` (full `attackers_to` on king) and `blockers_for_king` on demand every call | INVESTIGATE | **Per-node NPS cost** (candidate for the separate ~0.63× NPS gap). AND it's exactly the infrastructure a real `gives_check` (Q1 prereq) needs. Porting SF's `set_check_info` into `do_move` kills two birds: enables Q1's `gives_check` + cuts repeated check/blocker recompute |
| P9 | position.cpp:627-678 | absent | `gives_check(Move)` not ported (search derives check post-`do_move` via `in_check()`) | DEFER (planned) | The Q1 prerequisite. See readiness below |
| P13 | position.cpp:262-273 | fen.rs:58-129 | FEN parse skips SF's ep validation (keeps a "phantom" ep with no capturer) | INVESTIGATE | Internally consistent with P1's bug (why tests pass), but diverges from SF. Fix together with P1 |
| P8 | position.cpp:950-986 | make_move.rs:237-266 | Null move: no `pliesFromNull`, no TT prefetch | INVESTIGATE | Benign; re-verify ep-key interaction once P1 lands |
| P2-P7,P11,P12,P14 | — | — | StateInfo minimalism, key-update order, SEE indexing/values, slider_blockers, attackers_to, castling-rights mask, move counters | OK/DEFER | Verified equivalent (SEE piece values match SF exactly; pinner-index convention differs but the SEE-filter set is identical) |

**`gives_check` port readiness (for Q1):** SF's `gives_check` needs (1) a cached `checkSquares[PieceType]` array (built by `set_check_info` from the *opponent* king), (2) a cached opponent `blockersForKing` + an `aligned(a,b,c)` 3-square collinearity helper, (3) promotion/ep/castling special cases (all buildable from existing `rook_attacks`/`bishop_attacks` magics). **Missing:** the `checkSquares` cache, cached opponent blockers, and `aligned` (only `between_bb` exists at blockers.rs:6). **Cleanest path: port SF's `set_check_info` into `do_move`/`from_fen`** — this is the same work that resolves P10's NPS cost. Confirm no `aligned`/collinear helper exists before building one.

### movegen.cpp ↔ core/engine/src/movegen.rs

_pending_

### tt.cpp/h ↔ core/engine/src/tt.rs

_pending_

### bitboard / pawns / material / psqt

_pending — lower priority per ROADMAP_

---

## Recommended A/B order (search.cpp findings)

Each is landed and benched individually (single-thread `bench 16 1 14` for turnaround, `bench 128 1 20` to confirm, watching the d=20 outlier set). Ordered by expected node-reduction × low-risk-of-regression.

**Tier 1 — expected node *reduction*, SF-verbatim, low risk:**
1. **Q1 — qsearch futility pruning.** Biggest expected win (qsearch dominates deep node counts). Pure subtractive prune of captures that can't raise α.
2. **S14 — `ttCapture → r++` in LMR.** Trivial, no prereqs, tightens reduction.
3. **S10 — LMR on captures.** Structural; we currently full-depth every capture. SF reduces them under 4 conditions. Bigger change → bench carefully.

> **Q1 implementation decision (user, 2026-05-26):** port a real `Position::gives_check(move)` (SF11 has it; used pre-move in qsearch and the move loop). We currently derive check status only *after* `do_move` via `pos.in_check()`. The new method should reuse our existing blockers/check-squares state. This is itself a parity item — log it under the position.cpp walk too.
>
> **Fix-phase sequencing decision (user, 2026-05-26):** finish the full multi-file walk (movepick → eval → position → movegen → tt → bitboard/pawns/material/psqt) and complete the catalog *before* landing any fix, because divergences interact across files (e.g. LMR adjusters depend on movepick ordering).

**Tier 2 — quality/correctness, net node effect needs measuring:**
4. **S7 — NMP verification search.** Zugzwang guard; aligns with the teaching mission (the student loses to slow endgame zugzwang). Adds a verify search at depth≥13 but prevents bad cutoffs.
5. **S8 — Internal Iterative Deepening.** Self-contained ordering improvement.
6. **Q5 — qsearch evasion pruning.** Targets the check-extension outliers (FEN 20/26).
7. **S4+Q3 — "ttValue as better eval"** (land together). Changes pruning-gate eval.
8. **S2 — TT-cutoff history updates.** Ordering quality.
9. **S5+Q4 — statScore eval bonus + after-null tempo** (land together).
10. **S13 / S15 / S17 / S18 — LMR adjusters + re-search guards.** Small, mixed-direction.

**Tier 3 — deferred / needs prerequisites (documented):**
- S6 razoring (cheap, but `//!`-deferred — could promote to Tier 1 if a quick win is wanted)
- S9 singular+multicut (regressed 3×; revisit after outliers tamed)
- S11/S12 ttHitAverage & ttPv LMR (need machinery / regressed without prereqs)
- S1 has_game_cycle (cuckoo subsystem; ~1 Elo)

## Fixes landed

(empty — append commit hash + bench delta per ROADMAP rule "A/B each fix against bench")

| Commit | Divergence # | Before (nodes) | After (nodes) | Notes |
|---|---|---:|---:|---|
| `4efaec6` | **E1** king-ring color | 13,321,304 | 14,208,970 | d=14 1T warm. +6.7% nodes — *expected*: correctness fix (king-safety eval now matches SF), not a perf lever. SF's 6.57M baseline already uses correct king safety, so this isn't a regression vs SF, just our tree shifting to the correct-eval shape. All 888 tests pass, clippy introduces nothing new |
| `4efaec6` | **P1 + P13** ep key | 14,208,970 | **12,390,837** | d=14 1T warm. **−12.8%** on top of E1 (−7% vs the pre-audit 13.32M baseline). ep now only set when capturable → fewer phantom-ep key divergences → more TT/repetition hits. Correctness fix that *also* closes the gap: **2.03× → 1.89× SF** at d=14. Tests: 723→725 engine (replaced 1 ep test with corrected capturable + phantom-drop coverage, +1 do_move capturable test); all green |

**d=20 1T confirmation (E1+P1+P13 combined):** pre-audit 225,009,232 → **191,634,106 (−14.8%)**, NPS 1.88M→1.99M. **d=20 gap: 3.3× → 2.82× SF.** The ep-key fix helps more at depth (bigger tree, more recovered TT/repetition hits). Both bug fixes are pure correctness — no tuning risk — and together they close a meaningful slice of the gap before any pruning-lever work. Committed as `4efaec6` (one commit, both bugs; the unrelated per_piece_mobility WIP in eval/mod.rs was kept unstaged).

### Lever A/B results (baseline after bug fixes: d=14 1T warm = 12,390,837)

| Lever | Result | Δ nodes | Decision |
|---|---|---:|---|
| **MP1** capture ordering (drop static LVA, match SF pure-MVV) | 12,390,837 → 12,560,583 | **+1.4%** | **REVERTED.** Our MVV-LVA beats SF's pure-MVV on our node count (our capture-history is less developed in short searches, so the static LVA tiebreak still adds signal). Justified deviation — documented in `movepick.rs` mvv_lva |
| **S8** Internal Iterative Deepening (depth≥7, no tt_move → depth-7 re-search) | 12,390,837 → 12,461,084 | **+0.57%** | **REVERTED.** Depth-7 re-search overhead slightly exceeds the ordering gain in isolation at d=14. ~1 Elo in SF; depth-sensitive. Bundle-revisit candidate (interacts with other ordering levers) |
| **S10** LMR on captures (SF entry conditions + capture `r++`) | 12,390,837 → 15,263,822 | **+23%** | **REVERTED.** Big regression: reducing forcing captures triggers expensive full-depth re-searches without SF's companion relaxers (ttHitAverage gate, full adjuster stack) to rebalance. Classic interaction effect |
| **Q1** qsearch futility pruning (via new `Position::gives_check`) | 12,390,837 → 13,356,292 | **+7.8%** | **REVERTED** (the prune). The biggest *expected* win also regressed: pruning qsearch captures perturbs qsearch return values, which nets a larger negamax tree for our (already-tuned) stack. Also bumped the MultiPV-convergence canary test (expected). **`gives_check` itself is KEPT** (commit `26d64cd`) — correct, oracle-tested, reusable infra |

**Verdict on isolated pruning levers:** all FOUR tested regress (MP1 +1.4%, S8 +0.57%, S10 +23%, Q1 +7.8%) — including Q1, the biggest expected win. This confirms the hard-won lesson ([[feedback_pruning_bundles]]: bundle attempt regressed 28×; [[feedback_lever2_regressed]]: 58×; [[feedback_singular_extensions_third_attempt]]): SF's pruning features are a *balanced set* — adding one in isolation to our already-tuned engine unbalances the tree. The remaining isolated candidates (S7 NMP-verification, S13/S15 LMR relaxers, S14, Q5) are mostly node-*adders* or balancers that would regress alone too. **The win in this audit came from the two correctness bugs, not from porting more pruning.**

**Q1 outcome (2026-05-26):** built `Position::gives_check` (P9, commit `26d64cd`, oracle-tested) and wired qsearch futility on top — it regressed +7.8% (see table). Reverted the prune; kept gives_check. So even the one genuine subtractive prune doesn't help our tuned stack in isolation.

**Conclusion of the fix phase:** the audit's node-count win came entirely from the two correctness bugs (E1, P1+P13): **d=14 2.03×→1.89× SF, d=20 3.3×→2.82× SF.** Every isolated pruning lever regressed. Closing the residual gap further would require either (a) porting SF's LMR adjusters as a *balanced bundle* (high risk — past bundle regressed 28×), or (b) the `set_check_info` *caching* rework for the NPS half of the gap (a hot-path change, deferred). Both are larger projects with uncertain payoff; the correctness fixes are the safe, banked result. `gives_check` remains available for any future bundle (it'd let Q1 ride a balanced set).

---

## Faithful-port bundle phase (2026-05-26, session 2)

Re-audit (3 subagents + direct verification) overturned the prior "faithful port, only balance-dependent pruning left" verdict. Found **latent tuning bugs the earlier audit rubber-stamped**, all verified against both source trees:

- **LMR table `23.4` — SF11 is `24.8`** (search.cpp:197 single-thread). `23.4` appears nowhere in SF11; the code comment + a memory note both wrongly claimed it matched. Systematic under-reduction.
- **LMR move-count gate `4` — SF reduces from move 2** (`moveCount > 1 + rootNode + …`, search.cpp:1118). We full-depthed 3 extra moves/node.
- **Missing `d != newDepth` re-search guard** (search.cpp:1197).
- Capture SEE-prune margin `200` vs SF `194` + an invented `depth ≤ 6` cap; NMP base wrong at depth 3 (`/200` vs `/192`); MP1 capture ordering subtracts attacker value where SF subtracts nothing.
- **Structural NPS leak the prior audit missed:** we `do_move` *before* pruning then `undo_move` on a prune (SEE-surviving captures `do_move` twice); SF prunes before making the move. Node-neutral; ~half the runtime gap.

**Key reframe:** these weren't independent — the LMR de-tuning (gate 4, table 23.4, no relaxers) was *compensation* for SF's LMR relaxers being absent. The prior A/Bs that "proved SF's values regress" ran on a tree already distorted by these bugs, so those conclusions were untrustworthy. The fix is the **complete balanced subsystem**, never tried before.

### Landed (each A/B'd, warm-TT, 1T)

| Commit | Change | d=14 nodes | d=20 nodes |
|---|---|---:|---:|
| (baseline `fbec20f`) | — | 12,390,837 | 191,634,106 |
| `5a2a68a` | Quiet-LMR faithful bundle (gate→2, table→24.8, re-search guard, relaxers ttPv/oppMC/ttCapture/escape/ttHitAverage, didLMR feedback) | 11,806,689 (−4.7%) | 183,254,002 (−4.4%) |
| `063266e` | Capture-LMR (SF 4-condition gate + late-capture r++) | 10,361,130 (−12.2%) | 152,970,498 (−16.5%) |
| `4c40c1d` | Refined `eval` vs raw `staticEval` for pruning gates (S4) | 10,227,018 (−1.3%) | 147,316,914 (−3.7%) |
| `45be8a7` | Faithful NMP bundle (NMP refined-eval gate + `(854+68·depth)/258` R + depth≥13 verification w/ nmpMinPly/nmpColor + parent-not-null via `was_null` + Q3/Q4 qsearch stand-pat refinement + razoring S6 + S5/C2 negamax after-null/−statScore eval) | 9,739,495 (−4.8%) | 138,713,681 (−5.8%) |

**Cumulative: d=14 −21.4% (1.89× → 1.48× SF), d=20 −27.6% (2.82× → 2.04× SF). 704 engine lib tests pass; clippy clean.** Capture-LMR — which regressed +23% *in isolation* in the prior audit — is a −16.5% win once the relaxers are present, decisively confirming SF's pruning is a balanced set that cannot be A/B'd piece-by-piece. The faithful NMP bundle is the same lesson again: NMP regressed +4–12% *alone* but is a −5–6% win bundled with its eval-balance companions (Q3/Q4 + razoring + S5/C2).

### Attempted, regressed, reverted

- **Faithful NMP (refined-eval gate + `(854+68·depth)/258` reduction + depth≥13 verification w/ nmpMinPly/nmpColor).** A/B at d=14: +4.3% with our old R, +12.4% with SF's R. SF's aggressive NMP is balanced by companions not yet ported (qsearch ttValue refinement Q3/Q4, razoring S6, after-null eval S5). Reverted at the time. **✅ LANDED 2026-05-26 (commit `45be8a7`)** once bundled with exactly those companions — see the Landed table above (d=14 −4.8%, d=20 −5.8%). Confirms the prediction: NMP needed its eval-balance companions, not a different reduction formula.
- **qsearch stand-pat ttValue refinement (Q3) alone:** +0.4% (neutral/noise). Reverted as a solo lever; **landed inside the NMP bundle** (`45be8a7`) where it belongs.

### Faithful NMP bundle (Phase B) — LANDED (2026-05-26, commit `45be8a7`)

The full SF11 NMP family, landed and benched as one balanced bundle (per [[feedback_pruning_bundles]] / [[feedback_lmr_base_divergence]] — never A/B a single lever of a balanced set against our tree). Components:

1. **NMP** (search.cpp:838-885): gate on the *refined* `eval` (`eval ≥ beta` **and** `eval ≥ staticEval`; floor `staticEval ≥ beta − 32·depth + 292 − 30·improving`); skip when the parent itself nulled (new `StackEntry.was_null`, set true only in the NMP block, false at every real-move recursion site incl. ProbCut); reduction `R = (854+68·depth)/258 + min((eval−beta)/192, 3)` (fixes the depth-3 base + `/200`→`/192`); `reduced = depth − R` with **no `.max(1)` clamp** (faithful — lets the null child dive into qsearch, which makes Q4 reachable); depth≥13 **verification** re-search at the same ply with NMP suspended for the cutting side via `Search.nmp_min_ply`/`nmp_color` (the `nmp_min_ply != 0` guard forbids recursive verification, matching SF's `assert(!nmpMinPly)`).
2. **S5/C2** (search.cpp:808-820): on a TT-miss out of check, `raw = evaluate + (−parentStatScore/512)` for a real-move parent, or `raw = −(parent raw staticEval) + 2·Tempo` after a null move. Kept contempt-free via a new `StackEntry.raw_static_eval` (the after-null negate reads the parent's *raw* value, so the TT-persisted eval stays clean).
3. **Q3** (search.cpp:1422-1425): qsearch stand-pat ttValue refinement (mirror of S4).
4. **Q4** (search.cpp:1428-1430): qsearch after-null stand-pat (reads parent `raw_static_eval`).
5. **Razoring / S6** (search.cpp:822-826, `RAZOR_MARGIN = 531`): `!root && depth < 2 && eval ≤ alpha − 531 → qsearch`.

**Kept divergences** (deliberate, documented in code): `NULL_MIN_DEPTH = 3` floor retained (SF nulls at any depth; low depths are covered by razoring `<2` + RFP `<6`) — a follow-up lever if more is wanted. The verification ply-offset `(3·reduced/4).max(0)` saturates at 0 to keep the usize arithmetic safe in shallow mate-territory verifications (SF uses signed plies where a negative floor is simply always-satisfied — equivalent).

A/B result in the Landed table. NPS rose at both depths (after-null skips `evaluate()` calls; razoring trims shallow nodes) — a rare node-*and*-NPS win.

### Structural NPS (Phase A) — finding: B1 and B3 are coupled

Built **B3** (`set_check_info` caching: cache `king_blockers`/`king_pinners`/`checkers` in Position, save/restore in StateInfo, maintained in do_move/do_null_move/from_fen; SEE + `checkers()` + `blockers_for_king()` read the cache). It is **correct and node-neutral** (bench node count stayed byte-identical at 10,227,018; all 726 tests pass after a robustness guard for transient kingless positions in the do/undo legality filter). **But it regressed NPS ~7% at d=14 and was reverted**, because:

`compute_check_info` runs on *every* `do_move`, but our move loop currently `do_move`s **before** pruning (CMP/futility/SEE all `do_move` then `undo`). So every pruned-move do/undo pays a full `compute_check_info` (2× `slider_blockers`) whose result is never consumed. The cache only pays off once pruning moves **before** `do_move` (**B1**), so that one `compute_check_info` per *node* serves that node's `in_check` + all its moves' `legal()`/`gives_check`/SEE.

**So the structural NPS win requires the full B1+B3 bundle, landed together:**
1. Port `Position::legal(mv)` (pin-aware, pre-move) — reads cached `king_blockers` + `aligned()` (exists) + king-move-safety + ep/castling special cases. Add a do/undo oracle test (like `gives_check`).
2. Restructure the negamax move loop to SF's pre-`do_move` order (search.cpp:962-1113): `legal()` → `gives_check` (cached) → extensions → Step-13 pruning, then a single `do_move` for survivors only. Must preserve exact prune semantics, `quiets_tried`, `move_count`, and node-neutrality (node count must remain 10,227,018).
3. Re-add B3 caching; SEE/checkers/blockers read the cache; B4 falls out.

This is a large, correctness-critical restructure of the hottest path — deferred to a focused session rather than rushed. The B3 code is straightforward to reconstruct (see git reflog / this entry).

---

## Bench output archive

### `bench 16 1 13` — ours, warm-TT (2026-05-25)

```
  31/45  depth 13      207807 nodes      104 ms    1.98 Mnps
  32/45  depth 13      372870 nodes      211 ms    1.76 Mnps
  33/45  depth 13      219199 nodes      121 ms    1.80 Mnps
  34/45  depth 13      137458 nodes       85 ms    1.61 Mnps
  35/45  depth 13      119063 nodes       31 ms    3.74 Mnps
  36/45  depth 13       71331 nodes       21 ms    3.25 Mnps
  37/45  depth 13      183218 nodes       40 ms    4.56 Mnps
  38/45  depth 13      213561 nodes       66 ms    3.23 Mnps
  39/45  depth 13      149800 nodes       48 ms    3.06 Mnps
  40/45  depth 13      302491 nodes       66 ms    4.51 Mnps
  41/45  depth 13      758990 nodes      283 ms    2.67 Mnps
  42/45  depth 11       29052 nodes       12 ms    2.27 Mnps
  43/45  depth  9       11643 nodes        5 ms    2.02 Mnps
  44/45  (terminal — stalemate)
  45/45  (terminal — checkmate)
Total time (ms) : 3079    Nodes : 7,141,275    NPS : 2,319,288
```

Only the tail of the per-position output was captured; the head is similar order of magnitude per position.

### `bench 16 1 14` — ours, warm-TT (2026-05-25)
`Total: 5860 ms, 13,321,304 nodes, 2.27 Mnps`

### `bench 16 1 14` — ours, cold-TT (2026-05-25)
`Total: 5259 ms, 11,755,764 nodes, 2.23 Mnps`  
(Note: cold-TT < warm-TT here; likely TT-aging interaction within a warm run. Not investigated.)

### `bench 16 1 13` — SF11, warm-TT (2026-05-25)
`Total: 1442 ms, 5,156,767 nodes, 3.58 Mnps`  
SF11 runs 46 positions to our 45 (we drop the Chess960 entry from SF11's `Defaults[]`).

### `bench 16 1 14` — SF11, warm-TT (2026-05-25)
`Total: 1920 ms, 6,567,129 nodes, 3.42 Mnps`

### `bench 128 8 20` — ours, warm-TT (2026-05-25)
`Total: 95,384 ms, 768,775,887 nodes, 8.06 Mnps`

### `bench 128 8 20` — SF11, warm-TT (2026-05-25)
`Total: 13,326 ms, 278,148,269 nodes, 20.87 Mnps`
