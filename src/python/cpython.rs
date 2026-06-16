use std::ffi::{c_char, c_int, CString};
use std::path::{Path, PathBuf};
use std::sync::Once;

use super::ExecutionResult;

// ── embedded CPython libraries (gzip compressed, extracted at runtime) ──
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const EMBEDDED_CPYTHON: &[u8] =
    include_bytes!("../../vendor/python/aarch64-apple-darwin/libpython3.12.dylib.gz");
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const EMBEDDED_CPYTHON: &[u8] =
    include_bytes!("../../vendor/python/x86_64-apple-darwin/libpython3.12.dylib.gz");
#[cfg(all(target_os = "android", target_arch = "aarch64"))]
const EMBEDDED_CPYTHON: &[u8] =
    include_bytes!("../../vendor/python/aarch64-linux-android/libpython3.12.so.gz");
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const EMBEDDED_CPYTHON: &[u8] =
    include_bytes!("../../vendor/python/x86_64-unknown-linux-gnu/libpython3.12.so.gz");

#[cfg(not(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "macos", target_arch = "x86_64"),
    all(target_os = "android", target_arch = "aarch64"),
    all(target_os = "linux", target_arch = "x86_64"),
)))]
const EMBEDDED_CPYTHON: &[u8] = b"";

#[cfg(not(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "macos", target_arch = "x86_64"),
    all(target_os = "android", target_arch = "aarch64"),
    all(target_os = "linux", target_arch = "x86_64"),
)))]
const HAS_EMBEDDED_CPYTHON: bool = false;
#[cfg(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "macos", target_arch = "x86_64"),
    all(target_os = "android", target_arch = "aarch64"),
    all(target_os = "linux", target_arch = "x86_64"),
))]
const HAS_EMBEDDED_CPYTHON: bool = true;

static PYTHON_INIT: Once = Once::new();

static SHELL_EXECUTE_FN: std::sync::OnceLock<unsafe extern "C" fn(*const c_char) -> *const c_char> = std::sync::OnceLock::new();

pub fn register_shell_execute(f: unsafe extern "C" fn(*const c_char) -> *const c_char) {
    SHELL_EXECUTE_FN.set(f).ok();
}

static SHELL_FREE_FN: std::sync::OnceLock<unsafe extern "C" fn(*mut c_char)> = std::sync::OnceLock::new();

pub fn register_shell_free(f: unsafe extern "C" fn(*mut c_char)) {
    SHELL_FREE_FN.set(f).ok();
}

#[no_mangle]
pub extern "C" fn fastshell_python_shell_exec(cmd: *const c_char) -> *const c_char {
    if let Some(&f) = SHELL_EXECUTE_FN.get() {
        unsafe { f(cmd) }
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub extern "C" fn fastshell_python_shell_free(ptr: *mut c_char) {
    if let Some(&f) = SHELL_FREE_FN.get() {
        unsafe { f(ptr) }
    }
}

const PYTHON_WRAPPER: &str = r#"
import ctypes, json, asyncio, io

try:
    _fs_lib = ctypes.CDLL(None)
    _fs_exec = ctypes.CFUNCTYPE(ctypes.c_char_p, ctypes.c_char_p)({EXEC_ADDR})
    _fs_free = ctypes.CFUNCTYPE(None, ctypes.c_char_p)({FREE_ADDR})
except Exception as _e:
    _fs_exec = None

if _fs_exec is not None:
    def _fs_run(command, cwd=None, timeout=None):
        if cwd:
            cmd_str = "cd " + cwd + " && " + command
        else:
            cmd_str = command
        ptr = _fs_exec(cmd_str.encode('utf-8'))
        if ptr is None:
            return {"stdout": "", "stderr": "fastshell: internal error", "returncode": 127}
        raw = ctypes.c_char_p(ptr).value
        if raw is None:
            return {"stdout": "", "stderr": "fastshell: null response", "returncode": 127}
        r = json.loads(raw.decode('utf-8'))
        _fs_free(ptr)
        return r

    class _FastShellProcess:
        def __init__(self, command, cwd=None, env=None):
            r = _fs_run(command, cwd)
            self.returncode = r.get("returncode", 0)
            self._stdout = r.get("stdout", "")
            self._stderr = r.get("stderr", "")
            self._pid = -1
            self._stdout_io = io.StringIO(self._stdout)
            self._stderr_io = io.StringIO(self._stderr)

        async def communicate(self, input=None):
            return (self._stdout.encode('utf-8'), self._stderr.encode('utf-8'))

        async def wait(self):
            return self.returncode

        def kill(self):
            pass

        def terminate(self):
            pass

        @property
        def stdout(self):
            class _Reader:
                async def read(self, n=-1):
                    return self._data.encode('utf-8')
                async def readline(self):
                    line = self._io.readline()
                    return line.encode('utf-8') if line else b''
                async def readlines(self):
                    return [l.encode('utf-8') for l in self._io.readlines()]
            r = _Reader()
            r._data = self._stdout
            r._io = self._stdout_io
            return r

        @property
        def stderr(self):
            class _Reader:
                async def read(self, n=-1):
                    return self._data.encode('utf-8')
                async def readline(self):
                    line = self._io.readline()
                    return line.encode('utf-8') if line else b''
            r = _Reader()
            r._data = self._stderr
            r._io = self._stderr_io
            return r

    _orig_csp = asyncio.create_subprocess_shell
    async def _hooked_csp(cmd, *args, **kwargs):
        return _FastShellProcess(cmd, kwargs.get('cwd'), kwargs.get('env'))
    asyncio.create_subprocess_shell = _hooked_csp

    import subprocess as _sp
    _orig_run = _sp.run
    def _hooked_run(cmd, **kwargs):
        cmd_str = cmd if isinstance(cmd, str) else ' '.join(str(a) for a in cmd)
        r = _fs_run(cmd_str, kwargs.get('cwd'))
        return _sp.CompletedProcess(cmd, r.get("returncode", 0), r.get("stdout", ""), r.get("stderr", ""))
    _sp.run = _hooked_run

    def _hooked_Popen(cmd, **kwargs):
        cmd_str = cmd if isinstance(cmd, str) else ' '.join(cmd)
        return _FastShellProcess(cmd_str, kwargs.get('cwd'), kwargs.get('env'))
    _sp.Popen = _hooked_Popen

    import os as _os
    _os.system = lambda cmd: _fs_run(cmd).get("returncode", 0)

    del ctypes, json, asyncio, io, _sp, _os, _orig_csp, _orig_run
del _fs_exec, _fs_free, _fs_run, _FastShellProcess, _hooked_csp, _hooked_run, _hooked_Popen
"#;

pub struct CpythonEngine {
    lib: Option<&'static libloading::Library>,
    wrapper_injected: bool,
}

impl CpythonEngine {
    fn extract_bundled(sandbox: &Path) -> Option<PathBuf> {
        if !HAS_EMBEDDED_CPYTHON { return None; }
        let lib_name = Self::lib_name();
        let lib_dir = sandbox.join("python/lib");
        let lib_path = lib_dir.join(lib_name);
        if lib_path.exists() { return Some(lib_path); }
        std::fs::create_dir_all(&lib_dir).ok()?;
        let mut decoder = flate2::read::GzDecoder::new(EMBEDDED_CPYTHON);
        let mut decompressed = Vec::new();
        use std::io::Read;
        decoder.read_to_end(&mut decompressed).ok()?;
        std::fs::write(&lib_path, &decompressed).ok()?;
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&lib_path, std::fs::Permissions::from_mode(0o755));
        }
        Some(lib_path)
    }

    fn lib_name() -> &'static str {
        #[cfg(target_os = "android")] return "libpython3.12.so";
        #[cfg(target_os = "linux")] return "libpython3.12.so.1.0";
        #[cfg(target_os = "macos")] return "libpython3.12.dylib";
        #[cfg(not(any(target_os = "android", target_os = "linux", target_os = "macos")))]
        "libpython3.12.so"
    }

    fn find_lib_in_sandbox(sandbox: &Path) -> Option<PathBuf> {
        let lib_name = Self::lib_name();
        let path = sandbox.join("python/lib").join(lib_name);
        if path.exists() { Some(path) } else { None }
    }

    fn find_lib_system() -> Option<String> {
        let c: Vec<&str> = vec![
            #[cfg(target_os = "macos")] "/opt/homebrew/opt/python@3.12/Frameworks/Python.framework/Versions/3.12/lib/libpython3.12.dylib",
            #[cfg(target_os = "macos")] "/Library/Frameworks/Python.framework/Versions/3.12/lib/libpython3.12.dylib",
            #[cfg(target_os = "macos")] "/Library/Frameworks/Python.framework/Versions/3.11/lib/libpython3.11.dylib",
            #[cfg(target_os = "linux")] "libpython3.12.so.1.0",
            #[cfg(target_os = "linux")] "libpython3.11.so.1.0",
            #[cfg(target_os = "android")] "libpython3.12.so",
            #[cfg(target_os = "android")] "/data/data/com.fastshell/python/lib/libpython3.12.so",
        ];
        for x in &c { if Path::new(x).exists() { return Some(x.to_string()); } }
        None
    }

    pub fn new(sandbox: &Path) -> Self {
        let lib_path = Self::extract_bundled(sandbox)
            .or_else(|| Self::find_lib_in_sandbox(sandbox))
            .map(|p| p.to_string_lossy().to_string())
            .or_else(Self::find_lib_system);
        let lib_path = match lib_path { Some(p) => p, None => return CpythonEngine { lib: None, wrapper_injected: false } };

        let lib: Option<&'static libloading::Library> = match unsafe { libloading::Library::new(&lib_path) } {
            Ok(l) => Some(Box::leak(Box::new(l)) as &'static libloading::Library),
            Err(_) => None,
        };
        if lib.is_some() {
            PYTHON_INIT.call_once(|| {
                if let Some(lib) = lib {
                    if let Ok(init) = unsafe { lib.get::<unsafe extern "C" fn()>(b"Py_Initialize\0") } {
                        unsafe { init() };
                    }
                }
            });
        }
        CpythonEngine { lib, wrapper_injected: false }
    }

    pub fn is_available(&self) -> bool { self.lib.is_some() }

    pub fn version(&self) -> Option<String> {
        let lib = self.lib.as_ref()?;
        unsafe {
            let f: libloading::Symbol<unsafe extern "C" fn() -> *const c_char> = lib.get(b"Py_GetVersion\0").ok()?;
            let ptr = f();
            if ptr.is_null() { None } else { Some(std::ffi::CStr::from_ptr(ptr).to_string_lossy().to_string()) }
        }
    }

    pub fn execute(&mut self, code: &str, cwd: &Path) -> ExecutionResult {
        let lib = match &self.lib { Some(l) => l, None => return ExecutionResult::error("CPython library not loaded".to_string(), 127) };

        if !self.wrapper_injected {
            self.wrapper_injected = true;
            let exec_addr = fastshell_python_shell_exec as usize;
            let free_addr = fastshell_python_shell_free as usize;
            let wrapper = PYTHON_WRAPPER
                .replace("{EXEC_ADDR}", &exec_addr.to_string())
                .replace("{FREE_ADDR}", &free_addr.to_string());
            let wc = CString::new(wrapper).unwrap();
            unsafe {
                let py_run: libloading::Symbol<unsafe extern "C" fn(*const c_char) -> c_int> = lib.get(b"PyRun_SimpleString\0").unwrap();
                py_run(wc.as_ptr());
            }
        }

        let capture_out = cwd.join(".fs_capture_out");
        let capture_err = cwd.join(".fs_capture_err");
        let wrapped = format!(
            r#"
import sys
_fs_out = open("{}", 'w')
_fs_err = open("{}", 'w')
_saved_out, _saved_err = sys.stdout, sys.stderr
sys.stdout, sys.stderr = _fs_out, _fs_err
try:
    exec({:?})
finally:
    sys.stdout, sys.stderr = _saved_out, _saved_err
    _fs_out.close()
    _fs_err.close()
"#,
            capture_out.display(), capture_err.display(), code,
        );

        let code_c = match CString::new(wrapped) { Ok(c) => c, Err(e) => return ExecutionResult::error(format!("invalid code: {}", e), 1) };
        let exit_code = unsafe {
            let py_run: libloading::Symbol<unsafe extern "C" fn(*const c_char) -> c_int> = lib.get(b"PyRun_SimpleString\0").unwrap();
            py_run(code_c.as_ptr())
        };
        let stdout = std::fs::read_to_string(&capture_out).unwrap_or_default();
        let stderr = std::fs::read_to_string(&capture_err).unwrap_or_default();
        let _ = std::fs::remove_file(&capture_out);
        let _ = std::fs::remove_file(&capture_err);
        if exit_code != 0 { ExecutionResult { stdout, stderr, exit_code: 1 } }
        else { ExecutionResult { stdout, stderr, exit_code: 0 } }
    }

    pub fn execute_script(&mut self, script_path: &Path, _cwd: &Path) -> ExecutionResult {
        let content = match std::fs::read_to_string(script_path) { Ok(c) => c, Err(e) => return ExecutionResult::error(format!("Cannot read {}: {}", script_path.display(), e), 1) };
        self.execute(&content, _cwd)
    }
}

pub struct CpythonBundler;

impl CpythonBundler {
    pub const CDN_URL: &str = "https://cdn.fastshell.dev/python";

    pub fn platform_triple() -> &'static str {
        #[cfg(all(target_os = "android", target_arch = "aarch64"))] { return "android-arm64"; }
        #[cfg(all(target_os = "android", target_arch = "arm"))] { return "android-armv7"; }
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))] { return "linux-x86_64"; }
        #[cfg(all(target_os = "linux", target_arch = "aarch64"))] { return "linux-arm64"; }
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))] { return "macos-arm64"; }
        #[cfg(all(target_os = "macos", target_arch = "x86_64"))] { return "macos-x86_64"; }
        "unknown"
    }

    pub fn is_bundled(sandbox: &Path) -> bool { CpythonEngine::find_lib_in_sandbox(sandbox).is_some() }

    pub fn ensure_available(sandbox: &Path) -> Result<(), String> {
        if Self::is_bundled(sandbox) { return Ok(()); }
        let url = format!("{}/cpython-3.12-{}.tar.gz", Self::CDN_URL, Self::platform_triple());
        let python_dir = sandbox.join("python");
        std::fs::create_dir_all(&python_dir).map_err(|e| e.to_string())?;
        let response = ureq::get(&url).call().map_err(|e| format!("Download failed: {}", e))?;
        let mut compressed = Vec::new();
        use std::io::Read;
        response.into_reader().read_to_end(&mut compressed).map_err(|e| e.to_string())?;
        let decoder = flate2::read::GzDecoder::new(&compressed[..]);
        let mut archive = tar::Archive::new(decoder);
        for entry in archive.entries().map_err(|e| e.to_string())? {
            let mut entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path().map_err(|e| e.to_string())?;
            let dest_path = python_dir.join(&*path);
            if let Some(parent) = dest_path.parent() { std::fs::create_dir_all(parent).map_err(|e| e.to_string())?; }
            if entry.header().entry_type() == tar::EntryType::Directory { std::fs::create_dir_all(&dest_path).map_err(|e| e.to_string())?; }
            else { let mut data = Vec::new(); entry.read_to_end(&mut data).map_err(|e| e.to_string())?; std::fs::write(&dest_path, &data).map_err(|e| e.to_string())?; }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedded_data_exists() { if HAS_EMBEDDED_CPYTHON { assert!(!EMBEDDED_CPYTHON.is_empty()); } }

    #[test]
    fn test_embedded_data_decompresses() {
        if !HAS_EMBEDDED_CPYTHON { return; }
        let mut decoder = flate2::read::GzDecoder::new(EMBEDDED_CPYTHON);
        let mut decompressed = Vec::new();
        use std::io::Read;
        assert!(decoder.read_to_end(&mut decompressed).is_ok());
        assert!(!decompressed.is_empty());
    }

    #[test]
    fn test_extract_bundled() {
        let tmp = std::env::temp_dir().join(format!("fastshell_cpy_extract_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).ok();
        let result = CpythonEngine::extract_bundled(&tmp);
        if HAS_EMBEDDED_CPYTHON { assert!(result.is_some()); } else { assert!(result.is_none()); }
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
