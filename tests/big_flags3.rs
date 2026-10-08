// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Final flag batch for grep / xargs / sort / misc_utils / ps / touch / js.

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::sync::atomic::{AtomicUsize, Ordering};

static SEQ: AtomicUsize = AtomicUsize::new(0);

fn fresh() -> Fastshell {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_big3_{}_{}", std::process::id(), n));
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
    s.write_file("b.txt", "beta\ndelta\n").unwrap();
    s.write_file("nums.txt", "10\n2\n33\n").unwrap();
    s.write_file("pat.txt", "beta\n").unwrap();
    s.write_file("code.js", "var x = 1;\n").unwrap();
    s
}

fn run_all(cases: &[&str]) {
    for cmd in cases {
        let s = fresh();
        let r = s.execute(cmd);
        assert!(
            (0..=255).contains(&r.exit_code),
            "cmd {cmd:?} exit {}: {}",
            r.exit_code,
            r.stderr
        );
    }
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn grep_more_flags() {
    run_all(&[
        "grep -a alpha a.txt",
        "grep -I alpha a.txt",
        "grep -d skip alpha a.txt",
        "grep -z alpha a.txt",
        "grep -Z alpha a.txt",
        "grep -T alpha a.txt",
        "grep -U alpha a.txt",
        "grep -f pat.txt a.txt",
        "grep --null -l alpha a.txt",
        "grep --line-buffered alpha a.txt",
        "grep --color=never alpha a.txt",
        "grep -c -v alpha a.txt",
        "grep -o -E 'a[a-z]+' a.txt",
        "grep -n -i -w alpha a.txt",
        "grep -E '(alpha|gamma)' a.txt",
        "grep -E 'a{1,2}' a.txt",
        "grep -F -e alpha -e beta a.txt",
        "grep -v -e alpha -e beta a.txt",
        "grep '\\<alpha\\>' a.txt",
        "grep -q -E 'alpha' a.txt && echo found",
    ]);
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn xargs_more_flags() {
    run_all(&[
        "printf 'a\\nb\\n' | xargs -n1 -I{} echo {}",
        "printf 'a b c\\n' | xargs -n3 echo",
        "printf 'a\\nb\\n' | xargs -L2 echo",
        "printf 'a b\\n' | xargs -s 100 echo",
        "printf 'a\\0b\\0' | xargs -0 -n1 echo",
        "printf 'a,b,c' | xargs -d, echo",
        "printf 'x\\n' | xargs -t echo",
        "printf 'x\\n' | xargs -r echo",
        "printf 'x\\ny\\n' | xargs -P1 echo",
        "printf 'x\\n' | xargs echo extra1 extra2",
        "xargs -a a.txt echo",
        "printf '' | xargs echo",
        "printf 'x\\n' | xargs",
    ]);
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn sort_and_misc_more() {
    run_all(&[
        "sort -M a.txt",
        "sort -R nums.txt",
        "sort -d a.txt",
        "sort -n -k1 nums.txt",
        "sort -k1,1 a.txt",
        "sort -t, -k2,2 csv.txt",
        "sort -m nums.txt nums.txt",
        "sort --help",
        "printf 'a\\nb\\n' | comm - -",
        "expr 2 + 3 \\* 4",
        "expr \\( 2 + 3 \\) \\* 4",
        "expr 0",
        "expr abc : 'a\\(b\\)c'",
        "seq 5 -1 1",
        "seq 0.5 0.5 2",
        "seq -w 1 10",
        "seq -s: 1 3",
        "split -n 2 a.txt sn; ls sn* | sort",
        "xxd -g 1 a.txt",
        "xxd -e a.txt",
        "xxd -i a.txt",
        "od -c a.txt",
        "od -f a.txt",
        "od -v a.txt",
        "od --help",
    ]);
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn ps_touch_js_more() {
    run_all(&[
        "ps -o pid,comm",
        "ps -u",
        "ps -p 1",
        "ps -C sh",
        "ps --sort pid",
        "ps w",
        "ps -w",
        "touch -a a.txt",
        "touch -m a.txt",
        "touch -r b.txt a.txt",
        "touch -c nope.txt",
        "touch -h a.txt",
        "touch --help",
        "jscheck code.js",
        "node --check code.js",
        "node -e '1+1'",
        "jslint code.js",
        "js --version",
        "jscheck --help",
    ]);
}
