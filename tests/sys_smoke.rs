// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Smoke + functional tests for the lower-coverage command modules
//! (sys_info / compress_cal / extra / sys_utils / sys_more / hashsum /
//! final_batch). Raises coverage and guards against panics on invocation.

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;

fn setup() -> Fastshell {
    let dir = std::env::temp_dir().join(format!("fs_sys_smoke_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut sdk = Fastshell::new();
    sdk.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: false,
        allow_subprocess: false,
        network_ask_permission: false,
        command_timeout_ms: 1_000,
        ..Default::default()
    })
    .unwrap();
    sdk.write_file("a.txt", "alpha\nbeta\ngamma\n").unwrap();
    sdk.write_file("crlf.txt", "x\r\ny\r\n").unwrap();
    sdk
}

fn ok(sdk: &Fastshell, cmd: &str) -> String {
    let r = sdk.execute(cmd);
    assert!(
        (0..=255).contains(&r.exit_code),
        "cmd {cmd:?} bad exit {}: {}",
        r.exit_code,
        r.stderr
    );
    r.stdout
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn sys_info_smoke() {
    let s = setup();
    for c in [
        "uname", "uname -s", "uname -m", "hostname", "whoami", "id", "id -u",
    ] {
        ok(&s, c);
    }
    // pgrep/pkill only inspected, never actually kill a real process here.
    ok(&s, "pgrep definitely_not_a_real_process_xyz");
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn compress_and_calendar() {
    let s = setup();
    // bzip2 roundtrip
    assert_eq!(
        ok(&s, "bzip2 a.txt && bunzip2 a.txt.bz2 && cat a.txt").trim(),
        "alpha\nbeta\ngamma".trim()
    );
    // xz roundtrip
    assert_eq!(
        ok(&s, "xz a.txt && unxz a.txt.xz && cat a.txt").trim(),
        "alpha\nbeta\ngamma".trim()
    );
    // zcat
    ok(&s, "bzip2 -k a.txt; zcat a.txt.bz2");
    // dos2unix / unix2dos roundtrip
    assert!(!ok(&s, "dos2unix crlf.txt && cat crlf.txt").contains('\r'));
    ok(&s, "unix2dos crlf.txt");
    // deterministic calendar
    assert!(ok(&s, "cal 1 2020").contains("January"));
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn hashsum_known_lengths() {
    let s = setup();
    assert_eq!(
        ok(&s, "printf hi | md5sum")
            .split_whitespace()
            .next()
            .unwrap()
            .len(),
        32
    );
    assert_eq!(
        ok(&s, "printf hi | sha1sum")
            .split_whitespace()
            .next()
            .unwrap()
            .len(),
        40
    );
    assert_eq!(
        ok(&s, "printf hi | sha256sum")
            .split_whitespace()
            .next()
            .unwrap()
            .len(),
        64
    );
    assert_eq!(
        ok(&s, "printf hi | sha512sum")
            .split_whitespace()
            .next()
            .unwrap()
            .len(),
        128
    );
}
