// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Fourth coverage batch: misc_utils, sys_utils, extra flags.

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::sync::atomic::{AtomicUsize, Ordering};

static SEQ: AtomicUsize = AtomicUsize::new(0);

fn setup() -> Fastshell {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_flags4_{}_{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    let mut s = Fastshell::new();
    s.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: false,
        allow_subprocess: false,
        network_ask_permission: false,
        command_timeout_ms: 200,
        ..Default::default()
    })
    .unwrap();
    s.write_file("a.txt", "alpha\nbeta\ngamma\n").unwrap();
    s.write_file("b.txt", "beta\n").unwrap();
    s.write_file("nums.txt", "10\n2\n33\n").unwrap();
    s
}

/// Fresh SDK per command so a blocking command cannot poison later ones.
fn ok(_s: &Fastshell, cmd: &str) {
    let s = setup();
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
fn misc_utils_more() {
    let s = setup();
    for c in [
        "expr 1 = 1",
        "expr 1 \\< 2",
        "expr abc : '\\(a\\)bc'",
        "split -b 4 a.txt p1; ls p1* | sort",
        "split -a 3 -l 1 a.txt p2; ls p2* | sort",
        "split -d -l 1 a.txt p3; ls p3* | sort",
        "comm -1 a.txt b.txt",
        "comm -2 a.txt b.txt",
        "comm -3 a.txt b.txt",
        "xxd a.txt",
        "xxd -p a.txt",
        "xxd -l 4 a.txt",
        "xxd -c 4 a.txt",
        "xxd -s 2 a.txt",
        "od a.txt",
        "od -A d a.txt",
        "od -A x -t x1z a.txt",
        "od -j 2 -N 3 a.txt",
        "od -t d1 a.txt",
    ] {
        ok(&s, c);
    }
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn sys_utils_more() {
    let s = setup();
    for c in [
        "hexdump a.txt",
        "hexdump -C a.txt",
        "hexdump -b a.txt",
        "hexdump -d a.txt",
        "hexdump -x a.txt",
        "hexdump -n 4 a.txt",
        "pstree",
        "pstree -p",
        "pstree -a",
        "dmesg",
        "dmesg -T",
        "sha3sum a.txt",
        "sha3sum -c <(sha3sum a.txt)",
        "killall -l",
        "killall nope_xyz",
        "logname",
        "who",
        "reset",
        "printf 'a b\\n' | tsort",
        "tsort a.txt",
    ] {
        ok(&s, c);
    }
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn extra_more() {
    let s = setup();
    for c in [
        "ifconfig",
        "ifconfig -a",
        "netstat",
        "netstat -tunlp",
        "netstat -rn",
        "nc --help",
        "telnet --help",
        "traceroute --help",
        "patch --help",
        "mknod",
        "mount",
        "umount",
        "chroot",
        "renice 0",
        "nohup echo hi",
        "mkfifo /fifo1",
        "install f.txt /inst.txt",
        "shred f.txt",
        "shred -n 2 f.txt",
        "fallocate -l 8 f.bin",
        "fallocate --help",
    ] {
        ok(&s, c);
    }
}
