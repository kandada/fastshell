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
| `fastshell_execute_in` | `nativeExecuteIn` |
| `fastshell_execute_python` | `nativeExecutePython` |
| `fastshell_execute_python_script` | `nativeExecutePythonScript` |
| `fastshell_get_cwd` | `nativeGetCwd` |
| `fastshell_set_permission` | `nativeSetPermission` |
| `fastshell_cancel_execution` | `nativeCancelExecution` |
| `fastshell_free_string` | —（C 层内部释放 Rust 字符串）|

### aacode-rs Agent API（句柄式，从 `libaacode_rs.a` 导出）

原生 Agent 通过**句柄式 C ABI**（见 `aacode-rs/src/ffi.rs`）嵌入，`jni_glue.c` 包装为：

| C 函数 | Kotlin `external fun` |
|--------|------------------------|
| `aacode_task_start` | `nativeAgentRunTaskWithCallback(json, cb): Long` — 非阻塞，返回句柄 |
| `aacode_task_wait` | `nativeAgentWaitTask(handle): String` — 阻塞取终态 JSON |
| `aacode_task_cancel` | `nativeAgentCancelTask(handle)` |
| `aacode_task_free` | `nativeAgentFreeTask(handle)` |
| `aacode_validate_api_key` | `nativeAgentValidateApiKey` |
| `aacode_list_sessions` | `nativeAgentListSessions` |
| `aacode_get_session_messages` | `nativeAgentGetSessionMessages` |
| `aacode_free_string` | —（内部）|

**流式回调**：`aacode_task_start` 接收 `(cb, userdata)`；`jni_glue.c` 为每个任务分配一个
`jni_cb_ctx`（Kotlin 回调的 GlobalRef + `onChunk` methodID）作为 userdata。Rust worker 线程
调用 `event_trampoline`，按线程重新获取 `JNIEnv`（`get_env`，一次性 attach）后把每条 JSONL
行转发给 `onChunk`。**没有全局回调槽、没有线程局部 hack**——每个任务自带上下文，并发任务
天然隔离。一个小型 handle→ctx 注册表（`g_agent_tasks`）在 `free` 时释放 GlobalRef。

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
            path = file("/Volumes/lenovops9/fastshell_local/fastshell_c/CMakeLists.txt")
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
    // ... aacode-rs agent（句柄式）：
    external fun nativeAgentRunTaskWithCallback(taskJson: String, callback: Any): Long
    external fun nativeAgentWaitTask(handle: Long): String
    external fun nativeAgentCancelTask(handle: Long)
    external fun nativeAgentFreeTask(handle: Long)
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
