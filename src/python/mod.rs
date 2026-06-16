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

impl PythonEngine for SubprocessPython {
    fn execute(&mut self, code: &str, cwd: &Path) -> ExecutionResult {
        if !self.available {
            return ExecutionResult::error(
                "Python is not available on this system".to_string(),
                127,
            );
        }

        match Command::new(&self.python_bin)
            .arg("-c")
            .arg(code)
            .current_dir(cwd)
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

pub use cpython::{CpythonEngine, CpythonBundler};

pub fn detect_python_engine(_sandbox: &Path) -> Box<dyn PythonEngine> {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        let cpython = CpythonEngine::new(_sandbox);
        if cpython.is_available() {
            return Box::new(CpythonEngineWrapper::new(cpython));
        }
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let subprocess = SubprocessPython::new();
        if subprocess.is_available() {
            return Box::new(subprocess);
        }
    }

    Box::new(PocketPyPlaceholder::new())
}

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
        let mut engine = SubprocessPython::new();
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
