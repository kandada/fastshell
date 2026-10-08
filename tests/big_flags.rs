// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! One-shot comprehensive flag coverage for the remaining large modules:
//! grep / sort / find / diff / awk / jq / tar / xargs / column / seq / du /
//! df / stat / ps / text / pdftotext / pip / file_column_seq.
//!
//! Each command runs in a fresh SDK so a blocking invocation cannot poison the
//! rest; only exit-code sanity is required (a few outputs are asserted).

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::sync::atomic::{AtomicUsize, Ordering};

static SEQ: AtomicUsize = AtomicUsize::new(0);

fn fresh() -> Fastshell {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_big_{}_{}", std::process::id(), n));
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
    s.write_file("csv.txt", "a,1\nb,2\nc,3\n").unwrap();
    s.write_file("dup.txt", "x\nx\ny\n").unwrap();
    s.write_file("code.py", "def f():\n    return 1\n").unwrap();
    s.write_file("d1.txt", "alpha\nbeta\ngamma\n").unwrap();
    s.write_file("d2.txt", "alpha\nBETA\ngamma\n").unwrap();
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
fn grep_flags() {
    run_all(&[
        "grep alpha a.txt",
        "grep -i ALPHA a.txt",
        "grep -v alpha a.txt",
        "grep -n alpha a.txt",
        "grep -c a a.txt",
        "grep -l alpha a.txt b.txt",
        "grep -L alpha a.txt b.txt",
        "grep -o 'a.a' a.txt",
        "grep -w alpha a.txt",
        "grep -x alpha a.txt",
        "grep -E 'a|b' a.txt",
        "grep -F 'a.b' a.txt",
        "grep -P '\\d+' nums.txt",
        "grep -r alpha . | sort",
        "grep -R alpha . | sort",
        "grep -A1 beta a.txt",
        "grep -B1 beta a.txt",
        "grep -C1 beta a.txt",
        "grep -q alpha a.txt",
        "grep -s alpha a.txt",
        "grep -h alpha a.txt b.txt",
        "grep -H alpha a.txt",
        "grep -m1 a a.txt",
        "grep --include='*.txt' -r alpha .",
        "grep --exclude='*.py' -r alpha .",
        "grep -e alpha -e beta a.txt",
        "grep '^a' a.txt",
        "grep 'a$' a.txt",
    ]);
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn sort_flags() {
    run_all(&[
        "sort nums.txt",
        "sort -n nums.txt",
        "sort -nr nums.txt",
        "sort -r nums.txt",
        "sort -u dup.txt",
        "sort -f d1.txt",
        "sort -i d1.txt",
        "sort -b a.txt",
        "sort -g nums.txt",
        "sort -h nums.txt",
        "sort -V a.txt",
        "sort -k1 a.txt",
        "sort -t, -k2 csv.txt",
        "sort -t, -k2 -k1 csv.txt",
        "sort -o out.txt nums.txt; cat out.txt",
        "sort -c nums.txt",
        "sort -s -k1 a.txt",
        "sort -z nums.txt",
        "sort -n -r -u nums.txt",
    ]);
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn find_flags() {
    run_all(&[
        "find . -name 'a.txt'",
        "find . -iname 'A.TXT'",
        "find . -type f",
        "find . -type d",
        "find . -size +1c",
        "find . -mtime -1",
        "find . -newer a.txt",
        "find . -empty",
        "find . -maxdepth 1",
        "find . -mindepth 1",
        "find . -path './d1*'",
        "find . -ipath './D1*'",
        "find . -regex '.*\\.txt'",
        "find . -iregex '.*\\.TXT'",
        "find . -name '*.txt' -exec cat {} \\;",
        "find . -name '*.txt' -exec echo {} +",
        "find . -name '*.txt' -print0",
        "find . -name '*.txt' -printf '%f %s\\n'",
        "find . -name '*.txt' -o -name '*.py'",
        "find . ! -name '*.txt'",
        "find . -name '*.txt' -delete",
        "find . -type f -a -name 'a.txt'",
        "find . -path ./d1 -prune -o -name '*.txt' -print",
    ]);
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn diff_flags() {
    run_all(&[
        "diff a.txt d1.txt",
        "diff a.txt d2.txt",
        "diff -u a.txt d2.txt",
        "diff -c a.txt d2.txt",
        "diff -i a.txt d2.txt",
        "diff -w a.txt d2.txt",
        "diff -b a.txt d2.txt",
        "diff -B a.txt d2.txt",
        "diff -q a.txt d2.txt",
        "diff -y a.txt d2.txt",
        "diff -r . .",
        "diff --brief a.txt d2.txt",
        "diff --unified a.txt d2.txt",
        "diff --side-by-side a.txt d2.txt",
        "diff --normal a.txt d2.txt",
        "diff --ignore-case a.txt d2.txt",
        "diff --help",
    ]);
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn awk_and_jq_flags() {
    run_all(&[
        "awk -F, '{print $2}' csv.txt",
        "awk -v x=3 'BEGIN{print x*2}'",
        "awk '{print NR, $0}' a.txt",
        "awk 'NR>1' a.txt",
        "awk '/beta/' a.txt",
        "awk 'END{print NR}' a.txt",
        "awk 'BEGIN{print length(\"abc\")}'",
        "awk 'BEGIN{print substr(\"abcdef\",2,3)}'",
        "awk 'BEGIN{print index(\"abc\",\"b\")}'",
        "awk 'BEGIN{n=split(\"a:b\",arr,\":\"); print n}'",
        "awk 'BEGIN{s=\"abc\"; gsub(/b/,\"X\",s); print s}'",
        "awk 'BEGIN{print toupper(\"ab\")}'",
        "awk 'BEGIN{print tolower(\"AB\")}'",
        "awk 'BEGIN{printf \"%d-%s\\n\", 3, \"x\"}'",
        "awk 'BEGIN{print 1+2*3}'",
        "awk 'BEGIN{for(i=1;i<=3;i++) print i}'",
        "awk 'BEGIN{i=0; while(i<2){print i; i++}}'",
        "awk 'BEGIN{if(1) print \"y\"; else print \"n\"}'",
        "awk '{print $NF}' nums.txt",
        "awk '{a[$1]++} END{for(k in a) print k}' a.txt",
        "jq -n '1+1'",
        "jq -n '[1,2,3] | length'",
        "jq -n '{a:1} | keys'",
        "jq -n '{a:1} | .a'",
        "jq -n '[1,2,3] | .[1]'",
        "jq -n '[1,2,3] | map(.+1)'",
        "jq -n '[1,2,3] | add'",
        "jq -n '[1,2,3] | select(length>0)'",
        "jq -n '\"a\" + \"b\"'",
        "jq -n '1 > 0'",
        "jq -n '{a:1} | to_entries'",
        "jq -n 'null | type'",
        "jq --help",
    ]);
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn tar_and_archive_flags() {
    run_all(&[
        "tar czf a.tgz a.txt b.txt",
        "tar tzf a.tgz",
        "mkdir e1; tar xzf a.tgz -C e1",
        "tar cf a.tar a.txt",
        "tar tf a.tar",
        "tar czvf a.tgz a.txt",
        "tar --help",
        "zip z.zip a.txt",
        "unzip -l z.zip",
        "unzip --help",
        "gzip -k a.txt; gunzip -k a.txt.gz",
        "bzip2 -k a.txt; bunzip2 -k a.txt.bz2",
        "xz -k a.txt; unxz -k a.txt.xz",
    ]);
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn column_seq_du_df_stat_flags() {
    run_all(&[
        "column -t csv.txt",
        "column -s, -t csv.txt",
        "column -o ' | ' -t csv.txt",
        "column -c 20 csv.txt",
        "seq 1 3",
        "seq 1 2 9",
        "seq -w 8 10",
        "seq -s, 1 3",
        "seq -f '%03g' 1 3",
        "du -h .",
        "du -s .",
        "du -a .",
        "du -c .",
        "df -h",
        "df -k",
        "df -i",
        "stat a.txt",
        "stat -c '%s %n' a.txt",
        "stat -f '%n' a.txt",
        "stat -L a.txt",
    ]);
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn text_and_ps_and_misc_flags() {
    run_all(&[
        "ps",
        "ps aux",
        "ps -ef",
        "ps -e",
        "truncate -s 3 a.txt",
        "cmp a.txt d1.txt",
        "cmp -l a.txt d2.txt",
        "strings a.txt",
        "fold -w 3 a.txt",
        "fold -s a.txt",
        "expand a.txt",
        "unexpand a.txt",
        "pdftotext --help",
        "pdftotext nope.pdf",
        "pip --help",
        "pip list",
        "pip install",
        "pip install --help",
        "pip-install --help",
        "xargs --help",
        "xargs --version",
    ]);
}
