# fastshell_c — Android & iOS 集成（方案 B：静态库 + NDK CMake）

`fastshell` 的 Rust 代码编译为**静态库（`libfastshell.a`）**，仅导出**纯 C ABI**。
由一层 C 桥接（`jni_glue.c`）实现 `Java_com_fastshell_Sdk_native*` JNI 入口，
再用 **NDK clang + CMake** 链接成 `libfastshell_jni.so`——产物是标准 NDK 格式的 ELF，
规避了 Rust-cdylib 在部分设备上的 JNI trampoline 兼容性问题。

iOS 上，`libfastshell.a` 通过 `fastshell.h` 中相同的 C ABI 直接链接到 Xcode 项目。

**Python 由内嵌 [RustPython](https://github.com/RustPython/RustPython)（MIT）提供**——
纯 Rust，直接编译进静态库（feature `python-rustpython`，冻结标准库）。
没有 `libpython*.so`、没有资产、没有 dlopen。（旧的 Chaquopy CPython 集成因真机
崩溃已移除，见 `fastshell/vendor/README.zh.md`。）

> `fastshell_c` 是**独立目录**（非 Cargo workspace 成员）。

---

## 快速开始

```bash
# 1. 编译 Rust 静态库 + JNI .so（内含 RustPython + git2）
cd fastshell_c && ./build.sh --so

# 2. 或一步编译并部署 .a 到 App 工程
./build.sh --deploy
```

---

## 目录结构

```
fastshell_c/
├── include/fastshell.h   # Rust 导出的纯 C ABI 声明（Android/iOS/Desktop 统一）
├── src/jni_glue.c        # JNI ↔ C ABI 桥接 + 流式回调 trampoline
├── CMakeLists.txt        # Android Studio 用；链接 c_dist/ 中的 .a
├── build.sh              # 编译 .a → c_dist/，可选 --so / --deploy
├── README.md             # English documentation
└── README.zh.md          # 中文文档
c_dist/arm64-v8a/
└── libfastshell.a        # 由 build.sh 产出，供 CMake 链接
```

---

## C ABI 映射

| C 函数 (`fastshell.h`) | Kotlin `external fun` |
|--------------------------|------------------------|
| `fastshell_init` | `nativeInit` |
| `fastshell_execute` | `nativeExecute` |
| `fastshell_execute_python` | `nativeExecutePython` |
| `fastshell_execute_python_script` | `nativeExecutePythonScript` |
| `fastshell_get_cwd` | `nativeGetCwd` |
| `fastshell_set_permission` | `nativeSetPermission` |
| `fastshell_cancel_execution` | `nativeCancelExecution` |
| `fastshell_register_stream_callback` | `nativeRegisterStreamCallback`（回调 `onChunk`）|
| `fastshell_free_string` | —（C 层内部释放 Rust 字符串）|

流式回调：C 层在 `JNI_OnLoad` 缓存 `JavaVM`，注册时缓存回调对象 GlobalRef + `onChunk` methodID，
向 Rust 传入 `stream_trampoline`；每个 chunk 到来时按需 `AttachCurrentThread` 后回调 Java。

---

## 构建

```bash
# 编译静态库并放入 c_dist/
./fastshell_c/build.sh

# 用 NDK 本地验证能链接出 .so
./fastshell_c/build.sh --so

# 部署到 AACodeApp
./fastshell_c/build.sh --deploy
```

---

## App 侧接入

**`app/build.gradle.kts`：**
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

**`com.fastshell.Sdk.kt`：**
```kotlin
object Sdk {
    init { System.loadLibrary("fastshell_jni") }
    fun initCore() {
    }
    external fun nativeInit(sandboxPath: String): String
    external fun nativeExecute(command: String): String
    external fun nativeExecutePython(code: String): String
    // ...
}
```

**`jniLibs/arm64-v8a/`** 只需 `libaacode_rs.a`（CMake 构建期使用）；运行时唯一的 `.so` 是 CMake 产出的 `libfastshell_jni.so`。

---

## 与方案 A 的关系

- **方案 B（本目录）**：默认构建，无 `Java_*` 符号在 Rust 侧，`jni` crate 不参与编译
- **方案 A（旧 cdylib）**：`cargo build --features jni_direct` 仍可用，但不推荐

---

## 许可

Apache 2.0。内嵌 RustPython 为 MIT，其冻结标准库为 PSF-2.0；Android 附带静态 libffi（MIT）。RustPython 的 LGPL malachite 依赖已由净室 Apache-2.0 替身替换，见 `fastshell/num_bigint/README.zh.md`。
