// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn setup_sdk() -> fastshell::sdk::Fastshell {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fastshell_int_sdk_{}_{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dir);

    let mut sdk = fastshell::sdk::Fastshell::new();
    let config = fastshell::sdk::types::Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: true,
        ..Default::default()
    };
    sdk.init(config).unwrap();
    sdk
}

#[test]
fn test_sdk_initialization() {
    let sdk = setup_sdk();
    assert!(sdk.is_initialized());

    let info = sdk.get_info();
    assert_eq!(info.version, "0.2.2");
    assert_eq!(info.platform, std::env::consts::OS);
}

#[test]
fn test_sdk_shell_workflow() {
    let sdk = setup_sdk();

    let result = sdk.execute("echo hello_sdk");
    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.contains("hello_sdk"));

    let result = sdk.execute("mkdir project");
    assert_eq!(result.exit_code, 0);

    sdk.execute("cd project");
    let result = sdk.execute("touch main.py");
    assert_eq!(result.exit_code, 0);

    let result = sdk.execute("ls");
    assert!(result.stdout.contains("main.py"));

    let result = sdk.execute("pwd");
    assert!(result.stdout.contains("project"));
}

#[test]
fn test_sdk_error_not_initialized() {
    let sdk = fastshell::sdk::Fastshell::new();
    let result = sdk.execute("ls");
    assert_ne!(result.exit_code, 0);
}

#[test]
fn test_sdk_file_scenario() {
    let sdk = setup_sdk();

    sdk.execute("mkdir data");
    sdk.execute("cd data");
    sdk.execute("touch notes.txt");
    sdk.execute("touch todo.txt");

    let result = sdk.execute("find . -name '*.txt'");
    assert!(result.stdout.contains("notes.txt"));
    assert!(result.stdout.contains("todo.txt"));

    sdk.execute("rm notes.txt");
    let result = sdk.execute("ls");
    assert!(!result.stdout.contains("notes.txt"));
    assert!(result.stdout.contains("todo.txt"));
}

#[test]
fn test_sdk_python_execution() {
    let sdk = setup_sdk();
    let result = sdk.execute_python("print('integration')");
    let info = sdk.get_info();
    if info.python_available {
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("integration"));
    }
}
