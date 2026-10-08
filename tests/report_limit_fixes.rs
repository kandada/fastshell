// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Dedicated regression tests for the reproducible LIMIT items in the iOS
//! fastshell capability report. `allow_subprocess: false` mirrors the mobile
//! environment (no external binaries), so every case must be handled in-kernel.

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn sdk() -> Fastshell {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_report_{}_{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Fastshell::new();
    s.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: true,
        allow_subprocess: false,
        command_timeout_ms: 5_000,
        ..Default::default()
    })
    .unwrap();
    s
}

fn run(s: &Fastshell, cmd: &str) -> (String, String, i32) {
    let r = s.execute(cmd);
    (r.stdout, r.stderr, r.exit_code)
}

fn out(s: &Fastshell, cmd: &str) -> String {
    let (o, e, rc) = run(s, cmd);
    assert_eq!(rc, 0, "cmd={cmd:?} err={e}");
    o
}

// ── #1 `;` inside nested quotes within `$(...)` must not split commands ──────

#[test]
fn item1_semicolon_inside_nested_quotes() {
    let s = sdk();
    assert_eq!(out(&s, "echo \"[$(echo \"a;b\")]\""), "[a;b]\n");
    assert_eq!(out(&s, "echo \"$(echo 'x;y')\""), "x;y\n");
    // a real `;` between commands in the substitution still separates
    assert_eq!(out(&s, "echo \"[$(echo a; echo b)]\""), "[a\nb]\n");
    // a `|` inside quotes in the substitution must not become a pipe
    assert_eq!(out(&s, "echo \"$(printf '%s' 'p|q')\""), "p|q\n");
}

// ── #2 quotes in an expansion result are data, not syntax ────────────────────

#[test]
fn item2_quotes_survive_expansion() {
    let s = sdk();
    assert_eq!(out(&s, "V=$(printf '\"q\"'); echo \"[$V]\""), "[\"q\"]\n");
    assert_eq!(out(&s, "V='\"q\"'; echo \"[$V]\""), "[\"q\"]\n");
    assert_eq!(out(&s, "printf '%s|' \"$(printf '\"q\"')\""), "\"q\"|");
    // no re-expansion of variable content
    assert_eq!(out(&s, "V='$HOME'; echo \"[$V]\""), "[$HOME]\n");
    assert_eq!(out(&s, "V='a\\b'; echo \"[$V]\""), "[a\\b]\n");
    // word-splitting still applies to unquoted expansion
    assert_eq!(
        out(&s, "V='a b'; for w in $V; do echo \"<$w>\"; done"),
        "<a>\n<b>\n"
    );
}

// ── #3 brace group / subshell can be piped ───────────────────────────────────

#[test]
fn item3_brace_group_and_subshell_piped() {
    let s = sdk();
    assert_eq!(out(&s, "{ echo a; echo b; } | tr a-z A-Z"), "A\nB\n");
    assert_eq!(out(&s, "(echo a; echo b) | tr a-z A-Z"), "A\nB\n");
    assert_eq!(out(&s, "{ echo x; } | wc -l").trim(), "1");
    assert_eq!(out(&s, "(printf 'a\\nb\\n') | wc -l").trim(), "2");
}

// ── #4/#5 Python stages in a pipeline ────────────────────────────────────────

#[test]
fn item4_python_script_file_into_pipe() {
    let s = sdk();
    s.execute("printf 'print(1+1)' > s.py");
    assert_eq!(out(&s, "python3 s.py | cat"), "2\n");
}

#[test]
fn item5_python_c_into_pipe() {
    let s = sdk();
    assert_eq!(out(&s, "python3 -c 'print(1+1)' | cat"), "2\n");
    assert_eq!(out(&s, "python3 -c 'print(2*3)' | tr -d '\\n'"), "6");
}

#[test]
fn item5_python_stage_in_middle_of_pipeline() {
    let s = sdk();
    assert_eq!(
        out(
            &s,
            "printf 'x\\ny\\n' | python3 -c 'import sys; sys.stdout.write(sys.stdin.read().upper())' | tr a-z A-Z"
        ),
        "X\nY\n"
    );
    assert_eq!(
        out(&s, "printf 'ab\\n' | python3 -c 'print(1+1)' | wc -l").trim(),
        "1"
    );
}

// ── #6 Python sandbox is enforced on every entry point ───────────────────────

#[test]
fn item6_python_sandbox_active() {
    let s = sdk();
    let cwd = out(&s, "python3 -c 'import os; print(os.path.realpath(\".\"))'");
    assert!(cwd.contains("fs_report_"), "cwd={cwd:?}");
    // `-c` must be sandboxed too (no escape to a real absolute path)
    let (o, e, rc) = run(&s, "python3 -c 'print(open(\"/etc/hostname\").read())'");
    assert!(rc != 0 || o.trim().is_empty(), "o={o:?} e={e:?} rc={rc}");
}

// ── #7 jq assignment / update / delete ───────────────────────────────────────

#[test]
fn item7_jq_assignment_ops() {
    let s = sdk();
    assert_eq!(out(&s, "echo '{\"a\":1}' | jq '.a=6'").trim(), "{\"a\":6}");
    assert_eq!(out(&s, "echo '{\"a\":1}' | jq '.a+=1'").trim(), "{\"a\":2}");
    assert_eq!(
        out(&s, "echo '{\"a\":1}' | jq '.a |= .+10'").trim(),
        "{\"a\":11}"
    );
    assert_eq!(
        out(&s, "echo '{\"a\":1,\"b\":2}' | jq 'del(.a)'").trim(),
        "{\"b\":2}"
    );
}

// ── #8 `sqlite3 :memory:` is in-memory, never a sandbox file ─────────────────

#[test]
fn item8_sqlite3_memory_db() {
    let s = sdk();
    assert_eq!(out(&s, "sqlite3 :memory: 'select 1+1'").trim(), "2");
    assert_ne!(run(&s, "ls :memory:").2, 0, "a real file was created");
    let r = out(
        &s,
        "sqlite3 :memory: 'create table t(x); insert into t values(1),(2); select count(*) from t'",
    );
    assert!(r.contains('2'), "r={r:?}");
}

// ── #9 xargs defaults to echo ────────────────────────────────────────────────

#[test]
fn item9_xargs_default_echo() {
    let s = sdk();
    assert_eq!(out(&s, "printf 'a\\nb\\n' | xargs").trim(), "a b");
    assert_eq!(out(&s, "printf 'a\\nb\\n' | xargs -n1"), "a\nb\n");
}

// ── #10 `file <dir>` reports a directory ─────────────────────────────────────

#[test]
fn item10_file_reports_directory() {
    let s = sdk();
    s.execute("mkdir -p d");
    assert_eq!(out(&s, "file d").trim(), "d: directory");
    assert_eq!(out(&s, "file -b d").trim(), "directory");
}

// ── #11 which / command -v know runtime builtins ─────────────────────────────

#[test]
fn item11_which_runtime_builtins() {
    let s = sdk();
    assert!(out(&s, "which render").contains("render"));
    assert_eq!(out(&s, "command -v render").trim(), "render");
    assert!(out(&s, "which sqlite3").contains("sqlite3"));
}

// ── #12 `id` honors flags ────────────────────────────────────────────────────

#[test]
fn item12_id_flags() {
    let s = sdk();
    assert!(out(&s, "id -u").trim().parse::<u32>().is_ok());
    assert!(out(&s, "id -g").trim().parse::<u32>().is_ok());
    assert!(out(&s, "id").contains("uid="));
}

#[cfg(feature = "git")]
#[test]
fn item12_git_version() {
    let s = sdk();
    assert!(out(&s, "git --version").starts_with("git version"));
}
