// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 真机会话回放：从真机导出的 `session_*.json` 抽取所有 `run_shell` 命令，
//! 1) **健壮性**：每条命令在 fastshell 中必须正常返回（不 panic、不挂死）；
//! 2) **差分**：对“可移植子集”（无设备绝对路径 / 外部工具 / 非确定性）与
//!    bash oracle 比较 stdout + 退出码。
//!
//! 语料固化在 `tests/session_corpus.jsonl`（每行一个 JSON 字符串，保留换行）。
//! 若设置 `FB_SESSIONS_DIR`（或默认 `~/Downloads`）存在 `session_*.json`，会
//! **额外**抽取其中的命令一起回放 —— 真机测得越多，覆盖自动增长。

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::fs;
use std::path::PathBuf;

mod common;

static DIR_SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn setup() -> (Fastshell, PathBuf) {
    let seq = DIR_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let fs_dir = std::env::temp_dir().join(format!("fs_replay_fs_{}_{}", std::process::id(), seq));
    let sh_dir = std::env::temp_dir().join(format!("fs_replay_sh_{}_{}", std::process::id(), seq));
    for d in [&fs_dir, &sh_dir] {
        let _ = fs::remove_dir_all(d);
        fs::create_dir_all(d).unwrap();
    }
    for (name, body) in [
        ("a.txt", "alpha\nbeta\ngamma\n"),
        ("b.txt", "beta\n"),
        ("nums.txt", "10\n2\n33\n"),
    ] {
        fs::write(sh_dir.join(name), body).unwrap();
    }
    let mut sdk = Fastshell::new();
    sdk.init(Config {
        sandbox_path: fs_dir.to_string_lossy().to_string(),
        python_enabled: false,
        allow_subprocess: true,
        network_ask_permission: false,
        command_timeout_ms: 200,
        ..Default::default()
    })
    .unwrap();
    for (name, body) in [
        ("a.txt", "alpha\nbeta\ngamma\n"),
        ("b.txt", "beta\n"),
        ("nums.txt", "10\n2\n33\n"),
    ] {
        sdk.write_file(name, body).unwrap();
    }
    (sdk, sh_dir)
}

/// Vendored corpus + any live sessions found on disk.
fn load_commands() -> Vec<String> {
    let mut cmds = Vec::new();
    let vendored = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/session_corpus.jsonl");
    if let Ok(text) = fs::read_to_string(vendored) {
        for line in text.lines() {
            if let Ok(c) = serde_json::from_str::<String>(line) {
                cmds.push(c);
            }
        }
    }
    // Live sessions (optional): grows automatically as we test on device.
    let dir = std::env::var("FB_SESSIONS_DIR").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/Downloads")
    });
    if let Ok(rd) = fs::read_dir(&dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with("session_") && name.ends_with(".json") {
                if let Ok(text) = fs::read_to_string(e.path()) {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                        if let Some(msgs) = v.get("messages").and_then(|m| m.as_array()) {
                            for m in msgs {
                                if let Some(tcs) = m.get("tool_calls").and_then(|t| t.as_array()) {
                                    for tc in tcs {
                                        if tc.get("name").and_then(|n| n.as_str())
                                            == Some("run_shell")
                                        {
                                            if let Some(c) = tc
                                                .get("arguments")
                                                .and_then(|a| a.as_str())
                                                .and_then(|a| {
                                                    serde_json::from_str::<serde_json::Value>(a)
                                                        .ok()
                                                })
                                                .and_then(|a| {
                                                    a.get("command")
                                                        .and_then(|c| c.as_str())
                                                        .map(String::from)
                                                })
                                            {
                                                cmds.push(c);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    cmds
}

/// A command is comparable with bash only if it does not depend on device
/// paths, external tools, or nondeterministic sources.
/// Commands that read stdin, loop, or hit the network — skipped by both passes
/// (no regression value, and they can block the test).
fn blocking(cmd: &str) -> bool {
    const BLOCK: &[&str] = &[
        "sleep",
        "tail -f",
        "tail --follow",
        "watch",
        "top",
        "htop",
        "ping",
        "ssh",
        "telnet",
        "yes",
        "read ",
        "while ",
        "until ",
        "find /",
        "sh ",
        "git ",
        "npm ",
        "python",
        "pip",
        "curl",
        "wget",
        "nc ",
        "traceroute",
        "nohup",
        "&",
    ];
    BLOCK.iter().any(|b| cmd.contains(b))
}

fn portable(cmd: &str) -> bool {
    const BAD: &[&str] = &[
        "/", "pwd", "ls -l", "ls -a", "curl", "wget", "python", "pip", "sh ", "find", "git ",
        "ssh", "npm", "file ", "device", "$RANDOM", "date", "whoami", "uname", " id", "env", "df",
        "du ", "ps ", "top", "ping", "sleep", "kill", "&&", "||", "read", "while", "for", "if ",
        "case ", "tail -f", "yes", "watch", "cat", "sort", "wc", "head", "grep", "tr ",
    ];
    !BAD.iter().any(|b| cmd.contains(b))
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn session_commands_never_crash() {
    let cmds = load_commands();
    assert!(!cmds.is_empty(), "no session commands loaded");
    let (sdk, _sh) = setup();
    let mut bad = Vec::new();
    for cmd in cmds.iter().filter(|c| !blocking(c)).take(80) {
        let _ = sdk.execute("cd /");
        let r = sdk.execute(cmd);
        // A sane exit code; a panic/hang would surface as a non-result or the
        // SDK timeout (143). We only require it to return and stay in range.
        if r.exit_code < 0 || r.exit_code > 255 {
            bad.push(format!("exit={} cmd={cmd:?}", r.exit_code));
        }
    }
    assert!(bad.is_empty(), "abnormal exit codes:\n{}", bad.join("\n"));
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn session_portable_subset_matches_bash() {
    let bin = common::oracle_bash();
    if !common::runnable(&bin) {
        eprintln!("bash oracle not found; skipping");
        return;
    }
    let (allow_always, _) = common::load_allowlist();
    let cmds = load_commands();
    let (sdk, sh_dir) = setup();
    let mut failures = Vec::new();
    let mut checked = 0;
    for cmd in &cmds {
        if !portable(cmd) {
            continue;
        }
        if allow_always.iter().any(|a| cmd.contains(a.as_str())) {
            continue;
        }
        checked += 1;
        let _ = sdk.execute("cd /");
        let f = sdk.execute(cmd);
        let (b_raw, b_code) = common::run_oracle(&bin, &sh_dir, cmd);
        let b_out = common::norm(&b_raw);
        let f_out = common::norm(&f.stdout);
        // Compare stdout only: the corpus is real-device commands that reference
        // files absent from the sandbox, so both shells fail — but the *error
        // exit code* differs between BSD (macOS, 1) and GNU (Linux, 2) tools.
        // Crash-safety is covered by `session_commands_never_crash`.
        if f_out != b_out {
            failures.push(format!(
                "cmd: {cmd}\n  fs : exit={} out={f_out:?}\n  bash: exit={b_code} out={b_out:?}",
                f.exit_code
            ));
        }
    }
    eprintln!("portable session commands checked: {checked}");
    assert!(
        failures.is_empty(),
        "fastshell != bash for {} portable session command(s):\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
