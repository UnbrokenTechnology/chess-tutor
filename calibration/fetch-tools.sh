#!/usr/bin/env bash
#
# Fetch the external tooling for the ELO-calibration harness.
#
# This is the ONLY network step in the whole project — one-time downloads,
# after which the harness runs fully offline (see CLAUDE.md "fully offline
# on-device"). Versions are pinned below and documented with rationale in
# calibration/README.md; keep the two in sync.
#
# Run from git-bash:  bash calibration/fetch-tools.sh
# Downloads via curl; extracts zips via PowerShell Expand-Archive (always
# present on Windows — avoids depending on `unzip` being installed in
# git-bash). Maia nets are .pb.gz and need no extraction (lc0 reads them
# directly).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$here"

# --- pinned versions (verified 2026-06-04 via the GitHub release API) ---
FASTCHESS_VER="v1.8.0-alpha"   # Disservin/fastchess (latest; replaced cutechess as SF's runner)
LC0_VER="v0.32.1"              # LeelaChessZero/lc0
ORDO_VER="v1.2.6"             # michiguel/Ordo  (asset filename has no 'v')
MAIA_REL="v1.0"               # CSSLab/maia-chess release holding the 9 nets
BOOK="8moves_v3.pgn"          # official-stockfish/books — BALANCED 4-move book

mkdir -p tools nets books

dl() { # url dest
  echo ">> $(basename "$2")"
  curl -fL --retry 3 -o "$2" "$1"
}

unzip_to() { # zipfile destdir
  # -LiteralPath/-Force so re-runs overwrite cleanly. Backslash the paths
  # for PowerShell's sake by letting it resolve relative to the cwd.
  powershell.exe -NoProfile -Command \
    "Expand-Archive -LiteralPath '$1' -DestinationPath '$2' -Force"
}

# --- fastchess (the match runner) ---
dl "https://github.com/Disservin/fastchess/releases/download/${FASTCHESS_VER}/fastchess-windows-x86-64.zip" tools/fastchess.zip
unzip_to "tools/fastchess.zip" "tools/fastchess"

# --- lc0 (engine body for the Maia nets) ---
# CPU 'dnnl' backend on purpose: at `go nodes 1` (pure policy, no tree
# search) a CPU forward pass is plenty fast, it's deterministic, and it
# pulls in NO CUDA/GPU dependency. The dnnl runtime DLLs ship inside the
# zip. (openblas is the fallback if dnnl misbehaves on this CPU.)
dl "https://github.com/LeelaChessZero/lc0/releases/download/${LC0_VER}/lc0-${LC0_VER}-windows-cpu-dnnl.zip" tools/lc0.zip
unzip_to "tools/lc0.zip" "tools/lc0"

# --- Ordo (rating calculator) ---
dl "https://github.com/michiguel/Ordo/releases/download/${ORDO_VER}/ordo-1.2.6-win.zip" tools/ordo.zip
unzip_to "tools/ordo.zip" "tools/ordo"

# --- Maia nets, 1100..1900 (the human-calibrated anchor ladder) ---
for r in 1100 1200 1300 1400 1500 1600 1700 1800 1900; do
  dl "https://github.com/CSSLab/maia-chess/releases/download/${MAIA_REL}/maia-${r}.pb.gz" "nets/maia-${r}.pb.gz"
done

# --- balanced opening book (fed to BOTH engines by fastchess) ---
# 8moves_v3 = balanced, popular openings, 8 plies. Deliberately NOT a UHO
# / "+90..+149" book: those are engine-A/B *sensitivity* suites that hand
# one side ~+1 pawn and push Maia off its human training distribution.
dl "https://github.com/official-stockfish/books/raw/master/${BOOK}.zip" "books/${BOOK}.zip"
unzip_to "books/${BOOK}.zip" "books"

echo
echo "=== fetched. quick layout check ==="
ls -1 tools nets books 2>/dev/null || true
echo
echo "Next: locate the extracted exes (paths vary by zip layout):"
echo "  find tools -name '*.exe'"
echo "Then smoke-test Maia under lc0 (pure policy):"
echo "  printf 'uci\\nposition startpos\\ngo nodes 1\\nquit\\n' | \\"
echo "    tools/lc0/<...>/lc0.exe --weights=nets/maia-1100.pb.gz"
