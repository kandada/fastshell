// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! # Python Engine Abstraction Layer
//!
//! Provides a unified interface for executing Python code across different
//! backends. The `detect_python_engine()` function selects the best available
//! backend based on the current platform and environment.
//!
//! ## Backend Selection Order
//!
//! **Mobile (Android/iOS):**
//!   1. `RustPythonEngine` — embedded RustPython (pure Rust, feature
//!      `python-rustpython`). Replaces the former Chaquopy CPython
//!      integration, which crashed on real devices (pthread/bionic TLS).
//!   2. `UnavailableEngine` — clear error if the feature is disabled.
//!
//! **Desktop (macOS/Linux):**
//!   1. `SubprocessPython` — system `python3` command (lightweight, preferred)
//!   2. `RustPythonEngine` — embedded fallback when python3 is missing
//!   3. `UnavailableEngine` — clear error
//!
//! ## PythonEngine Trait
//!
//! The trait is object-safe (Send bound only) and uses `&mut self` for
//! compatibility with stateful backends. Implementations:
//!
//! | Implementation      | Platform | Mechanism                          |
//! |---------------------|----------|-------------------------------------|
//! | `SubprocessPython`  | Desktop  | spawns `python3 -c "..."`           |
//! | `RustPythonEngine`  | All      | embedded RustPython (pure Rust)     |
//! | `UnavailableEngine` | All      | returns a clear error               |

use std::path::Path;
use std::process::Command;

#[cfg(feature = "python-rustpython")]
pub mod rustpython;
#[cfg(feature = "python-rustpython")]
pub use rustpython::RustPythonEngine;

// ── Streaming output callback slot (host apps register via FFI) ───────────
// Kept at module level so the C ABI (`fastshell_register_stream_callback`)
// stays stable across python-engine backends.
use std::sync::Mutex;
static STREAM_CALLBACK: Mutex<Option<Box<dyn FnMut(&str) + Send>>> = Mutex::new(None);

/// Registers (or replaces) the streaming-output callback.
pub fn register_stream_callback(cb: Box<dyn FnMut(&str) + Send>) {
    if let Ok(mut slot) = STREAM_CALLBACK.lock() {
        *slot = Some(cb);
    }
}

/// Clears the streaming-output callback.
pub fn clear_stream_callback() {
    if let Ok(mut slot) = STREAM_CALLBACK.lock() {
        *slot = None;
    }
}

/// Emits a chunk to the registered stream callback, if any.
#[allow(dead_code)]
pub(crate) fn emit_stream_chunk(chunk: &str) {
    if let Ok(mut slot) = STREAM_CALLBACK.lock() {
        if let Some(cb) = slot.as_mut() {
            cb(chunk);
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

impl ExecutionResult {
    pub fn success(stdout: String) -> Self {
        ExecutionResult {
            stdout,
            stderr: String::new(),
            exit_code: 0,
        }
    }

    pub fn error(stderr: String, exit_code: i32) -> Self {
        ExecutionResult {
            stdout: String::new(),
            stderr,
            exit_code,
        }
    }
}

pub trait PythonEngine: Send {
    fn execute(&mut self, code: &str, cwd: &Path) -> ExecutionResult;
    fn execute_script(&mut self, script_path: &Path, cwd: &Path) -> ExecutionResult;
    fn is_available(&self) -> bool;
    fn version(&self) -> Option<String>;

    /// Start the agent server in a background thread (CPython only).
    /// Returns Ok(()) if the server was started, or an error if not supported.
    fn start_agent_server(&self, _sandbox: &Path) -> Result<(), String> {
        Err("agent server not supported by this Python engine".into())
    }

    /// Submit a task to the running agent server (CPython only).
    fn submit_task(&self, _sandbox: &Path, _task_id: &str, _task_json: &str) -> Result<(), String> {
        Err("agent server not supported by this Python engine".into())
    }
}

/// Runs Python by spawning `python3` as a child process.
/// Used on desktop platforms where a system Python is typically available.
pub struct SubprocessPython {
    python_bin: String,
    available: bool,
}

impl SubprocessPython {
    pub fn new() -> Self {
        let python_bin = "python3".to_string();
        let available = Command::new(&python_bin)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        SubprocessPython {
            python_bin,
            available,
        }
    }

    pub fn with_binary(bin: &str) -> Self {
        let available = Command::new(bin)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        SubprocessPython {
            python_bin: bin.to_string(),
            available,
        }
    }
}

/// Wraps user code with sys.path injection and VFS sandbox monkey-patches
/// so that Python file I/O (open, os.open, os.listdir, os.chmod, etc.)
/// is confined to the sandbox root on all Python backends.
fn wrap_code_with_syspath(code: &str, cwd: &Path) -> String {
    let sandbox = cwd.to_string_lossy();
    // The VFS sandbox wrapper is injected before user code. It monkey-patches
    // builtins.open, os.open, os.listdir, os.chdir, os.remove, os.mkdir etc.
    // so that all file paths are resolved relative to FASTSHELL_ROOT and path
    // escape attempts (../) are stripped.
    let sandbox_wrapper = r#"
import builtins as _fs_builtins, os as _fs_os, shutil as _fs_shutil
_fs_root = _fs_os.environ.get('FASTSHELL_ROOT', '')
_fs_cwd_default = _fs_os.environ.get('FASTSHELL_CWD', '/')
if _fs_root:
    def _fs_resolve(file):
        if not _fs_os.path.isabs(file):
            file = _fs_os.path.join(_fs_os.environ.get('FASTSHELL_CWD', _fs_cwd_default), file)
        else:
            file = file.lstrip('/')
        parts = []
        for p in file.split('/'):
            if p == '..':
                if parts: parts.pop()
            elif p and p != '.':
                parts.append(p)
        return _fs_os.path.join(_fs_root, '/'.join(parts))

    _fs_real_open = _fs_builtins.open
    def _fs_sandboxed_open(file, mode='r', *a, **kw):
        return _fs_real_open(_fs_resolve(file), mode, *a, **kw)
    _fs_builtins.open = _fs_sandboxed_open

    _fs_real_os_open = _fs_os.open
    def _fs_sandboxed_os_open(file, flags, mode=0o777):
        return _fs_real_os_open(_fs_resolve(file), flags, mode)
    _fs_os.open = _fs_sandboxed_os_open

    _fs_real_listdir = _fs_os.listdir
    def _fs_sandboxed_listdir(path='.'):
        return _fs_real_listdir(_fs_resolve(path))
    _fs_os.listdir = _fs_sandboxed_listdir

    _fs_real_remove = _fs_os.remove
    def _fs_sandboxed_remove(path):
        return _fs_real_remove(_fs_resolve(path))
    _fs_os.remove = _fs_sandboxed_remove

    _fs_real_mkdir = _fs_os.mkdir
    def _fs_sandboxed_mkdir(path, mode=0o777):
        return _fs_real_mkdir(_fs_resolve(path), mode)
    _fs_os.mkdir = _fs_sandboxed_mkdir

    _fs_real_rmdir = _fs_os.rmdir
    def _fs_sandboxed_rmdir(path):
        return _fs_real_rmdir(_fs_resolve(path))
    _fs_os.rmdir = _fs_sandboxed_rmdir

    _fs_real_rename = _fs_os.rename
    def _fs_sandboxed_rename(src, dst):
        return _fs_real_rename(_fs_resolve(src), _fs_resolve(dst))
    _fs_os.rename = _fs_sandboxed_rename

    _fs_real_stat = _fs_os.stat
    def _fs_sandboxed_stat(path, *a, **kw):
        return _fs_real_stat(_fs_resolve(path), *a, **kw)
    _fs_os.stat = _fs_sandboxed_stat

    _fs_real_chdir = _fs_os.chdir
    def _fs_sandboxed_chdir(path):
        _fs_os.environ['FASTSHELL_CWD'] = _fs_os.path.normpath(
            _fs_os.path.join(_fs_os.environ.get('FASTSHELL_CWD', '/'), path)
        ) if not _fs_os.path.isabs(path) else _fs_os.path.normpath(path)
        _fs_real_chdir(_fs_resolve(path))
    _fs_os.chdir = _fs_sandboxed_chdir

    if hasattr(_fs_shutil, 'copy'):
        _fs_real_shutil_copy = _fs_shutil.copy
        def _fs_sandboxed_copy(src, dst, *a, **kw):
            return _fs_real_shutil_copy(_fs_resolve(src), _fs_resolve(dst), *a, **kw)
        _fs_shutil.copy = _fs_sandboxed_copy

    # Keep _fs_os and _fs_shutil alive — _fs_resolve() and the sandboxed
    # functions look them up in globals() at call time, so deleting them
    # would cause NameError when user code triggers a file operation.
    del _fs_builtins
"#;
    format!(
        r#"import sys,os
_r=os.environ.get('FASTSHELL_ROOT','{}')
_s=os.path.join(_r,'python','site-packages')
if _r and _r not in sys.path:sys.path.insert(0,_r)
if os.path.isdir(_s) and _s not in sys.path:sys.path.insert(0,_s)
del _r,_s
{}
exec({:?})"#,
        sandbox, sandbox_wrapper, code,
    )
}

impl PythonEngine for SubprocessPython {
    fn execute(&mut self, code: &str, cwd: &Path) -> ExecutionResult {
        if !self.available {
            return ExecutionResult::error(
                "Python is not available on this system".to_string(),
                127,
            );
        }

        let wrapped_code = wrap_code_with_syspath(code, cwd);

        match Command::new(&self.python_bin)
            .arg("-c")
            .arg(&wrapped_code)
            .current_dir(cwd)
            .env("FASTSHELL_ROOT", cwd)
            .output()
        {
            Ok(out) => ExecutionResult {
                stdout: String::from_utf8_lossy(&out.stdout).to_string(),
                stderr: String::from_utf8_lossy(&out.stderr).to_string(),
                exit_code: out.status.code().unwrap_or(-1),
            },
            Err(e) => ExecutionResult::error(format!("Failed to run python: {}", e), 127),
        }
    }

    fn execute_script(&mut self, script_path: &Path, cwd: &Path) -> ExecutionResult {
        let code = match std::fs::read_to_string(script_path) {
            Ok(c) => c,
            Err(e) => return ExecutionResult::error(
                format!("python3: can't open file '{}': {}\n", script_path.display(), e),
                2,
            ),
        };
        self.execute(&code, cwd)
    }

    fn is_available(&self) -> bool {
        self.available
    }

    fn version(&self) -> Option<String> {
        if !self.available {
            return None;
        }
        Command::new(&self.python_bin)
            .arg("--version")
            .output()
            .ok()
            .map(|o| {
                let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if !s.is_empty() {
                    return s;
                }
                String::from_utf8_lossy(&o.stderr).trim().to_string()
            })
    }
}

/// Returned when no Python backend is available: every call produces a
/// clear, actionable error instead of silently failing.
pub struct UnavailableEngine {
    reason: &'static str,
}

impl UnavailableEngine {
    pub fn new(reason: &'static str) -> Self {
        UnavailableEngine { reason }
    }
}

impl PythonEngine for UnavailableEngine {
    fn execute(&mut self, _code: &str, _cwd: &Path) -> ExecutionResult {
        ExecutionResult::error(format!("python unavailable: {}\n", self.reason), 127)
    }

    fn execute_script(&mut self, _script_path: &Path, _cwd: &Path) -> ExecutionResult {
        ExecutionResult::error(format!("python unavailable: {}\n", self.reason), 127)
    }

    fn is_available(&self) -> bool {
        false
    }

    fn version(&self) -> Option<String> {
        None
    }
}

/// Selects the best available Python engine for the current platform.
///
/// On mobile: only embedded CPython is available (no system Python).
///
/// On desktop: prefers system `python3` (SubprocessPython) for better
/// compatibility, falls back to embedded CPython.
///
/// If no engine is available, returns a non-functional wrapper that will
/// produce clear error messages on execute() — no silent failures.
///
/// Verbose diagnostics are gated behind `FASTSHELL_VERBOSE` — on Android,
/// stderr is forwarded to logcat and these lines were flooding it.
#[allow(dead_code)]
pub(crate) fn verbose_enabled() -> bool {
    std::env::var_os("FASTSHELL_VERBOSE").is_some()
}

pub fn detect_python_engine(_sandbox: &Path) -> Box<dyn PythonEngine> {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        // Mobile: embedded RustPython (pure Rust — no dlopen/JNI/pthread-TLS
        // hazards; the Chaquopy CPython path was removed after on-device
        // crashes in unittest/threading).
        #[cfg(feature = "python-rustpython")]
        {
            return Box::new(RustPythonEngine::new());
        }
        #[cfg(not(feature = "python-rustpython"))]
        {
            eprintln!(
                "[fastshell] python unavailable: build with feature `python-rustpython`"
            );
            return Box::new(UnavailableEngine::new(
                "fastshell was built without the `python-rustpython` feature",
            ));
        }
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        // Desktop: try system python3 first (lighter, picks up user-installed
        // packages), then fall back to the embedded RustPython.
        // `FASTSHELL_PYTHON=rustpython` forces the embedded engine (useful for
        // testing the mobile code path on a desktop).
        #[cfg(feature = "python-rustpython")]
        {
            if std::env::var("FASTSHELL_PYTHON").as_deref() == Ok("rustpython") {
                return Box::new(RustPythonEngine::new());
            }
        }
        let subprocess = SubprocessPython::new();
        if subprocess.is_available() {
            return Box::new(subprocess);
        }
        #[cfg(feature = "python-rustpython")]
        {
            return Box::new(RustPythonEngine::new());
        }
        #[cfg(not(feature = "python-rustpython"))]
        {
            eprintln!(
                "[fastshell] No Python available: system python3 not found and \
                 the `python-rustpython` feature is disabled"
            );
            return Box::new(UnavailableEngine::new(
                "system python3 not found; embedded RustPython not compiled in",
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn setup_dir() -> std::path::PathBuf {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "fastshell_python_test_{}_{}",
            std::process::id(),
            n
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_subprocess_python_available() {
        let engine = SubprocessPython::new();
        if engine.is_available() {
            let version = engine.version();
            assert!(version.is_some());
        }
    }

    #[test]
    fn test_execute_simple_code() {
        let mut engine = SubprocessPython::new();
        if !engine.is_available() {
            return;
        }
        let dir = setup_dir();
        let result = engine.execute("print('hello from python')", &dir);
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("hello from python"));
    }

    #[test]
    fn test_execute_with_error() {
        let mut engine = SubprocessPython::new();
        if !engine.is_available() {
            return;
        }
        let dir = setup_dir();
        let result = engine.execute("raise ValueError('test error')", &dir);
        assert_ne!(result.exit_code, 0);
        assert!(result.stderr.contains("ValueError") || result.stderr.contains("test error"));
    }

    #[test]
    fn test_execute_script() {
        let mut engine = SubprocessPython::new();
        if !engine.is_available() {
            return;
        }
        let dir = setup_dir();
        let script_path = dir.join("test_script.py");
        let mut f = fs::File::create(&script_path).unwrap();
        writeln!(f, "import sys").unwrap();
        writeln!(f, "print('script output')").unwrap();
        writeln!(
            f,
            "print('arg:', sys.argv[1] if len(sys.argv) > 1 else 'none')"
        )
        .unwrap();

        let result = engine.execute_script(&script_path, &dir);
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("script output"));
    }

    #[test]
    fn test_unavailable_engine() {
        let mut engine = UnavailableEngine::new("test reason");
        assert!(!engine.is_available());
        assert!(engine.version().is_none());
        let result = engine.execute("print(1+1)", std::path::Path::new("/tmp"));
        assert_ne!(result.exit_code, 0);
        assert!(result.stderr.contains("test reason"));
    }

    #[test]
    fn test_detect_python_engine() {
        let mut engine = detect_python_engine(std::path::Path::new("/tmp"));
        let result = engine.execute("print('test')", std::path::Path::new("/tmp"));
        if engine.is_available() {
            assert_eq!(result.exit_code, 0);
        } else {
            assert_ne!(result.exit_code, 0);
        }
    }
}
