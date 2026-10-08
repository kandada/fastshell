// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

// Shared test helpers: not every integration test uses every helper.
#![allow(dead_code)]

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static COMMON_COUNTER: AtomicUsize = AtomicUsize::new(0);

pub fn unique_dir(prefix: &str) -> PathBuf {
    let n = COMMON_COUNTER.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), n))
}

pub fn setup_sdk() -> Fastshell {
    let dir = unique_dir("fastshell_common");
    let _ = fs::remove_dir_all(&dir);
    let mut sdk = Fastshell::new();
    sdk.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: true,
        allow_subprocess: true,
        network_ask_permission: false,
        command_timeout_ms: 30_000,
        ..Default::default()
    })
    .unwrap();
    sdk
}

pub fn setup_sdk_no_subprocess() -> Fastshell {
    let dir = unique_dir("fastshell_common_ns");
    let _ = fs::remove_dir_all(&dir);
    let mut sdk = Fastshell::new();
    sdk.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: false,
        allow_subprocess: false,
        network_ask_permission: false,
        command_timeout_ms: 5_000,
        ..Default::default()
    })
    .unwrap();
    sdk
}

pub fn setup_sdk_with_timeout(timeout_ms: u64) -> Fastshell {
    let dir = unique_dir("fastshell_common_to");
    let _ = fs::remove_dir_all(&dir);
    let mut sdk = Fastshell::new();
    sdk.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: false,
        allow_subprocess: false,
        network_ask_permission: false,
        command_timeout_ms: timeout_ms,
        ..Default::default()
    })
    .unwrap();
    sdk
}

pub fn populate_vfs(sdk: &Fastshell) {
    sdk.execute("mkdir -p dir1/sub");
    sdk.execute("mkdir -p dir2");
    sdk.write_file("dir1/a.txt", "alpha").unwrap();
    sdk.write_file("dir1/sub/b.txt", "beta").unwrap();
    sdk.write_file("dir2/c.txt", "gamma").unwrap();
    sdk.write_file("root.txt", "i am root").unwrap();
    sdk.write_file("numbers.txt", "1\n2\n3\n4\n5\n").unwrap();
    sdk.write_file("words.txt", "apple\nbanana\nApple\ncherry\ndate\n")
        .unwrap();
    sdk.write_file("data.csv", "name,age\nAlice,30\nBob,25\nCharlie,35\n")
        .unwrap();
}

pub fn assert_cmd_ok(sdk: &Fastshell, cmd: &str) -> String {
    let r = sdk.execute(cmd);
    assert_eq!(r.exit_code, 0, "cmd '{}' failed: stderr={}", cmd, r.stderr);
    r.stdout
}

pub fn assert_cmd_contains(sdk: &Fastshell, cmd: &str, expected: &str) {
    let out = assert_cmd_ok(sdk, cmd);
    assert!(
        out.contains(expected),
        "cmd '{}' output does not contain '{}':\n{}",
        cmd,
        expected,
        out
    );
}

pub fn assert_cmd_stderr_contains(sdk: &Fastshell, cmd: &str, expected: &str) {
    let r = sdk.execute(cmd);
    assert!(
        r.stderr.contains(expected),
        "cmd '{}' stderr does not contain '{}':\nstderr={}\nstdout={}",
        cmd,
        expected,
        r.stderr,
        r.stdout
    );
}

pub fn assert_cmd_fails(sdk: &Fastshell, cmd: &str) -> String {
    let r = sdk.execute(cmd);
    assert_ne!(
        r.exit_code, 0,
        "cmd '{}' should have failed but succeeded",
        cmd
    );
    r.stderr
}

pub fn assert_file_content(sdk: &Fastshell, path: &str, expected: &str) {
    let content = sdk.read_file(path).unwrap();
    assert_eq!(content, expected, "file '{}' content mismatch", path);
}

pub fn assert_file_exists(sdk: &Fastshell, path: &str) {
    assert!(sdk.exists(path), "file '{}' should exist", path);
}

pub fn assert_file_not_exists(sdk: &Fastshell, path: &str) {
    assert!(!sdk.exists(path), "file '{}' should NOT exist", path);
}

// ═══════════════════════════════════════════════════════════════════
// Differential-test oracles (bash 5 / mksh / dash) + divergence allowlist.
// ═══════════════════════════════════════════════════════════════════

/// `<workspace>/tools/oracle/bin` — locally built oracle shells.
pub fn workspace_tools() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join("tools")
        .join("oracle")
        .join("bin")
}

/// Best available bash oracle (`$FB_ORACLE_BASH` > tools/oracle > brew > PATH).
pub fn oracle_bash() -> String {
    if let Ok(p) = std::env::var("FB_ORACLE_BASH") {
        if std::path::Path::new(&p).exists() {
            return p;
        }
    }
    let local = workspace_tools().join("bash");
    if local.exists() {
        return local.to_string_lossy().to_string();
    }
    for cand in ["/opt/homebrew/bin/bash", "/usr/local/bin/bash"] {
        if std::path::Path::new(cand).exists() {
            return cand.to_string();
        }
    }
    "bash".to_string()
}

/// mksh (Android's real `sh`), if built into `tools/oracle/bin`.
pub fn oracle_mksh() -> Option<String> {
    let p = workspace_tools().join("mksh");
    p.exists().then(|| p.to_string_lossy().to_string())
}

/// A POSIX `sh` (dash preferred).
pub fn oracle_sh() -> String {
    for cand in ["/bin/dash", "/usr/bin/dash", "/bin/sh"] {
        if std::path::Path::new(cand).exists() {
            return cand.to_string();
        }
    }
    "sh".to_string()
}

pub fn runnable(bin: &str) -> bool {
    if std::process::Command::new(bin)
        .arg("-c")
        .arg(":")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return true;
    }
    std::process::Command::new(bin)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Major version of a bash oracle (0 if unknown).
pub fn bash_major(bin: &str) -> u32 {
    let Ok(o) = std::process::Command::new(bin).arg("--version").output() else {
        return 0;
    };
    String::from_utf8_lossy(&o.stdout)
        .split("version ")
        .nth(1)
        .and_then(|r| r.split('.').next())
        .and_then(|m| m.trim().parse().ok())
        .unwrap_or(0)
}

pub fn norm(s: &str) -> String {
    s.trim_end_matches('\n').trim_end_matches('\r').to_string()
}

/// Runs `cmd` in `dir` with the given oracle → (stdout, exit_code).
///
/// stdin is `/dev/null`; a 5s wall-clock cap is enforced and the stdout pipe is
/// read on a detached thread with a bounded wait, so a killed oracle whose
/// grandchild still holds the pipe can never block the test forever.
pub fn run_oracle(bin: &str, dir: &std::path::Path, cmd: &str) -> (String, i32) {
    let mut c = std::process::Command::new(bin);
    c.arg("-c").arg(cmd);
    run_oracle_cmd(c, dir)
}

/// Runs `cmd` through an argv prefix (e.g. `["busybox","sh"]`).
pub fn run_oracle_argv(prefix: &[String], dir: &std::path::Path, cmd: &str) -> (String, i32) {
    let mut c = std::process::Command::new(&prefix[0]);
    for a in &prefix[1..] {
        c.arg(a);
    }
    c.arg("-c").arg(cmd);
    run_oracle_cmd(c, dir)
}

fn run_oracle_cmd(mut c: std::process::Command, dir: &std::path::Path) -> (String, i32) {
    use std::io::Read;
    let mut child = c
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("oracle spawn");
    let stdout = child.stdout.take();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    if let Some(out) = stdout {
        // Detached reader: a grandchild may keep the pipe open after the
        // direct child is killed, so we must not join this thread.
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = out.take(1 << 20).read_to_string(&mut s);
            let _ = tx.send(s);
        });
    } else {
        let _ = tx.send(String::new());
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut code = -1;
    loop {
        match child.try_wait() {
            Ok(Some(s)) => {
                code = s.code().unwrap_or(-1);
                break;
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(_) => break,
        }
    }
    let buf = rx
        .recv_timeout(std::time::Duration::from_millis(300))
        .unwrap_or_default();
    (buf, code)
}

/// Allowlist split into (always-skip, bash3-only-skip).
pub fn load_allowlist() -> (Vec<String>, Vec<String>) {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/known_divergences.txt");
    let mut always = Vec::new();
    let mut bash3 = Vec::new();
    for line in fs::read_to_string(path).unwrap_or_default().lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if let Some(rest) = l.strip_prefix("bash3:") {
            bash3.push(rest.trim().to_string());
        } else {
            always.push(l.to_string());
        }
    }
    (always, bash3)
}

/// busybox `sh` (Android's real `sh` is often busybox/ash). Returns the argv
/// prefix, e.g. `["busybox", "sh"]` or `["/path/to/busybox", "sh"]`.
pub fn oracle_busybox() -> Option<Vec<String>> {
    let local = workspace_tools().join("busybox");
    if local.exists() {
        return Some(vec![local.to_string_lossy().to_string(), "sh".into()]);
    }
    if std::process::Command::new("busybox")
        .args(["sh", "-c", ":"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return Some(vec!["busybox".into(), "sh".into()]);
    }
    None
}

/// Best POSIX `sh` oracle: busybox ash > mksh > dash > sh.
pub fn oracle_posix() -> Vec<String> {
    if let Some(b) = oracle_busybox() {
        return b;
    }
    if let Some(m) = oracle_mksh() {
        return vec![m];
    }
    vec![oracle_sh()]
}
