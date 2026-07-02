//! # Python Engine Abstraction Layer
//!
//! Provides a unified interface for executing Python code across different
//! backends. The `detect_python_engine()` function selects the best available
//! backend based on the current platform and environment.
//!
//! ## Backend Selection Order
//!
//! **Mobile (Android/iOS):**
//!   1. `CpythonEngine` — embedded CPython 3.12 from vendor/python/ (production)
//!   2. `CpythonEngine` with error — reports build instructions to stderr
//!
//! **Desktop (macOS/Linux):**
//!   1. `SubprocessPython` — system `python3` command (lightweight, preferred)
//!   2. `CpythonEngine` — embedded CPython as fallback
//!   3. `CpythonEngine` with error — reports diagnostics
//!
//! ## PythonEngine Trait
//!
//! The trait is object-safe (Send bound only) and uses `&mut self` for
//! compatibility with stateful backends. Implementations:
//!
//! | Implementation           | Platform | Mechanism                    |
//! |--------------------------|----------|------------------------------|
//! | `SubprocessPython`       | Desktop  | spawns `python3 -c "..."`    |
//! | `CpythonEngineWrapper`   | All      | embedded CPython 3.12 via FFI|
//! | `PocketPyPlaceholder`    | All      | returns error (not yet impl) |

use std::path::Path;
use std::process::Command;

pub(crate) mod cpython;

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

/// Wraps user code with sys.path injection so that pre-bundled modules
/// under <sandbox>/python/site-packages/ and the sandbox root itself are
/// discoverable by `import`.
fn wrap_code_with_syspath(code: &str, cwd: &Path) -> String {
    let sandbox = cwd.to_string_lossy();
    format!(
        r#"import sys,os;_r=os.environ.get('FASTSHELL_ROOT','{}');_s=os.path.join(_r,'python','site-packages')
if _r and _r not in sys.path:sys.path.insert(0,_r)
if os.path.isdir(_s)and _s not in sys.path:sys.path.insert(0,_s)
del _r,_s
exec({:?})"#,
        sandbox, code,
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
        if !self.available {
            return ExecutionResult::error(
                "Python is not available on this system".to_string(), 127
            );
        }

        match Command::new(&self.python_bin)
            .arg(script_path)
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
                if !s.is_empty() { return s; }
                String::from_utf8_lossy(&o.stderr).trim().to_string()
            })
    }
}

pub struct PocketPyPlaceholder;

impl PocketPyPlaceholder {
    pub fn new() -> Self {
        PocketPyPlaceholder
    }
}

impl PythonEngine for PocketPyPlaceholder {
    fn execute(&mut self, _code: &str, _cwd: &Path) -> ExecutionResult {
        ExecutionResult::error(
            "pocketpy engine is not yet integrated (WIP)".to_string(),
            127,
        )
    }

    fn execute_script(&mut self, _script_path: &Path, _cwd: &Path) -> ExecutionResult {
        ExecutionResult::error(
            "pocketpy engine is not yet integrated (WIP)".to_string(),
            127,
        )
    }

    fn is_available(&self) -> bool {
        false
    }

    fn version(&self) -> Option<String> {
        None
    }
}

pub use cpython::{CpythonEngine, CpythonDownloader};

/// Selects the best available Python engine for the current platform.
///
/// On mobile: only embedded CPython is available (no system Python).
/// Wraps the result in `CpythonEngineWrapper` which satisfies `PythonEngine`.
///
/// On desktop: prefers system `python3` (SubprocessPython) for better
/// compatibility, falls back to embedded CPython.
///
/// If no engine is available, returns a non-functional wrapper that will
/// produce clear error messages on execute() — no silent failures.
pub fn detect_python_engine(sandbox: &Path) -> Box<dyn PythonEngine> {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        // Mobile: only embedded CPython can work. No forking, no system python3.
        let cpython = CpythonEngine::new(sandbox);
        if cpython.is_available() {
            return Box::new(CpythonEngineWrapper::new(cpython));
        }
        // CPython not embedded → print diagnostic to stderr so the developer
        // knows exactly what went wrong and how to fix it.
        let (_, reason) = cpython.is_available_with_reason();
        let reason_str = reason.as_deref().unwrap_or("unknown error");
        eprintln!("[fastshell] CPython not available: {}", reason_str);
        eprintln!("[fastshell] CPython must be embedded at compile time (vendor/python/).");
        eprintln!("[fastshell] Build steps:");
        eprintln!("[fastshell]   1. Cross-compile CPython 3.12 for this platform");
        eprintln!("[fastshell]   2. gzip libpython3.12.{{so,dylib}} → vendor/python/<target>/");
        eprintln!("[fastshell]   3. Rebuild the app — CPython is now embedded, works offline");
        eprintln!("[fastshell] See docs/integration.md#cpython-embedding-production-build");
        eprintln!("[fastshell] Development: CpythonDownloader::ensure_available() downloads at runtime.");
        return Box::new(CpythonEngineWrapper::new(cpython));
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        // Desktop: try system python3 first (lighter, picks up user-installed packages)
        let subprocess = SubprocessPython::new();
        if subprocess.is_available() {
            return Box::new(subprocess);
        }
        // Fall back to embedded CPython
        let cpython = CpythonEngine::new(sandbox);
        if cpython.is_available() {
            return Box::new(CpythonEngineWrapper::new(cpython));
        }
        // Neither available → report the combined error
        let (_, reason) = cpython.is_available_with_reason();
        eprintln!("[fastshell] No Python available: system python3 not found, {}", reason.as_deref().unwrap_or("embedded CPython also missing"));
        return Box::new(CpythonEngineWrapper::new(cpython));
    }
}

/// Adapts `CpythonEngine` (which takes `&self`) to the `PythonEngine` trait
/// (which requires `&mut self`). The inner engine is stateless at the Rust
/// level — all state is in the CPython VM — so `&self` is sufficient.
pub struct CpythonEngineWrapper {
    engine: CpythonEngine,
}

impl CpythonEngineWrapper {
    pub fn new(engine: CpythonEngine) -> Self {
        CpythonEngineWrapper { engine }
    }
}

impl PythonEngine for CpythonEngineWrapper {
    fn execute(&mut self, code: &str, cwd: &Path) -> ExecutionResult {
        self.engine.execute(code, cwd)
    }

    fn execute_script(&mut self, script_path: &Path, cwd: &Path) -> ExecutionResult {
        self.engine.execute_script(script_path, cwd)
    }

    fn is_available(&self) -> bool {
        self.engine.is_available()
    }

    fn version(&self) -> Option<String> {
        self.engine.version()
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
        let dir = std::env::temp_dir()
            .join(format!("fastshell_python_test_{}_{}", std::process::id(), n));
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
        writeln!(f, "print('arg:', sys.argv[1] if len(sys.argv) > 1 else 'none')").unwrap();

        let result = engine.execute_script(&script_path, &dir);
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("script output"));
    }

    #[test]
    fn test_pocketpy_placeholder() {
        let mut engine = PocketPyPlaceholder::new();
        assert!(!engine.is_available());
        assert!(engine.version().is_none());
        let result = engine.execute("print(1+1)", std::path::Path::new("/tmp"));
        assert_ne!(result.exit_code, 0);
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
