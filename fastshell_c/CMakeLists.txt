# Copyright (c) 2025 xiefujin <490021684@qq.com>
# Licensed under Apache-2.0, see LICENSE file for full license terms.
#
# CMakeLists.txt — builds libfastshell_jni.so (Android integration 方案 B).
#
# Links the C JNI glue layer (src/jni_glue.c) against the prebuilt Rust
# staticlib (c_dist/${ANDROID_ABI}/libfastshell.a) using the NDK toolchain.
# The resulting .so has a standard NDK ELF layout, avoiding the Rust-cdylib
# compatibility issues on certain devices.
#
# Consumed from Android Studio via app/build.gradle.kts:
#   externalNativeBuild { cmake { path = file(".../fastshell_c/CMakeLists.txt") } }

cmake_minimum_required(VERSION 3.22)
project(fastshell_jni C)

# Root of this fastshell_local workspace (…/fastshell_c/..).
get_filename_component(FASTSHELL_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/.." ABSOLUTE)

# Location of the prebuilt Rust staticlib, per ABI.
# Override with -DFASTSHELL_ADIR=/path when integrating from another project.
# Default: c_dist/${ANDROID_ABI}/libaacode_rs.a — contains BOTH the fastshell
# sandbox engine AND the aacode-rs native agent C ABI symbols.
# Build it with:
#   cargo build --release --target aarch64-linux-android -p aacode-rs --lib
#   cp target/aarch64-linux-android/release/libaacode_rs.a c_dist/arm64-v8a/
if(NOT DEFINED FASTSHELL_ADIR)
    set(FASTSHELL_ADIR "${FASTSHELL_ROOT}/c_dist/${ANDROID_ABI}")
endif()

set(FASTSHELL_A "${FASTSHELL_ADIR}/libaacode_rs.a")

if(NOT EXISTS "${FASTSHELL_A}")
    message(FATAL_ERROR
        "libaacode_rs.a not found at ${FASTSHELL_A}\n"
        "Build it first:  cargo build --release --target aarch64-linux-android -p aacode-rs --lib && cp target/aarch64-linux-android/release/libaacode_rs.a c_dist/arm64-v8a/")
endif()

add_library(fastshell_static STATIC IMPORTED)
set_target_properties(fastshell_static PROPERTIES IMPORTED_LOCATION "${FASTSHELL_A}")

# Static libffi (rustpython-vm ctypes support; the NDK ships no libffi).
# Built from vanilla libffi 3.4.6 (MIT) — see fastshell/vendor/README.md.
add_library(ffi_static STATIC IMPORTED)
set_target_properties(ffi_static PROPERTIES IMPORTED_LOCATION
    "${FASTSHELL_ROOT}/fastshell/vendor/libffi/${ANDROID_ABI}-placeholder-unused/libffi.a")
if(ANDROID_ABI STREQUAL "arm64-v8a")
    set_target_properties(ffi_static PROPERTIES IMPORTED_LOCATION
        "${FASTSHELL_ROOT}/fastshell/vendor/libffi/aarch64-linux-android/libffi.a")
endif()

add_library(fastshell_jni SHARED src/jni_glue.c)

target_include_directories(fastshell_jni PRIVATE "${CMAKE_CURRENT_SOURCE_DIR}/include")

# find_library locates NDK system libs (log, dl, m are part of libc/liblog).
find_library(log-lib log)

target_link_libraries(fastshell_jni
    fastshell_static
    ffi_static  # libffi: rustpython ctypes (must come after the staticlib)
    ${log-lib}
    dl
    m
    z   # zlib: required by libgit2 (fastshell "git" feature)
)

# Strip unused symbols from the large staticlib.
# Google Play targetSdk 35+ requires all LOAD segments p_align >= 16384.
target_link_options(fastshell_jni PRIVATE
    "-Wl,--gc-sections"
    "-Wl,--exclude-libs,ALL"
    "-Wl,-z,max-page-size=16384"
)
