#!/usr/bin/env bash
# Cross-compile the spike for aarch64 Android from a linux x86_64 host.
# Prereqs: rustup target aarch64-linux-android, NDK r27c at $NDK, host libclang.
set -euo pipefail

NDK="${NDK:-$HOME/android-ndk-r27c}"
TB="$NDK/toolchains/llvm/prebuilt/linux-x86_64"
[ -d "$TB" ] || { echo "ERROR: NDK not found at $NDK (set NDK=...)"; exit 1; }

export LIBCLANG_PATH="${LIBCLANG_PATH:-/usr/lib64}"
export CC_aarch64_linux_android="$TB/bin/aarch64-linux-android24-clang"
export AR_aarch64_linux_android="$TB/bin/llvm-ar"
export CFLAGS_aarch64_linux_android="--target=aarch64-linux-android24 --sysroot=$TB/sysroot"
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="--sysroot=$TB/sysroot --target=aarch64-linux-android24"

# NOTE: .cargo/config.toml in this dir hardcodes a linker path — fix it to your NDK path,
# or override here:
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$TB/bin/aarch64-linux-android24-clang"

MODE="${1:-dynamic}"
if [ "$MODE" = "static" ]; then
  # static build runs under qemu-user without Android's /system/bin/linker64
  RUSTFLAGS="-C target-feature=+crt-static" cargo build --release --target aarch64-linux-android
else
  cargo build --release --target aarch64-linux-android
fi
file target/aarch64-linux-android/release/qjs-spike
echo "OK. To test on host: qemu-aarch64-static target/aarch64-linux-android/release/qjs-spike (static build)"
echo "To test on device:  adb push + run, or build natively in Termux with ./build-termux.sh"
