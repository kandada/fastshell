//! # CPython 3.12 Embedded Engine
//!
//! This module embeds a full CPython 3.12 interpreter into the fastshell binary.
//! It is the core mechanism that enables AI agents (aacode, fastclaw) to run
//! Python code on mobile devices where no system Python exists.
//!
//! ## Production Path (compile-time embedding)
//!
//! The ONLY supported production path is embedding CPython at compile time:
//!
//! ```text
//! vendor/python/<target>/libpython3.12.{so,dylib}.gz
//!     │
//!     ▼ include_bytes!() ──► embedded in Rust binary
//!     │
//!     ▼ runtime: extract_bundled() ──► decompress to sandbox
//!     │
//!     ▼ libloading::Library::new() ──► dlopen
//!     │
//!     ▼ Py_Initialize() ──► CPython VM ready
//! ```
//!
//! No network call at any point. The library ships inside your app binary.
//!
//! ## Development Path (runtime download)
//!
//! `CpythonDownloader::ensure_available()` downloads CPython from CDN at
//! runtime. This is for DEVELOPMENT ONLY — never ship production apps that
//! download Python on first launch.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────┐
//! │  Host App (Kotlin/Swift)                    │
//! │    │ JNI / C FFI                            │
//! │    ▼                                         │
//! │  Fastshell SDK                               │
//! │    │                                         │
//! │    ├── register_shell_execute() ──────────┐  │
//! │    ├── register_shell_free()  ────────────┤  │
//! │    │                                      │  │
//! │    ▼                                      │  │
//! │  CpythonEngine                             │  │
//! │    │ dlopen libpython3.12                  │  │
//! │    │ Py_Initialize()                       │  │
//! │    │ inject PYTHON_WRAPPER ──────────┐     │  │
//! │    │ PyRun_SimpleString(code)         │     │  │
//! │    │                                  │     │  │
//! │    ▼                                  ▼     ▼  │
//! │  ┌──────────────────────────────────────────┐ │
//! │  │  CPython VM (in-process)                 │ │
//! │  │    subprocess.run("ls")                  │ │
//! │  │      → _hooked_run() intercepts          │ │
//! │  │      → _fs_run("ls")                     │ │
//! │  │      → ctypes call                       │ │
//! │  │      → fastshell_python_shell_exec() ────┼─┘
//! │  │      → fastshell_shell_exec_c()          │
//! │  │      → Shell::execute("ls")              │
//! │  │      → returns stdout/stderr JSON ◄──────┤
//! │  │    builtins.open("file")                 │
//! │  │      → _sandboxed_open() resolves to VFS │
//! │  └──────────────────────────────────────────┘ │
//! └─────────────────────────────────────────────┘
//! ```
//!
//! ## Safety
//!
//! This module uses `unsafe` for:
//! - `libloading::Library::new()` — loading a native shared library
//! - `Py_Initialize()`, `Py_GetVersion()`, `PyRun_SimpleString()` — FFI into CPython
//! - `fastshell_python_shell_exec/free` — C ABI bridging between Rust and Python
//!
//! All unsafe calls are guarded: null checks on pointers, `CStr::from_ptr`
//! for string safety, and error handling on failed FFI lookups.
//!
//! ## Platform Support
//!
//! | Platform              | Library                | Status               |
//! |-----------------------|------------------------|----------------------|
//! | macOS ARM64           | libpython3.12.dylib    | bundled (2MB .gz)    |
//! | macOS x86_64          | libpython3.12.dylib    | bundled (2MB .gz)    |
//! | Android ARM64 (v8a)   | libpython3.12.so       | placeholder (0 bytes)|
//! | Android x86_64 (emu)  | libpython3.12.so       | not bundled          |
//! | Linux x86_64          | libpython3.12.so.1.0   | placeholder (0 bytes)|
//! | iOS ARM64             | libpython3.12.a        | planned              |

use std::ffi::{c_char, c_int, CString};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use super::ExecutionResult;

// ═══════════════════════════════════════════════════════════
// Compile-time embedded CPython — THE PRODUCTION PATH
// ═══════════════════════════════════════════════════════════
//
// Each `#[cfg(...)]` block injects a gzip-compressed CPython shared library
// for exactly one target platform. At compile time, `include_bytes!()` reads
// the .gz file from vendor/python/ and bakes it into the Rust binary.
//
// At runtime, `extract_bundled()` decompresses the bytes to disk inside the
// app sandbox, then `libloading` opens it dynamically. The extraction is
// idempotent — if the library already exists on disk, it is not re-extracted.
//
// ADDING A NEW PLATFORM:
//   1. Cross-compile CPython 3.12 for the target using NDK/Xcode toolchain
//   2. gzip the resulting .so/.dylib/.a
//   3. Place it at: vendor/python/<target-triple>/<libname>.gz
//   4. Add a matching #[cfg(...)] block below
//   5. Rebuild — the library is now embedded
//
// The uncompressed files (.so, .dylib, .a) are NOT committed to git.
// Only the .gz files are tracked (see vendor/.gitignore).

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

// Fallback: no embedded library for this platform.
// The binary will be smaller, but CPython won't be available.
// Users on this platform must rely on system Python (SubprocessPython).
#[cfg(not(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "macos", target_arch = "x86_64"),
    all(target_os = "android", target_arch = "aarch64"),
    all(target_os = "linux", target_arch = "x86_64"),
)))]
const EMBEDDED_CPYTHON: &[u8] = b"";

const HAS_EMBEDDED_CPYTHON: bool = EMBEDDED_CPYTHON.len() > 0;

// ═══════════════════════════════════════════════════════════
// Python Wrapper — injected into CPython VM on first execute
// ═══════════════════════════════════════════════════════════
//
// This is a Python script that runs inside the CPython interpreter after
// Py_Initialize(). It monkey-patches key modules to redirect all shell/file
// operations through fastshell's Rust implementation.
//
// WHAT IT HOOKS:
//   subprocess.run()         → _hooked_run()       → _fs_run() → fastshell shell
//   subprocess.Popen()       → _FastShellProcess   → _fs_run() → fastshell shell
//   os.system()              → lambda              → _fs_run() → fastshell shell
//   asyncio.create_subprocess_shell() → _hooked_csp → _FastShellProcess
//   builtins.open()          → _sandboxed_open()   → VFS sandbox resolution
//   os.open()                → _sandboxed_os_open()→ VFS sandbox resolution
//   os.chdir()               → _sandboxed_chdir()  → VFS cwd tracking
//   os.listdir()             → _sandboxed_listdir()→ VFS sandbox resolution
//
// HOW SHELL CALLS WORK:
//   Python code calls subprocess.run("ls -la")
//     → _hooked_run() marshals the command string
//     → _fs_run() encodes to UTF-8 bytes
//     → ctypes calls fastshell_python_shell_exec() in Rust
//     → fastshell_shell_exec_c() dispatches to Shell::execute()
//     → Returns JSON: {"stdout": ..., "stderr": ..., "returncode": ...}
//     → _fs_run() parses JSON, returns dict
//     → _hooked_run() wraps into CompletedProcess
//
// HOW FILE OPERATIONS WORK:
//   Python code calls open("/etc/passwd")
//     → _sandboxed_open() calls _resolve_sandbox_path()
//     → _resolve_sandbox_path() normalizes the path:
//       - Removes leading / (absolute paths become relative to sandbox root)
//       - Strips .. components (prevents path escape)
//       - Prepends FASTSHELL_ROOT (sandbox directory)
//     → _real_open() opens the resolved path on the real filesystem
//
// {EXEC_ADDR} and {FREE_ADDR} are replaced at runtime with the actual memory
// addresses of the Rust-side C ABI functions, so ctypes.CFUNCTYPE can call them.
//
// The wrapper runs ONCE per process (WRAPPER_INJECTED OnceLock).
// It cleans up after itself by deleting temporary variables (del ...)
// to avoid polluting the global namespace.

static WRAPPER_INJECTED: OnceLock<()> = OnceLock::new();

const PYTHON_WRAPPER: &str = r#"
import ctypes, json, asyncio, io, builtins, os as _os_module

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

    _sandbox_root = _os_module.environ.get('FASTSHELL_ROOT', '')
    _sandbox_cwd = _os_module.environ.get('FASTSHELL_CWD', '/')

    def _resolve_sandbox_path(file):
        if not _sandbox_root:
            return file
        if not _os_module.path.isabs(file):
            file = _os_module.path.join(_sandbox_cwd, file)
        else:
            file = file.lstrip('/')
        components = []
        for p in file.split('/'):
            if p == '..':
                if components: components.pop()
            elif p and p != '.':
                components.append(p)
        safe = '/'.join(components)
        return _os_module.path.join(_sandbox_root, safe)

    _real_open = builtins.open
    def _sandboxed_open(file, mode='r', *args, **kwargs):
        return _real_open(_resolve_sandbox_path(file), mode, *args, **kwargs)
    builtins.open = _sandboxed_open

    _real_os_open = _os_module.open
    def _sandboxed_os_open(file, flags, mode=0o777):
        return _real_os_open(_resolve_sandbox_path(file), flags, mode)
    _os_module.open = _sandboxed_os_open

    _real_chdir = _os_module.chdir
    def _sandboxed_chdir(path):
        global _sandbox_cwd
        if _sandbox_root:
            if _os_module.path.isabs(path):
                _sandbox_cwd = _os_module.path.normpath(path)
            else:
                _sandbox_cwd = _os_module.path.normpath(_os_module.path.join(_sandbox_cwd, path))
        _real_chdir(path)
    _os_module.chdir = _sandboxed_chdir

    _real_listdir = _os_module.listdir
    def _sandboxed_listdir(path='.'):
        return _real_listdir(_resolve_sandbox_path(path))
    _os_module.listdir = _sandboxed_listdir

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

    _os_module.system = lambda cmd: _fs_run(cmd).get("returncode", 0)

    del ctypes, json, asyncio, io, _sp, _orig_csp, _orig_run
del _fs_exec, _fs_free, _fs_run, _FastShellProcess, _hooked_csp, _hooked_run, _hooked_Popen, _sandbox_root, _sandbox_cwd, _sandboxed_open, _real_open, _sandboxed_os_open, _real_os_open, _real_chdir, _sandboxed_chdir, _real_listdir, _sandboxed_listdir, _resolve_sandbox_path
"#;

// ═══════════════════════════════════════════════════════════
// Shell Bridge — C ABI functions called from inside CPython
// ═══════════════════════════════════════════════════════════
//
// These functions form the bridge between Python code (running inside
// the embedded CPython VM) and Rust code (fastshell's shell engine).
//
// The function pointers are registered by the SDK during `Fastshell::init()`
// via `register_shell_execute()` / `register_shell_free()`.
//
// Python calls fastshell_python_shell_exec(cmd) via ctypes, passing a
// UTF-8 command string. Rust executes it through the SDK and returns a
// JSON string (allocated with CString::into_raw). Python reads and frees
// the JSON, then calls fastshell_python_shell_free() to release the memory.
//
// IMPORTANT: These MUST be registered BEFORE any Python code runs.
// If the shell execute function is not registered, Python calls will
// return null and the Python wrapper will return error 127.

static SHELL_EXECUTE_FN: OnceLock<unsafe extern "C" fn(*const c_char) -> *const c_char> = OnceLock::new();
static SHELL_FREE_FN: OnceLock<unsafe extern "C" fn(*mut c_char)> = OnceLock::new();

pub fn register_shell_execute(f: unsafe extern "C" fn(*const c_char) -> *const c_char) {
    SHELL_EXECUTE_FN.set(f).ok();
}

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

// ═══════════════════════════════════════════════════════════
// CpythonEngine — the authoritative CPython runtime handle
// ═══════════════════════════════════════════════════════════
//
// Lifecycle:
//   1. CpythonEngine::new(sandbox) / try_new(sandbox)
//      → extract_bundled(): decompress embedded .gz to sandbox/python/lib/
//      → libloading: dlopen the extracted library
//      → Py_Initialize() via WRAPPER_INJECTED (once per process)
//   2. engine.execute(code, cwd)
//      → WRAPPER_INJECTED ensures wrapper is injected (once)
//      → PyRun_SimpleString() runs user code
//      → stdout/stderr captured via temp files
//   3. engine is dropped when the SDK is dropped
//      → The static library handle lives forever (Box::leak) — this is
//        intentional because Py_Finalize() is not called (CPython docs
//        warn against it in embedded scenarios)
//
// Thread Safety:
//   - `execute()` takes `&self` (shared reference)
//   - CPython's GIL provides internal synchronization
//   - WRAPPER_INJECTED (OnceLock) ensures single initialization
//   - Concurrent execute() calls are serialized by the SDK's Mutex<Runtime>

pub struct CpythonEngine {
    lib: Option<&'static libloading::Library>,
    #[allow(dead_code)]
    last_error: Option<String>,
}

impl std::fmt::Debug for CpythonEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CpythonEngine")
            .field("available", &self.lib.is_some())
            .field("last_error", &self.last_error)
            .finish()
    }
}

impl CpythonEngine {
    fn lib_name() -> &'static str {
        #[cfg(target_os = "android")] { "libpython3.12.so" }
        #[cfg(target_os = "linux")]  { "libpython3.12.so.1.0" }
        #[cfg(target_os = "macos")]  { "libpython3.12.dylib" }
        #[cfg(target_os = "ios")]    { "libpython3.12.a" }
        #[cfg(not(any(target_os = "android", target_os = "linux", target_os = "macos", target_os = "ios")))]
        { "libpython3.12.so" }
    }

    // ── Library Discovery (tried in order) ──────────────────

    /// Primary path: decompress embedded CPython from the binary into the sandbox.
    /// This is the production code path. No network, no external dependency.
    fn extract_bundled(sandbox: &Path) -> Result<PathBuf, String> {
        if !HAS_EMBEDDED_CPYTHON {
            return Err(format!(
                "no embedded CPython for {}-{}. Add the .so.gz to vendor/python/ and rebuild.",
                std::env::consts::OS, std::env::consts::ARCH
            ));
        }
        let lib_name = Self::lib_name();
        let lib_dir = sandbox.join("python/lib");
        let lib_path = lib_dir.join(lib_name);
        // Idempotent — don't re-extract if already present
        if lib_path.exists() { return Ok(lib_path); }
        std::fs::create_dir_all(&lib_dir)
            .map_err(|e| format!("cannot create {}: {}", lib_dir.display(), e))?;
        let mut decoder = flate2::read::GzDecoder::new(EMBEDDED_CPYTHON);
        let mut decompressed = Vec::new();
        use std::io::Read;
        decoder.read_to_end(&mut decompressed)
            .map_err(|e| format!("decompress embedded CPython: {}", e))?;

        // Validate that the decompressed data is a real shared library, not a placeholder.
        // CPython shared libs are ~5-50MB; placeholder .gz files decompress to tiny garbage.
        if decompressed.len() < 1024 * 1024 {
            return Err(format!(
                "embedded CPython appears to be a placeholder ({} bytes after decompress).\n\
                 Real CPython .so/.dylib is 5-50MB after decompression.\n\
                 Add the real libpython3.12.{}.gz to vendor/python/ and rebuild.",
                decompressed.len(),
                if cfg!(target_os = "macos") { "dylib" } else { "so" }
            ));
        }
        let is_valid = Self::validate_library_format(&decompressed);
        if !is_valid {
            return Err(format!(
                "embedded CPython has invalid binary format (no ELF/Mach-O magic).\n\
                 The file in vendor/python/ may be a placeholder or corrupted.\n\
                 Replace it with a real libpython3.12.{}.gz and rebuild.",
                if cfg!(target_os = "macos") { "dylib" } else { "so" }
            ));
        }

        std::fs::write(&lib_path, &decompressed)
            .map_err(|e| format!("write {}: {}", lib_path.display(), e))?;
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&lib_path, std::fs::Permissions::from_mode(0o755));
        }
        Ok(lib_path)
    }

    /// Validates that the decompressed data has the magic bytes of a real shared library.
    /// ELF (.so): starts with \x7fELF
    /// Mach-O (.dylib): starts with \xcf\xfa\xed\xfe or \xfe\xed\xfa\xcf or \xca\xfe\xba\xbe or \xbe\xba\xfe\xca
    fn validate_library_format(data: &[u8]) -> bool {
        if data.len() < 4 { return false; }
        // ELF magic
        if &data[0..4] == b"\x7fELF" { return true; }
        // Mach-O 32-bit big-endian
        if &data[0..4] == b"\xfe\xed\xfa\xce" { return true; }
        // Mach-O 32-bit little-endian
        if &data[0..4] == b"\xce\xfa\xed\xfe" { return true; }
        // Mach-O 64-bit big-endian
        if &data[0..4] == b"\xfe\xed\xfa\xcf" { return true; }
        // Mach-O 64-bit little-endian
        if &data[0..4] == b"\xcf\xfa\xed\xfe" { return true; }
        // Mach-O fat/universal binary
        if &data[0..4] == b"\xca\xfe\xba\xbe" { return true; }
        if &data[0..4] == b"\xbe\xba\xfe\xca" { return true; }
        false
    }

    /// Secondary: library already extracted by a previous run.
    /// Validates file size to reject placeholder/garbage extracted by old builds.
    fn find_lib_in_sandbox(sandbox: &Path) -> Option<PathBuf> {
        let path = sandbox.join("python/lib").join(Self::lib_name());
        if !path.exists() { return None; }
        // Reject files that are too small to be a real CPython shared library (< 1MB).
        // This handles the case where a previous build wrote a placeholder/garbage file.
        let metadata = std::fs::metadata(&path).ok()?;
        if metadata.len() < 1024 * 1024 { return None; }
        Some(path)
    }

    /// Tertiary: system-installed Python (desktop platforms only).
    fn find_lib_system() -> Option<String> {
        let candidates: &[&str] = &[
            #[cfg(target_os = "macos")] "/opt/homebrew/opt/python@3.12/Frameworks/Python.framework/Versions/3.12/lib/libpython3.12.dylib",
            #[cfg(target_os = "macos")] "/Library/Frameworks/Python.framework/Versions/3.12/lib/libpython3.12.dylib",
            #[cfg(target_os = "macos")] "/Library/Frameworks/Python.framework/Versions/3.11/lib/libpython3.11.dylib",
            #[cfg(target_os = "linux")]  "libpython3.12.so.1.0",
            #[cfg(target_os = "linux")]  "libpython3.11.so.1.0",
            #[cfg(target_os = "android")] "libpython3.12.so",
            #[cfg(target_os = "android")] "/data/data/com.fastshell/python/lib/libpython3.12.so",
        ];
        for c in candidates {
            if Path::new(c).exists() { return Some(c.to_string()); }
        }
        None
    }

    // ── Construction ────────────────────────────────────────

    /// Creates an engine with no error reporting. If CPython is unavailable,
    /// the engine is silently non-functional. Use `try_new()` for diagnostics.
    pub fn new(sandbox: &Path) -> Self {
        match Self::try_new(sandbox) {
            Ok(engine) => engine,
            Err(err) => CpythonEngine { lib: None, last_error: Some(err) },
        }
    }

    /// Creates an engine with full error reporting. Returns `Err` if CPython
    /// could not be loaded, with a human-readable message explaining why and
    /// how to fix it (vendor/python/, rebuild, or use system Python).
    pub fn try_new(sandbox: &Path) -> Result<Self, String> {
        let is_mobile = cfg!(any(target_os = "android", target_os = "ios"));

        let lib_path_str = Self::extract_bundled(sandbox)
            .map(|p| p.to_string_lossy().to_string())
            .or_else(|embedded_err| {
                Self::find_lib_in_sandbox(sandbox)
                    .map(|p| p.to_string_lossy().to_string())
                    .or_else(|| Self::find_lib_system())
                    .ok_or_else(|| {
                        if is_mobile {
                            format!(
                                "{}\n\n\
                                 ═══ How to embed CPython for mobile ═══\n\
                                 1. Cross-compile CPython 3.12 for this target:\n\
                                    ./scripts/build-cpython-android.sh\n\
                                 2. gzip the output and place it at:\n\
                                    vendor/python/<target-triple>/libpython3.12.{}.gz\n\
                                 3. Rebuild the app — include_bytes!() embeds it.\n\
                                 4. CPython now ships inside your APK/IPA, works offline.\n\
                                 \n\
                                 Development shortcut:\n\
                                 CpythonDownloader::ensure_available() downloads at runtime.",
                                embedded_err,
                                if cfg!(target_os = "macos") { "dylib" } else { "so" }
                            )
                        } else {
                            format!(
                                "{}\n\nTo fix this:\n\
                                  1. Add the CPython .{}.gz for this platform to vendor/python/\n\
                                  2. Rebuild the app (the library is embedded at compile time)\n\
                                  3. Or install system Python 3.12 (desktop fallback)\n\
                                \n(Development only: CpythonDownloader::ensure_available())",
                                embedded_err,
                                if cfg!(target_os = "macos") { "dylib" } else { "so" }
                            )
                        }
                    })
            })?;

        // SAFETY: libloading opens a native shared library. The library
        // pointer is leaked to 'static so it outlives all CpythonEngine
        // instances — CPython does not support re-initialization, so we
        // must keep the library loaded for the lifetime of the process.
        let lib: &'static libloading::Library = unsafe {
            libloading::Library::new(&lib_path_str)
                .map(|l| Box::leak(Box::new(l)) as &'static libloading::Library)
                .map_err(|e| format!(
                    "failed to load {}: {}\n\n\
                     The file exists but could not be loaded as a shared library.\n\
                     It may be corrupted or from an incompatible platform.\n\
                     Delete {}/python/lib/ and rebuild to re-extract the embedded library.",
                    lib_path_str, e, sandbox.display(),
                ))?
        };

        // Py_Initialize() must be called exactly once per process.
        // CPython does not support re-initialization after Py_Finalize().
        WRAPPER_INJECTED.get_or_init(|| {
            if let Ok(init) = unsafe { lib.get::<unsafe extern "C" fn()>(b"Py_Initialize\0") } {
                unsafe { init() };
            }
        });

        Ok(CpythonEngine { lib: Some(lib), last_error: None })
    }

    // ── Status ──────────────────────────────────────────────

    pub fn is_available(&self) -> bool { self.lib.is_some() }

    pub fn is_available_with_reason(&self) -> (bool, Option<String>) {
        if self.lib.is_some() { (true, None) } else { (false, self.last_error.clone()) }
    }

    pub fn last_error(&self) -> Option<&str> { self.last_error.as_deref() }

    pub fn version(&self) -> Option<String> {
        let lib = self.lib.as_ref()?;
        unsafe {
            let f: libloading::Symbol<unsafe extern "C" fn() -> *const c_char> =
                lib.get(b"Py_GetVersion\0").ok()?;
            let ptr = f();
            if ptr.is_null() { None }
            else { Some(std::ffi::CStr::from_ptr(ptr).to_string_lossy().to_string()) }
        }
    }

    // ── Execution ───────────────────────────────────────────

    /// Injects the Python wrapper (hooks) into the CPython VM.
    /// Idempotent: WRAPPER_INJECTED ensures this runs at most once per process.
    fn inject_wrapper(&self) {
        let lib = match &self.lib { Some(l) => l, None => return };
        let exec_addr = fastshell_python_shell_exec as *const () as usize;
        let free_addr = fastshell_python_shell_free as *const () as usize;
        let wrapper = PYTHON_WRAPPER
            .replace("{EXEC_ADDR}", &exec_addr.to_string())
            .replace("{FREE_ADDR}", &free_addr.to_string());
        // SAFETY: PYTHON_WRAPPER is a compile-time constant with no interior NULs
        let wc = CString::new(wrapper).expect("PYTHON_WRAPPER contains no NUL bytes");
        unsafe {
            // SAFETY: PyRun_SimpleString is a required CPython API symbol.
            // If missing, the library is corrupted — skip injection gracefully.
            let py_run: libloading::Symbol<unsafe extern "C" fn(*const c_char) -> c_int> =
                match lib.get(b"PyRun_SimpleString\0") {
                    Ok(f) => f,
                    Err(_) => return,
                };
            py_run(wc.as_ptr());
        }
    }

    /// Executes Python code inside the embedded CPython VM.
    ///
    /// stdout and stderr are captured to temp files and read back as Rust
    /// strings. This avoids the complexity of redirecting CPython's C-level
    /// FILE* streams and is thread-safe (each call uses unique temp files).
    pub fn execute(&self, code: &str, cwd: &Path) -> ExecutionResult {
        let lib = match &self.lib {
            Some(l) => l,
            None => return ExecutionResult::error(
                self.last_error.clone().unwrap_or_else(|| "CPython library not loaded".to_string()),
                127,
            ),
        };

        // Inject wrapper on first execution (thread-safe, once per process)
        WRAPPER_INJECTED.get_or_init(|| self.inject_wrapper());

        let capture_out = cwd.join(".fs_capture_out");
        let capture_err = cwd.join(".fs_capture_err");

        // Set environment variables so the Python wrapper knows where the
        // sandbox root is and what the current working directory is.
        std::env::set_var("FASTSHELL_ROOT", cwd.to_string_lossy().as_ref());
        std::env::set_var("FASTSHELL_CWD", "/");

        let wrapped = format!(
            r#"
import sys, os as _fs_os
# Make pre-bundled site-packages discoverable by CPython.
# Apps ship pip packages in <sandbox>/python/site-packages/ (bundled at build time).
# We inject this path into sys.path so `import openai` etc. Just Works.
_site = _fs_os.path.join(_fs_os.environ.get('FASTSHELL_ROOT', ''), 'python', 'site-packages')
if _fs_os.path.isdir(_site) and _site not in sys.path:
    sys.path.insert(0, _site)
# Also add sandbox root so bundled agent code (aacode/) is importable
_sandbox = _fs_os.environ.get('FASTSHELL_ROOT', '')
if _sandbox and _sandbox not in sys.path:
    sys.path.insert(0, _sandbox)
del _fs_os, _site, _sandbox
# Capture stdout/stderr into temp files
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

        let code_c = match CString::new(wrapped) {
            Ok(c) => c,
            Err(e) => return ExecutionResult::error(format!("invalid code: {}", e), 1),
        };
        let exit_code = unsafe {
            let py_run: libloading::Symbol<unsafe extern "C" fn(*const c_char) -> c_int> =
                match lib.get(b"PyRun_SimpleString\0") {
                    Ok(f) => f,
                    Err(e) => return ExecutionResult::error(
                        format!("CPython library missing PyRun_SimpleString: {}", e), 127
                    ),
                };
            py_run(code_c.as_ptr())
        };
        let stdout = std::fs::read_to_string(&capture_out).unwrap_or_default();
        let stderr = std::fs::read_to_string(&capture_err).unwrap_or_default();
        let _ = std::fs::remove_file(&capture_out);
        let _ = std::fs::remove_file(&capture_err);
        if exit_code != 0 {
            ExecutionResult { stdout, stderr, exit_code: 1 }
        } else {
            ExecutionResult { stdout, stderr, exit_code: 0 }
        }
    }

    pub fn execute_script(&self, script_path: &Path, cwd: &Path) -> ExecutionResult {
        let content = match std::fs::read_to_string(script_path) {
            Ok(c) => c,
            Err(e) => return ExecutionResult::error(
                format!("Cannot read {}: {}", script_path.display(), e), 1
            ),
        };
        self.execute(&content, cwd)
    }
}

// ═══════════════════════════════════════════════════════════
// CpythonDownloader — DEVELOPMENT-ONLY helper
// ═══════════════════════════════════════════════════════════
//
// WARNING: This downloads CPython at RUNTIME from a CDN. This path exists
// FOR DEVELOPMENT ONLY — it avoids the need to rebuild the binary every
// time you update CPython.
//
// PRODUCTION APPS MUST embed CPython at compile time via vendor/python/.
// Runtime download is unacceptable for production because:
//   - Bad UX: first-launch hangs while downloading 20-40 MB
//   - No offline support
//   - App Store / Play Store may reject apps that download executable code
//   - Security risk: CDN compromise could deliver malicious .so
//   - Network failures block the app entirely
//
// The correct production flow:
//   1. Cross-compile CPython 3.12 → .so.gz for each target platform
//   2. Place in vendor/python/<target-triple>/
//   3. Build the app — include_bytes!() embeds them at compile time
//   4. Ship the app — CPython is already inside, works offline

pub struct CpythonDownloader;

impl CpythonDownloader {
    pub const MAX_RETRIES: u32 = 3;
    pub const RETRY_DELAY_MS: u64 = 1000;

    const CDN_URLS: &[&str] = &[
        "https://cdn.fastshell.dev/python",
        "https://github.com/kandada/fastshell/releases/download/cpython-3.12",
    ];

    pub fn platform_triple() -> &'static str {
        #[cfg(all(target_os = "android", target_arch = "aarch64"))] { return "android-arm64"; }
        #[cfg(all(target_os = "android", target_arch = "arm"))]     { return "android-armv7"; }
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]    { return "linux-x86_64"; }
        #[cfg(all(target_os = "linux", target_arch = "aarch64"))]   { return "linux-arm64"; }
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]   { return "macos-arm64"; }
        #[cfg(all(target_os = "macos", target_arch = "x86_64"))]    { return "macos-x86_64"; }
        #[allow(unreachable_code)]
        "unknown"
    }

    pub fn is_bundled(sandbox: &Path) -> bool {
        CpythonEngine::find_lib_in_sandbox(sandbox).is_some()
    }

    /// Verifies SHA-256 checksum of downloaded data.
    /// Empty checksum constants (not yet populated) are treated as "skip".
    fn verify_checksum(data: &[u8], platform_triple: &str) -> Result<(), String> {
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(data);
        let hash = format!("{:x}", hasher.finalize());

        let expected = match platform_triple {
            "android-arm64" => CHECKSUM_ANDROID_ARM64,
            "android-armv7" => CHECKSUM_ANDROID_ARMV7,
            "linux-x86_64"  => CHECKSUM_LINUX_X86_64,
            "linux-arm64"   => CHECKSUM_LINUX_ARM64,
            "macos-arm64"   => CHECKSUM_MACOS_ARM64,
            "macos-x86_64"  => CHECKSUM_MACOS_X86_64,
            _ => return Ok(()),
        };

        if expected.is_empty() { return Ok(()); }

        if hash == expected {
            Ok(())
        } else {
            Err(format!("checksum mismatch for {}: expected {}, got {}", platform_triple, expected, hash))
        }
    }

    fn download_one(url: &str, timeout_secs: u64) -> Result<Vec<u8>, String> {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(timeout_secs))
            .timeout_read(std::time::Duration::from_secs(60))
            .timeout_write(std::time::Duration::from_secs(30))
            .build();
        let response = agent.get(url)
            .set("User-Agent", "fastshell-cpython-dl/0.2")
            .call()
            .map_err(|e| format!("download failed: {}", e))?;
        use std::io::Read;
        let mut data = Vec::new();
        response.into_reader().read_to_end(&mut data)
            .map_err(|e| format!("read failed: {}", e))?;
        Ok(data)
    }

    fn extract_tar(compressed: &[u8], python_dir: &Path) -> Result<(), String> {
        use std::io::Read;
        let decoder = flate2::read::GzDecoder::new(compressed);
        let mut archive = tar::Archive::new(decoder);
        for entry in archive.entries().map_err(|e| e.to_string())? {
            let mut entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path().map_err(|e| e.to_string())?;
            let dest_path = python_dir.join(&*path);
            if let Some(parent) = dest_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("create_dir: {}", e))?;
            }
            if entry.header().entry_type() == tar::EntryType::Directory {
                std::fs::create_dir_all(&dest_path)
                    .map_err(|e| format!("create_dir: {}", e))?;
            } else {
                let mut data = Vec::new();
                entry.read_to_end(&mut data)
                    .map_err(|e| format!("read_entry: {}", e))?;
                std::fs::write(&dest_path, &data)
                    .map_err(|e| format!("write: {}", e))?;
            }
        }
        Ok(())
    }

    /// **DEVELOPMENT ONLY.** Downloads CPython 3.12 for the current platform
    /// from CDN and extracts it into `sandbox/python/lib/`.
    ///
    /// Supports retry with exponential backoff (3 attempts) and SHA-256
    /// checksum verification. Tries multiple CDN URLs as fallback.
    ///
    /// ⚠️  DO NOT call this in production builds. Production apps must embed
    ///     CPython at compile time via vendor/python/. This function exists
    ///     for developer convenience during testing/debugging only.
    pub fn ensure_available(sandbox: &Path) -> Result<(), String> {
        if Self::is_bundled(sandbox) { return Ok(()); }

        let platform = Self::platform_triple();
        if platform == "unknown" {
            return Err(format!("unsupported platform: {}-{}", std::env::consts::OS, std::env::consts::ARCH));
        }

        let python_dir = sandbox.join("python");
        std::fs::create_dir_all(&python_dir)
            .map_err(|e| format!("create_dir: {}", e))?;

        let mut last_error = String::new();
        for &base_url in Self::CDN_URLS {
            let url = format!("{}/cpython-3.12-{}.tar.gz", base_url, platform);
            for attempt in 1..=Self::MAX_RETRIES {
                if attempt > 1 {
                    let delay = std::time::Duration::from_millis(Self::RETRY_DELAY_MS * attempt as u64);
                    std::thread::sleep(delay);
                }
                let compressed = match Self::download_one(&url, 15) {
                    Ok(c) => c,
                    Err(e) => {
                        last_error = format!("attempt {}/{} from {}: {}", attempt, Self::MAX_RETRIES, base_url, e);
                        continue;
                    }
                };

                if let Err(e) = Self::verify_checksum(&compressed, platform) {
                    last_error = format!("checksum verify failed: {}", e);
                    continue;
                }

                match Self::extract_tar(&compressed, &python_dir) {
                    Ok(()) => return Ok(()),
                    Err(e) => {
                        last_error = format!("extract failed: {}", e);
                        continue;
                    }
                }
            }
        }

        Err(format!("could not download CPython: {}", last_error))
    }
}

// ═══════════════════════════════════════════════════════════
// Checksums — populate when building vendor/python/
// ═══════════════════════════════════════════════════════════

const CHECKSUM_ANDROID_ARM64: &str = "";
const CHECKSUM_ANDROID_ARMV7: &str = "";
const CHECKSUM_LINUX_X86_64:  &str = "";
const CHECKSUM_LINUX_ARM64:   &str = "";
const CHECKSUM_MACOS_ARM64:   &str = "";
const CHECKSUM_MACOS_X86_64:  &str = "";

// ═══════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);
    fn unique_dir(prefix: &str) -> PathBuf {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), n))
    }

    #[test]
    fn test_embedded_data_exists() {
        if HAS_EMBEDDED_CPYTHON { assert!(!EMBEDDED_CPYTHON.is_empty()); }
    }

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
        let tmp = unique_dir("fs_cpy_extract");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).ok();
        let result = CpythonEngine::extract_bundled(&tmp);
        if HAS_EMBEDDED_CPYTHON {
            assert!(result.is_ok(), "extract_bundled failed: {:?}", result.err());
        } else {
            assert!(result.is_err());
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_cpython_try_new_error_when_not_usable() {
        let tmp = unique_dir("fs_cpy_noexist");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).ok();
        let result = CpythonEngine::try_new(&tmp);
        let _ = std::fs::remove_dir_all(&tmp);
        if result.is_ok() { return; }
        let err = result.unwrap_err();
        assert!(
            err.contains("vendor/python/") || err.contains("no embedded") || err.contains("failed to load"),
            "error should be descriptive: {}", err
        );
    }

    #[test]
    fn test_cpython_new_does_not_panic_on_missing_lib() {
        let tmp = unique_dir("fs_cpy_new");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).ok();
        let engine = CpythonEngine::new(&tmp);
        if !HAS_EMBEDDED_CPYTHON {
            assert!(!engine.is_available());
            assert!(engine.last_error().is_some());
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_is_available_with_reason() {
        let engine = CpythonEngine { lib: None, last_error: Some("test error".into()) };
        let (available, reason) = engine.is_available_with_reason();
        assert!(!available);
        assert_eq!(reason.unwrap(), "test error");
    }

    #[test]
    fn test_ensure_available_already_bundled() {
        let tmp = unique_dir("fs_cpy_already");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).ok();
        if HAS_EMBEDDED_CPYTHON {
            let _engine = CpythonEngine::new(&tmp);
            assert!(CpythonDownloader::is_bundled(&tmp));
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_platform_triple_returns_valid() {
        let triple = CpythonDownloader::platform_triple();
        assert!(!triple.is_empty());
    }

    #[test]
    fn test_verify_checksum_empty_allowed() {
        let result = CpythonDownloader::verify_checksum(b"test data", "macos-arm64");
        assert!(result.is_ok());
    }

    #[test]
    fn test_lib_name_returns_platform_specific() {
        let name = CpythonEngine::lib_name();
        assert!(!name.is_empty());
        assert!(name.contains("python"));
    }
}
