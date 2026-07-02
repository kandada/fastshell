#!/bin/bash
# Copyright (c) 2025 xiefujin <490021684@qq.com>
# Licensed under Apache-2.0, see LICENSE file for full license terms.

# fastshell build environment setup
# Run once after cloning: ./scripts/setup.sh
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"   # fastshell/ directory
NDK_VERSION="r27c"

# Check multiple locations for NDK (workspace root, fastshell/, or fastshell/../)
NDK_DIR=""
for candidate in \
    "$PROJECT_ROOT/android-ndk-$NDK_VERSION" \
    "$PROJECT_ROOT/../android-ndk-$NDK_VERSION"; do
    if [ -d "$candidate" ]; then
        NDK_DIR="$candidate"
        break
    fi
done
if [ -z "$NDK_DIR" ]; then
    NDK_DIR="$PROJECT_ROOT/android-ndk-$NDK_VERSION"  # default install location
fi

echo "=== fastshell build environment setup ==="

# ── 1. Rust targets ──
echo "[1/4] Installing Rust targets..."
rustup target add aarch64-apple-darwin x86_64-apple-darwin 2>/dev/null || true
rustup target add aarch64-apple-ios 2>/dev/null || true
rustup target add aarch64-linux-android x86_64-unknown-linux-gnu 2>/dev/null || true
echo "  Rust targets ready."

# ── 2. Detect host OS ──
HOST_OS="$(uname -s)"
HOST_ARCH="$(uname -m)"
case "$HOST_OS" in
    Darwin) HOST_TRIPLE="darwin-x86_64" ;;
    Linux)  HOST_TRIPLE="linux-x86_64" ;;
    *)      HOST_TRIPLE="unknown" ;;
esac

# ── 3. Android NDK ──
echo "[2/4] Android NDK..."
if [ -d "$NDK_DIR" ]; then
    echo "  NDK already exists: $NDK_DIR"
else
    echo "  NDK not found."
    echo "  Download https://developer.android.com/ndk/downloads"
    echo "  Extract to: $NDK_DIR"
    echo ""
    echo "  Or skip — Android target only needs NDK."
    echo "  macOS / iOS / Linux targets build without it."
    echo ""
    read -p "  Download NDK $NDK_VERSION now? (~1GB) [y/N] " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        NDK_URL="https://dl.google.com/android/repository/android-ndk-${NDK_VERSION}-${HOST_OS,,}.zip"
        echo "  Downloading $NDK_URL ..."
        curl -L -o /tmp/android-ndk.zip "$NDK_URL"
        echo "  Extracting..."
        unzip -q /tmp/android-ndk.zip -d "$PROJECT_ROOT"
        rm /tmp/android-ndk.zip
        echo "  NDK installed."
    fi
fi

# ── 4. cargo-zigbuild (Linux cross-compile) ──
echo "[3/4] cargo-zigbuild..."
if command -v cargo-zigbuild &>/dev/null; then
    echo "  cargo-zigbuild already installed."
else
    echo "  cargo-zigbuild not found (needed for Linux x86_64 cross-compile on macOS)."
    echo "  Install: pip3 install cargo-zigbuild"
fi

# ── 5. Write .cargo/config.toml ──
echo "[4/4] Generating .cargo/config.toml..."
CARGO_CONFIG="$PROJECT_ROOT/.cargo/config.toml"
mkdir -p "$(dirname "$CARGO_CONFIG")"

if [ -d "$NDK_DIR" ] && [ "$HOST_TRIPLE" != "unknown" ]; then
    NDK_TOOLCHAIN="$NDK_DIR/toolchains/llvm/prebuilt/$HOST_TRIPLE/bin"
    cat > "$CARGO_CONFIG" << ENDCONFIG
[target.aarch64-linux-android]
linker = "$NDK_TOOLCHAIN/aarch64-linux-android21-clang"

[target.aarch64-apple-ios]
rustflags = ["-C", "link-arg=-target", "-C", "link-arg=arm64-apple-ios16.0"]

[env]
CC_aarch64_linux_android = "$NDK_TOOLCHAIN/aarch64-linux-android21-clang"
AR_aarch64_linux_android = "$NDK_TOOLCHAIN/llvm-ar"
IPHONEOS_DEPLOYMENT_TARGET = "16.0"
ENDCONFIG
    echo "  config.toml generated (Android + iOS)."
else
    cat > "$CARGO_CONFIG" << ENDCONFIG
[target.aarch64-apple-ios]
rustflags = ["-C", "link-arg=-target", "-C", "link-arg=arm64-apple-ios16.0"]

[env]
IPHONEOS_DEPLOYMENT_TARGET = "16.0"
ENDCONFIG
    echo "  config.toml generated (iOS only, no Android NDK detected)."
fi

echo ""
echo "=== Setup complete ==="
echo ""
echo "Build commands:"
echo "  cargo build --release --target aarch64-apple-darwin"
echo "  cargo build --release --target x86_64-apple-darwin"
if [ -d "$NDK_DIR" ]; then
    echo "  cargo build --release --target aarch64-linux-android"
fi
echo "  cargo build --release --target aarch64-apple-ios"
echo "  cargo zigbuild --release --target x86_64-unknown-linux-gnu"
echo ""
echo "Run tests: cargo test"
