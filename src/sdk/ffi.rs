// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::sdk::Fastshell;
use std::sync::{Mutex, OnceLock};

static SDK_INSTANCE: OnceLock<Mutex<Fastshell>> = OnceLock::new();

/// Returns a reference to the shared SDK instance if it has already been
/// initialized (e.g. by the host app via fastshell_init()). Returns None
/// if no SDK instance has been created yet.
///
/// This is the integration point for aacode-rs on mobile: the agent reuses
/// the App's fastshell instead of creating a second instance (which would
/// conflict on CPython initialization and VFS root paths).
pub fn try_get_sdk_instance() -> Option<&'static Mutex<Fastshell>> {
    SDK_INSTANCE.get()
}

fn get_sdk() -> &'static Mutex<Fastshell> {
    SDK_INSTANCE.get_or_init(|| Mutex::new(Fastshell::new()))
}

// Used by the Android jni_direct module below; dead on other targets.
#[cfg_attr(not(all(target_os = "android", feature = "jni_direct")), allow(dead_code))]
pub(crate) fn get_sdk_internal() -> &'static Mutex<Fastshell> {
    SDK_INSTANCE.get_or_init(|| Mutex::new(Fastshell::new()))
}

fn result_to_json(result: &crate::sdk::types::CommandResult) -> String {
    serde_json::json!({
        "stdout": result.stdout,
        "stderr": result.stderr,
        "exit_code": result.exit_code,
    }).to_string()
}

fn error_to_json(msg: &str, code: i32) -> String {
    serde_json::json!({
        "stdout": "",
        "stderr": msg,
        "exit_code": code,
    }).to_string()
}

#[no_mangle]
// Standard C-ABI free contract: the pointer must come from a fastshell_*
// function (documented for hosts); marking it `unsafe` would not change the
// C caller's obligations.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn fastshell_free_string(ptr: *mut std::os::raw::c_char) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let _ = std::ffi::CString::from_raw(ptr);
    }
}

#[cfg(all(target_os = "android", feature = "jni_direct"))]
pub mod android {
    use super::*;
    use jni::JNIEnv;
    use jni::objects::{JClass, JObject, JString};
    use jni::sys::jstring;
    use std::sync::OnceLock;

    #[no_mangle]
    pub extern "system" fn Java_com_fastshell_Sdk_nativeInit<'local>(
        mut env: JNIEnv<'local>,
        _class: JClass<'local>,
        sandbox_path: JString<'local>,
    ) -> jstring {
        let sandbox_path: String = match env.get_string(&sandbox_path) {
            Ok(s) => s.into(),
            Err(_) => {
                let json = error_to_json("Invalid path argument", 1);
                return env.new_string(json).unwrap().into_raw();
            }
        };

        let mut sdk = get_sdk().lock().unwrap_or_else(|e| e.into_inner());
        let config = crate::sdk::types::Config {
            sandbox_path: sandbox_path.clone(),
            python_enabled: true,
            python_home: format!("{}/python", sandbox_path),
            allow_subprocess: false,
            network_ask_permission: true,
            command_timeout_ms: 300_000,
        };

        let json = match sdk.init(config) {
            Ok(()) => error_to_json("", 0),
            Err(e) => error_to_json(&e, 1),
        };
        env.new_string(json).unwrap().into_raw()
    }

    #[no_mangle]
    pub extern "system" fn Java_com_fastshell_Sdk_nativeExecute<'local>(
        mut env: JNIEnv<'local>,
        _class: JClass<'local>,
        command: JString<'local>,
    ) -> jstring {
        let command: String = match env.get_string(&command) {
            Ok(s) => s.into(),
            Err(_) => {
                let json = error_to_json("Invalid command argument", 1);
                return env.new_string(json).unwrap().into_raw();
            }
        };

        let sdk = get_sdk().lock().unwrap_or_else(|e| e.into_inner());
        let result = sdk.execute(&command);
        let json = result_to_json(&result);
        env.new_string(json).unwrap().into_raw()
    }

    #[no_mangle]
    pub extern "system" fn Java_com_fastshell_Sdk_nativeGetCwd<'local>(
        env: JNIEnv<'local>,
        _class: JClass<'local>,
    ) -> jstring {
        let sdk = get_sdk().lock().unwrap_or_else(|e| e.into_inner());
        let cwd = sdk.get_cwd();
        env.new_string(cwd).unwrap().into_raw()
    }

    #[no_mangle]
    pub extern "system" fn Java_com_fastshell_Sdk_nativeSetPermission<'local>(
        mut env: JNIEnv<'local>,
        _class: JClass<'local>,
        resource: JString<'local>,
        allowed: jni::sys::jboolean,
    ) {
        let resource: String = match env.get_string(&resource) {
            Ok(s) => s.into(),
            Err(_) => return,
        };
        let sdk = get_sdk().lock().unwrap_or_else(|e| e.into_inner());
        sdk.set_permission(&resource, allowed != 0);
    }

    #[no_mangle]
    pub extern "system" fn Java_com_fastshell_Sdk_nativeCancelExecution<'local>(
        _env: JNIEnv<'local>,
        _class: JClass<'local>,
    ) {
        let sdk = get_sdk().lock().unwrap_or_else(|e| e.into_inner());
        sdk.cancel_execution();
    }

    #[no_mangle]
    pub extern "system" fn Java_com_fastshell_Sdk_nativeRegisterStreamCallback<'local>(
        env: JNIEnv<'local>,
        _class: JClass<'local>,
        callback: JObject<'local>,
    ) {
        if callback.is_null() {
            crate::python::clear_stream_callback();
            return;
        }
        let vm = match env.get_java_vm() {
            Ok(vm) => vm,
            Err(_) => return,
        };
        let global_ref = match env.new_global_ref(callback) {
            Ok(r) => r,
            Err(_) => return,
        };

        static VM: OnceLock<jni::JavaVM> = OnceLock::new();
        static CB: OnceLock<jni::objects::GlobalRef> = OnceLock::new();

        if VM.set(vm).is_err() || CB.set(global_ref).is_err() {
            return;
        }

        crate::python::register_stream_callback(Box::new(move |chunk: &str| {
            let vm = match VM.get() {
                Some(v) => v,
                None => return,
            };
            let global_ref = match CB.get() {
                Some(r) => r,
                None => return,
            };
            let mut env = match vm.attach_current_thread() {
                Ok(e) => e,
                Err(_) => return,
            };
            let jstr = match env.new_string(chunk) {
                Ok(s) => s,
                Err(_) => return,
            };
            let _ = env.call_method(
                &global_ref,
                "onChunk",
                "(Ljava/lang/String;)V",
                &[jni::objects::JValue::Object(&jstr)],
            );
        }));
    }

    #[no_mangle]
    pub extern "system" fn Java_com_fastshell_Sdk_nativeExecutePython<'local>(
        mut env: JNIEnv<'local>,
        _class: JClass<'local>,
        code: JString<'local>,
    ) -> jstring {
        let code: String = match env.get_string(&code) {
            Ok(s) => s.into(),
            Err(_) => {
                let json = error_to_json("Invalid code argument", 1);
                return env.new_string(json).unwrap().into_raw();
            }
        };
        let sdk = get_sdk().lock().unwrap_or_else(|e| e.into_inner());
        let result = sdk.execute_python(&code);
        let json = result_to_json(&result);
        env.new_string(json).unwrap().into_raw()
    }

    #[no_mangle]
    pub extern "system" fn Java_com_fastshell_Sdk_nativeExecutePythonScript<'local>(
        mut env: JNIEnv<'local>,
        _class: JClass<'local>,
        script_path: JString<'local>,
    ) -> jstring {
        let script_path: String = match env.get_string(&script_path) {
            Ok(s) => s.into(),
            Err(_) => {
                let json = error_to_json("Invalid script path", 1);
                return env.new_string(json).unwrap().into_raw();
            }
        };
        let sdk = get_sdk().lock().unwrap_or_else(|e| e.into_inner());
        let result = sdk.execute_python_script(&script_path);
        let json = result_to_json(&result);
        env.new_string(json).unwrap().into_raw()
    }
}

#[cfg(target_os = "ios")]
pub mod ios {
    use super::*;

    #[no_mangle]
    pub extern "C" fn fastshell_ios_init(
        sandbox_path: *const std::os::raw::c_char,
    ) -> *const std::os::raw::c_char {
        let sandbox_path = unsafe {
            std::ffi::CStr::from_ptr(sandbox_path)
                .to_string_lossy()
                .to_string()
        };

        let mut sdk = get_sdk().lock().unwrap_or_else(|e| e.into_inner());
        let config = crate::sdk::types::Config {
            sandbox_path: sandbox_path.clone(),
            python_enabled: true,
            python_home: format!("{}/python", sandbox_path),
            allow_subprocess: false,
            network_ask_permission: true,
            command_timeout_ms: 30_000,
        };

        match sdk.init(config) {
            Ok(()) => std::ffi::CString::new(error_to_json("", 0)).unwrap().into_raw(),
            Err(e) => std::ffi::CString::new(error_to_json(&e, 1)).unwrap().into_raw(),
        }
    }

    #[no_mangle]
    pub extern "C" fn fastshell_ios_execute(
        command: *const std::os::raw::c_char,
    ) -> *const std::os::raw::c_char {
        let command = unsafe {
            std::ffi::CStr::from_ptr(command)
                .to_string_lossy()
                .to_string()
        };

        let sdk = get_sdk().lock().unwrap_or_else(|e| e.into_inner());
        let result = sdk.execute(&command);
        std::ffi::CString::new(result_to_json(&result)).unwrap().into_raw()
    }

    #[no_mangle]
    pub extern "C" fn fastshell_ios_set_permission(
        resource: *const std::os::raw::c_char,
        allowed: u8,
    ) {
        let resource = unsafe {
            std::ffi::CStr::from_ptr(resource)
                .to_string_lossy()
                .to_string()
        };
        let sdk = get_sdk().lock().unwrap_or_else(|e| e.into_inner());
        sdk.set_permission(&resource, allowed != 0);
    }

    #[no_mangle]
    pub extern "C" fn fastshell_ios_cancel_execution() {
        let sdk = get_sdk().lock().unwrap_or_else(|e| e.into_inner());
        sdk.cancel_execution();
    }
}

// ═══════════════════════════════════════════════════════════
// Pure C ABI — compiled on ALL platforms.
//
// This is the entry point used by the NDK CMake bridge (fastshell_c,
// 方案 B): `libfastshell.a` exposes these `extern "C"` symbols, and a
// C JNI glue layer (jni_glue.c) forwards Java_* JNI calls to them. It
// is also usable by desktop/iOS C hosts.
//
// All functions returning `*mut c_char` allocate a NUL-terminated
// UTF-8 string that MUST be released with `fastshell_free_string`.
// ═══════════════════════════════════════════════════════════
pub mod capi {
    use super::*;
    use std::os::raw::c_char;

    fn cstr_to_string(ptr: *const c_char) -> Option<String> {
        if ptr.is_null() {
            return None;
        }
        Some(
            unsafe { std::ffi::CStr::from_ptr(ptr) }
                .to_string_lossy()
                .to_string(),
        )
    }

    fn into_c_string(s: String) -> *mut c_char {
        std::ffi::CString::new(s)
            .unwrap_or_else(|_| std::ffi::CString::new("").unwrap())
            .into_raw()
    }

    #[no_mangle]
    pub extern "C" fn fastshell_init(sandbox_path: *const c_char) -> *mut c_char {
        let sandbox_path = match cstr_to_string(sandbox_path) {
            Some(s) => s,
            None => return into_c_string(error_to_json("Invalid path argument", 1)),
        };

        let mut sdk = get_sdk().lock().unwrap_or_else(|e| e.into_inner());
        let config = crate::sdk::types::Config {
            sandbox_path: sandbox_path.clone(),
            python_enabled: std::env::var("FASTSHELL_DISABLE_PYTHON").is_err(),
            python_home: format!("{}/python", sandbox_path),
            allow_subprocess: false,
            network_ask_permission: true,
            command_timeout_ms: 300_000,
        };

        let json = match sdk.init(config) {
            Ok(()) => error_to_json("", 0),
            Err(e) => error_to_json(&e, 1),
        };
        into_c_string(json)
    }

    /// Registers (or clears with NULL) the host device-capability callback.
    /// The callback receives `(method, args_json)` and returns a malloc'd
    /// JSON string; fastshell frees it with `free()`. Call AFTER
    /// `fastshell_init` — the shell's device commands (`camera`, `record`,
    /// `location`, `sensor`, …) become functional once registered.
    #[no_mangle]
    pub extern "C" fn fastshell_register_device_callback(
        cb: Option<crate::sdk::device_callback::DeviceCallbackFn>,
    ) {
        // Store globally so per-task fastshell instances (aacode-rs) inherit it.
        crate::sdk::device_callback::set_global_device_callback(cb);
        // Also install into the current global SDK instance if already init'd.
        let sdk = get_sdk().lock().unwrap_or_else(|e| e.into_inner());
        match cb {
            Some(f) => sdk.register_plugin(Box::new(
                crate::sdk::device_callback::CallbackDevicePlugin::new(f),
            )),
            None => {
                if let Ok(mut p) = sdk.plugin_ref.lock() {
                    *p = None;
                }
            }
        }
    }

    #[no_mangle]
    pub extern "C" fn fastshell_execute(command: *const c_char) -> *mut c_char {
        let command = match cstr_to_string(command) {
            Some(s) => s,
            None => return into_c_string(error_to_json("Invalid command argument", 1)),
        };
        let sdk = get_sdk().lock().unwrap_or_else(|e| e.into_inner());
        let result = sdk.execute(&command);
        into_c_string(result_to_json(&result))
    }

    #[no_mangle]
    pub extern "C" fn fastshell_execute_python(code: *const c_char) -> *mut c_char {
        let code = match cstr_to_string(code) {
            Some(s) => s,
            None => return into_c_string(error_to_json("Invalid code argument", 1)),
        };
        let sdk = get_sdk().lock().unwrap_or_else(|e| e.into_inner());
        let result = sdk.execute_python(&code);
        into_c_string(result_to_json(&result))
    }

    #[no_mangle]
    pub extern "C" fn fastshell_execute_python_script(script_path: *const c_char) -> *mut c_char {
        let script_path = match cstr_to_string(script_path) {
            Some(s) => s,
            None => return into_c_string(error_to_json("Invalid script path", 1)),
        };
        let sdk = get_sdk().lock().unwrap_or_else(|e| e.into_inner());
        let result = sdk.execute_python_script(&script_path);
        into_c_string(result_to_json(&result))
    }

    #[no_mangle]
    pub extern "C" fn fastshell_get_cwd() -> *mut c_char {
        let sdk = get_sdk().lock().unwrap_or_else(|e| e.into_inner());
        into_c_string(sdk.get_cwd())
    }

    #[no_mangle]
    pub extern "C" fn fastshell_set_permission(resource: *const c_char, allowed: u8) {
        let resource = match cstr_to_string(resource) {
            Some(s) => s,
            None => return,
        };
        let sdk = get_sdk().lock().unwrap_or_else(|e| e.into_inner());
        sdk.set_permission(&resource, allowed != 0);
    }

    #[no_mangle]
    pub extern "C" fn fastshell_cancel_execution() {
        let sdk = get_sdk().lock().unwrap_or_else(|e| e.into_inner());
        sdk.cancel_execution();
    }

    /// Executes `command` with `dir` as the working directory (restored
    /// afterwards). Concurrent-host safe: no `cd X && ...` prefix needed and
    /// the shared cwd is never left polluted.
    #[no_mangle]
    pub extern "C" fn fastshell_execute_in(
        dir: *const c_char,
        command: *const c_char,
    ) -> *mut c_char {
        let dir = match cstr_to_string(dir) {
            Some(s) => s,
            None => return into_c_string(error_to_json("Invalid dir argument", 1)),
        };
        let command = match cstr_to_string(command) {
            Some(s) => s,
            None => return into_c_string(error_to_json("Invalid command argument", 1)),
        };
        let sdk = get_sdk().lock().unwrap_or_else(|e| e.into_inner());
        let result = sdk.execute_in(&dir, &command);
        into_c_string(result_to_json(&result))
    }

    /// Starts the agent server in a background Python thread.
    /// Returns JSON: {"ok": true} or {"ok": false, "error": "..."}
    #[no_mangle]
    pub extern "C" fn fastshell_start_agent_server() -> *mut c_char {
        let sdk = get_sdk().lock().unwrap_or_else(|e| e.into_inner());
        match sdk.start_agent_server() {
            Ok(()) => into_c_string(r#"{"ok":true}"#.to_string()),
            Err(e) => into_c_string(serde_json::json!({"ok": false, "error": e}).to_string()),
        }
    }

    /// Submits a task to the running agent server.
    /// `task_id` is a short unique identifier.
    /// `task_json` is a JSON object with "task", "project_path", "session_id".
    /// Returns JSON: {"ok": true} or {"ok": false, "error": "..."}
    #[no_mangle]
    pub extern "C" fn fastshell_submit_task(
        task_id: *const c_char,
        task_json: *const c_char,
    ) -> *mut c_char {
        let task_id = match cstr_to_string(task_id) {
            Some(s) => s,
            None => return into_c_string(error_to_json("Invalid task_id", 1)),
        };
        let task_json = match cstr_to_string(task_json) {
            Some(s) => s,
            None => return into_c_string(error_to_json("Invalid task_json", 1)),
        };
        let sdk = get_sdk().lock().unwrap_or_else(|e| e.into_inner());
        match sdk.submit_task(&task_id, &task_json) {
            Ok(()) => into_c_string(r#"{"ok":true}"#.to_string()),
            Err(e) => into_c_string(serde_json::json!({"ok": false, "error": e}).to_string()),
        }
    }

    /// C function pointer invoked for every stdout/stderr chunk produced
    /// by streaming Python execution. The `*const c_char` is a temporary
    /// NUL-terminated UTF-8 buffer valid only for the duration of the call;
    /// the callee must copy it if retention is needed.
    pub type StreamCallback = extern "C" fn(*const c_char);

    /// Registers (or, with a NULL pointer, clears) the stream callback.
    ///
    /// The C glue layer (jni_glue.c) passes a trampoline that forwards each
    /// chunk to the Java `onChunk(String)` method via JNI.
    #[no_mangle]
    pub extern "C" fn fastshell_register_stream_callback(cb: Option<StreamCallback>) {
        match cb {
            None => crate::python::clear_stream_callback(),
            Some(f) => {
                crate::python::register_stream_callback(Box::new(move |chunk: &str| {
                    if let Ok(cstr) = std::ffi::CString::new(chunk) {
                        f(cstr.as_ptr());
                    }
                }));
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::ffi::{CStr, CString};

        // The C ABI drives a single process-global SDK instance, so every
        // assertion runs inside ONE serial test to avoid cross-test races.
        fn take_json(ptr: *mut c_char) -> serde_json::Value {
            assert!(!ptr.is_null(), "FFI returned null");
            let s = unsafe { CStr::from_ptr(ptr) }.to_string_lossy().to_string();
            fastshell_free_string(ptr);
            serde_json::from_str(&s).unwrap_or_else(|_| panic!("not JSON: {s}"))
        }

        fn call_str(f: extern "C" fn(*const c_char) -> *mut c_char, arg: &str) -> serde_json::Value {
            let c = CString::new(arg).unwrap();
            take_json(f(c.as_ptr()))
        }

        static CB_HITS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        extern "C" fn test_stream_cb(chunk: *const c_char) {
            if !chunk.is_null() {
                let _ = unsafe { CStr::from_ptr(chunk) }.to_string_lossy();
                CB_HITS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }

        #[test]
        fn capi_full_surface() {
            let dir = std::env::temp_dir().join(format!("fastshell_capi_{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);

            // init
            let init = call_str(fastshell_init, dir.to_str().unwrap());
            assert_eq!(init["exit_code"], 0, "init failed: {init}");

            // execute (shell)
            let echo = call_str(fastshell_execute, "echo hello_capi");
            assert_eq!(echo["exit_code"], 0);
            assert!(echo["stdout"].as_str().unwrap().contains("hello_capi"));

            // get_cwd
            let cwd_ptr = fastshell_get_cwd();
            let cwd = unsafe { CStr::from_ptr(cwd_ptr) }.to_string_lossy().to_string();
            fastshell_free_string(cwd_ptr);
            assert!(!cwd.is_empty());

            // permission + cancel (must not panic). Unique resource name —
            // the permission table is process-global and example.com would
            // interfere with the sdk permission tests running in parallel.
            let res = CString::new("network:capi-roundtrip.internal").unwrap();
            fastshell_set_permission(res.as_ptr(), 1);
            fastshell_cancel_execution();

            // NULL-safety: passing null argument returns an error JSON, no crash
            let bad = take_json(fastshell_execute(std::ptr::null()));
            assert_ne!(bad["exit_code"], 0);

            // stream callback register / clear round-trip (no crash)
            fastshell_register_stream_callback(Some(test_stream_cb));
            fastshell_register_stream_callback(None);

            // execute_python — only assert output when a python engine exists
            let py = call_str(fastshell_execute_python, "print(6 * 7)");
            {
                let rt = crate::sdk::ffi::get_sdk_internal()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .runtime_ref();
                let rt = rt.lock().unwrap_or_else(|e| e.into_inner());
                if rt.python_available() {
                    assert_eq!(py["exit_code"], 0);
                    assert!(py["stdout"].as_str().unwrap().contains("42"));
                }
            }

            // set_permission with null resource must be a no-op, not a crash
            fastshell_set_permission(std::ptr::null(), 0);
        }
    }
}
