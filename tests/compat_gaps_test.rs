// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Compatibility-gap regressions: `find -path/-not/-print`, `declare` flags,
//! and `exec` — all of which previously errored in real sessions.

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn setup() -> Fastshell {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_gaps_{}_{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dir);
    let mut sdk = Fastshell::new();
    sdk.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: false,
        allow_subprocess: true,
        network_ask_permission: false,
        command_timeout_ms: 30_000,
        ..Default::default()
    })
    .unwrap();
    sdk
}

#[test]
fn find_path_and_not_path() {
    let s = setup();
    s.execute("mkdir -p .git sub");
    s.write_file(".git/config", "x").unwrap();
    s.write_file("a.jpg", "x").unwrap();
    s.write_file("sub/b.jpg", "x").unwrap();
    let r = s.execute("find . -name '*.jpg' -not -path './.git/*'");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("a.jpg"), "stdout={}", r.stdout);
    assert!(
        !r.stdout.contains("config"),
        "should exclude .git: {}",
        r.stdout
    );
    assert!(
        !r.stderr.contains("unsupported option"),
        "stderr={}",
        r.stderr
    );
}

#[test]
fn find_print_accepted() {
    let s = setup();
    s.write_file("a.txt", "x").unwrap();
    let r = s.execute("find . -name 'a.txt' -print");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("a.txt"), "stdout={}", r.stdout);
    assert!(
        !r.stderr.contains("unsupported option"),
        "stderr={}",
        r.stderr
    );
}

#[test]
fn declare_flags_and_assignment() {
    let s = setup();
    let r = s.execute("declare -A M; declare -x FOO=bar; echo $FOO");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("bar"), "stdout={}", r.stdout);
    assert!(
        !r.stderr.contains("unknown option"),
        "declare flags must be accepted: {}",
        r.stderr
    );
}

#[test]
fn exec_redirection_is_noop() {
    let s = setup();
    let r = s.execute("exec 2>&1; echo hi");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("hi"), "stdout={}", r.stdout);
    assert!(
        !r.stderr.contains("command not found"),
        "exec must be known: {}",
        r.stderr
    );
}

#[test]
fn exec_with_command_runs_it() {
    let s = setup();
    let r = s.execute("exec echo hello");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("hello"), "stdout={}", r.stdout);
}

#[test]
fn find_exec_and_printf() {
    let s = setup();
    s.write_file("a.txt", "alpha\n").unwrap();
    // -exec runs a builtin per match.
    let r = s.execute("find . -name 'a.txt' -exec cat {} \\;");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("alpha"), "stdout={}", r.stdout);
    // -printf with common specifiers (%f basename, %s size).
    let r = s.execute("find . -name 'a.txt' -printf '%f %s\\n'");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert_eq!(r.stdout.trim(), "a.txt 6", "stdout={:?}", r.stdout);
}
