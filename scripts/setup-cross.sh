#!/usr/bin/env bash
# Reproduce the exact cross-compile environment this kit was verified with.
# Target: linux x86_64 host (Amazon Linux 2023 / Fedora). Adjust pkg mgr for Debian/Ubuntu.
set -euo pipefail

echo "=== 1. Rust stable + aarch64 android target ==="
if ! command -v rustc >/dev/null; then
  curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable
fi
source "$HOME/.cargo/env"
rustup target add aarch64-linux-android
rustup target add armv7-linux-androideabi x86_64-linux-android || true
rustc --version

echo "=== 2. libclang (for bindgen) ==="
if command -v dnf >/dev/null; then sudo dnf install -y clang clang-libs llvm-libs file unzip
elif command -v apt-get >/dev/null; then sudo apt-get update && sudo apt-get install -y clang libclang-dev llvm file unzip
fi

echo "=== 3. Android NDK r27c ==="
cd "$HOME"
if [ ! -d "$HOME/android-ndk-r27c" ]; then
  curl -sSL -o ndk.zip https://dl.google.com/android/repository/android-ndk-r27c-linux.zip
  unzip -q ndk.zip
fi
cat "$HOME/android-ndk-r27c/source.properties"

echo "=== 4. qemu-aarch64-static (to run aarch64 binaries on x86_64) ==="
if [ ! -x "$HOME/qemu-aarch64-static" ]; then
  curl -sSL -o "$HOME/qemu-aarch64-static" \
    https://github.com/multiarch/qemu-user-static/releases/download/v7.2.0-1/qemu-aarch64-static
  chmod +x "$HOME/qemu-aarch64-static"
fi
"$HOME/qemu-aarch64-static" --version | head -1

echo "=== DONE. Now: export the NDK env block (see docs/GUIDE.md 1B) and run spike/build-cross.sh ==="
