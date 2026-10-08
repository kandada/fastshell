// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Second flag/edge batch for the remaining low-coverage modules.

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::sync::atomic::{AtomicUsize, Ordering};

static SEQ: AtomicUsize = AtomicUsize::new(0);

fn setup() -> Fastshell {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_flags2_{}_{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    let mut s = Fastshell::new();
    s.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: false,
        allow_subprocess: false,
        network_ask_permission: false,
        command_timeout_ms: 250,
        ..Default::default()
    })
    .unwrap();
    s.write_file("a.txt", "alpha\nbeta\ngamma\n").unwrap();
    s.write_file("b.txt", "beta\n").unwrap();
    s
}

fn ok(s: &Fastshell, cmd: &str) {
    let r = s.execute(cmd);
    assert!(
        (0..=255).contains(&r.exit_code),
        "cmd {cmd:?} exit {}: {}",
        r.exit_code,
        r.stderr
    );
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn text_more_flags() {
    let s = setup();
    for c in [
        "truncate -s 5 a.txt; wc -c < a.txt",
        "cmp a.txt a.txt",
        "cmp a.txt b.txt",
        "strings a.txt",
        "fold -w 3 a.txt",
        "expand a.txt",
        "unexpand a.txt",
        "yes x | head -n 2",
        "truncate -s 0 a.txt",
    ] {
        ok(&s, c);
    }
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn ps_and_proc_flags() {
    let s = setup();
    for c in [
        "ps",
        "ps aux",
        "ps -ef",
        "ps -e",
        "uname -a",
        "uname -r",
        "uname -n",
        "id -u",
        "id -g",
        "whoami",
        "hostname",
        "pgrep sh",
        "pgrep -f sh",
        "pkill -0 definitely_not_running_xyz",
        "pidof sh",
    ] {
        ok(&s, c);
    }
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn base64_alias_kill_flags() {
    let s = setup();
    for c in [
        "printf hello | base64",
        "printf hello | base64 -w 2",
        "printf hello | base64 --wrap=2",
        "printf aGVsbG8= | base64 -d",
        "alias ll='echo aliased'; ll",
        "alias",
        "alias t1='echo x'; unalias t1",
        "unalias -a",
        "kill -l",
        "kill -0 999999",
        "kill -s TERM 999999",
        "kill 999999",
    ] {
        ok(&s, c);
    }
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn sys_more_flags() {
    let s = setup();
    for c in [
        "sha1sum -c <(sha1sum a.txt)",
        "sum -s a.txt",
        "nice -n 5 echo hi",
        "nice --adjustment=5 echo hi",
        "nproc --all",
        "tty -s",
        "free -m",
        "free -k",
        "od -x a.txt",
        "od -d a.txt",
        "od -b a.txt",
        "od -An -tx1 a.txt",
        "chown 0 a.txt",
        "chgrp 0 a.txt",
        "groups",
        "dd if=a.txt of=o.bin bs=2 count=1; wc -c < o.bin",
    ] {
        ok(&s, c);
    }
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn xargs_more_flags() {
    let s = setup();
    for c in [
        "printf 'a b c\\n' | xargs -n2 echo",
        "printf 'a\\nb\\n' | xargs -t echo",
        "printf 'a b\\n' | xargs -P2 -n1 echo",
        "printf 'a:b:c\\n' | xargs -d: echo",
        "printf 'a\\nb\\n' | xargs -I{} echo '<{}>'",
        "printf 'a\\nb\\n' | xargs -E b echo",
        "printf 'abc\\n' | xargs -s 2 echo",
    ] {
        ok(&s, c);
    }
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn misc_and_sys_utils_flags() {
    let s = setup();
    for c in [
        "seq -f '%03g' 1 3",
        "seq -w 1 3",
        "seq 1 2 9",
        "split -a 2 -l 1 a.txt part; ls part* | sort",
        "comm -123 a.txt b.txt",
        "xxd a.txt",
        "hexdump -C a.txt",
        "pstree -p",
        "dmesg -T",
        "sha3sum a.txt",
        "logger hello",
        "fallocate -l 8 fall.bin",
        "install -m 644 b.txt inst.txt",
        "renice -n 1 1",
        "ifconfig -a",
        "netstat -tunlp",
        "patch --help",
    ] {
        ok(&s, c);
    }
}
