"""Targeted low-band eval-mask experiment.

Positional understanding is the 1000-1500 jump, so that's the band where
disabling eval signals should matter for backbone rules. This measures
each mask group's Elo cost on the LOW tactical-vision bots (d1/d2 x small
qdepth), and includes a clean test of "do masks bite harder on a
non-best-move bot": d1-q0 vs d1-q0-r3 differ ONLY in move selection
(best vs avg-rank-3), same depth and tactical vision.

Run: python run_lowband_masks.py
"""

from __future__ import annotations

import csv
from dataclasses import replace

from harness.engines import BotConfig
from harness.experiment import run_and_rate
from harness.pools import MASK_GROUPS

# Low-band bases (approx probe Elos in comments).
BASES = [
    BotConfig("d1-q0", depth=1, qsearch_depth=0),                      # ~879
    BotConfig("d1-q1", depth=1, qsearch_depth=1),                      # ~1504
    BotConfig("d1-q2", depth=1, qsearch_depth=2),                      # ~1666
    BotConfig("d2-q0", depth=2, qsearch_depth=0),                      # ~1590
    BotConfig("d2-q2", depth=2, qsearch_depth=2),                      # ~1683
    BotConfig("d1-q0-r3", depth=1, qsearch_depth=0, avg_move_rank=3.0),  # non-best-move
]
SETTINGS = [("none", ())] + [(g, s) for g, s in MASK_GROUPS.items()]


def build_subjects() -> list[BotConfig]:
    subs = []
    for base in BASES:
        for label, slugs in SETTINGS:
            if label == "none":
                subs.append(base)
            else:
                subs.append(replace(base, name=f"{base.name}__{label}", disable_eval=slugs))
    return subs


def main() -> None:
    result = run_and_rate(
        build_subjects(),
        out_subdir="lowband_masks",
        games_per_config=300,
        concurrency=16,
        sims=300,
    )
    r = result.ratings

    csv_path = result.out_dir / "lowband_mask_results.csv"
    with open(csv_path, "w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["base", "mask", "elo", "elo_error", "games"])
        for base in BASES:
            for label, _ in SETTINGS:
                name = base.name if label == "none" else f"{base.name}__{label}"
                rr = r.get(name)
                if rr:
                    w.writerow([base.name, label, f"{rr.rating:.0f}",
                                "" if rr.error is None else f"{rr.error:.0f}", rr.played])
    print(f"\nwrote {csv_path}\n")

    groups = list(MASK_GROUPS)
    print("base (un-masked Elo)   " + "".join(f"{g:>12}" for g in groups))
    print("-" * (23 + 12 * len(groups)))
    for base in BASES:
        be = r.get(base.name)
        if not be:
            print(f"{base.name:<10} (excluded — too weak for the pool floor)")
            continue
        row = f"{base.name:<10} ({be.rating:>5.0f})    "
        for g in groups:
            mk = r.get(f"{base.name}__{g}")
            row += f"{mk.rating - be.rating:>+12.0f}" if mk else f"{'--':>12}"
        print(row)

    # Clean non-best-move test.
    print("\nNon-best-move test (same depth/qdepth, differ only in move pick):")
    for nm in ("d1-q0", "d1-q0-r3"):
        be = r.get(nm)
        if be:
            print(f"  {nm:<10} base Elo {be.rating:.0f}")
    print("  -> compare the mask deltas in the two rows above.")


if __name__ == "__main__":
    main()
