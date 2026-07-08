#!/data/data/com.termux/files/usr/bin/bash
# Native on-device setup. Run inside Termux on the Android phone.
set -euo pipefail
pkg update -y && pkg upgrade -y
pkg install -y rust clang git make cmake binutils
echo "--- versions ---"
rustc --version
cargo --version
clang --version | head -1
echo "libclang:"; ls "$PREFIX"/lib/libclang.so* 2>/dev/null || find "$PREFIX" -name 'libclang*.so*'
echo "DONE. Termux is native aarch64-linux-android: 'cargo build' produces a runnable binary."
