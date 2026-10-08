// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! `rm` on symlinks must remove the LINK, not its target (POSIX), including
//! `rm -r link-to-dir`. Also checks regular file/dir removal still works.

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn setup() -> Fastshell {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_symlink_{}_{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    let mut s = Fastshell::new();
    s.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: false,
        allow_subprocess: false,
        network_ask_permission: false,
        command_timeout_ms: 2000,
        ..Default::default()
    })
    .unwrap();
    s
}

fn out(s: &Fastshell, cmd: &str) -> String {
    let r = s.execute(cmd);
    assert_eq!(r.exit_code, 0, "cmd {cmd:?} failed: {}", r.stderr);
    r.stdout
}

#[test]
fn rm_symlink_keeps_target() {
    let s = setup();
    s.write_file("target.txt", "data").unwrap();
    out(&s, "ln -s target.txt link.txt");
    out(&s, "rm link.txt");
    assert!(
        !out(&s, "ls").contains("link.txt"),
        "link should be removed"
    );
    assert!(out(&s, "ls").contains("target.txt"), "target must remain");
    assert_eq!(out(&s, "cat target.txt"), "data");
}

#[test]
fn rm_r_symlink_to_dir_keeps_dir() {
    let s = setup();
    s.write_file("d/keep.txt", "k").unwrap();
    out(&s, "ln -s d dlink");
    out(&s, "rm -r dlink");
    assert!(!out(&s, "ls").contains("dlink"), "link removed");
    assert!(out(&s, "ls").contains("d"), "dir must remain");
    assert_eq!(out(&s, "cat d/keep.txt"), "k");
}

#[test]
fn rm_broken_symlink_and_regular() {
    let s = setup();
    // A dangling symlink is removed as a link.
    out(&s, "ln -s missing.txt broken");
    out(&s, "rm broken");
    assert!(!out(&s, "ls").contains("broken"));
    // Regular file + recursive dir still work.
    s.write_file("f.txt", "x").unwrap();
    out(&s, "rm f.txt");
    assert!(!out(&s, "ls").contains("f.txt"));
    s.write_file("dd/x", "x").unwrap();
    out(&s, "rm -r dd");
    assert!(!out(&s, "ls").contains("dd"));
}

#[test]
fn rmdir_on_symlink_errors_and_keeps_both() {
    let s = setup();
    s.write_file("d/x", "x").unwrap();
    out(&s, "ln -s d dl");
    let r = s.execute("rmdir dl");
    assert_ne!(r.exit_code, 0, "rmdir on a symlink should fail");
    assert!(out(&s, "ls").contains("dl"));
    assert_eq!(out(&s, "cat d/x"), "x");
}

#[test]
fn rm_missing_is_error_unless_forced() {
    let s = setup();
    let r = s.execute("rm nope.txt");
    assert_ne!(r.exit_code, 0);
    // -f suppresses the error.
    out(&s, "rm -f nope.txt");
}
