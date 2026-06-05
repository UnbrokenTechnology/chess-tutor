"""Pre-grid probe: the depth x qsearch-depth Elo landscape (best move, no
noise) + how eval masks interact with a tactically-limited bot.

Answers, before committing to a full grid:
  1. What Elo does each (depth, qsearch-depth) combo land at when it always
     plays its best move? In particular the "positionally smart but
     tactically short-sighted" corner (e.g. depth 2, qdepth 2: sees the
     immediate recapture so it won't hang a queen, but misses deeper
     tactics) -- a believable ~1000-1500 human shape.
  2. Do eval masks still bite once tactical vision is capped, or does
     limited qsearch already dominate the weakness?

Run (calibration/ dir, venv python):
  python run_qdepth_probe.py
"""

from __future__ import annotations

import argparse
import csv
from dataclasses import replace

from harness.engines import BotConfig
from harness.experiment import run_and_rate
from harness.pools import MASK_GROUPS

# qsearch-depth values to sweep (None = full vision / infinite horizon).
QDEPTHS: list[int | None] = [0, 1, 2, 6, None]
DEPTHS = [1, 2, 4, 6]


def qlabel(qd: int | None) -> str:
    return "inf" if qd is None else str(qd)


def build_subjects() -> list[BotConfig]:
    subs: list[BotConfig] = []
    # 1. depth x qsearch-depth, best move (no noise).
    for d in DEPTHS:
        for qd in QDEPTHS:
            subs.append(BotConfig(f"d{d}-q{qlabel(qd)}", depth=d, qsearch_depth=qd))
    # 2. masks on two tactical-vision levels at depth 4: capped (q2) vs full.
    for qd in (2, None):
        base = BotConfig(f"mbase-d4-q{qlabel(qd)}", depth=4, qsearch_depth=qd)
        subs.append(base)  # the un-masked baseline for delta
        for gname, slugs in MASK_GROUPS.items():
            subs.append(replace(base, name=f"{base.name}__{gname}", disable_eval=slugs))
    return subs


def main() -> None:
    ap = argparse.ArgumentParser(description="depth x qsearch-depth probe")
    ap.add_argument("--games-per-config", type=int, default=300)
    ap.add_argument("--concurrency", type=int, default=16)
    ap.add_argument("--sims", type=int, default=300)
    args = ap.parse_args()

    subjects = build_subjects()
    result = run_and_rate(
        subjects,
        out_subdir="qdepth_probe",
        games_per_config=args.games_per_config,
        concurrency=args.concurrency,
        sims=args.sims,
    )
    r = result.ratings

    # CSV dump
    csv_path = result.out_dir / "qdepth_results.csv"
    with open(csv_path, "w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["name", "depth", "qsearch_depth", "disable_eval", "elo", "elo_error", "games"])
        for name, rr in sorted(r.items(), key=lambda kv: -kv[1].rating):
            c = result.subjects_by_name.get(name)
            if c is None:
                continue
            w.writerow([name, c.depth, c.qsearch_depth, "|".join(c.disable_eval),
                        f"{rr.rating:.0f}", "" if rr.error is None else f"{rr.error:.0f}", rr.played])
    print(f"\nwrote {csv_path}")

    # 1. depth x qsearch-depth Elo table.
    print("\n=== Elo: depth (rows) x qsearch-depth (cols), best move ===")
    hdr = "depth ".ljust(7) + "".join(f"q{qlabel(qd):>5}" for qd in QDEPTHS)
    print(hdr); print("-" * len(hdr))
    for d in DEPTHS:
        row = f"d{d}".ljust(7)
        for qd in QDEPTHS:
            rr = r.get(f"d{d}-q{qlabel(qd)}")
            row += f"{rr.rating:>6.0f}" if rr else "    --"
        print(row)

    # 2. mask deltas at q2 vs full vision.
    print("\n=== mask Elo delta (vs un-masked base) at depth 4 ===")
    print("group".ljust(14) + "q2".rjust(8) + "qinf".rjust(8))
    for gname in MASK_GROUPS:
        cells = ""
        for qd in (2, None):
            base = r.get(f"mbase-d4-q{qlabel(qd)}")
            mk = r.get(f"mbase-d4-q{qlabel(qd)}__{gname}")
            cells += (f"{mk.rating - base.rating:>8.0f}" if base and mk else "      --")
        print(gname.ljust(14) + cells)


if __name__ == "__main__":
    main()
