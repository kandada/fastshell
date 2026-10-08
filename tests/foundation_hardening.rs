// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Dedicated tests for the "thicken the foundation" hardening batch
//! (shell subprocess isolation, arrays, awk printf, binary safety,
//! here-strings, gzip short-flag parsing, misc builtins).

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn sdk() -> Fastshell {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_foundation_{}_{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Fastshell::new();
    s.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: true,
        allow_subprocess: true,
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

// ── sh -c / bash -c run in an isolated subshell ────────────────────────────

#[test]
fn sh_dash_c_exit_does_not_kill_parent() {
    let s = sdk();
    assert_eq!(
        out(&s, "echo a; sh -c 'exit 7'; echo b; echo rc=$?"),
        "a\nb\nrc=0\n"
    );
    assert_eq!(out(&s, "echo a; bash -c 'exit 9'; echo b"), "a\nb\n");
}

#[test]
fn sh_dash_c_exit_code_surfaces() {
    let s = sdk();
    assert_eq!(out(&s, "sh -c 'exit 5'; echo rc=$?"), "rc=5\n");
    assert_eq!(out(&s, "sh -c 'true'; echo rc=$?"), "rc=0\n");
}

#[test]
fn sh_dash_c_environment_is_isolated() {
    let s = sdk();
    assert_eq!(out(&s, "sh -c 'X=1'; echo [${X}]"), "[]\n");
    assert_eq!(out(&s, "sh -c 'cd /; pwd' > /dev/null; echo ok"), "ok\n");
    // `set -e` inside the child must not leak to the parent.
    assert_eq!(
        out(&s, "sh -c 'set -e; false'; sl_then=$?; echo done"),
        "done\n"
    );
}

// ── array element assignment ───────────────────────────────────────────────

#[test]
fn associative_array_assignment() {
    let s = sdk();
    let (o, e, rc) = run(&s, "declare -A m; m[foo]=bar; echo ${m[foo]}");
    assert_eq!(rc, 0);
    assert!(e.is_empty(), "stderr={e:?}");
    assert_eq!(o, "bar\n");
    assert_eq!(
        out(&sdk(), "declare -A m; m[a]=1; m[b]=2; echo ${m[a]}${m[b]}"),
        "12\n"
    );
    assert_eq!(out(&sdk(), "declare -A m; m[x]=1; echo ${#m[@]}"), "1\n");
    assert_eq!(
        out(&sdk(), "declare -A m; m[b]=2; m[a]=1; echo ${m[@]}"),
        "1 2\n"
    );
}

// ── awk printf expression arguments ────────────────────────────────────────

#[test]
fn awk_printf_evaluates_expression_args() {
    let s = sdk();
    assert_eq!(out(&s, "awk 'BEGIN{printf \"%d\\n\", 3*4}'"), "12\n");
    assert_eq!(out(&s, "awk 'BEGIN{printf \"%.2f\\n\", 10/3}'"), "3.33\n");
    assert_eq!(out(&s, "awk 'BEGIN{printf \"%d\\n\", 2+3*4}'"), "14\n");
    assert_eq!(
        out(
            &s,
            "echo one two three | awk '{printf \"%s-%d\\n\", $1, 2+3}'"
        ),
        "one-5\n"
    );
    assert_eq!(
        out(
            &s,
            "printf 'a 3\\n' | awk '{printf \"%s=%d\\n\", $1, $2*2}'"
        ),
        "a=6\n"
    );
}

// ── binary safety through commands / pipes ─────────────────────────────────

#[test]
fn binary_preserved_through_pipes_and_files() {
    let s = sdk();
    assert_eq!(
        out(&s, "printf 'iVBORw0KGgo=' | base64 -d | od -An -tx1"),
        " 89 50 4e 47 0d 0a 1a 0a\n"
    );
    out(&s, "printf 'iVBORw0KGgo=' > b.txt");
    out(&s, "base64 -d b.txt > png.bin");
    assert_eq!(out(&s, "head -c8 png.bin | xxd -p"), "89504e470d0a1a0a\n");
    assert_eq!(out(&s, "head -c4 png.bin | od -An -tx1"), " 89 50 4e 47\n");
    assert_eq!(
        out(&s, "cat png.bin | od -An -tx1"),
        " 89 50 4e 47 0d 0a 1a 0a\n"
    );
}

#[test]
fn printf_emits_raw_bytes() {
    let s = sdk();
    assert_eq!(out(&s, "printf '\\x89PNG' | od -An -tx1"), " 89 50 4e 47\n");
    assert_eq!(out(&s, "printf '\\211PNG' | od -An -tx1"), " 89 50 4e 47\n");
    assert_eq!(out(&s, "printf '\\x89' | wc -c"), "1\n");
}

// ── ANSI-C here-string `<<< $'...'` ────────────────────────────────────────

#[test]
fn here_string_ansi_c() {
    let s = sdk();
    assert_eq!(out(&s, "cat <<< $'a\\nb' | wc -l"), "2\n");
    assert_eq!(
        out(&s, "cat <<< $'x\\ty' | od -An -c"),
        "   x  \\t   y  \\n\n"
    );
    assert_eq!(out(&s, "cat <<< $'\\x41\\x42'"), "AB\n");
    // single-quoted is literal; double-quoted expands.
    assert_eq!(out(&s, "cat <<< 'a $HOME' | wc -c"), "8\n");
    assert_eq!(out(&s, "x=hi; cat <<< \"a $x\""), "a hi\n");
    assert_eq!(out(&s, "read a b <<< $'x y'; echo \"$a|$b\""), "x|y\n");
}

// ── timed-out network must not wedge the next command ──────────────────────

fn sdk_timeout(ms: u64) -> Fastshell {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_foundation_to_{}_{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Fastshell::new();
    s.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: false,
        allow_subprocess: false,
        network_ask_permission: false,
        command_timeout_ms: ms,
        ..Default::default()
    })
    .unwrap();
    s
}

/// A TCP server that accepts connections and then never sends a response.
fn hanging_server() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            if let Ok(c) = conn {
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(20));
                    drop(c);
                });
            }
        }
    });
    port
}

#[test]
fn timed_out_network_does_not_wedge_next_command() {
    let port = hanging_server();
    let s = sdk_timeout(800);

    let t0 = Instant::now();
    let r = s.execute(&format!("curl http://127.0.0.1:{port}/"));
    // Either the network layer bounds it first (curl exit 28) or the SDK
    // deadline fires (exit 124) — both mean "bounded", not wedged.
    assert_ne!(r.exit_code, 0, "expected a timeout error, got: {r:?}");
    assert!(
        t0.elapsed() < Duration::from_secs(5),
        "curl took too long: {:?}",
        t0.elapsed()
    );

    // The follow-up command must run promptly — previously the timed-out
    // worker held the runtime for up to 30s and chained further timeouts.
    let t = Instant::now();
    let r2 = s.execute("echo ok");
    assert_eq!(r2.stdout, "ok\n", "follow-up failed: {r2:?}");
    assert!(
        t.elapsed() < Duration::from_secs(3),
        "next command wedged for {:?}",
        t.elapsed()
    );

    // And another one right after.
    let r3 = s.execute("echo again");
    assert_eq!(r3.stdout, "again\n");
}
