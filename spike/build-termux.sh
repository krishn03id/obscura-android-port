#!/data/data/com.termux/files/usr/bin/bash
# Native build+run in Termux on the Android device itself. No cross-compile, no NDK.
set -euo pipefail

command -v cargo >/dev/null || { echo "Installing rust..."; pkg install -y rust clang binutils; }

# Termux IS aarch64-linux-android, so a plain build is already a native Android binary.
# IMPORTANT: the .cargo/config.toml here sets cross linkers for a linux host — Termux must
# ignore it, so we neutralize with an env override pointing at Termux's own clang:
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=""
mv .cargo/config.toml .cargo/config.toml.cross 2>/dev/null || true

cargo run --release
# Expected:
# [spike] 1 + 1 = 2
# [spike] optional-chaining/?? = 42
# [spike] 2n ** 64n = 18446744073709551616
# [spike] typeof globalThis = object
# [spike] OK
