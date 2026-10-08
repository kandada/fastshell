// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Fourth big flag batch: grep / xargs / column+seq / misc_utils / tar /
//! pdftotext / ping / cp / alias / final_batch deep flags & error paths.

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::sync::atomic::{AtomicUsize, Ordering};

static SEQ: AtomicUsize = AtomicUsize::new(0);

fn fresh() -> Fastshell {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_big4_{}_{}", std::process::id(), n));
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
    s.write_file("a.txt", "alpha\nbeta\ngamma\ndelta\n")
        .unwrap();
    s.write_file("b.txt", "beta\nx\n").unwrap();
    s.write_file("nums.txt", "10\n2\n33\n").unwrap();
    s.write_file("csv.txt", "a,1\nbb,2\nccc,3\n").unwrap();
    s.write_file("pat.txt", "beta\ndelta\n").unwrap();
    s.execute("mkdir -p sub");
    s.write_file("sub/c.txt", "nested\n").unwrap();
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
#[ignore = "heavy: run with --ignored"]
fn grep_deep() {
    run_all(&[
        "grep -r beta .",
        "grep -R beta .",
        "grep -rl beta .",
        "grep -rh beta .",
        "grep -m0 a a.txt",
        "grep -m2 a a.txt",
        "grep '[' a.txt",
        "grep -E '[' a.txt",
        "grep -f pat.txt a.txt",
        "grep -f nope a.txt",
        "grep --include='*.txt' -r beta .",
        "grep --exclude='b.txt' -r beta .",
        "grep -A2 -B1 beta a.txt",
        "grep -C2 beta a.txt",
        "grep -E '(al|be)ta' a.txt",
        "grep -P '(?<=a)lpha' a.txt",
        "grep -oE '[a-z]+' a.txt",
        "grep -c -r a .",
        "grep -nH beta a.txt",
        "grep -Z -l beta a.txt",
        "grep -a -o a a.txt",
        "grep '' a.txt",
        "grep -v '' a.txt",
        "grep -w 'a' a.txt",
        "grep -x 'beta' a.txt",
        "grep --color=always a a.txt",
        "grep --label=x -H a a.txt",
    ]);
}

#[test]
#[ignore = "heavy: run with --ignored"]
fn xargs_deep() {
    run_all(&[
        "xargs -a a.txt echo",
        "printf 'a\\nb\\n' | xargs -I{} -n1 echo {}",
        "printf 'a\\nb\\n' | xargs -E '' echo",
        "printf 'a b c d\\n' | xargs -n2 echo",
        "printf 'a b c d\\n' | xargs -n10 echo",
        "printf 'a\\nb\\n' | xargs --max-lines=1 echo",
        "printf 'a\\nb\\n' | xargs --max-args=1 echo",
        "printf 'x\\n' | xargs --no-run-if-empty echo",
        "printf 'a\\tb\\n' | xargs -d '\\t' echo",
        "printf 'a\\nb\\n' | xargs -t -n1 echo",
        "printf 'a\\nb\\n' | xargs -P0 echo",
        "printf 'a\\nb\\n' | xargs -s 1000 echo",
        "printf '' | xargs -r echo done",
        "xargs echo < a.txt",
        "printf 'a b\\n' | xargs echo pre",
    ]);
}

#[test]
#[ignore = "heavy: run with --ignored"]
fn column_seq_misc_deep() {
    run_all(&[
        "column -t csv.txt",
        "column -s, -t csv.txt",
        "column -t -s , csv.txt",
        "column -c 10 csv.txt",
        "column -x csv.txt",
        "column -o ':' -t csv.txt",
        "column -e csv.txt",
        "seq 1 1 5",
        "seq 5 -1 1",
        "seq -w 1 10",
        "seq -s ' ' 1 3",
        "seq -f '%g' 1 3",
        "seq 0.5 0.5 2",
        "seq 1 0 5",
        "expr 1 \\& 0",
        "expr 1 \\| 0",
        "expr ! 0",
        "expr 3 \\< 4",
        "expr 4 \\>= 4",
        "expr abc : '\\(a\\)\\(b\\)c'",
        "expr length ''",
        "split -l 2 a.txt s1; ls s1* | sort",
        "split -b 3 a.txt s2; ls s2* | sort",
        "split -d -l 2 a.txt s3; ls s3* | sort",
        "split -a 3 -l 2 a.txt s4; ls s4* | sort",
        "comm -12 a.txt b.txt",
        "comm -13 a.txt b.txt",
        "comm -23 a.txt b.txt",
        "comm -123 a.txt b.txt",
        "xxd a.txt",
        "xxd -p a.txt",
        "xxd -i a.txt",
        "xxd -e a.txt",
        "xxd -g 1 a.txt",
        "xxd -l 4 a.txt",
        "xxd -s 3 a.txt",
        "xxd -c 4 a.txt",
        "od a.txt",
        "od -A x a.txt",
        "od -A d a.txt",
        "od -j 2 -N 3 a.txt",
        "od -t x1z a.txt",
        "od -t d1 a.txt",
        "od -t c a.txt",
    ]);
}

#[test]
#[ignore = "heavy: run with --ignored"]
fn tar_pdf_ping_cp_alias_deep() {
    run_all(&[
        "tar cf x.tar a.txt; tar tf x.tar",
        "tar czf x.tgz a.txt; tar tzf x.tgz",
        "tar cjf x.tbz a.txt; tar tjf x.tbz",
        "tar czf x.tgz a.txt; mkdir d; tar xzf x.tgz -C d",
        "tar czf x.tgz .; tar tzf x.tgz",
        "tar --exclude='*.txt' czf x.tgz .",
        "tar czvf x.tgz a.txt",
        "tar --help",
        "pdftotext a.txt",
        "pdftotext -f 1 -l 1 a.txt",
        "pdftotext -layout a.txt",
        "pdftotext -raw a.txt",
        "pdftotext",
        "pdftotext --help",
        "ping -c 1 127.0.0.1",
        "ping 127.0.0.1",
        "ping --help",
        "cp a.txt c.txt; cat c.txt",
        "cp -v a.txt c2.txt",
        "cp -p a.txt c3.txt",
        "cp -i a.txt c4.txt",
        "cp -L a.txt c5.txt",
        "cp -P a.txt c6.txt",
        "cp -d a.txt c7.txt",
        "cp a.txt sub/; ls sub/a.txt",
        "cp nope.txt c.txt",
        "cp a.txt b.txt sub/; ls sub",
        "alias l='ls'; alias",
        "alias x='echo x'; unalias x; unalias nope",
        "alias",
        "unalias -a",
        "hostid",
        "blkid",
        "lsof",
        "vmstat",
        "iostat",
        "showmount",
    ]);
}
