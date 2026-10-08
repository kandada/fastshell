// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! POSIX `sh` 差分：用 **busybox ash > mksh > dash** 作为 oracle，验证 fastshell
//! 在 POSIX 子集上的语义与移动端真实 `sh` 一致（不测 bash 扩展）。
//!
//! 无 POSIX oracle 时 SKIP。

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::fs;

mod common;

fn setup() -> (Fastshell, std::path::PathBuf) {
    let fs_dir = std::env::temp_dir().join(format!("fs_posix_fs_{}", std::process::id()));
    let sh_dir = std::env::temp_dir().join(format!("fs_posix_sh_{}", std::process::id()));
    for d in [&fs_dir, &sh_dir] {
        let _ = fs::remove_dir_all(d);
        fs::create_dir_all(d).unwrap();
    }
    for (name, body) in [
        ("a.txt", "alpha\nbeta\ngamma\n"),
        ("b.txt", "beta\n"),
        ("nums.txt", "10\n2\n33\n"),
        ("csv.txt", "a,1\nb,2\nc,3\n"),
        ("dup.txt", "x\nx\ny\n"),
    ] {
        fs::write(sh_dir.join(name), body).unwrap();
    }
    let mut sdk = Fastshell::new();
    sdk.init(Config {
        sandbox_path: fs_dir.to_string_lossy().to_string(),
        python_enabled: false,
        allow_subprocess: true,
        network_ask_permission: false,
        command_timeout_ms: 10_000,
        ..Default::default()
    })
    .unwrap();
    for (name, body) in [
        ("a.txt", "alpha\nbeta\ngamma\n"),
        ("b.txt", "beta\n"),
        ("nums.txt", "10\n2\n33\n"),
        ("csv.txt", "a,1\nb,2\nc,3\n"),
        ("dup.txt", "x\nx\ny\n"),
    ] {
        sdk.write_file(name, body).unwrap();
    }
    (sdk, sh_dir)
}

/// POSIX-only constructs (no bash-isms like arrays, `[[ ]]`, `$'...'`, `(( ))`).
const POSIX: &[&str] = &[
    "echo hello",
    "echo a b c",
    "printf '%s\\n' a b",
    "printf '%d\\n' 42",
    "v=abc; echo $v",
    "v=abc; echo ${v}",
    "v=abc; echo ${#v}",
    "unset v; echo ${v:-def}",
    "echo `echo nested`",
    "echo $(echo sub)",
    "for i in 1 2 3; do echo $i; done",
    "i=0; while [ $i -lt 3 ]; do echo $i; i=$((i+1)); done",
    "if [ -f a.txt ]; then echo y; else echo n; fi",
    "if [ 3 -gt 2 ]; then echo g; fi",
    "case abc in a*) echo A;; *) echo B;; esac",
    "f() { echo \"f:$1\"; }; f x",
    "echo one | cat",
    "cat a.txt | wc -l",
    "cat a.txt | head -n 1",
    "grep alpha a.txt",
    "grep -v alpha a.txt",
    "grep -c a a.txt",
    "sort nums.txt",
    "sort -n nums.txt",
    "sort -r nums.txt",
    "uniq dup.txt",
    "cut -d, -f1 csv.txt",
    "tr a-z A-Z < a.txt",
    "sed 's/a/X/' a.txt",
    "awk '{print $1}' a.txt",
    "awk -F, '{print $2}' csv.txt",
    "head -n 2 a.txt",
    "tail -n 1 a.txt",
    "wc -l a.txt",
    "wc -w a.txt",
    "basename /a/b/c.txt",
    "dirname /a/b/c.txt",
    "echo a > o.txt; cat o.txt",
    "cat < a.txt | wc -l",
    "x=$(printf 'a\\nb\\n'); echo \"$x\"",
    "set -- a b c; echo $#; echo $1",
    "true && echo t",
    "false || echo f",
    ": ; echo colon",
    "echo {a,b}",
    "ls a.txt",
    "test -f a.txt && echo yes",
];

/// Collapse whitespace runs (BSD vs GNU padding is not semantic).
fn norm_ws(s: &str) -> String {
    let trimmed = common::norm(s);
    let mut out = String::with_capacity(trimmed.len());
    let mut prev_space = false;
    for c in trimmed.chars() {
        if c.is_whitespace() {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn posix_subset_matches_sh() {
    let oracle = common::oracle_posix();
    if !common::runnable(&oracle[0]) {
        eprintln!("no POSIX sh oracle; skipping");
        return;
    }
    eprintln!("POSIX oracle: {oracle:?}");
    let (sdk, sh_dir) = setup();
    let mut failures = Vec::new();
    for cmd in POSIX {
        let _ = sdk.execute("cd /");
        let f = sdk.execute(cmd);
        let (b_raw, b_code) = common::run_oracle_argv(&oracle, &sh_dir, cmd);
        // POSIX output padding (e.g. `wc` column width) differs between BSD
        // and GNU userlands; collapse whitespace runs before comparing.
        let b_out = norm_ws(&b_raw);
        let f_out = norm_ws(&f.stdout);
        if f_out != b_out || f.exit_code != b_code {
            failures.push(format!(
                "cmd: {cmd}\n  fs  : exit={} out={f_out:?}\n  sh  : exit={b_code} out={b_out:?}",
                f.exit_code
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "fastshell != POSIX sh for {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
