// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Mobile-only Python pipeline path: `PRODUCER | python3 -c '...'` and
//! `PRODUCER | python3 script.py`.
//!
//! On mobile `allow_subprocess` is false, so the threaded pipeline path cannot
//! run Python. The executor instead runs the producer, writes its stdout to
//! `_py_stdin` in the sandbox cwd, and the embedded RustPython wrapper opens it
//! as `sys.stdin` (see `src/bridge/executor.rs::try_pipe_into_python` and
//! `src/python/rustpython.rs`). That path is only exercised with the embedded
//! engine, so this file forces `FASTSHELL_PYTHON=rustpython`.
#![cfg(feature = "python-rustpython")]

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn setup() -> Fastshell {
    // Force the embedded engine: on desktop the system `python3` would be
    // preferred and would ignore the `_py_stdin` bridge.
    std::env::set_var("FASTSHELL_PYTHON", "rustpython");
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_pypipe_{}_{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dir);
    let mut sdk = Fastshell::new();
    sdk.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: true,
        // Mobile profile: no subprocess → the Python pipe bridge is used.
        allow_subprocess: false,
        network_ask_permission: false,
        command_timeout_ms: 30_000,
        ..Default::default()
    })
    .unwrap();
    sdk
}

#[test]
fn pipe_into_python_c_reads_stdin() {
    let sdk = setup();
    let r = sdk.execute(
        "printf 'a\\nb\\nc\\n' | python3 -c \"import sys; print(sum(1 for _ in sys.stdin))\"",
    );
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(
        r.stdout.contains('3'),
        "stdout={} stderr={}",
        r.stdout,
        r.stderr
    );
}

#[test]
fn pipe_into_python_c_transforms_text() {
    let sdk = setup();
    let r = sdk
        .execute("echo hello | python3 -c \"import sys; print(sys.stdin.read().strip().upper())\"");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("HELLO"), "stdout={}", r.stdout);
}

#[test]
fn pipe_into_python_script_reads_stdin() {
    let sdk = setup();
    sdk.write_file(
        "upper.py",
        "import sys\nfor line in sys.stdin:\n    print(line.rstrip().upper())\n",
    )
    .unwrap();
    let r = sdk.execute("printf 'one\\ntwo\\n' | python3 upper.py");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(
        r.stdout.contains("ONE") && r.stdout.contains("TWO"),
        "stdout={}",
        r.stdout
    );
}

#[test]
fn pipe_into_python_cleans_up_stdin_file() {
    let sdk = setup();
    let r = sdk.execute("echo x | python3 -c \"import sys; print(sys.stdin.read().strip())\"");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    // `_py_stdin` must be removed after the run so a later Python invocation
    // does not inherit stale stdin.
    assert!(!sdk.exists("_py_stdin"), "stale _py_stdin left behind");
    let r =
        sdk.execute("python3 -c \"import sys; print('no-stdin' if sys.stdin is None else 'has')\"");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
}

#[test]
fn pipe_into_python_producer_failure_is_reported() {
    let sdk = setup();
    // Producer writes to stderr and exits non-zero with no stdout: its error
    // must surface instead of running Python on empty input.
    let r = sdk.execute("cat missing_file_xyz | python3 -c \"print('ran')\"");
    assert_ne!(r.exit_code, 0, "stdout={} stderr={}", r.stdout, r.stderr);
    assert!(!r.stdout.contains("ran"), "stdout={}", r.stdout);
}

#[test]
fn pipe_into_python_cwd_is_respected() {
    let sdk = setup();
    sdk.execute("mkdir -p /work");
    sdk.write_file("/work/data.txt", "in-work\n").unwrap();
    let r = sdk.execute(
        "cd /work && printf 'x\\n' | python3 -c \"import sys,os; print(os.path.basename(os.getcwd())); print(sys.stdin.read().strip())\"",
    );
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("work"), "stdout={}", r.stdout);
    assert!(r.stdout.contains('x'), "stdout={}", r.stdout);
}
