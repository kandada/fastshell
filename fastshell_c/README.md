# fastshell_c — Android & iOS Integration (Plan B: staticlib + NDK CMake)

`fastshell` Rust code compiled as **static library (`libfastshell.a`)** exporting **pure C ABI**.
A thin C bridge (`jni_glue.c`) implements `Java_com_fastshell_Sdk_native*` JNI entry points,
linked by **NDK clang + CMake** into `libfastshell_jni.so` — a standard NDK-format ELF
that avoids Rust-cdylib JNI trampoline issues on certain devices.

For iOS, `libfastshell.a` links directly into Xcode via the same C ABI in `fastshell.h`.

**Python is provided by embedded [RustPython](https://github.com/RustPython/RustPython) (MIT)** —
pure Rust, compiled straight into the staticlib (feature `python-rustpython`, frozen stdlib).
No `libpython*.so`, no assets, no dlopen. (The former Chaquopy CPython integration was
removed after on-device crashes; see `fastshell/vendor/README.md`.)

> `fastshell_c` is an **independent directory** (not a Cargo workspace member).

---

## Quick Start

```bash
# 1. Build Rust staticlib + JNI .so (includes RustPython + git2)
cd fastshell_c && ./build.sh --so

# 2. Or build + deploy the .a straight into the app project
./build.sh --deploy
```

---

## Directory Structure

```
fastshell_c/
├── include/fastshell.h   # Unified C ABI (Android / iOS / Desktop)
├── src/jni_glue.c        # JNI ↔ C ABI bridge + streaming trampoline
├── CMakeLists.txt        # Android Studio CMake integration
├── build.sh              # Build .a → c_dist/ (--so for verification)
├── README.md             # English documentation
└── README.zh.md          # Chinese documentation (中文文档)
c_dist/arm64-v8a/
└── libfastshell.a        # Built by build.sh, consumed by CMake
```

---

## C ABI Mapping

| C Function (`fastshell.h`) | Kotlin `external fun` |
|----------------------------|------------------------|
| `fastshell_init` | `nativeInit` |
| `fastshell_execute` | `nativeExecute` |
| `fastshell_execute_python` | `nativeExecutePython` |
| `fastshell_execute_python_script` | `nativeExecutePythonScript` |
| `fastshell_get_cwd` | `nativeGetCwd` |
| `fastshell_set_permission` | `nativeSetPermission` |
| `fastshell_cancel_execution` | `nativeCancelExecution` |
| `fastshell_register_stream_callback` | `nativeRegisterStreamCallback` (calls `onChunk`) |
| `fastshell_free_string` | — (internal, frees Rust strings) |

**Streaming callback**: `JNI_OnLoad` caches `JavaVM`. On registration, caches callback
GlobalRef + `onChunk` methodID, passes `stream_trampoline` to Rust. Each chunk triggers
`AttachCurrentThread` + JNI callback to Java.

---

## Build

```bash
# Build static library into c_dist/
./fastshell_c/build.sh

# Verify JNI .so links correctly (requires NDK)
./fastshell_c/build.sh --so

# Deploy to AACodeApp
./fastshell_c/build.sh --deploy
```

---

## App Integration

**`app/build.gradle.kts`:**
```kotlin
android {
    externalNativeBuild {
        cmake {
            path = file("../../RustroverProjects/fastshell_local/fastshell_c/CMakeLists.txt")
            version = "3.22.1"
        }
    }
    packaging { jniLibs { useLegacyPackaging = true } }
}
```

**`com.fastshell.Sdk.kt`:**
```kotlin
object Sdk {
    init { System.loadLibrary("fastshell_jni") }  // everything is inside this one .so
    external fun nativeInit(sandboxPath: String): String
    external fun nativeExecute(command: String): String
    external fun nativeExecuteIn(dir: String, command: String): String
    external fun nativeExecutePython(code: String): String
    // ... agent entry points: nativeAgentRunTaskWithCallback / nativeAgentCancelTask / ...
}
```

**`jniLibs/arm64-v8a/`** only needs `libaacode_rs.a` (consumed by CMake at build
time); no runtime `.so` besides the CMake-produced `libfastshell_jni.so`.

---

## Plan A vs Plan B

- **Plan B (this directory)**: Default build, no `Java_*` symbols on Rust side, no `jni` crate.
- **Plan A (old cdylib)**: `cargo build --features jni_direct` — legacy, not recommended.

---

## License

Apache 2.0. Embedded RustPython is MIT; its frozen stdlib is PSF-2.0; a static
libffi (MIT) is bundled for Android. RustPython's LGPL malachite dependencies
are replaced by clean-room Apache-2.0 shims — see `fastshell/num_bigint/README.md`.
