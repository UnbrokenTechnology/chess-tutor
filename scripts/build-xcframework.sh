#!/usr/bin/env bash
# Build chess-tutor-ffi for every Apple target and combine into a single
# .xcframework consumable by the SwiftUI app.
#
# Populated in Phase 3 once uniffi is enabled in core/chess-tutor-ffi/Cargo.toml.
set -euo pipefail

TARGETS=(
    aarch64-apple-ios
    aarch64-apple-ios-sim
    aarch64-apple-darwin
    x86_64-apple-darwin
)

echo "scripts/build-xcframework.sh: not implemented yet (Phase 3)." >&2
echo "Expected targets: ${TARGETS[*]}" >&2
exit 1
