// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Flag/edge coverage for the remaining low-coverage command modules:
//! hashsum, xargs, misc_utils, sys_more, sys_utils, base64, touch, mkdir,
//! export.

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::sync::atomic::{AtomicUsize, Ordering};

static SEQ: AtomicUsize = AtomicUsize::new(0);

fn setup() -> Fastshell {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_flags_{}_{}", std::process::id(), n));
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

fn run(s: &Fastshell, cmd: &str) -> (i32, String) {
    let r = s.execute(cmd);
    assert!(
        (0..=255).contains(&r.exit_code),
        "cmd {cmd:?} exit {}",
        r.exit_code
    );
    (r.exit_code, r.stdout)
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn hashsum_flags() {
    let s = setup();
    // single / multiple files
    assert!(run(&s, "sha256sum a.txt").1.contains("a.txt"));
    assert!(run(&s, "sha512sum a.txt").1.contains("a.txt"));
    assert!(run(&s, "md5sum a.txt b.txt").1.contains("b.txt"));
    // check mode
    run(&s, "md5sum a.txt > s.md5");
    let out = run(&s, "md5sum -c s.md5").1;
    assert!(out.contains("OK"), "{out}");
    run(&s, "sha256sum a.txt > s.sha");
    let out = run(&s, "sha256sum -c s.sha").1;
    assert!(out.contains("OK"), "{out}");
    // missing file → exercised (exit code not asserted)
    let _ = run(&s, "md5sum nope.txt");
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn xargs_flags() {
    let s = setup();
    assert_eq!(run(&s, "printf 'a b c\\n' | xargs -n1 echo").1, "a\nb\nc\n");
    assert_eq!(run(&s, "printf 'a b c\\n' | xargs -n2 echo").1, "a b\nc\n");
    assert_eq!(run(&s, "printf 'x\\ny\\n' | xargs -L1 echo").1, "x\ny\n");
    assert_eq!(run(&s, "printf 'a\\0b\\0' | xargs -0 echo").1, "a b\n");
    assert_eq!(
        run(&s, "printf 'a\\nb\\n' | xargs -I{} echo \"[{}]\"").1,
        "[a]\n[b]\n"
    );
    assert_eq!(
        run(&s, "printf 'abcdef\\n' | xargs -s 4 echo").1,
        "abcdef\n"
    );
    let _ = run(&s, "printf 'a b\\n' | xargs -t echo");
    let _ = run(&s, "printf '' | xargs -r echo");
    let _ = run(&s, "printf 'a,b,c\\n' | xargs -d, echo");
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn misc_utils_edges() {
    let s = setup();
    assert_eq!(run(&s, "expr 6 \\* 7").1.trim(), "42");
    assert_eq!(run(&s, "expr 10 % 3").1.trim(), "1");
    assert_eq!(run(&s, "expr abc : 'a.c'").1.trim(), "3");
    assert_eq!(run(&s, "split -b 4 a.txt p; ls p* | sort").0, 0);
    assert_eq!(run(&s, "comm -12 a.txt b.txt").1.trim(), "beta");
    assert_eq!(run(&s, "comm -3 a.txt b.txt").0, 0);
    assert_eq!(run(&s, "printf hi | base64").1.trim(), "aGk=");
    assert_eq!(run(&s, "printf aGk= | base64 -d").1.trim(), "hi");
    let _ = run(&s, "xxd a.txt");
    let _ = run(&s, "printf '0000000: 68 69\\n' | xxd -r");
    let _ = run(&s, "od -An -c a.txt");
    let _ = run(&s, "printf 'a b\\n' | paste -s -d, -");
    let _ = run(&s, "paste -d, a.txt b.txt");
    let _ = run(&s, "nl -ba a.txt");
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn sys_more_and_utils() {
    let s = setup();
    let _ = run(&s, "sum a.txt");
    let _ = run(&s, "sha1sum a.txt");
    let _ = run(&s, "sha3sum a.txt");
    let _ = run(&s, "pidof sh");
    let _ = run(&s, "nproc");
    let _ = run(&s, "nice echo hi");
    let _ = run(&s, "dd if=a.txt of=out.bin");
    let _ = run(&s, "od -c a.txt");
    let _ = run(&s, "uptime");
    let _ = run(&s, "free");
    let _ = run(&s, "groups");
    let _ = run(&s, "hexdump a.txt");
    let _ = run(&s, "dmesg");
    let _ = run(&s, "pstree");
    let _ = run(&s, "killall definitely_not_running_xyz");
    let _ = run(&s, "logname");
    let _ = run(&s, "who");
    // tsort from a file and from stdin
    s.write_file("edges.txt", "a b\nb c\n").unwrap();
    assert!(run(&s, "tsort edges.txt").1.contains('a'));
    assert!(run(&s, "printf 'x y\\n' | tsort").1.contains('x'));
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn touch_mkdir_export_flags() {
    let s = setup();
    let _ = run(&s, "touch -t 202001010000 old.txt");
    assert_eq!(run(&s, "ls old.txt").0, 0);
    let _ = run(&s, "touch -d 2020-01-01 d.txt");
    let _ = run(&s, "mkdir -p x/y/z");
    assert_eq!(run(&s, "ls -d x/y/z").0, 0);
    let _ = run(&s, "mkdir -m 755 m1");
    assert_eq!(run(&s, "export X=1; echo $X").1.trim(), "1");
    let _ = run(&s, "export -p");
    let _ = run(&s, "export X; export -n X; echo done");
}
