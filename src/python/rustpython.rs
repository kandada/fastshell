// Copyright (c) 2026 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Embedded RustPython engine (feature `python-rustpython`).
//!
//! Replaces the former Chaquopy CPython integration on mobile. Pure Rust:
//! no dlopen, no JNI, no C TLS — the entire class of in-process CPython
//! crashes (pthread/bionic incompatibilities) is eliminated by construction.
//!
//! Design (process-level persistent interpreter):
//!   * RustPython interpreter instances leak ~20 MB each (internal reference
//!     cycles are not reclaimed on drop), so a fresh-interpreter-per-exec
//!     model is not viable. Instead, ONE interpreter lives on a dedicated
//!     big-stack worker thread for the whole process; all engines submit
//!     jobs through a channel. Memory stays flat (~45 MB one-time).
//!   * Isolation between executions comes from the wrapper script: a fresh
//!     `__main__` module + fresh scope per run, stdout/stderr captured into
//!     new StringIO objects, cwd switched and restored, `sys.argv` reset,
//!     user modules imported from the sandbox evicted from `sys.modules`
//!     afterwards, and `gc.collect()` at the end.
//!   * The frozen stdlib (`rustpython-pylib` + `freeze-stdlib`) is compiled
//!     into the binary — no on-disk stdlib, no PYTHONHOME. Stdlib modules
//!     stay cached across runs (imports are fast after first use).
//!   * Python executions serialize process-wide (single interpreter) — same
//!     contract as the previous CPython engine; concurrent agent tasks
//!     queue for Python while everything else runs in parallel.

use super::{ExecutionResult, PythonEngine, PY_COMPAT_PRELUDE};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};

use rustpython_vm as vm;
use vm::{AsObject, Interpreter};

/// Python wrapper that isolates one execution inside the persistent
/// interpreter. The user code + cwd are injected as scope globals (never via
/// string formatting, so arbitrary code/quotes are safe).
use super::SANDBOX_WRAPPER;

const WRAPPER: &str = r#"
import sys, io, os, types, traceback
sys.stdout = io.StringIO()
sys.stderr = io.StringIO()
__aacode_exit = 0
# Reset any sandbox patches left by a previous run in this persistent
# interpreter, so the WRAPPER's own file calls use the REAL functions
# (otherwise os.chdir(real_cwd) gets re-translated → doubled path).
try:
    import builtins as _rb, shutil as _rs, io as _rio
    if hasattr(os, '_fs_orig'):
        for _rk, _rv in list(os._fs_orig.items()):
            if _rk.startswith('shutil_'):
                setattr(_rs, _rk[7:], _rv)
            else:
                setattr(os, _rk, _rv)
    if hasattr(_rb, '_fs_orig_open'):
        _rb.open = _rb._fs_orig_open
    if hasattr(_rio, '_fs_orig_open'):
        _rio.open = _rio._fs_orig_open
    del _rb, _rs, _rio
except Exception:
    pass
__aacode_prev_cwd = os.getcwd()
try:
    if __aacode_cwd:
        os.chdir(__aacode_cwd)
        # Allow `import localmodule` from the sandbox, like `python3 script.py`.
        if __aacode_cwd not in sys.path:
            sys.path.insert(0, __aacode_cwd)
        # Project-local site-packages (pip-install target). `pip install`
        # writes pure-Python wheels into <cwd>/site-packages; make them
        # importable without manual sys.path fiddling.
        __aacode_sp = os.path.join(__aacode_cwd, 'site-packages')
        if os.path.isdir(__aacode_sp) and __aacode_sp not in sys.path:
            sys.path.insert(0, __aacode_sp)
    # argv[0] like `python3 script.py` (unittest.main/argparse read it).
    sys.argv = [__aacode_file]
    # Register a real __main__ module so `import __main__` (unittest.main,
    # pickle, argparse prog detection, ...) works like `python3 script.py`.
    __aacode_main = types.ModuleType('__main__')
    __aacode_main.__file__ = __aacode_file
    sys.modules['__main__'] = __aacode_main
    # Pipeline stdin: when a pipe (e.g. `echo x | python3 script.py`) feeds
    # the interpreter, the executor writes the captured stdout to a temp file
    # in the sandbox cwd and the wrapper opens it as sys.stdin.
    try:
        if os.path.exists('_py_stdin'):
            sys.stdin = open('_py_stdin')
    except OSError:
        pass
    # Compatibility dialect (py2 builtins + legacy module aliases), best-effort.
    try:
        exec(compile(__aacode_prelude, '<compat>', 'exec'), __aacode_main.__dict__)
    except BaseException:
        pass
    # Sandbox path translation: resolve the shell's virtual root (`/projects/...`)
    # and relative paths under FASTSHELL_ROOT (matches the shell VFS).
    try:
        os.environ['FASTSHELL_ROOT'] = __aacode_root
        os.environ['FASTSHELL_CWD'] = __aacode_logical_cwd
        exec(compile(__aacode_sandbox, '<sandbox>', 'exec'), __aacode_main.__dict__)
    except BaseException:
        pass
    # sqlite3: RustPython ships no `_sqlite3` on mobile, so bridge to the native
    # rusqlite implementation via a pure-Python `sqlite3` shim.
    try:
        import sqlite3 as _fs_real_sqlite3
    except BaseException:
        try:
            import types as _fs_t, sys as _fs_s
            _fs_m = _fs_t.ModuleType("sqlite3")
            exec(compile(__aacode_sqlite_shim, "<sqlite3>", "exec"), _fs_m.__dict__)
            _fs_s.modules["sqlite3"] = _fs_m
        except BaseException:
            pass
    exec(compile(
        __aacode_code,
        __aacode_file,
        'exec',
    ), __aacode_main.__dict__)
except SystemExit as e:
    c = e.code
    __aacode_exit = c if isinstance(c, int) else (0 if c is None else 1)
except BaseException:
    traceback.print_exc()
    __aacode_exit = 1
finally:
    try:
        os.chdir(__aacode_prev_cwd)
    except OSError:
        pass
    # Cleanup so the persistent interpreter stays fresh + memory-flat:
    # evict user modules loaded from the sandbox (stale code otherwise
    # shadows edited files on the next run), drop __main__, collect cycles.
    try:
        if __aacode_cwd:
            for __aacode_m in [k for k, v in list(sys.modules.items())
                               if getattr(v, '__file__', None)
                               and str(getattr(v, '__file__', '')).startswith(__aacode_cwd)]:
                del sys.modules[__aacode_m]
            if __aacode_cwd in sys.path:
                sys.path.remove(__aacode_cwd)
            # Symmetric cleanup for the injected site-packages path so a
            # different project's site-packages never leaks into sys.path of
            # the persistent interpreter.
            __aacode_sp = os.path.join(__aacode_cwd, 'site-packages')
            if __aacode_sp in sys.path:
                sys.path.remove(__aacode_sp)
        sys.modules.pop('__main__', None)
        import gc
        gc.collect()
    except Exception:
        pass
"#;

/// One queued Python execution.
struct Job {
    root: String,
    logical_cwd: String,
    code: String,
    file_label: String,
    cwd: String,
    reply: mpsc::Sender<ExecutionResult>,
}

/// Handle to the process-wide interpreter worker. Wrapped in a Mutex so a
/// dead worker (poisoned/panicked thread) can be respawned transparently.
static WORKER: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();

fn worker_sender() -> mpsc::Sender<Job> {
    let slot = WORKER.get_or_init(|| Mutex::new(spawn_worker()));
    let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
    // Respawn if the previous worker thread died (channel disconnected).
    let (probe_tx, _probe_rx) = mpsc::channel();
    let alive = guard
        .send(Job {
            code: String::new(),
            file_label: String::new(),
            cwd: String::new(),
            root: String::new(),
            logical_cwd: String::new(),
            reply: probe_tx,
        })
        .is_ok();
    if !alive {
        *guard = spawn_worker();
    }
    guard.clone()
}

/// Spawns the persistent interpreter thread (16 MB stack — the interpreter
/// recurses deeply, especially in debug builds; host threads often have
/// small stacks: 2 MB Rust test threads, ~1 MB Android JNI threads).
fn spawn_worker() -> mpsc::Sender<Job> {
    let (tx, rx) = mpsc::channel::<Job>();
    std::thread::Builder::new()
        .name("rustpython-worker".to_string())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            let interp = build_interpreter();
            for job in rx {
                if job.code.is_empty() && job.file_label.is_empty() {
                    // Liveness probe — reply not expected.
                    continue;
                }
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_one(
                        &interp,
                        &job.code,
                        &job.file_label,
                        &job.cwd,
                        &job.root,
                        &job.logical_cwd,
                    )
                }))
                .unwrap_or_else(|_| {
                    ExecutionResult::error(
                        "python: interpreter panicked while executing\n".to_string(),
                        1,
                    )
                });
                let _ = job.reply.send(result);
            }
        })
        .expect("rustpython worker thread spawn failed");
    tx
}

fn build_interpreter() -> Interpreter {
    let settings = vm::Settings::default();
    let builder = Interpreter::builder(settings);
    let defs = rustpython_stdlib::stdlib_module_defs(&builder.ctx);
    builder
        .add_native_modules(&defs)
        .add_frozen_modules(rustpython_pylib::FROZEN_STDLIB)
        .build()
}

/// Executes one job inside the persistent interpreter.
fn run_one(
    interp: &Interpreter,
    code: &str,
    file_label: &str,
    cwd_str: &str,
    root: &str,
    logical_cwd: &str,
) -> ExecutionResult {
    interp.enter(|vm| {
        let scope = vm.new_scope_with_builtins();

        // Inject inputs as plain globals (no string interpolation).
        let set = |name: &str, value: vm::PyObjectRef| {
            scope.globals.set_item(name, value, vm).map_err(|_| ()).ok();
        };
        set("__aacode_code", vm.ctx.new_str(code).into());
        set("__aacode_cwd", vm.ctx.new_str(cwd_str).into());
        set("__aacode_file", vm.ctx.new_str(file_label).into());
        set("__aacode_prelude", vm.ctx.new_str(PY_COMPAT_PRELUDE).into());
        set("__aacode_sandbox", vm.ctx.new_str(SANDBOX_WRAPPER).into());
        set("__aacode_root", vm.ctx.new_str(root).into());
        set("__aacode_sqlite_shim", vm.ctx.new_str(SQLITE_SHIM).into());
        // Native sqlite bridge (rusqlite) exposed as a builtin so the pure-Python
        // `sqlite3` shim can call it from any module.
        let sqlite_fn = vm.new_function("_fs_sqlite", fs_sqlite_native);
        let _ = vm.builtins.set_attr("_fs_sqlite", sqlite_fn, vm);
        set("__aacode_logical_cwd", vm.ctx.new_str(logical_cwd).into());

        let run_result = vm.run_string(scope.clone(), WRAPPER, "<aacode-wrapper>".to_owned());

        // Collect captured stdout/stderr regardless of outcome.
        let read_stream = |name: &'static str| -> String {
            (|| -> Option<String> {
                let sys = vm.import("sys", 0).ok()?;
                let stream = sys.get_attr(name, vm).ok()?;
                let getvalue = stream.get_attr("getvalue", vm).ok()?;
                let value = getvalue.call((), vm).ok()?;
                value.str(vm).ok().map(|s| s.to_string())
            })()
            .unwrap_or_default()
        };
        let stdout = read_stream("stdout");
        let mut stderr = read_stream("stderr");

        let exit_code = match run_result {
            Ok(_) => scope
                .globals
                .get_item("__aacode_exit", vm)
                .ok()
                .and_then(|v| v.try_to_value::<i32>(vm).ok())
                .unwrap_or(0),
            Err(exc) => {
                // The wrapper itself failed (should be rare): format the
                // exception into stderr.
                let mut msg = String::new();
                if let Ok(s) = exc.as_object().str(vm) {
                    msg.push_str(&s.to_string());
                }
                if !stderr.is_empty() && !stderr.ends_with('\n') {
                    stderr.push('\n');
                }
                stderr.push_str("wrapper error: ");
                stderr.push_str(&msg);
                stderr.push('\n');
                1
            }
        };

        ExecutionResult {
            stdout,
            stderr,
            exit_code,
        }
    })
}

/// Embedded RustPython interpreter engine (thin handle to the shared worker).
#[derive(Debug, Default)]
pub struct RustPythonEngine;

impl RustPythonEngine {
    pub fn new() -> Self {
        RustPythonEngine
    }

    fn run(&self, code: &str, file_label: &str, cwd: &Path) -> ExecutionResult {
        let (reply_tx, reply_rx) = mpsc::channel();
        let (root, logical_cwd) = crate::python::python_sandbox().unwrap_or_default();
        let job = Job {
            code: code.to_string(),
            file_label: file_label.to_string(),
            cwd: cwd.to_string_lossy().to_string(),
            root,
            logical_cwd,
            reply: reply_tx,
        };
        if worker_sender().send(job).is_err() {
            return ExecutionResult::error(
                "python: interpreter worker unavailable\n".to_string(),
                1,
            );
        }
        reply_rx.recv().unwrap_or_else(|_| {
            ExecutionResult::error(
                "python: interpreter worker terminated unexpectedly\n".to_string(),
                1,
            )
        })
    }
}

impl PythonEngine for RustPythonEngine {
    fn execute(&mut self, code: &str, cwd: &Path) -> ExecutionResult {
        self.run(code, "<string>", cwd)
    }

    fn execute_script(&mut self, script_path: &Path, cwd: &Path) -> ExecutionResult {
        // Resolve relative to cwd like `python3 script.py` would.
        let resolved = if script_path.is_absolute() {
            script_path.to_path_buf()
        } else {
            cwd.join(script_path)
        };
        let code = match std::fs::read_to_string(&resolved) {
            Ok(c) => c,
            Err(e) => {
                return ExecutionResult::error(
                    format!(
                        "python3: can't open file '{}': {}\n",
                        script_path.display(),
                        e
                    ),
                    2,
                )
            }
        };
        self.run(&code, &resolved.to_string_lossy(), cwd)
    }

    fn is_available(&self) -> bool {
        true
    }

    fn version(&self) -> Option<String> {
        // Derive the Python version from RustPython itself so it can't drift.
        Some(format!(
            "Python {} (RustPython 0.5.0, embedded)",
            rustpython_vm::version::get_version_number()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("rp_engine_{}_{}", tag, std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        d
    }

    #[test]
    fn basic_print() {
        let mut e = RustPythonEngine::new();
        let out = e.execute("print(6*7)", &tmp_dir("print"));
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert_eq!(out.stdout.trim(), "42");
    }

    #[test]
    fn stderr_and_exit_code_on_exception() {
        let mut e = RustPythonEngine::new();
        let out = e.execute("raise ValueError('boom')", &tmp_dir("exc"));
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("ValueError"), "stderr={}", out.stderr);
        assert!(out.stderr.contains("boom"));
    }

    #[test]
    fn system_exit_code() {
        let mut e = RustPythonEngine::new();
        let out = e.execute("import sys; sys.exit(3)", &tmp_dir("exit"));
        assert_eq!(out.exit_code, 3);
    }

    #[test]
    fn ast_module_works() {
        // The agent's syntax-check idiom must work.
        let mut e = RustPythonEngine::new();
        let out = e.execute(
            "import ast; ast.parse('def f(x):\\n    return x*2'); print('OK')",
            &tmp_dir("ast"),
        );
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert!(out.stdout.contains("OK"));
    }

    #[test]
    fn json_re_os_work() {
        let mut e = RustPythonEngine::new();
        let out = e.execute(
            r#"
import json, re, os
d = json.loads('{"a": [1, 2, 3]}')
assert d["a"][2] == 3
assert re.match(r"\d+", "123abc").group() == "123"
print("stdlib-ok", len(os.listdir(".")) >= 0)
"#,
            &tmp_dir("stdlib"),
        );
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert!(out.stdout.contains("stdlib-ok"));
    }

    #[test]
    fn cwd_is_respected_and_files_work() {
        let dir = tmp_dir("cwd");
        let mut e = RustPythonEngine::new();
        let out = e.execute(
            "open('rp_out.txt', 'w').write('hello-rp')\nprint(open('rp_out.txt').read())",
            &dir,
        );
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert!(out.stdout.contains("hello-rp"));
        assert!(dir.join("rp_out.txt").exists());
        let _ = std::fs::remove_file(dir.join("rp_out.txt"));
    }

    #[test]
    fn unittest_runs() {
        // The core requirement that broke Chaquopy on-device.
        let mut e = RustPythonEngine::new();
        let out = e.execute(
            r#"
import unittest

class TestMath(unittest.TestCase):
    def test_add(self):
        self.assertEqual(2 + 2, 4)
    def test_str(self):
        self.assertIn("py", "rustpython")

unittest.main(exit=False, verbosity=1)
print("UNITTEST-DONE")
"#,
            &tmp_dir("unittest"),
        );
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert!(
            out.stdout.contains("UNITTEST-DONE"),
            "stdout={}",
            out.stdout
        );
        assert!(
            out.stderr.contains("OK") || out.stdout.contains("OK"),
            "unittest result missing: stdout={} stderr={}",
            out.stdout,
            out.stderr
        );
    }

    #[test]
    fn execute_script_from_file() {
        let dir = tmp_dir("script");
        std::fs::write(dir.join("s.py"), "print('from-script', __name__)").unwrap();
        let mut e = RustPythonEngine::new();
        let out = e.execute_script(Path::new("s.py"), &dir);
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert!(out.stdout.contains("from-script __main__"));
    }

    #[test]
    fn bigint_arithmetic_via_shims() {
        // Exercises the clean-room malachite shims: big ints + true division.
        let mut e = RustPythonEngine::new();
        let out = e.execute(
            r#"
big = 10**100
assert big // (10**99) == 10
assert (10**400) / (10**399) == 10.0
assert 1/3 == 0.3333333333333333
assert (0.5).as_integer_ratio() == (1, 2)
print("BIGINT-OK", big % 97)
"#,
            &tmp_dir("bigint"),
        );
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert!(out.stdout.contains("BIGINT-OK"), "stdout={}", out.stdout);
    }

    #[test]
    fn executions_are_isolated() {
        // Persistent interpreter must NOT leak user globals between runs.
        let dir = tmp_dir("iso");
        let mut e = RustPythonEngine::new();
        let out = e.execute("leak_probe = 42; print('set')", &dir);
        assert_eq!(out.exit_code, 0);
        let out = e.execute(
            "print('leaked' if 'leak_probe' in dir() else 'clean')",
            &dir,
        );
        assert!(out.stdout.contains("clean"), "stdout={}", out.stdout);
    }

    #[test]
    fn local_module_import_and_eviction() {
        // `import localmod` must work from the sandbox cwd, and edited code
        // must be picked up on the next execution (no stale module cache).
        let dir = tmp_dir("localmod");
        std::fs::write(dir.join("localmod.py"), "VALUE = 1").unwrap();
        let mut e = RustPythonEngine::new();
        let out = e.execute("import localmod; print(localmod.VALUE)", &dir);
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert!(out.stdout.contains('1'), "stdout={}", out.stdout);

        std::fs::write(dir.join("localmod.py"), "VALUE = 2").unwrap();
        let out = e.execute("import localmod; print(localmod.VALUE)", &dir);
        assert!(
            out.stdout.contains('2'),
            "stale module cache: {}",
            out.stdout
        );
    }

    #[test]
    fn site_packages_is_importable() {
        // pip-install writes wheels into <cwd>/site-packages. The wrapper must
        // inject that dir into sys.path so installed packages import directly
        // without manual `sys.path.insert(0, 'site-packages')`.
        let dir = tmp_dir("sitepkg");
        let sp = dir.join("site-packages");
        std::fs::create_dir_all(&sp).unwrap();
        std::fs::write(sp.join("pkg_mod.py"), "VERSION = '1.0.0'\n").unwrap();

        let mut e = RustPythonEngine::new();
        let out = e.execute("import pkg_mod; print('OK', pkg_mod.VERSION)", &dir);
        assert_eq!(out.exit_code, 0, "stderr={}", out.stderr);
        assert!(
            out.stdout.contains("OK 1.0.0"),
            "site-packages module should import, stdout={} stderr={}",
            out.stdout,
            out.stderr
        );
        let _ = std::fs::remove_dir_all(&sp);
    }

    #[test]
    fn site_packages_path_does_not_leak_across_projects() {
        // The injected site-packages path must be removed after execution so a
        // different project's site-packages never shadows another's modules.
        let dir_a = tmp_dir("sitepkg_a");
        let dir_b = tmp_dir("sitepkg_b");
        let sp_a = dir_a.join("site-packages");
        let sp_b = dir_b.join("site-packages");
        std::fs::create_dir_all(&sp_a).unwrap();
        std::fs::create_dir_all(&sp_b).unwrap();
        std::fs::write(sp_a.join("sharedmod.py"), "WHO = 'A'\n").unwrap();
        std::fs::write(sp_b.join("sharedmod.py"), "WHO = 'B'\n").unwrap();

        let mut e = RustPythonEngine::new();
        // Project A sees its own module.
        let out_a = e.execute("import sharedmod; print(sharedmod.WHO)", &dir_a);
        assert!(
            out_a.stdout.contains("A"),
            "A should see its own module: {}",
            out_a.stdout
        );
        // Project B sees its own module (not A's leaked path).
        let out_b = e.execute("import sharedmod; print(sharedmod.WHO)", &dir_b);
        assert!(
            out_b.stdout.contains("B"),
            "B should see its own module: {}",
            out_b.stdout
        );

        let _ = std::fs::remove_dir_all(&sp_a);
        let _ = std::fs::remove_dir_all(&sp_b);
    }

    #[test]
    fn memory_stays_flat_across_runs() {
        // The persistent-interpreter design must not grow per execution.
        fn rss_kb() -> u64 {
            let out = std::process::Command::new("ps")
                .args(["-o", "rss=", "-p", &std::process::id().to_string()])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout)
                .trim()
                .parse()
                .unwrap_or(0)
        }
        let dir = tmp_dir("mem");
        let mut e = RustPythonEngine::new();
        for _ in 0..3 {
            e.execute("import json, re; print(1)", &dir);
        }
        let before = rss_kb();
        for _ in 0..15 {
            e.execute("import json, re, unittest; print(sum(range(10000)))", &dir);
        }
        let after = rss_kb();
        let grown_mb = (after.saturating_sub(before)) as f64 / 1024.0;
        assert!(
            grown_mb < 30.0,
            "memory grew {grown_mb:.0} MB over 15 executions (leak)"
        );
    }
}

// ── Python `sqlite3` bridge (rusqlite) ───────────────────────────────────
//
// RustPython's stdlib excludes `_sqlite3` on Android (`libsqlite3-sys` is
// cfg-gated out), so we expose one native function `__fs_sqlite` backed by
// fastshell's bundled rusqlite, and inject a pure-Python `sqlite3` shim that
// wraps it into a small DB-API subset.

thread_local! {
    static FS_SQLITE_CONNS: RefCell<HashMap<i64, rusqlite::Connection>> =
        RefCell::new(HashMap::new());
    static FS_SQLITE_NEXT: Cell<i64> = const { Cell::new(1) };
}

fn fs_json_to_sql(v: &serde_json::Value) -> rusqlite::types::Value {
    use rusqlite::types::Value as RV;
    match v {
        serde_json::Value::Null => RV::Null,
        serde_json::Value::Bool(b) => RV::Integer(if *b { 1 } else { 0 }),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                RV::Integer(i)
            } else if let Some(f) = n.as_f64() {
                RV::Real(f)
            } else {
                RV::Null
            }
        }
        serde_json::Value::String(s) => RV::Text(s.clone()),
        other => RV::Text(other.to_string()),
    }
}

fn fs_sql_to_json(v: rusqlite::types::ValueRef<'_>) -> serde_json::Value {
    use rusqlite::types::ValueRef as RV;
    match v {
        RV::Null => serde_json::Value::Null,
        RV::Integer(i) => serde_json::json!(i),
        RV::Real(f) => serde_json::json!(f),
        RV::Text(t) => serde_json::json!(String::from_utf8_lossy(t).to_string()),
        RV::Blob(b) => serde_json::json!(format!("<blob {} bytes>", b.len())),
    }
}

fn fs_sqlite_dispatch(op: &str, a: &str, b: &str, c: &str) -> String {
    match op {
        "connect" => {
            let conn = if a.is_empty() || a == ":memory:" {
                rusqlite::Connection::open_in_memory()
            } else {
                rusqlite::Connection::open(a)
            };
            match conn {
                Ok(conn) => {
                    let id = FS_SQLITE_NEXT.with(|n| {
                        let v = n.get();
                        n.set(v + 1);
                        v
                    });
                    FS_SQLITE_CONNS.with(|m| m.borrow_mut().insert(id, conn));
                    serde_json::json!({ "handle": id }).to_string()
                }
                Err(e) => serde_json::json!({ "error": e.to_string() }).to_string(),
            }
        }
        "close" => {
            if let Ok(id) = a.parse::<i64>() {
                FS_SQLITE_CONNS.with(|m| {
                    m.borrow_mut().remove(&id);
                });
            }
            "{}".to_string()
        }
        "commit" => {
            if let Ok(id) = a.parse::<i64>() {
                FS_SQLITE_CONNS.with(|m| {
                    if let Some(conn) = m.borrow().get(&id) {
                        let _ = conn.execute_batch("COMMIT;");
                    }
                });
            }
            "{}".to_string()
        }
        "version" => serde_json::json!({ "version": rusqlite::version() }).to_string(),
        "execute" => {
            let id = a.parse::<i64>().unwrap_or(0);
            let params: Vec<serde_json::Value> = serde_json::from_str(b).unwrap_or_default();
            FS_SQLITE_CONNS.with(|m| {
                let map = m.borrow();
                let Some(conn) = map.get(&id) else {
                    return serde_json::json!({ "error": "no such connection" }).to_string();
                };
                let mut stmt = match conn.prepare(c) {
                    Ok(s) => s,
                    Err(e) => return serde_json::json!({ "error": e.to_string() }).to_string(),
                };
                let cols: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
                let ncols = stmt.column_count();
                let rp: Vec<rusqlite::types::Value> = params.iter().map(fs_json_to_sql).collect();
                if ncols == 0 {
                    return match stmt.execute(rusqlite::params_from_iter(rp.iter())) {
                        Ok(n) => serde_json::json!({ "rows": [], "rowcount": n, "columns": [] })
                            .to_string(),
                        Err(e) => serde_json::json!({ "error": e.to_string() }).to_string(),
                    };
                }
                let mut rows = match stmt.query(rusqlite::params_from_iter(rp.iter())) {
                    Ok(r) => r,
                    Err(e) => return serde_json::json!({ "error": e.to_string() }).to_string(),
                };
                let mut out: Vec<Vec<serde_json::Value>> = Vec::new();
                loop {
                    match rows.next() {
                        Ok(Some(row)) => {
                            let mut r = Vec::with_capacity(ncols);
                            for i in 0..ncols {
                                r.push(
                                    row.get_ref(i)
                                        .map(fs_sql_to_json)
                                        .unwrap_or(serde_json::Value::Null),
                                );
                            }
                            out.push(r);
                        }
                        Ok(None) => break,
                        Err(e) => return serde_json::json!({ "error": e.to_string() }).to_string(),
                    }
                }
                serde_json::json!({ "rows": out, "rowcount": -1, "columns": cols }).to_string()
            })
        }
        _ => serde_json::json!({ "error": format!("unknown op {op}") }).to_string(),
    }
}

fn fs_sqlite_native(
    op: String,
    a: String,
    b: String,
    c: String,
    vm: &vm::VirtualMachine,
) -> vm::PyObjectRef {
    let out = fs_sqlite_dispatch(&op, &a, &b, &c);
    vm.ctx.new_str(out).into()
}

/// Pure-Python `sqlite3` shim (DB-API subset) over the native `__fs_sqlite`.
const SQLITE_SHIM: &str = r#"
import json as _fs_json
import sys as _fs_sys

class _FsError(Exception):
    pass

class _FsOperationalError(_FsError):
    pass

class _FsIntegrityError(_FsError):
    pass

class _FsProgrammingError(_FsError):
    pass

class _FsCursor:
    def __init__(self, conn):
        self.connection = conn
        self._rows = []
        self._idx = 0
        self.rowcount = -1
        self.description = None
        self.lastrowid = None
        self.arraysize = 1

    def execute(self, sql, params=()):
        r = _fs_json.loads(_fs_sqlite("execute", str(self.connection._h), _fs_json.dumps(list(params)), sql))
        if "error" in r:
            raise _FsOperationalError(r["error"])
        self._rows = r.get("rows", [])
        self._idx = 0
        self.rowcount = r.get("rowcount", -1)
        cols = r.get("columns", [])
        self.description = [(c, None, None, None, None, None, None) for c in cols] if cols else None
        return self

    def executemany(self, sql, seq):
        n = 0
        for p in seq:
            self.execute(sql, p)
            n += 1
        self.rowcount = n
        return self

    def executescript(self, script):
        for stmt in script.split(";"):
            if stmt.strip():
                self.execute(stmt)
        return self

    def fetchone(self):
        if self._idx < len(self._rows):
            row = self._rows[self._idx]
            self._idx += 1
            return tuple(row)
        return None

    def fetchmany(self, size=1):
        rows = self._rows[self._idx:self._idx + size]
        self._idx += len(rows)
        return [tuple(r) for r in rows]

    def fetchall(self):
        rows = self._rows[self._idx:]
        self._idx = len(self._rows)
        return [tuple(r) for r in rows]

    def close(self):
        pass

    def __iter__(self):
        return iter(self.fetchall())

class _FsConnection:
    def __init__(self, path=":memory:"):
        r = _fs_json.loads(_fs_sqlite("connect", str(path), "", ""))
        if "error" in r:
            raise _FsOperationalError(r["error"])
        self._h = r["handle"]
        self._closed = False

    def cursor(self):
        return _FsCursor(self)

    def execute(self, sql, params=()):
        return _FsCursor(self).execute(sql, params)

    def executemany(self, sql, seq):
        return _FsCursor(self).executemany(sql, seq)

    def executescript(self, script):
        return _FsCursor(self).executescript(script)

    def commit(self):
        _fs_sqlite("commit", str(self._h), "", "")

    def rollback(self):
        pass

    def close(self):
        if not self._closed:
            _fs_sqlite("close", str(self._h), "", "")
            self._closed = True

    def __enter__(self):
        return self

    def __exit__(self, *a):
        self.commit()

def connect(path=":memory:", *a, **k):
    return _FsConnection(path)

try:
    sqlite_version = _fs_json.loads(_fs_sqlite("version", "", "", ""))["version"]
except Exception:
    sqlite_version = "3"

version = "2.6.0"
paramstyle = "qmark"
Error = _FsError
DatabaseError = _FsError
OperationalError = _FsOperationalError
IntegrityError = _FsIntegrityError
ProgrammingError = _FsProgrammingError
Warning = Warning
"#;
