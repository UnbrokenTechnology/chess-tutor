# ELO-calibration harness — tooling

External tooling for measuring our bot configs against the Maia ladder to
build the dials→Elo surface (the product interpolates over the measured grid;
see [`HANDOFF-solver.md`](../HANDOFF-solver.md)). See
[`HANDOFF-calibration.md`](HANDOFF-calibration.md) for the harness internals
and the repo-root [`HANDOFF-solver.md`](../HANDOFF-solver.md) for the live
calibration work; this file is the **tooling download list** plus the durable
Maia anchor findings.

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
| **fastchess** | `v1.8.0-alpha` | `fastchess-windows-x86-64.zip` | The SF Fishtest runner since 2024. UCI-only, concurrent, built-in SPRT. (Flags in use: `-openings`, `-pgnout`, `-concurrency`, `-resume`.) |
| **lc0** | `v0.32.1` | `lc0-v0.32.1-windows-cpu-dnnl.zip` | **CPU dnnl** backend on purpose: at `go nodes 1` (pure policy) a CPU pass is fast + deterministic + needs no CUDA. `openblas` is the fallback. |
| **Ordo** | `v1.2.6` | `ordo-1.2.6-win.zip` | Rating calc the SF team used to calibrate UCI_Elo; `-A` anchors the pool. |
| **Maia nets** | `v1.0` release | `maia-{1100..1900}.pb.gz` (9 files) | The human-calibrated anchor ladder. Run under lc0 with `go nodes 1`. |
| **Opening book** | `master` | `8moves_v3.pgn.zip` (807 KB, balanced) | **Balanced** 4-move popular openings, fed to BOTH engines. **NOT** a UHO / `+90..+149` book — those are engine-A/B *sensitivity* suites that gift one side ~+1 pawn and push Maia off its human training distribution. |

Run `bash calibration/fetch-tools.sh` to fetch + lay everything out under
`tools/`, `nets/`, `books/`.

## Maia anchor findings (durable)

Two facts about the Maia nets shape how the pool is anchored:

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

**How we anchor (production).** Don't treat the 9 labels as 9 anchors:

- Run a round-robin (Maia nets + our configs). Ordo produces ratings on the
  standard Elo scale *by construction*, so the pool needs only its offset +
  scale pinned to the measured human points.
- **Loose multi-anchoring on all three measured points** (`maia1 ≈ 1565` /
  `maia5 ≈ 1680` / `maia9 ≈ 1855` rapid) is the **production default**
  (`rate(loose_anchors=...)`). A single hard anchor *compresses* the pool —
  the pilot placed maia-1900 ~70 Elo low off a single maia-1500 pin — so all
  three points are needed to stretch the pool onto the human scale.
- **Maia is a noisy ruler regardless:** non-transitive and compressed (the
  measured 1100→1900 span is ~290 Elo, narrower than the labels suggest), so
  absolute calibration is **±~100**. Optimize for ladder *shape* (even
  spacing) and let chess.com feel-tests pin the absolute offset — which
  landed at **≈ 0** (target Elo ≈ chess.com Elo directly; see
  [`HANDOFF-perception.md`](../HANDOFF-perception.md)).
- **Pick ONE time control** for the measured anchors and stick to it (rapid
  is closest to "thinking" human play; our bot is depth-budget, not
  time-budget, so TC only matters for the anchor lookup, not our engine).
- **Extremes float.** A rung that loses ~100% to everything above it floats
  down hundreds of Elo in a sparse pool. Measure the basement/ceiling
  **densely with boundary anchors** (a self-connected sub-ladder pinned to a
  few stable rungs), not as isolated configs — see `HANDOFF-calibration.md`.

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
