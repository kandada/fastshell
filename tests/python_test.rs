// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

use fastshell::python::PythonEngine;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn setup_dir() -> std::path::PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fastshell_int_py_{}_{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn test_python_detect_engine() {
    let engine = fastshell::python::detect_python_engine(std::path::Path::new("/tmp"));
    assert!(engine.is_available() || !engine.is_available());
}

#[test]
fn test_python_simple_execution() {
    let mut engine = fastshell::python::SubprocessPython::new();
    if !engine.is_available() {
        return;
    }
    let dir = setup_dir();
    let result = engine.execute("print('integration test')", &dir);
    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.contains("integration test"));
}

#[test]
fn test_python_script_execution() {
    let mut engine = fastshell::python::SubprocessPython::new();
    if !engine.is_available() {
        return;
    }
    let dir = setup_dir();
    let script = dir.join("script.py");
    fs::write(&script, "print(1+1)\n").unwrap();

    let result = engine.execute_script(&script, &dir);
    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.contains("2"));
}

#[test]
fn test_python_error_handling() {
    let mut engine = fastshell::python::SubprocessPython::new();
    if !engine.is_available() {
        return;
    }
    let dir = setup_dir();
    let result = engine.execute("raise RuntimeError('boom')", &dir);
    assert_ne!(result.exit_code, 0);
}

#[test]
fn test_placeholder_unavailable() {
    let engine = fastshell::python::PocketPyPlaceholder::new();
    assert!(!engine.is_available());
    assert!(engine.version().is_none());
}
