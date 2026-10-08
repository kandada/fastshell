// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Command "dialect" compatibility: DOS/Unix alternative names + harmless
//! GNU/BSD flag spellings.

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn setup() -> Fastshell {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_dialect_{}_{}", std::process::id(), n));
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
fn dos_style_aliases() {
    let s = setup();
    // md = mkdir, dir = ls
    out(&s, "md work");
    s.write_file("work/a.txt", "hi").unwrap();
    assert!(out(&s, "dir work").contains("a.txt"));
    // copy = cp
    out(&s, "copy work/a.txt work/b.txt");
    assert!(out(&s, "dir work").contains("b.txt"));
    // move/ren = mv
    out(&s, "ren work/b.txt work/c.txt");
    assert!(out(&s, "dir work").contains("c.txt"));
    // del = rm
    out(&s, "del work/c.txt");
    assert!(!out(&s, "dir work").contains("c.txt"));
    out(&s, "del work/a.txt");
    // rd = rmdir
    out(&s, "rd work");
    assert!(!out(&s, "dir").contains("work"));
}

#[test]
fn where_and_cls() {
    let s = setup();
    // where = which
    let r = s.execute("where ls");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    // cls = clear (no-op, must not error)
    out(&s, "cls");
}

#[test]
fn harmless_flag_dialects() {
    let s = setup();
    s.write_file("a.txt", "1\n2\n3\n4\n5\n").unwrap();
    // GNU colour flags are dropped, not errored.
    assert!(out(&s, "ls --color=never").contains("a.txt"));
    assert!(out(&s, "grep --colour=never 3 a.txt").contains("3"));
    // head/tail long options.
    assert_eq!(out(&s, "head --lines=2 a.txt"), "1\n2\n");
    assert_eq!(out(&s, "tail --bytes=2 a.txt"), "5\n");
}

#[test]
fn more_dialects() {
    let s = setup();
    s.write_file("a.txt", "foo\nbar\nfoo2\n").unwrap();
    // egrep = grep -E
    assert!(out(&s, "egrep 'fo+' a.txt").contains("foo"));
    // gawk / mawk = awk
    assert_eq!(out(&s, "gawk '{print $1}' a.txt").lines().count(), 3);
    // less / more = cat
    assert!(out(&s, "less a.txt").contains("bar"));
    // GNU stat -c
    let r = s.execute("stat -c '%n' a.txt");
    assert_eq!(r.exit_code, 0, "stat -c stderr={}", r.stderr);
    // BSD sed -i '' (empty backup suffix) edits in place.
    out(&s, "sed -i '' 's/bar/BAR/' a.txt");
    assert!(out(&s, "cat a.txt").contains("BAR"));
}

#[test]
fn combined_short_flags() {
    let s = setup();
    s.write_file("target.txt", "x").unwrap();
    // ln -sf (combined) is common in scripts.
    let r = s.execute("ln -sf target.txt link.txt");
    assert_eq!(r.exit_code, 0, "ln -sf stderr={}", r.stderr);
    // rm -rf (combined) already works; keep it covered.
    let r = s.execute("rm -rf link.txt");
    assert_eq!(r.exit_code, 0, "rm -rf stderr={}", r.stderr);
}
