# ELO-calibration harness — tooling

External tooling for measuring our bot configs against the Maia ladder and
fitting a dials→Elo model. See [`HANDOFF-calibration.md`](HANDOFF-calibration.md)
for the harness internals and the repo-root [`HANDOFF-solver.md`](../HANDOFF-solver.md)
for the live calibration work; this file is the **vetted download list** (for
sign-off before `fetch-tools.sh` runs) plus the anchor findings.

## Methodology validated (2026-06-04 pilot)

The end-to-end pipeline (fastchess gauntlet → Ordo → anchored Elo) was
validated on a 13-player round-robin (5-net Maia ladder + configs). Two
durable takeaways drove every run since (the dial-specific numbers are
stale — `wild`/`blunder`/`miss` have since been removed in favor of the
perception lever; see the repo-root calibration handoffs):

1. **Methodology sound; the Maia ladder is monotone.** With one net pinned,
   Ordo places the others in correct order.
2. **The pool scale is COMPRESSED vs the human measured scale**, more at the
   top (our pool ~200 Elo across maia-1100→1900; measured ~290). A single
   anchor *understates* human Elo away from the pin. **Fix: production uses
   loose multi-anchoring** (`rate(loose_anchors=...)`) to stretch the pool
   onto all three measured points. Residual is the genuine
   engine-pool-vs-human-pool width difference.

Everything here is a **one-time download**; the harness then runs fully
offline. Binaries / nets / books are git-ignored (large, externally
licensed, never shipped) — only our scripts + docs + summaries are tracked.

## Vetted versions (verified 2026-06-04 via the GitHub release API)

| Tool | Version | Asset | Why this one |
|---|---|---|---|
| **fastchess** | `v1.8.0-alpha` | `fastchess-windows-x86-64.zip` | Latest release; the SF Fishtest runner since 2024. UCI-only, concurrent, built-in SPRT. (Still alpha — the plan flagged re-verifying flags; man.md confirms `-openings`, `-pgnout`, `-concurrency`, `-resume`.) |
| **lc0** | `v0.32.1` | `lc0-v0.32.1-windows-cpu-dnnl.zip` | **CPU dnnl** backend on purpose: at `go nodes 1` (pure policy) a CPU pass is fast + deterministic + needs no CUDA. `openblas` is the fallback. |
| **Ordo** | `v1.2.6` | `ordo-1.2.6-win.zip` | Rating calc the SF team used to calibrate UCI_Elo; `-A` anchors the pool. |
| **Maia nets** | `v1.0` release | `maia-{1100..1900}.pb.gz` (9 files) | The human-calibrated anchor ladder. Run under lc0 with `go nodes 1`. |
| **Opening book** | `master` | `8moves_v3.pgn.zip` (807 KB, balanced) | **Balanced** 4-move popular openings, fed to BOTH engines. **NOT** a UHO / `+90..+149` book — those are engine-A/B *sensitivity* suites that gift one side ~+1 pawn and push Maia off its human training distribution. |

Run `bash calibration/fetch-tools.sh` to fetch + lay everything out under
`tools/`, `nets/`, `books/`.

## ⚠️ Anchor findings (changes the orchestration — resolve before Run 1)

Researching the "measured Maia ratings" open item (PLAN item 1) surfaced two
things that **must** shape how we anchor:

1. **Only 3 of the 9 nets have measured human ratings.** Maia runs as public
   Lichess bots only as `maia1` (net 1100), `maia5` (net 1500), `maia9` (net
   1900). The other six nets (1200/1300/1400/1600/1700/1800) have **no**
   measured human rating — only their training-target label.

2. **Measured ratings run well above the labels, non-uniformly, and are
   time-control-dependent / drift over time.** Snapshots found:

   | Net (label) | Lichess bot | Rapid | Bullet |
   |---|---|---|---|
   | 1100 | maia1 | ~1565 | ~1648 |
   | 1500 | maia5 | ~1680 | — |
   | 1900 | maia9 | ~1855 | ~1784 |

   The label→measured gap is **+465 at 1100 but ≈ −45 at 1900** — strongly
   compressed, *not* a constant offset. So we **cannot** anchor by adding a
   fixed correction to the band labels, and we cannot trust the labels as
   anchor values.

**Implication for the design.** Don't treat the 9 labels as 9 anchors.
Instead:

- Run a **local round-robin among the 9 Maia nets themselves** (plus our
  configs). This gives a self-consistent internal ladder. Ordo/BayesElo
  produce ratings on the standard Elo scale *by construction*, so the pool
  needs only an **offset** pinned — anchor on the measured points (start
  with `maia5 ≈ 1680` rapid, the middle, least extrapolated) and treat
  `maia1`/`maia9` as **cross-checks** on whether our local pool's *spacing*
  matches the human scale.
- If the local spacing between maia1/maia5/maia9 disagrees with their
  measured rapid spacing, that's the known **engine-pool-vs-human-pool scale
  gap** (engine pools run "wider"). Note it and prefer the rapid measured
  numbers as ground truth for the human scale we're targeting.
- **Pick ONE time control** for the measured anchors and stick to it
  (rapid is closest to "thinking" human play; our bot is depth-budget, not
  time-budget, so TC only matters for the anchor lookup, not our engine).
- This is consistent with the locked decision to handle the **extremes via
  self-play connectivity + extrapolation** — the same machinery (let Ordo
  place players by transitive results, anchor the offset once) covers both
  the 6 unmeasured intermediate nets and our sub-1100 / >1900 configs.

## Directory layout (after fetch)

```
calibration/
  README.md          (this file — tracked)
  fetch-tools.sh     (downloader — tracked)
  .gitignore
  tools/
    fastchess/  lc0/  ordo/        (extracted; git-ignored)
  nets/  maia-1100.pb.gz … 1900    (git-ignored)
  books/ 8moves_v3.pgn             (git-ignored)
  runs/  …                          (experiment PGNs/fits — git-ignored)
```

## Smoke test after fetch

```bash
find calibration/tools -name '*.exe'        # locate the extracted exes
# Maia under lc0 (pure policy, one node):
printf 'uci\nposition startpos\ngo nodes 1\nquit\n' \
  | calibration/tools/lc0/.../lc0.exe --weights=calibration/nets/maia-1100.pb.gz
# Our bot as UCI (already built):
printf 'uci\nposition startpos\ngo depth 8\nquit\n' \
  | target/release/chess-tutor.exe uci --depth 8
```

## Sources

- fastchess: <https://github.com/Disservin/fastchess> · man.md flags
- lc0: <https://github.com/LeelaChessZero/lc0/releases>
- Ordo: <https://github.com/michiguel/Ordo>
- Maia nets + run instructions: <https://github.com/CSSLab/maia-chess>
- Measured Maia bot ratings: <https://lichess.org/@/maia1> · <https://lichess.org/@/maia9> · Lichess forum threads
- Opening book: <https://github.com/official-stockfish/books>
