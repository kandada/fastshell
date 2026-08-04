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

use super::{ExecutionResult, PythonEngine};
use std::path::Path;
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};

use rustpython_vm as vm;
use vm::{AsObject, Interpreter};

/// Python wrapper that isolates one execution inside the persistent
/// interpreter. The user code + cwd are injected as scope globals (never via
/// string formatting, so arbitrary code/quotes are safe).
const WRAPPER: &str = r#"
import sys, io, os, types, traceback
sys.stdout = io.StringIO()
sys.stderr = io.StringIO()
__aacode_exit = 0
__aacode_prev_cwd = os.getcwd()
try:
    if __aacode_cwd:
        os.chdir(__aacode_cwd)
        # Allow `import localmodule` from the sandbox, like `python3 script.py`.
        if __aacode_cwd not in sys.path:
            sys.path.insert(0, __aacode_cwd)
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
    exec(compile(__aacode_code, __aacode_file, 'exec'), __aacode_main.__dict__)
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
        sys.modules.pop('__main__', None)
        import gc
        gc.collect()
    except Exception:
        pass
"#;

/// One queued Python execution.
struct Job {
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
                    run_one(&interp, &job.code, &job.file_label, &job.cwd)
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
fn run_one(interp: &Interpreter, code: &str, file_label: &str, cwd_str: &str) -> ExecutionResult {
    interp.enter(|vm| {
        let scope = vm.new_scope_with_builtins();

        // Inject inputs as plain globals (no string interpolation).
        let set = |name: &str, value: vm::PyObjectRef| {
            scope.globals.set_item(name, value, vm).map_err(|_| ()).ok();
        };
        set("__aacode_code", vm.ctx.new_str(code).into());
        set("__aacode_cwd", vm.ctx.new_str(cwd_str).into());
        set("__aacode_file", vm.ctx.new_str(file_label).into());

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
        let job = Job {
            code: code.to_string(),
            file_label: file_label.to_string(),
            cwd: cwd.to_string_lossy().to_string(),
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
        Some("Python 3.13 (RustPython 0.5.0, embedded)".to_string())
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
        assert!(out.stdout.contains("UNITTEST-DONE"), "stdout={}", out.stdout);
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
        assert!(out.stdout.contains('2'), "stale module cache: {}", out.stdout);
    }

    #[test]
    fn memory_stays_flat_across_runs() {
        // The persistent-interpreter design must not grow per execution.
        fn rss_kb() -> u64 {
            let out = std::process::Command::new("ps")
                .args(["-o", "rss=", "-p", &std::process::id().to_string()])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).trim().parse().unwrap_or(0)
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
