/*
 * Copyright (c) 2025 xiefujin <490021684@qq.com>
 * Licensed under Apache-2.0, see LICENSE file for full license terms.
 *
 * fastshell.h — Pure C ABI exported by libfastshell.a (Rust staticlib).
 *
 * One header. Three platforms. Same C ABI everywhere.
 *
 * ── Platform integration paths ──
 *
 *   ANDROID (方案 B: staticlib + NDK CMake → libfastshell_jni.so)
 *     Rust → libfastshell.a (extern "C" capi symbols)
 *           ↓ NDK clang + CMake
 *     jni_glue.c → Java_com_fastshell_Sdk_native* → libfastshell_jni.so
 *           ↓ System.loadLibrary("fastshell_jni")
 *     Kotlin  (com.fastshell.Sdk)
 *
 *   iOS (static linking → app binary)
 *     Rust → libfastshell.a (extern "C" capi symbols)
 *           ↓ Xcode "Link Binary with Libraries"
 *     Swift/ObjC → direct C function calls (e.g. fastshell_init, fastshell_execute)
 *
 *   DESKTOP (macOS / Linux, development & testing)
 *     Rust → libfastshell.{dylib,so} (cdylib)
 *           ↓ dlopen / link at build time
 *     C host → direct C function calls
 *
 * ── Memory ownership ──
 *   Functions returning `char *` allocate a NUL-terminated UTF-8 string on
 *   the Rust heap. The caller MUST release it with fastshell_free_string().
 *   Result strings (execute / python) are JSON:
 *       {"stdout": "...", "stderr": "...", "exit_code": <int>}
 *
 * ── Thread safety ──
 *   All functions are safe to call from any thread. The SDK uses an internal
 *   Mutex to serialize access to the shell and Python engine.
 *
 * ── Internal symbols (not part of the public API) ──
 *   fastshell_python_shell_exec()   — Python-to-shell bridge (called via ctypes)
 *   fastshell_python_shell_free()   — release memory returned by _shell_exec
 *   fastshell_python_stream_write() — streaming output from Python to host app
 *   These are exported so CPython's ctypes.CDLL(None) can resolve them at
 *   runtime. External consumers should NOT call them directly.
 */

#ifndef FASTSHELL_H
#define FASTSHELL_H

#ifdef __cplusplus
extern "C" {
#endif

/* ── Lifecycle ─────────────────────────────────────────── */

/*
 * Initializes the SDK with a sandbox directory.
 * Must be called once before any other function.
 * Returns JSON {"stdout":"","stderr":<err or "">,"exit_code":0|1}.
 *
 * On Android, this triggers extraction of the embedded CPython .so.gz to the
 * sandbox and calls Py_Initialize().
 * On iOS, CPython must be statically linked into the app binary.
 */
char *fastshell_init(const char *sandbox_path);

/*
 * Shuts down the SDK, releasing resources.
 * Call before app termination.
 * (Not yet exposed via C ABI — use atexit or process exit.)
 */

/* ── Shell execution ───────────────────────────────────── */

/* Executes a shell command. Returns JSON CommandResult. */
char *fastshell_execute(const char *command);

/*
 * Executes `command` with `dir` as the working directory (restored after the
 * call). Concurrent-host safe: no `cd X && ...` prefix needed and the shared
 * cwd is never left polluted. Returns malloc'd JSON like fastshell_execute.
 */
char *fastshell_execute_in(const char *dir, const char *command);

/* Executes inline Python code. Returns JSON CommandResult. */
char *fastshell_execute_python(const char *code);

/* Executes a Python script by path. Returns JSON CommandResult. */
char *fastshell_execute_python_script(const char *script_path);

/* Returns the current working directory (plain string, not JSON). */
char *fastshell_get_cwd(void);

/* ── Permissions ────────────────────────────────────────── */

/*
 * Grants (allowed != 0) or denies a permission for a resource key.
 * Resource keys:
 *   "network:<host>"    — network access to a specific host
 *   "camera"            — camera access
 *   "location"          — location access
 *   "microphone"        — microphone access
 *   "contacts"          — contacts access
 *   ...
 * On Android, the host app should map these to Android permissions.
 * On iOS, the host app should map these to Info.plist usage descriptions.
 */
void fastshell_set_permission(const char *resource, unsigned char allowed);

/* ── Execution control ──────────────────────────────────── */

/* Requests cancellation of the currently running command. */
void fastshell_cancel_execution(void);

/* ── Streaming output ───────────────────────────────────── */

/*
 * Stream callback type: invoked for each output chunk during streaming
 * Python execution. The `chunk` pointer is only valid for the duration
 * of the call; copy it if you need to retain it.
 */
typedef void (*fastshell_stream_callback)(const char *chunk);

/*
 * Registers a stream callback, or clears it when `cb` is NULL.
 * On Android, jni_glue.c forwards each chunk to Kotlin via JNI.
 * On iOS, the host app should implement a similar trampoline.
 */
void fastshell_register_stream_callback(fastshell_stream_callback cb);

/* ── Agent Server ──────────────────────────────────────── */

/* Starts the Python agent server in a background thread.
 * The server polls for task files in {sandbox}/tmp/tasks/.
 * Returns JSON {"ok":true} or {"ok":false,"error":"..."}. */
char *fastshell_start_agent_server(void);

/* Submits a task to the agent server. Non-blocking.
 * task_id: short unique id (used for output file naming)
 * task_json: JSON with keys "task", "project_path", "session_id"
 * Returns JSON {"ok":true} or {"ok":false,"error":"..."}. */
char *fastshell_submit_task(const char *task_id, const char *task_json);

/* ── Inspection ─────────────────────────────────────────── */

/* Returns a JSON string describing the shell features and capabilities
 * supported by this build. The caller must free with fastshell_free_string(). */
char *fastshell_get_features(void);

/* ── Memory ─────────────────────────────────────────────── */

/* Frees a string previously returned by any fastshell_* function. */
void fastshell_free_string(char *ptr);

/* ── Platform-specific notes ───────────────────────────── */

/*
 * ANDROID INTEGRATION
 *   See fastshell_c/README.md for full instructions.
 *   1. fastshell_c/build.sh → builds libfastshell.a into c_dist/
 *   2. CMakeLists.txt links jni_glue.c + libfastshell.a → libfastshell_jni.so
 *   3. Kotlin: System.loadLibrary("fastshell_jni") → Sdk.kt
 *   4. Kotlin: System.loadLibrary("python3.12") before nativeInit()
 *   5. app/build.gradle.kts: externalNativeBuild { cmake { path "...fastshell_c/CMakeLists.txt" } }
 *
 * IOS INTEGRATION
 *   1. Build libfastshell.a for aarch64-apple-ios
 *      cargo build --release --target aarch64-apple-ios -p fastshell
 *   2. Add libfastshell.a to Xcode "Link Binary with Libraries"
 *   3. Add fastshell.h to Xcode project
 *   4. Call fastshell_init(fastshell_execute(...), etc. directly from Swift/ObjC
 *   5. Python: embedded RustPython is inside the staticlib already (no libpython)
 *      See scripts/build_cpython.sh ios-arm64 for CPython build instructions.
 */

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* FASTSHELL_H */
