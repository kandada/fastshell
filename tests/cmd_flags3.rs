// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Third coverage batch: archive/file commands, shell builtins, and the SDK
//! file API (`bridge/fs.rs`).

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::sync::atomic::{AtomicUsize, Ordering};

static SEQ: AtomicUsize = AtomicUsize::new(0);

fn setup() -> Fastshell {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_flags3_{}_{}", std::process::id(), n));
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

fn ok(s: &Fastshell, cmd: &str) -> String {
    let r = s.execute(cmd);
    assert!(
        (0..=255).contains(&r.exit_code),
        "cmd {cmd:?} exit {}: {}",
        r.exit_code,
        r.stderr
    );
    r.stdout
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn sdk_file_api() {
    let s = setup();
    assert!(s.exists("a.txt"));
    assert!(!s.is_dir("a.txt"));
    assert_eq!(s.read_file("a.txt").unwrap(), "alpha\nbeta\ngamma\n");
    s.write_file("new.txt", "hi").unwrap();
    assert_eq!(s.read_file("new.txt").unwrap(), "hi");
    let entries = s.list_dir(".").unwrap();
    assert!(entries.iter().any(|e| e.name == "a.txt"));
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn archive_roundtrips() {
    let s = setup();
    // tar create / list / extract
    ok(&s, "tar czf a.tgz a.txt b.txt");
    assert!(ok(&s, "tar tzf a.tgz").contains("a.txt"));
    ok(&s, "mkdir ext; tar xzf a.tgz -C ext");
    // tar with dashless options
    ok(&s, "tar czf d.tgz a.txt");
    ok(&s, "tar tzf d.tgz");
    // zip / unzip
    ok(&s, "zip z.zip a.txt");
    ok(&s, "unzip -l z.zip");
    ok(&s, "unzip -o z.zip -d unz; cat unz/a.txt");
    ok(&s, "unzip -p z.zip a.txt");
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn cp_du_df_stat_flags() {
    let s = setup();
    ok(&s, "cp a.txt c1.txt; cat c1.txt");
    ok(
        &s,
        "mkdir srcd; cp a.txt srcd/x.txt; cp -r srcd dstd; ls dstd/x.txt",
    );
    ok(&s, "cp -n a.txt c2.txt");
    ok(&s, "cp -u a.txt c3.txt");
    ok(&s, "cp -f a.txt c4.txt");
    ok(&s, "du -h .");
    ok(&s, "du -s .");
    ok(&s, "du -a .");
    ok(&s, "df -h");
    ok(&s, "df -k");
    ok(&s, "stat a.txt");
    ok(&s, "stat -c '%s' a.txt");
    ok(&s, "stat -f '%n' a.txt");
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn shell_builtin_flags() {
    let s = setup();
    ok(&s, "f(){ echo \"in $1\"; }; declare -f f");
    ok(&s, "x=1; declare -p x");
    ok(&s, "f(){ :; }; unset -f f");
    ok(&s, "type echo");
    ok(&s, "command -v echo");
    ok(&s, "which echo");
    ok(&s, "alias q='echo q'; type q");
    ok(&s, "set -e; true; set +e");
    ok(&s, "shift 0; echo ok");
    ok(&s, "read -r x < a.txt; echo $x");
    ok(&s, "printf '%s\\n' \"$(command echo sub)\"");
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn ssh_and_misc_flags() {
    let s = setup();
    for c in [
        "ssh --help",
        "ssh -V",
        "ssh user@example.invalid",
        "ssh -p 22 host",
        "ssh -i key host",
        "expr 1 + 2",
        "expr 5 - 3",
        "expr 4 / 2",
        "expr length abc",
        "expr substr abcdef 2 3",
        "expr index abcdef c",
        "split -l 1 a.txt sp; ls sp* | sort",
        "comm a.txt b.txt",
        "xxd a.txt",
        "hexdump a.txt",
        "pstree",
        "dmesg",
        "killall nope_xyz",
        "logname",
        "who",
        "tsort a.txt",
    ] {
        ok(&s, c);
    }
}
