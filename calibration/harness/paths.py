"""Resolve the on-disk locations of the harness tools, nets, book, and
our engine binary — robust to the nested layouts the release zips extract
into (e.g. ``tools/fastchess/fastchess-windows-x86-64/fastchess.exe``).

Everything is resolved relative to the ``calibration/`` directory (the
parent of this package), so the harness works regardless of the cwd it's
launched from. A missing artifact raises immediately with a pointer to
``fetch-tools.sh`` rather than failing deep inside a fastchess call.
"""

from __future__ import annotations

from pathlib import Path

# calibration/harness/paths.py -> calibration/
CALIB_DIR = Path(__file__).resolve().parent.parent
REPO_DIR = CALIB_DIR.parent

TOOLS = CALIB_DIR / "tools"
NETS = CALIB_DIR / "nets"
BOOKS = CALIB_DIR / "books"
RUNS = CALIB_DIR / "runs"


def _one(pattern_root: Path, glob: str, what: str) -> Path:
    """Return the single file matching ``glob`` under ``pattern_root``."""
    matches = sorted(pattern_root.glob(glob))
    if not matches:
        raise FileNotFoundError(
            f"{what} not found (looked for {pattern_root}/{glob}). "
            f"Run `bash calibration/fetch-tools.sh` first."
        )
    return matches[0]


def fastchess_exe() -> Path:
    return _one(TOOLS, "fastchess/**/fastchess.exe", "fastchess.exe")


def lc0_exe() -> Path:
    return _one(TOOLS, "lc0/**/lc0.exe", "lc0.exe")


def ordo_exe() -> Path:
    # Prefer the 64-bit build.
    return _one(TOOLS, "ordo/**/ordo-win64.exe", "ordo-win64.exe")


def chess_tutor_exe() -> Path:
    """Our engine's release binary (built separately via cargo)."""
    p = REPO_DIR / "target" / "release" / "chess-tutor.exe"
    if not p.exists():
        raise FileNotFoundError(
            f"{p} not found. Build it: cargo build --release --bin chess-tutor"
        )
    return p


def maia_net(rating: int) -> Path:
    p = NETS / f"maia-{rating}.pb.gz"
    if not p.exists():
        raise FileNotFoundError(f"{p} not found. Run fetch-tools.sh.")
    return p


def opening_book() -> Path:
    return _one(BOOKS, "*.pgn", "opening book (.pgn)")


def runs_dir() -> Path:
    RUNS.mkdir(parents=True, exist_ok=True)
    return RUNS
