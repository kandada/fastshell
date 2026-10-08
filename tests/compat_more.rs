// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Additional command compatibility / edge-case coverage for commands whose
//! unit coverage is thin (wc/tr/cut/sort/tee/cat/head/tail/grep/date/printf/...).

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn setup() -> Fastshell {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_compatmore_{}_{}", std::process::id(), n));
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
fn text_utilities() {
    let s = setup();
    assert_eq!(out(&s, "seq 3"), "1\n2\n3\n");
    assert_eq!(out(&s, "echo hello | tr a-z A-Z"), "HELLO\n");
    assert_eq!(out(&s, "printf '%s-%s\\n' a b"), "a-b\n");
    assert_eq!(out(&s, "echo abcdef | cut -c 1-3"), "abc\n");
    assert_eq!(out(&s, "echo a,b,c | cut -d, -f2"), "b\n");
    assert_eq!(out(&s, "echo hello | rev"), "olleh\n");
    assert_eq!(out(&s, "echo abc | fold -w1").lines().count(), 3);
}

#[test]
fn sort_variants() {
    let s = setup();
    s.write_file("n.txt", "10\n2\n1\n").unwrap();
    assert_eq!(out(&s, "sort -n n.txt"), "1\n2\n10\n");
    assert_eq!(out(&s, "sort -nr n.txt"), "10\n2\n1\n");
    s.write_file("d.txt", "b\na\nb\n").unwrap();
    assert_eq!(out(&s, "sort -u d.txt"), "a\nb\n");
    // `uniq -c` needs sorted input (like real uniq).
    s.write_file("u.txt", "a\na\nb\n").unwrap();
    assert!(out(&s, "uniq -c u.txt").contains("2 a"));
}

#[test]
fn head_tail_cat_variants() {
    let s = setup();
    s.write_file("a.txt", "1\n2\n3\n4\n5\n").unwrap();
    assert_eq!(out(&s, "head -n 2 a.txt"), "1\n2\n");
    assert_eq!(out(&s, "tail -n 2 a.txt"), "4\n5\n");
    assert_eq!(out(&s, "head -c 1 a.txt"), "1");
    assert_eq!(out(&s, "cat -n a.txt").lines().count(), 5);
    assert!(out(&s, "cat -n a.txt").contains("     1"));
}

#[test]
fn grep_variants() {
    let s = setup();
    s.write_file("g.txt", "apple\nbanana\napricot\n").unwrap();
    assert_eq!(out(&s, "grep -c ap g.txt").trim(), "2");
    assert_eq!(out(&s, "grep -l apple g.txt").trim(), "g.txt");
    assert_eq!(out(&s, "grep -o 'ap' g.txt").lines().count(), 2);
    assert_eq!(out(&s, "grep -v apple g.txt").lines().count(), 2);
    assert_eq!(out(&s, "grep -i BANANA g.txt").trim(), "banana");
}

#[test]
fn find_stat_and_paths() {
    let s = setup();
    s.write_file("d/x.txt", "x").unwrap();
    s.write_file("d/y.log", "y").unwrap();
    assert!(out(&s, "find d -name '*.txt'").contains("x.txt"));
    assert_eq!(out(&s, "basename /a/b/c.txt"), "c.txt\n");
    assert_eq!(out(&s, "dirname /a/b/c.txt"), "/a/b\n");
    assert!(out(&s, "stat -c '%n' d/x.txt").contains("x.txt"));
}

#[test]
fn tee_and_redirect() {
    let s = setup();
    out(&s, "echo one | tee out.txt");
    out(&s, "echo two | tee -a out.txt");
    assert_eq!(out(&s, "cat out.txt"), "one\ntwo\n");
}

#[test]
fn jq_and_json() {
    let s = setup();
    assert_eq!(out(&s, "jq -n '1 + 1'").trim(), "2");
    assert_eq!(out(&s, "jq -n '[1,2,3] | length'").trim(), "3");
    assert_eq!(out(&s, "jq -n '{a:1} | .a'").trim(), "1");
}

#[test]
fn ln_sf_and_rm_rf() {
    let s = setup();
    s.write_file("t.txt", "x").unwrap();
    // `ln -sf` (combined) creates a working symlink.
    out(&s, "ln -sf t.txt l.txt");
    assert!(out(&s, "cat l.txt").contains("x"));
    // `rm -rf` (combined) removes a regular file.
    s.write_file("junk.txt", "y").unwrap();
    out(&s, "rm -rf junk.txt");
    assert!(!out(&s, "ls").contains("junk.txt"));
}

#[test]
fn multiline_for_with_trailing_command() {
    let s = setup();
    // Multi-line for loop followed by another command (the [187] pattern).
    let cmd = "X=hi\nfor i in 1 2 3; do\n  echo \"$X-$i\"\ndone\necho done-all";
    let r = s.execute(cmd);
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert_eq!(
        r.stdout, "hi-1\nhi-2\nhi-3\ndone-all\n",
        "got: {}",
        r.stdout
    );
}

#[test]
fn multiline_for_no_trailing() {
    let s = setup();
    let cmd = "X=hi\nfor i in 1 2; do\n  echo \"$X-$i\"\ndone";
    let r = s.execute(cmd);
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert_eq!(r.stdout, "hi-1\nhi-2\n", "got: {}", r.stdout);
}

#[test]
fn multiline_for_with_amp_urls() {
    let s = setup();
    let cmd = r#"UA="Mozilla/5.0 test"
for pid in 12787 3199015; do
  echo "curl -A \"$UA\" -H \"X-Requested-With: XMLHttpRequest\" \"https://x/ashx?t=tginfo&pid=$pid&r=0.5\" -o \"tu_$pid.json\""
  echo "fetched $pid"
done
echo after"#;
    let r = s.execute(cmd);
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("fetched 12787"), "got: {}", r.stdout);
    assert!(r.stdout.contains("fetched 3199015"), "got: {}", r.stdout);
    assert!(r.stdout.ends_with("after\n"), "got: {}", r.stdout);
}

#[test]
fn grep_h_suppresses_filename() {
    let s = setup();
    s.write_file("a.txt", "foo\nbar\n").unwrap();
    s.write_file("b.txt", "foo\n").unwrap();
    // Multiple files → filename prefix by default.
    let with = out(&s, "grep foo a.txt b.txt");
    assert!(with.contains("a.txt:foo"), "got: {with}");
    // `-h` (GNU: --no-filename) suppresses it.
    let without = out(&s, "grep -h foo a.txt b.txt");
    assert_eq!(without, "foo\nfoo\n", "got: {without}");
    // `--help` still shows help.
    let help = out(&s, "grep --help");
    assert!(help.contains("Usage"), "got: {help}");
}
