// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for license terms.

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn setup() -> Fastshell {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir =
        std::env::temp_dir().join(format!("fastshell_semantics_{}_{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dir);
    let mut sdk = Fastshell::new();
    sdk.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: true,
        allow_subprocess: false,
        network_ask_permission: false,
        command_timeout_ms: 10_000,
        ..Default::default()
    })
    .unwrap();
    sdk
}

fn ok(sdk: &Fastshell, command: &str) -> String {
    let r = sdk.execute(command);
    assert_eq!(
        r.exit_code, 0,
        "command failed: {command}\nstdout={}\nstderr={}",
        r.stdout, r.stderr
    );
    r.stdout
}

fn code(sdk: &Fastshell, command: &str) -> i32 {
    let r = sdk.execute(command);
    assert!(
        !r.stderr.is_empty() && r.stdout.is_empty(),
        "expected an error-like result for {command}: stdout={} stderr={}",
        r.stdout,
        r.stderr
    );
    r.exit_code
}

fn main_fixture(sdk: &Fastshell) {
    ok(sdk, "mkdir -p d");
    ok(sdk, "printf 'alpha\nbeta\n' > d/a.txt");
    ok(sdk, "printf 'alpha\ngamma\n' > d/b.txt");
    ok(sdk, "printf 'x' > d/x.txt");
}

#[test]
fn grep_exit_semantics() {
    let sdk = setup();
    main_fixture(&sdk);
    ok(&sdk, "grep -q alpha d/a.txt");
    let no_match = sdk.execute("grep -q missing d/a.txt");
    assert_eq!(no_match.exit_code, 1);
    assert!(no_match.stdout.is_empty() && no_match.stderr.is_empty());
    assert_eq!(code(&sdk, "grep missing missing.txt"), 2);
    assert_eq!(code(&sdk, "grep --definitely-invalid d/a.txt"), 2);
}

#[test]
fn diff_exit_semantics() {
    let sdk = setup();
    main_fixture(&sdk);
    ok(&sdk, "diff d/a.txt d/a.txt");
    let different = sdk.execute("diff d/a.txt d/b.txt");
    assert_eq!(different.exit_code, 1);
    assert!(!different.stdout.is_empty());
    assert!(different.stderr.is_empty());
    assert_eq!(code(&sdk, "diff d/a.txt missing.txt"), 2);
}

#[test]
fn test_exit_semantics() {
    let sdk = setup();
    main_fixture(&sdk);
    ok(&sdk, "test -f d/a.txt");
    let false_case = sdk.execute("test -f missing.txt");
    assert_eq!(false_case.exit_code, 1);
    assert!(false_case.stdout.is_empty() && false_case.stderr.is_empty());
    assert_ne!(sdk.execute("test -f").exit_code, 0);
}

#[test]
fn find_ls_cat_sed_awk_errors() {
    let sdk = setup();
    main_fixture(&sdk);
    assert!(ok(&sdk, "find d -name '*.txt'").contains("d/a.txt"));
    assert_eq!(code(&sdk, "find missing-dir"), 1);
    assert_eq!(code(&sdk, "ls missing-dir"), 1);
    assert_eq!(code(&sdk, "cat missing.txt"), 1);
    assert_eq!(code(&sdk, "sed -n '1p' missing.txt"), 1);
    assert_eq!(code(&sdk, "awk '{print $0}' missing.txt"), 1);
    assert!(ok(&sdk, "sed -n '1p' d/a.txt").contains("alpha"));
    assert!(ok(&sdk, "awk 'NR==1 {print $0}' d/a.txt").contains("alpha"));
}

#[test]
fn python_sqlite_semantics() {
    let sdk = setup();
    assert!(ok(&sdk, "python3 -c 'print(40+2)'").contains("42"));
    assert_ne!(
        sdk.execute("python3 -c 'raise RuntimeError(\"boom\")'")
            .exit_code,
        0
    );
    let syntax = sdk.execute("python3 -c 'if'");
    assert_ne!(syntax.exit_code, 0);
    assert!(!syntax.stderr.is_empty());
    assert_ne!(sdk.execute("python3 missing.py").exit_code, 0);

    ok(&sdk, "sqlite3 t.db 'CREATE TABLE t(x INTEGER)'");
    ok(&sdk, "sqlite3 t.db 'INSERT INTO t VALUES(7)'");
    assert!(ok(&sdk, "sqlite3 t.db 'SELECT x FROM t'").contains("7"));
    assert_ne!(sdk.execute("sqlite3 t.db 'not sql'").exit_code, 0);
}

#[test]
fn pipeline_and_shell_operator_semantics() {
    let sdk = setup();
    ok(&sdk, "printf 'abc' | grep -q abc");
    assert_eq!(sdk.execute("printf 'abc' | grep -q z").exit_code, 1);
    assert_eq!(sdk.execute("true | false").exit_code, 1);
    assert_eq!(sdk.execute("false | true").exit_code, 0);
    assert!(ok(&sdk, "true && echo yes").contains("yes"));
    assert_eq!(sdk.execute("false && echo no").exit_code, 1);
    assert!(ok(&sdk, "false || echo fallback").contains("fallback"));
    ok(&sdk, "false; echo continued");
    assert!(ok(&sdk, "echo left && echo right").contains("right"));
}

#[test]
fn curl_and_wget_usage_errors() {
    let sdk = setup();
    assert_ne!(sdk.execute("curl --definitely-invalid").exit_code, 0);
    assert_ne!(sdk.execute("wget --definitely-invalid").exit_code, 0);
}
