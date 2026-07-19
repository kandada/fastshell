#!/bin/bash
# ═══════════════════════════════════════════════════════════
# fastshell_c 构建脚本 (Android 集成方案 B: staticlib + NDK CMake)
#
# 步骤:
#   1. cargo 编译 Rust 静态库 libfastshell.a (aarch64-linux-android)
#   2. 拷贝到 c_dist/arm64-v8a/libfastshell.a  (供 CMake 链接)
#   3. (可选) 用 NDK 直接编译 libfastshell_jni.so 做本地验证
#
# 用法:
#   ./fastshell_c/build.sh              # 编译 .a 并放入 c_dist/
#   ./fastshell_c/build.sh --so         # 额外用 NDK 编译 .so 验证
#   ./fastshell_c/build.sh --deploy     # 额外部署 .a + 头文件到 AACodeApp
#
# 说明: 正常集成时无需 --so，Android Studio 的 CMake 会自动编译 .so。
# ═══════════════════════════════════════════════════════════

set -euo pipefail
cd "$(dirname "$0")/.."

TARGET="aarch64-linux-android"
ABI="arm64-v8a"
NDK_DIR="android-ndk-r27c/toolchains/llvm/prebuilt/darwin-x86_64/bin"
API=26
CDIST="c_dist/${ABI}"
AACODE_DIR="${AACODE_DIR:-../../AndroidStudioProjects/AACodeApp}"

# openssl-src (fastshell "git" feature) invokes aarch64-linux-android-ranlib
# during `make install`; make sure the NDK toolchain is on PATH.
export PATH="$(pwd)/${NDK_DIR}:${PATH}"

echo "=== [1/2] cargo build staticlib (${TARGET}) ==="
# Compile aacode-rs which INCLUDES fastshell as a dependency. This produces a
# single staticlib containing both fastshell's sandbox engine AND aacode-rs's
# native agent C ABI symbols (aacode_run_task, aacode_cancel, ...).
cargo build --release --target "${TARGET}" --lib -p aacode-rs

STATICLIB="target/${TARGET}/release/libaacode_rs.a"
mkdir -p "${CDIST}"
cp "${STATICLIB}" "${CDIST}/libaacode_rs.a"
echo "  → ${CDIST}/libaacode_rs.a ($(ls -lh "${CDIST}/libaacode_rs.a" | awk '{print $5}'))"

if [ "${1:-}" = "--so" ]; then
    echo "=== [2/2] NDK clang: compile + link libfastshell_jni.so (verify) ==="
    CC="${NDK_DIR}/aarch64-linux-android${API}-clang"
    OUT="c_dist/${ABI}/libfastshell_jni.so"
    # Must mirror fastshell_c/CMakeLists.txt: zlib (libgit2) + static libffi
    # (rustpython ctypes). --no-undefined makes this a REAL link check.
    "${CC}" -shared -fPIC \
        -I fastshell_c/include \
        fastshell_c/src/jni_glue.c \
        "${CDIST}/libaacode_rs.a" \
        fastshell/vendor/libffi/aarch64-linux-android/libffi.a \
        -Wl,--gc-sections -Wl,--exclude-libs,ALL -Wl,--no-undefined \
        -Wl,-z,max-page-size=16384 \
        -llog -ldl -lm -lz \
        -o "${OUT}"
    "${NDK_DIR}/llvm-strip" --strip-unneeded "${OUT}"
    echo "  → ${OUT} ($(ls -lh "${OUT}" | awk '{print $5}'))"
fi

if [ "${1:-}" = "--deploy" ]; then
    echo "=== [2/2] deploy to AACodeApp ==="
    DEST="${AACODE_DIR}/app/src/main/jniLibs/${ABI}"
    mkdir -p "${DEST}"
    cp "${CDIST}/libaacode_rs.a" "${DEST}/libaacode_rs.a"
    echo "  → ${DEST}/libaacode_rs.a"
    echo "  提示: 在 app/build.gradle.kts 里配置 externalNativeBuild 指向 fastshell_c/CMakeLists.txt"
fi

echo "Done."
