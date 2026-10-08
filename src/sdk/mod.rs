// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

pub mod device_callback;
pub mod ffi;
pub mod plugin;
pub mod types;

pub use ffi::try_get_sdk_instance;

use crate::bridge::Runtime;
use crate::python::{self, PythonEngine};
use crate::sdk::plugin::DevicePlugin;
use crate::shell::Shell;
use crate::vfs::Vfs;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use types::*;

/// Run a shell command closure under `catch_unwind` so a panic in any builtin
/// cannot kill the SDK worker thread. A panicking command is reported as a
/// clear `CommandOutput` error (exit 134, SIGABRT convention) instead of an
/// opaque "worker thread disconnected" message.
fn guarded_execute<F>(f: F, command: &str) -> crate::shell::CommandOutput
where
    F: FnOnce() -> crate::shell::CommandOutput,
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(output) => output,
        Err(payload) => {
            let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = payload.downcast_ref::<String>() {
                s.clone()
            } else {
                "unknown panic".to_string()
            };
            crate::shell::CommandOutput::error(
                format!(
                    "fastshell: internal error: command panicked ({msg})\n  command: {command}\n"
                ),
                134,
            )
        }
    }
}

/// Run `command` against an already-extracted runtime handle.
///
/// Crucially this does **not** touch the global SDK mutex (`SDK_INSTANCE`), so
/// a long-running command can never block `cancel_execution`, `set_permission`
/// or another command's FFI entry point. Hosts extract the handles under a
/// short lock (see `Fastshell::runtime_ref` / `cancel_handle`) and call this.
/// Arm the shared execution-deadline slot for a command of `timeout_ms`
/// (`0` = no deadline). Network / blocking builtins read this slot to bound
/// their own waits so a timed-out command releases the runtime promptly.
fn arm_deadline(deadline: &Arc<std::sync::atomic::AtomicU64>, timeout_ms: u64) {
    deadline.store(
        crate::shell::exec_deadline_after(timeout_ms),
        Ordering::SeqCst,
    );
}

pub fn execute_with_runtime(
    runtime: Arc<Mutex<Runtime>>,
    cancel: Arc<AtomicBool>,
    deadline: Arc<std::sync::atomic::AtomicU64>,
    timeout_ms: u64,
    command: &str,
) -> CommandResult {
    arm_deadline(&deadline, timeout_ms);
    if timeout_ms == 0 {
        let mut rt = runtime.lock().unwrap_or_else(|e| e.into_inner());
        let output = rt.execute_top(command);
        return CommandResult::from_code(output.stdout, output.stderr, output.exit_code);
    }

    cancel.store(false, Ordering::SeqCst);
    let rt = runtime;
    let cancel_for_thread = cancel.clone();
    let cmd = command.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("fastshell-exec".to_string())
        .spawn(move || {
            // Poll try_lock so we can still respond to cancel requests even
            // when a previous orphan thread is still holding the lock.
            let mut runtime = loop {
                if cancel_for_thread.load(Ordering::SeqCst) {
                    let _ = tx.send(crate::shell::CommandOutput::error(
                        "cancelled".to_string(),
                        143,
                    ));
                    return;
                }
                match rt.try_lock() {
                    Ok(r) => break r,
                    Err(std::sync::TryLockError::WouldBlock) => {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    Err(std::sync::TryLockError::Poisoned(e)) => break e.into_inner(),
                }
            };
            if cancel_for_thread.load(Ordering::SeqCst) {
                drop(runtime);
                let _ = tx.send(crate::shell::CommandOutput::error(
                    "cancelled".to_string(),
                    143,
                ));
                return;
            }
            let output = guarded_execute(|| runtime.execute_top(&cmd), &cmd);
            let _ = tx.send(output);
        })
        .expect("fastshell-exec thread spawn failed");

    match rx.recv_timeout(Duration::from_millis(timeout_ms)) {
        Ok(output) => CommandResult::from_code(output.stdout, output.stderr, output.exit_code),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            cancel.store(true, Ordering::SeqCst);
            CommandResult {
                stdout: String::new(),
                stderr: "command timed out\n".to_string(),
                exit_code: 124,
            }
        }
        Err(_) => CommandResult::error(format!("internal error: worker thread disconnected (shell panicked or was dropped) while running: {command}")),
    }
}

/// [`execute_with_runtime`] variant that runs `command` with `dir` as the
/// working directory and restores the previous cwd afterwards.
pub fn execute_in_with_runtime(
    runtime: Arc<Mutex<Runtime>>,
    cancel: Arc<AtomicBool>,
    deadline: Arc<std::sync::atomic::AtomicU64>,
    timeout_ms: u64,
    dir: &str,
    command: &str,
) -> CommandResult {
    arm_deadline(&deadline, timeout_ms);
    if timeout_ms == 0 {
        let mut rt = runtime.lock().unwrap_or_else(|e| e.into_inner());
        let output = rt.execute_with_cwd(dir, command);
        return CommandResult::from_code(output.stdout, output.stderr, output.exit_code);
    }

    cancel.store(false, Ordering::SeqCst);
    let rt = runtime;
    let cancel_for_thread = cancel.clone();
    let cmd = command.to_string();
    let dir = dir.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("fastshell-exec-cwd".to_string())
        .spawn(move || {
            let mut runtime = loop {
                if cancel_for_thread.load(Ordering::SeqCst) {
                    let _ = tx.send(crate::shell::CommandOutput::error(
                        "cancelled".to_string(),
                        143,
                    ));
                    return;
                }
                match rt.try_lock() {
                    Ok(r) => break r,
                    Err(std::sync::TryLockError::WouldBlock) => {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    Err(std::sync::TryLockError::Poisoned(e)) => break e.into_inner(),
                }
            };
            if cancel_for_thread.load(Ordering::SeqCst) {
                drop(runtime);
                let _ = tx.send(crate::shell::CommandOutput::error(
                    "cancelled".to_string(),
                    143,
                ));
                return;
            }
            let output = guarded_execute(|| runtime.execute_with_cwd(&dir, &cmd), &cmd);
            let _ = tx.send(output);
        })
        .expect("fastshell-exec-cwd thread spawn failed");

    match rx.recv_timeout(Duration::from_millis(timeout_ms)) {
        Ok(output) => CommandResult::from_code(output.stdout, output.stderr, output.exit_code),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            cancel.store(true, Ordering::SeqCst);
            CommandResult {
                stdout: String::new(),
                stderr: "command timed out\n".to_string(),
                exit_code: 124,
            }
        }
        Err(_) => CommandResult::error(format!("internal error: worker thread disconnected (shell panicked or was dropped) while running: {command}")),
    }
}

/// Run Python `code` against an extracted runtime handle (no global SDK lock).
pub fn execute_python_with_runtime(runtime: Arc<Mutex<Runtime>>, code: &str) -> CommandResult {
    let mut rt = runtime.lock().unwrap_or_else(|e| e.into_inner());
    let output = rt.execute_python_code(code);
    CommandResult::from_code(output.stdout, output.stderr, output.exit_code)
}

/// Run a Python `script_path` against an extracted runtime handle.
pub fn execute_python_script_with_runtime(
    runtime: Arc<Mutex<Runtime>>,
    script_path: &str,
) -> CommandResult {
    let mut rt = runtime.lock().unwrap_or_else(|e| e.into_inner());
    let output = rt.execute_python_script(script_path);
    CommandResult::from_code(output.stdout, output.stderr, output.exit_code)
}

/// Read the current working directory from an extracted runtime handle.
pub fn get_cwd_with_runtime(runtime: Arc<Mutex<Runtime>>) -> String {
    let rt = runtime.lock().unwrap_or_else(|e| e.into_inner());
    rt.cwd().to_string()
}

pub struct Fastshell {
    runtime: Arc<Mutex<Runtime>>,
    config: Config,
    initialized: bool,
    env_vars: std::collections::HashMap<String, String>,
    permissions: Arc<Mutex<HashMap<String, bool>>>,
    cancel_flag: Arc<AtomicBool>,
    deadline_flag: Arc<std::sync::atomic::AtomicU64>,
    plugin_ref: Arc<Mutex<Option<Box<dyn DevicePlugin>>>>,
}

impl Fastshell {
    pub fn new() -> Self {
        let permissions = Arc::new(Mutex::new(HashMap::new()));
        let plugin = Arc::new(Mutex::new(None));
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let deadline_flag = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let vfs = Vfs::new(std::env::temp_dir().join("fastshell")).unwrap_or_else(|_| {
            // Fallback: use /tmp/fastshell if temp dir creation fails
            Vfs::new(std::path::PathBuf::from("/tmp/fastshell"))
                .expect("VFS should always be creatable with /tmp fallback")
        });
        // (c) 2025 xiefujin <490021684@qq.com>
        let mut shell = Shell::with_plugin(vfs, true, false, permissions.clone(), plugin.clone());
        shell.set_cancel_flag(cancel_flag.clone());
        shell.set_exec_deadline_flag(deadline_flag.clone());
        Fastshell {
            runtime: Arc::new(Mutex::new(Runtime::new(shell, None))),
            config: Config::default(),
            initialized: false,
            env_vars: std::collections::HashMap::new(),
            permissions,
            cancel_flag,
            deadline_flag,
            plugin_ref: plugin,
        }
    }

    /// Initializes the SDK with a sandbox directory and configuration.
    ///
    /// This is the main entry point. It sets up:
    ///
    /// 1. **VFS (Virtual File System)** — creates/appends to the sandbox directory.
    ///    All file operations by shell commands and Python code are confined here.
    ///
    /// 2. **Shell Engine** — the built-in command executor (~180 commands).
    ///    Configured with subprocess policy and network permission gating.
    ///
    /// 3. **Python Engine** — auto-detected based on platform:
    ///    - Mobile (Android/iOS): embedded CPython 3.12 from vendor/python/
    ///    - Desktop: system `python3` preferred, embedded CPython as fallback
    ///
    /// 4. **Shell-Python Bridge** — registers C ABI function pointers so
    ///    Python code can call back into fastshell's shell executor.
    ///    This is what makes `subprocess.run("ls")` work inside Python.
    ///
    /// Must be called before execute() / execute_python() / file APIs.
    pub fn init(&mut self, config: Config) -> Result<(), String> {
        if config.sandbox_path.is_empty() {
            return Err("sandbox_path is required".to_string());
        }
        let sandbox_path = std::path::PathBuf::from(&config.sandbox_path);
        let vfs = Vfs::new(sandbox_path.clone())
            .map_err(|e| format!("Failed to initialize VFS: {}", e))?;

        let permissions = Arc::new(Mutex::new(HashMap::new()));
        // Inherit the host's device capabilities (camera/mic/location/…) if a
        // global callback was registered, so agent-spawned instances work too.
        let inherited = self
            .plugin_ref
            .lock()
            .unwrap()
            .take()
            .or_else(crate::sdk::device_callback::global_device_plugin);
        let plugin = Arc::new(Mutex::new(inherited));
        let mut shell = Shell::with_plugin(
            vfs,
            config.allow_subprocess,
            config.network_ask_permission,
            permissions.clone(),
            plugin.clone(),
        );
        // Share the SDK cancel flag with the shell engine so cooperative
        // cancellation works end-to-end (SDK → Runtime → Shell → builtins).
        shell.set_cancel_flag(self.cancel_flag.clone());
        shell.set_exec_deadline_flag(self.deadline_flag.clone());

        let python: Option<Box<dyn PythonEngine>> = if config.python_enabled {
            Some(python::detect_python_engine(&sandbox_path))
        } else {
            None
        };

        self.runtime = Arc::new(Mutex::new(Runtime::new(shell, python)));
        self.config = config;
        self.initialized = true;
        self.env_vars.clear();
        self.permissions = permissions;
        self.plugin_ref = plugin;

        // (c) 2025 xiefujin <490021684@qq.com>
        Ok(())
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    pub fn execute(&self, command: &str) -> CommandResult {
        if !self.initialized {
            return CommandResult::error("SDK not initialized. Call init() first.".to_string());
        }
        // Delegate to the handle-based free function so hosts (the C ABI) can
        // run a command WITHOUT holding the global SDK mutex for its whole
        // duration — a long command must not block cancel/permission/other
        // FFI calls. See `execute_with_runtime`.
        execute_with_runtime(
            self.runtime.clone(),
            self.cancel_flag.clone(),
            self.deadline_flag.clone(),
            self.config.command_timeout_ms,
            command,
        )
    }

    /// The configured per-command timeout (ms; 0 = run inline, no timeout).
    pub fn command_timeout_ms(&self) -> u64 {
        self.config.command_timeout_ms
    }

    /// A clone of the permission map, so a host can update/read permissions
    /// without holding the global SDK mutex.
    pub fn permissions_handle(&self) -> Arc<Mutex<HashMap<String, bool>>> {
        self.permissions.clone()
    }

    /// Execute a command with a per-call timeout override (milliseconds).
    /// When `per_call_timeout_ms` is `Some`, it overrides
    /// `self.config.command_timeout_ms` for this single invocation.
    pub fn execute_with_timeout(&self, command: &str, per_call_timeout_ms: u64) -> CommandResult {
        if !self.initialized {
            return CommandResult::error("SDK not initialized. Call init() first.".to_string());
        }

        self.cancel_flag.store(false, Ordering::SeqCst);
        arm_deadline(&self.deadline_flag, per_call_timeout_ms);
        let rt = self.runtime.clone();
        let cancel = self.cancel_flag.clone();
        let cmd = command.to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("fastshell-exec-tw".to_string())
            .spawn(move || {
                let mut runtime = loop {
                    if cancel.load(Ordering::SeqCst) {
                        let _ = tx.send(crate::shell::CommandOutput::error(
                            "cancelled".to_string(),
                            143,
                        ));
                        return;
                    }
                    match rt.try_lock() {
                        Ok(r) => break r,
                        Err(std::sync::TryLockError::WouldBlock) => {
                            std::thread::sleep(Duration::from_millis(50));
                        }
                        Err(std::sync::TryLockError::Poisoned(e)) => break e.into_inner(),
                    }
                };
                if cancel.load(Ordering::SeqCst) {
                    drop(runtime);
                    let _ = tx.send(crate::shell::CommandOutput::error(
                        "cancelled".to_string(),
                        143,
                    ));
                    return;
                }
                let output = guarded_execute(|| runtime.execute_top(&cmd), &cmd);
                let _ = tx.send(output);
            })
            .expect("fastshell-exec-tw thread spawn failed");

        match rx.recv_timeout(Duration::from_millis(per_call_timeout_ms)) {
            Ok(output) => CommandResult::from_code(output.stdout, output.stderr, output.exit_code),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                self.cancel_flag.store(true, Ordering::SeqCst);
                CommandResult {
                    stdout: String::new(),
                    stderr: "command timed out\n".to_string(),
                    exit_code: 124,
                }
            }
            Err(_) => CommandResult::error(format!("internal error: worker thread disconnected (shell panicked or was dropped) while running: {command}")),
        }
    }

    pub fn cancel_execution(&self) {
        self.cancel_flag.store(true, Ordering::SeqCst);
    }

    /// A clone of the internal cancel flag. Hosts can hold this handle and set
    /// it from another thread **without locking the SDK**, so a running command
    /// can be aborted even while the SDK mutex is held by that command (e.g. a
    /// cancelled task's `run_shell` that would otherwise keep holding the lock).
    pub fn cancel_handle(&self) -> Arc<AtomicBool> {
        self.cancel_flag.clone()
    }

    /// A clone of the internal execution-deadline slot (absolute monotonic ms;
    /// 0 = none). The SDK arms it before each timed command; network builtins
    /// read it so they abort at the deadline instead of holding the runtime
    /// until their own much longer socket timeout.
    pub fn deadline_handle(&self) -> Arc<std::sync::atomic::AtomicU64> {
        self.deadline_flag.clone()
    }

    /// Like [`execute`], but runs the command with `dir` as the working
    /// directory and restores the previous cwd afterwards. Safe for
    /// concurrent hosts: callers no longer need `cd X && ...` prefixes and
    /// never pollute the shared cwd.
    pub fn execute_in(&self, dir: &str, command: &str) -> CommandResult {
        self.execute_in_with_timeout(dir, command, self.config.command_timeout_ms)
    }

    /// Like [`execute_in`], but with a per-call timeout override (ms; 0 = no
    /// timeout). Runs `command` with `dir` as the working directory and restores
    /// the previous cwd afterwards, so callers never pollute the shared cwd.
    pub fn execute_in_with_timeout(
        &self,
        dir: &str,
        command: &str,
        per_call_timeout_ms: u64,
    ) -> CommandResult {
        if !self.initialized {
            return CommandResult::error("SDK not initialized. Call init() first.".to_string());
        }
        execute_in_with_runtime(
            self.runtime.clone(),
            self.cancel_flag.clone(),
            self.deadline_flag.clone(),
            per_call_timeout_ms,
            dir,
            command,
        )
    }

    pub fn execute_python(&self, code: &str) -> CommandResult {
        if !self.initialized {
            return CommandResult::error("SDK not initialized. Call init() first.".to_string());
        }
        // (c) 2025 xiefujin <490021684@qq.com>
        execute_python_with_runtime(self.runtime.clone(), code)
    }

    pub fn execute_python_script(&self, script_path: &str) -> CommandResult {
        if !self.initialized {
            return CommandResult::error("SDK not initialized. Call init() first.".to_string());
        }
        execute_python_script_with_runtime(self.runtime.clone(), script_path)
    }

    pub fn get_cwd(&self) -> String {
        if !self.initialized {
            return "/".to_string();
        }
        get_cwd_with_runtime(self.runtime.clone())
    }

    pub fn read_file(&self, path: &str) -> Result<String, String> {
        if !self.initialized {
            return Err("SDK not initialized".to_string());
        }
        let rt = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
        rt.read_file(path)
    }

    pub fn write_file(&self, path: &str, content: &str) -> Result<(), String> {
        if !self.initialized {
            return Err("SDK not initialized".to_string());
        }
        let rt = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
        rt.write_file(path, content)
    }

    pub fn list_dir(&self, path: &str) -> Result<Vec<FileEntry>, String> {
        if !self.initialized {
            return Err("SDK not initialized".to_string());
        }
        let rt = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
        rt.list_dir(path)
    }

    pub fn exists(&self, path: &str) -> bool {
        if !self.initialized {
            return false;
        }
        let rt = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
        rt.exists(path)
    }

    pub fn is_dir(&self, path: &str) -> bool {
        if !self.initialized {
            return false;
        }
        let rt = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
        rt.is_dir(path)
    }

    pub fn set_env(&mut self, key: &str, value: &str) {
        self.env_vars.insert(key.to_string(), value.to_string());
        std::env::set_var(key, value);
    }

    pub fn get_env(&self, key: &str) -> Option<String> {
        self.env_vars.get(key).cloned()
    }

    pub fn get_info(&self) -> SdkInfo {
        let python_available = if self.initialized {
            let rt = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
            rt.python_available()
        } else {
            false
        };

        SdkInfo {
            version: env!("CARGO_PKG_VERSION").to_string(),
            platform: std::env::consts::OS.to_string(),
            python_available,
            sandbox_path: self.config.sandbox_path.clone(),
            allow_subprocess: self.config.allow_subprocess,
        }
    }

    pub fn set_permission(&self, resource: &str, allowed: bool) {
        if let Ok(mut perms) = self.permissions.lock() {
            perms.insert(resource.to_string(), allowed);
        }
        // Mirror into the process-wide table so agent-spawned Fastshell
        // instances (fresh permission maps) see the grant too.
        crate::shell::set_global_permission(resource, allowed);
    }

    pub fn check_permission(&self, resource: &str) -> Option<bool> {
        self.permissions
            .lock()
            .ok()
            .and_then(|perms| perms.get(resource).copied())
    }

    pub fn clear_permissions(&self) {
        if let Ok(mut perms) = self.permissions.lock() {
            perms.clear();
        }
        // Keep the process-wide fallback in sync (see set_permission) —
        // stale grants there would survive a host-side permission reset.
        crate::shell::clear_global_permissions();
    }

    pub fn register_plugin(&self, plugin: Box<dyn DevicePlugin>) {
        if let Ok(mut p) = self.plugin_ref.lock() {
            *p = Some(plugin);
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn vfs_root(&self) -> String {
        let rt = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
        rt.shell_root_dir().to_string_lossy().to_string()
    }

    pub fn shutdown(&mut self) {
        // (c) 2025 xiefujin <490021684@qq.com>
        self.initialized = false;
        self.env_vars.clear();
        let rt = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
        let root = rt.shell_root_dir();
        let _ = std::fs::remove_dir_all(&root);
    }

    pub fn runtime_ref(&self) -> Arc<Mutex<Runtime>> {
        self.runtime.clone()
    }

    /// Starts the Python agent server in a background thread.
    /// The server runs an asyncio event loop and processes tasks
    /// submitted via `submit_task()`.
    pub fn start_agent_server(&self) -> Result<(), String> {
        let rt = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
        match &rt.python_engine_ref() {
            Some(engine) => engine.start_agent_server(&rt.shell_root_dir()),
            None => Err("Python engine not configured".into()),
        }
    }

    /// Submits a task to the running agent server.
    /// Returns immediately; output is written to the JSONL file.
    pub fn submit_task(&self, task_id: &str, task_json: &str) -> Result<(), String> {
        let rt = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
        match &rt.python_engine_ref() {
            Some(engine) => engine.submit_task(&rt.shell_root_dir(), task_id, task_json),
            None => Err("Python engine not configured".into()),
        }
    }
}

impl Default for Fastshell {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn setup_sdk() -> Fastshell {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("fastshell_sdk_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);

        let mut sdk = Fastshell::new();
        let config = Config {
            sandbox_path: dir.to_string_lossy().to_string(),
            python_enabled: true,
            allow_subprocess: true,
            network_ask_permission: false,
            ..Default::default()
        };
        sdk.init(config).unwrap();
        sdk
    }

    #[test]
    fn test_init() {
        let sdk = setup_sdk();
        assert!(sdk.is_initialized());
    }

    #[test]
    fn test_guarded_execute_catches_panic() {
        let out = guarded_execute(|| panic!("boom"), "some command");
        assert_eq!(out.exit_code, 134);
        assert!(
            out.stderr.contains("command panicked"),
            "stderr={}",
            out.stderr
        );
        assert!(out.stderr.contains("boom"), "stderr={}", out.stderr);
        assert!(out.stderr.contains("some command"), "stderr={}", out.stderr);
    }

    #[test]
    fn test_guarded_execute_passthrough() {
        let out = guarded_execute(
            || crate::shell::CommandOutput::success("ok".to_string()),
            "echo ok",
        );
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout, "ok");
    }

    #[test]
    fn test_execute_in_runs_in_dir_and_restores_cwd() {
        let sdk = setup_sdk();
        sdk.execute("mkdir -p subdir");
        let r = sdk.execute_in("/subdir", "pwd");
        assert_eq!(r.exit_code, 0, "{}", r.stderr);
        assert_eq!(r.stdout.trim(), "/subdir");
        // Shared cwd must be restored — a plain pwd still reports "/".
        let r = sdk.execute("pwd");
        assert_eq!(r.stdout.trim(), "/");
    }

    #[test]
    fn test_execute_in_bad_dir_errors() {
        let sdk = setup_sdk();
        let r = sdk.execute_in("/does_not_exist", "pwd");
        assert_ne!(r.exit_code, 0);
    }

    #[test]
    fn test_execute_in_supports_chaining() {
        let sdk = setup_sdk();
        sdk.execute("mkdir -p w");
        let r = sdk.execute_in("/w", "echo data > f.txt && cat f.txt");
        assert_eq!(r.exit_code, 0, "{}", r.stderr);
        assert!(r.stdout.contains("data"));
        // File landed inside /w, not at the root.
        let r = sdk.execute("cat /w/f.txt");
        assert_eq!(r.stdout.trim(), "data");
    }

    #[test]
    fn test_execute_not_initialized() {
        let sdk = Fastshell::new();
        let result = sdk.execute("ls");
        assert_ne!(result.exit_code, 0);
    }

    #[test]
    fn test_execute_shell() {
        let sdk = setup_sdk();
        let result = sdk.execute("echo hello_fastshell");
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("hello_fastshell"));
    }

    #[test]
    fn test_execute_ls() {
        let sdk = setup_sdk();
        let result = sdk.execute("ls -la");
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_execute_mkdir_cd_pwd() {
        let sdk = setup_sdk();
        sdk.execute("mkdir testdir");
        sdk.execute("cd testdir");
        let result = sdk.execute("pwd");
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("testdir"));
        assert!(sdk.get_cwd().contains("testdir"));
    }

    #[test]
    fn test_execute_file_operations() {
        let sdk = setup_sdk();
        sdk.execute("touch hello.txt");
        let result = sdk.execute("ls");
        assert!(result.stdout.contains("hello.txt"));
    }

    #[test]
    fn test_direct_file_api() {
        let sdk = setup_sdk();
        sdk.write_file("test.txt", "hello direct").unwrap();
        assert_eq!(sdk.read_file("test.txt").unwrap(), "hello direct");
        assert!(sdk.exists("test.txt"));
        assert!(!sdk.exists("nope.txt"));

        sdk.execute("mkdir subdir");
        let entries = sdk.list_dir("subdir").unwrap();
        assert!(entries.is_empty());

        sdk.write_file("subdir/a.txt", "a").unwrap();
        let entries = sdk.list_dir("subdir").unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn test_env_api() {
        let mut sdk = setup_sdk();
        sdk.set_env("MY_VAR", "my_value");
        assert_eq!(sdk.get_env("MY_VAR"), Some("my_value".to_string()));
    }

    #[test]
    fn test_execute_python_direct() {
        let sdk = setup_sdk();
        let result = sdk.execute_python("print(100 + 23)");
        {
            let rt = sdk.runtime_ref();
            let rt = rt.lock().unwrap_or_else(|e| e.into_inner());
            if rt.python_available() {
                assert_eq!(result.exit_code, 0);
                assert!(result.stdout.contains("123"));
            }
        }
    }

    #[test]
    fn test_get_info() {
        let sdk = setup_sdk();
        let info = sdk.get_info();
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
        assert!(!info.platform.is_empty());
    }

    #[test]
    fn test_shutdown() {
        let mut sdk = setup_sdk();
        let root = sdk.vfs_root();
        assert!(std::path::Path::new(&root).exists());
        sdk.shutdown();
        assert!(!sdk.is_initialized());
    }

    #[test]
    fn test_init_empty_path() {
        let mut sdk = Fastshell::new();
        let config = Config {
            sandbox_path: String::new(),
            ..Default::default()
        };
        assert!(sdk.init(config).is_err());
    }

    #[test]
    fn test_subprocess_disabled_by_default_on_mobile() {
        let default_config = Config::default();
        #[cfg(any(target_os = "android", target_os = "ios"))]
        assert!(!default_config.allow_subprocess);
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        assert!(default_config.allow_subprocess);
    }

    #[test]
    fn test_network_ask_permission_default_on_mobile() {
        let default_config = Config::default();
        #[cfg(any(target_os = "android", target_os = "ios"))]
        assert!(default_config.network_ask_permission);
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        assert!(!default_config.network_ask_permission);
    }

    /// Serializes tests that touch the process-wide permission table
    /// (set_permission mirrors globally; clear_permissions clears globally).
    static PERM_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_permission_management() {
        let _g = PERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let sdk = setup_sdk();
        // Unique resource name: the table is process-global, example.com is
        // shared with other tests and would race under parallel execution.
        assert_eq!(sdk.check_permission("network:perm-mgmt.internal"), None);
        sdk.set_permission("network:perm-mgmt.internal", true);
        assert_eq!(
            sdk.check_permission("network:perm-mgmt.internal"),
            Some(true)
        );
        sdk.set_permission("network:perm-mgmt.internal", false);
        assert_eq!(
            sdk.check_permission("network:perm-mgmt.internal"),
            Some(false)
        );
        sdk.clear_permissions();
        assert_eq!(sdk.check_permission("network:perm-mgmt.internal"), None);
        // The global fallback must be wiped too (agent instances read it).
        assert_eq!(
            crate::shell::global_permission("network:perm-mgmt.internal"),
            None
        );
    }

    #[test]
    fn test_network_permission_denied() {
        let _g = PERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut sdk = Fastshell::new();
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("fastshell_sdk_perm_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        let config = Config {
            sandbox_path: dir.to_string_lossy().to_string(),
            python_enabled: false,
            python_home: String::new(),
            allow_subprocess: false,
            network_ask_permission: true,
            command_timeout_ms: 5_000,
        };
        sdk.init(config).unwrap();

        let result = sdk.execute("curl http://denied-flow.internal");
        assert_eq!(result.exit_code, EXIT_NEED_PERMISSION);
        assert!(result
            .stderr
            .contains("PERMISSION_NEEDED:network:denied-flow.internal"));

        sdk.set_permission("network:denied-flow.internal", true);
        let result = sdk.execute("curl http://denied-flow.internal");
        assert_ne!(result.exit_code, EXIT_NEED_PERMISSION);
        sdk.clear_permissions();
    }

    #[test]
    fn test_clear_permissions_resets_global_fallback() {
        // Regression: a host-side permission reset must also revoke grants
        // for agent-spawned instances, which consult the global fallback.
        let _g = PERM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let host = setup_sdk();
        host.set_permission("network:reset-check.internal", true);
        assert_eq!(
            crate::shell::global_permission("network:reset-check.internal"),
            Some(true)
        );
        host.clear_permissions();
        assert_eq!(
            crate::shell::global_permission("network:reset-check.internal"),
            None
        );
    }

    #[test]
    fn test_command_not_found_subprocess_disabled() {
        let mut sdk = Fastshell::new();
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("fastshell_sdk_nosub_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        let config = Config {
            sandbox_path: dir.to_string_lossy().to_string(),
            python_enabled: false,
            python_home: String::new(),
            allow_subprocess: false,
            network_ask_permission: false,
            command_timeout_ms: 5_000,
        };
        sdk.init(config).unwrap();

        let result = sdk.execute("nonexistent_xyz_123");
        assert_eq!(result.exit_code, 127);
        assert!(result.stderr.contains("command not found"));
    }

    #[test]
    fn test_sdk_info_includes_allow_subprocess() {
        let sdk = setup_sdk();
        let info = sdk.get_info();
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
        assert!(info.allow_subprocess);
    }
}
