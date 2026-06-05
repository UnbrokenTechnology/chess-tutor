"""Eval-mask experiment — each mask's Elo effect at several base strengths.

Eval masks are a *different* weakening mechanism (knowledge gaps, not move
errors), and 2^8 full-factorial is infeasible, so they get their own small
experiment rather than a grid dimension. We take a handful of base configs
spanning the Elo range and, for each, disable each eval category (8
singles) and each thematic group (4), measuring the Elo delta vs the
un-masked base. That's exactly what backbone rules need ("how much does
hiding king-safety cost at ~1200 vs ~2000?").

Usage (calibration/ dir, venv python):
  python run_masks.py --dry-run
  python run_masks.py
"""

from __future__ import annotations

import argparse
import csv
from dataclasses import replace

from harness.engines import BotConfig
from harness.experiment import estimate, run_and_rate
from harness.pools import ALL_MASKS, MASK_GROUPS

# Bases chosen to separate the two questions a mask probe should answer:
#
#  (1) depth x mask  -- DEPTH-PURE bases (no noise), depth 1..8. The only
#      thing varying is search depth, so any change in a mask's Elo cost
#      across these is a real depth interaction.
#
#  (2) does the WEAKENING MECHANISM matter -- RANK-weakened bases. Rank
#      weakening is *eval-dependent*: the bot still ranks moves by eval
#      and just plays a worse-ranked one, so masking the eval changes its
#      ranking and should still bite. (Contrast the earlier WILD bases,
#      preserved in runs/masks_wild: wild plays random moves, ignores eval
#      entirely, so masks were redundant there.) Rank is the realistic
#      non-random strength lever -- "on average play the 2nd/3rd-best move".
BASES = [
    BotConfig("base-d1", depth=1),                        # depth-pure ~1750
    BotConfig("base-d2", depth=2),
    BotConfig("base-d4", depth=4),
    BotConfig("base-d6", depth=6),
    BotConfig("base-d8", depth=8),                        # ~2500
    BotConfig("base-d4-r2", depth=4, avg_move_rank=2.0),  # eval-dependent weak
    BotConfig("base-d4-r4", depth=4, avg_move_rank=4.0),
    BotConfig("base-d4-r6", depth=4, avg_move_rank=6.0),
]


def mask_settings() -> list[tuple[str, tuple[str, ...]]]:
    settings: list[tuple[str, tuple[str, ...]]] = [("none", ())]
    settings += [(m, (m,)) for m in ALL_MASKS]              # 8 singles
    settings += [(f"grp-{g}", s) for g, s in MASK_GROUPS.items()]  # 4 groups
    return settings


def build_subjects() -> list[BotConfig]:
    subjects = []
    for base in BASES:
        for label, slugs in mask_settings():
            subjects.append(replace(base, name=f"{base.name}__{label}", disable_eval=slugs))
    return subjects


def main() -> None:
    ap = argparse.ArgumentParser(description="Eval-mask experiment")
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--games-per-config", type=int, default=400)
    ap.add_argument("--concurrency", type=int, default=16)
    ap.add_argument("--sims", type=int, default=400)
    args = ap.parse_args()

    subjects = build_subjects()
    print(estimate(len(subjects), args.games_per_config, 49))
    if args.dry_run:
        return

    result = run_and_rate(
        subjects,
        out_subdir="masks",
        games_per_config=args.games_per_config,
        concurrency=args.concurrency,
        batch_size=120,
        sims=args.sims,
    )
    ratings = result.ratings

    csv_path = result.out_dir / "mask_results.csv"
    with open(csv_path, "w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["base", "mask_label", "disable_eval", "base_elo",
                    "masked_elo", "delta_elo", "masked_error", "games"])
        for base in BASES:
            none_name = f"{base.name}__none"
            base_r = ratings.get(none_name)
            base_elo = base_r.rating if base_r else None
            for label, slugs in mask_settings():
                if label == "none":
                    continue
                r = ratings.get(f"{base.name}__{label}")
                if not r:
                    continue
                delta = "" if base_elo is None else f"{r.rating - base_elo:.0f}"
                w.writerow([
                    base.name, label, "|".join(slugs),
                    "" if base_elo is None else f"{base_elo:.0f}",
                    f"{r.rating:.0f}", delta,
                    "" if r.error is None else f"{r.error:.0f}", r.played,
                ])
    print(f"\nwrote {csv_path}")
    # Quick view: average Elo cost of each mask across bases.
    print("mask -> mean Elo delta across bases:")
    for label, _ in mask_settings():
        if label == "none":
            continue
        deltas = []
        for base in BASES:
            br = ratings.get(f"{base.name}__none")
            mr = ratings.get(f"{base.name}__{label}")
            if br and mr:
                deltas.append(mr.rating - br.rating)
        if deltas:
            print(f"  {label:<16}{sum(deltas)/len(deltas):+6.0f}")


if __name__ == "__main__":
    main()
