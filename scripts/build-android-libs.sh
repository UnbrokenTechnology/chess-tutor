#!/usr/bin/env bash
# Build chess-tutor-ffi for every Android ABI and drop the resulting .so
# files into android/app/src/main/jniLibs/<abi>/.
#
# Populated in Phase 4 once uniffi is enabled in core/chess-tutor-ffi/Cargo.toml.
set -euo pipefail

ABIS=(
    "aarch64-linux-android:arm64-v8a"
    "armv7-linux-androideabi:armeabi-v7a"
    "x86_64-linux-android:x86_64"
)

echo "scripts/build-android-libs.sh: not implemented yet (Phase 4)." >&2
echo "Expected ABI pairs (rust-target:android-abi): ${ABIS[*]}" >&2
exit 1
