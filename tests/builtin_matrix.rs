// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 内建 × flag 矩阵：对每个命令的每个（常用）flag 生成一条确定性用例，与真实
//! bash 差分。手写用例覆盖“命令存在”，矩阵覆盖“命令的每个开关都行为一致”。
//!
//! 合法差异见 `tests/known_divergences.txt`。

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::fs;

mod common;

fn setup() -> (Fastshell, std::path::PathBuf) {
    let fs_dir = std::env::temp_dir().join(format!("fs_matrix_fs_{}", std::process::id()));
    let sh_dir = std::env::temp_dir().join(format!("fs_matrix_sh_{}", std::process::id()));
    for d in [&fs_dir, &sh_dir] {
        let _ = fs::remove_dir_all(d);
        fs::create_dir_all(d).unwrap();
    }
    let files: &[(&str, &str)] = &[
        ("a.txt", "alpha\nbeta\ngamma\n"),
        ("b.txt", "beta\n"),
        ("c.log", "gamma\ndelta\n"),
        ("nums.txt", "10\n2\n33\n"),
        ("csv.txt", "a,1\nb,2\nc,3\n"),
        ("dup.txt", "x\nx\ny\nz\n"),
        ("sp.txt", "  leading\ntrailing  \n\ttabbed\n"),
    ];
    for (name, body) in files {
        fs::write(sh_dir.join(name), body).unwrap();
    }
    fs::create_dir_all(sh_dir.join("sub")).unwrap();
    fs::write(sh_dir.join("sub/d.txt"), "delta\n").unwrap();

    let mut sdk = Fastshell::new();
    sdk.init(Config {
        sandbox_path: fs_dir.to_string_lossy().to_string(),
        python_enabled: false,
        allow_subprocess: true,
        network_ask_permission: false,
        command_timeout_ms: 15_000,
        ..Default::default()
    })
    .unwrap();
    for (name, body) in files {
        sdk.write_file(name, body).unwrap();
    }
    sdk.execute("mkdir -p sub");
    sdk.write_file("sub/d.txt", "delta\n").unwrap();
    (sdk, sh_dir)
}

/// (command-line, why) — grouped per command; each is one matrix cell.
const MATRIX: &[&str] = &[
    // echo
    "echo hello",
    "echo -n hello",
    "echo -e 'a\\tb'",
    "echo a b c",
    // printf
    "printf '%s\\n' a b",
    "printf '%d\\n' 42",
    "printf '%05d\\n' 7",
    "printf '%-5s|\\n' a",
    "printf '%x\\n' 255",
    "printf '%o\\n' 8",
    "printf '%c\\n' abc",
    "printf '%.2f\\n' 3.14159",
    "printf '%s %s\\n' a b",
    "printf 'no-newline'",
    "printf 'a\\nb\\n'",
    // head / tail
    "head -n 2 a.txt",
    "head -n 1 a.txt",
    "head -c 3 a.txt",
    "head -q a.txt b.txt",
    "head -v a.txt",
    "tail -n 1 a.txt",
    "tail -n 2 a.txt",
    "tail -c 3 a.txt",
    "tail -q a.txt b.txt",
    // wc
    "wc -l a.txt",
    "wc -w a.txt",
    "wc -c a.txt",
    "wc -L a.txt",
    "wc a.txt b.txt",
    // sort
    "sort nums.txt",
    "sort -n nums.txt",
    "sort -r nums.txt",
    "sort -u dup.txt",
    "sort -f a.txt",
    "sort -k1 csv.txt",
    "sort -t, -k2 csv.txt",
    "sort -nr nums.txt",
    // uniq
    "uniq dup.txt",
    "uniq -c dup.txt",
    "uniq -d dup.txt",
    "uniq -u dup.txt",
    "uniq -i dup.txt",
    // cut
    "cut -d, -f1 csv.txt",
    "cut -d, -f2 csv.txt",
    "cut -c1-3 a.txt",
    "cut -c2 a.txt",
    "cut -d, -f1,2 csv.txt",
    "cut -s -d, -f1 a.txt",
    // tr
    "tr a-z A-Z < a.txt",
    "tr -d 'aeiou' < a.txt",
    "tr -s ' ' < sp.txt",
    "tr -c 'a-z' '.' < a.txt",
    // grep
    "grep alpha a.txt",
    "grep -i ALPHA a.txt",
    "grep -v alpha a.txt",
    "grep -n alpha a.txt",
    "grep -c a a.txt",
    "grep -l alpha a.txt b.txt",
    "grep -o 'a.a' a.txt",
    "grep -w alpha a.txt",
    "grep -E 'a|b' a.txt",
    "grep -F 'a.b' a.txt || echo none",
    "grep -r alpha . | sort",
    "grep -A1 beta a.txt",
    "grep -B1 beta a.txt",
    "grep -q alpha a.txt; echo $?",
    // sed
    "sed -n '1p' a.txt",
    "sed 's/a/X/' a.txt",
    "sed 's/a/X/g' a.txt",
    "sed -e 's/a/1/' -e 's/b/2/' a.txt",
    "sed '1d' a.txt",
    "sed -n '1,2p' a.txt",
    "sed 's/^/> /' a.txt",
    "sed '2q' a.txt",
    // awk
    "awk '{print $1}' a.txt",
    "awk '{print NR\": \"$0}' a.txt",
    "awk 'NR==1' a.txt",
    "awk -F, '{print $2}' csv.txt",
    "awk -v x=5 'BEGIN{print x}'",
    "awk '{s+=$1} END{print s}' nums.txt",
    "awk '{print NF}' csv.txt",
    "awk 'END{print NR}' a.txt",
    "awk '/alpha/{print}' a.txt",
    "awk 'BEGIN{print 1+2*3}'",
    // seq / expr
    "seq 1 3",
    "seq 3",
    "seq 2 2 8",
    "seq -s, 1 3",
    "seq -w 8 10",
    "expr 3 + 4",
    "expr 10 - 3",
    "expr 3 \\* 4",
    "expr 10 / 3",
    "expr 10 % 3",
    "expr abc : 'a.c'",
    // base64 / checksums
    "printf hi | base64",
    "printf aGk= | base64 -d",
    "printf hi | md5sum | cut -c1-8",
    "printf hi | sha1sum | cut -c1-8",
    "printf hi | sha256sum | cut -c1-8",
    // basename / dirname
    "basename /a/b/c.txt",
    "basename /a/b/c.txt .txt",
    "basename a/b/",
    "dirname /a/b/c.txt",
    "dirname c.txt",
    // text extras
    "rev <<< abc",
    "nl a.txt",
    "nl -ba a.txt",
    "fold -w 3 a.txt",
    "expand sp.txt",
    "paste -d, a.txt b.txt",
    "comm -12 a.txt b.txt",
    "column -t csv.txt",
    "column -s, -t csv.txt",
    "split -l 1 a.txt p; ls p* | sort",
    // test / [
    "[ -f a.txt ]; echo $?",
    "[ -d sub ]; echo $?",
    "[ -e nope ]; echo $?",
    "[ -z '' ]; echo $?",
    "[ -n abc ]; echo $?",
    "[ 3 -gt 2 ]; echo $?",
    "[ abc = abc ]; echo $?",
    "test -s a.txt; echo $?",
    // [[ ]] (bash5)
    "[[ -f a.txt ]] && echo y || echo n",
    "[[ abc =~ ^a.c$ ]] && echo y || echo n",
    "[[ 3 -gt 2 && -f a.txt ]] && echo y || echo n",
    "[[ \"a b\" == *\" \"* ]] && echo y || echo n",
    // string / array / param
    "v=abc; echo ${#v}",
    "v=abc; echo ${v^^}",
    "v=ABC; echo ${v,,}",
    "v=abcdef; echo ${v:2:3}",
    "v=a.b.c; echo ${v%.*}",
    "v=abcabc; echo ${v//a/X}",
    "a=(x y z); echo ${#a[@]}",
    // files
    "cp a.txt a2.txt && cat a2.txt",
    "mv a2.txt a3.txt && cat a3.txt",
    "rm -f a3.txt; echo done",
    "mkdir -p x/y/z && ls -d x/y/z",
    "rmdir x/y/z x/y x",
    "ln -s a.txt lnk && readlink lnk",
    "readlink -f a.txt >/dev/null && echo real",
    "touch t1 && [ -f t1 ] && echo made",
    "echo data > f.txt && cat f.txt",
    "echo more >> f.txt && wc -l < f.txt",
    "truncate -s 2 f.txt && wc -c < f.txt",
    "cat a.txt > o.txt; cat o.txt",
    // ls / tree (formatting may differ → allowlisted if needed)
    "ls -1 | sort",
    "ls -a | sort",
    "ls sub",
    // pipes / redirection
    "cat a.txt | grep alpha | wc -l",
    "cat a.txt | head -n 1",
    "cat a.txt | sort | head -n 1",
    "echo x 2>/dev/null; echo done",
    "printf 'a\\nb\\n' | tee o2.txt >/dev/null; cat o2.txt",
    "cat < a.txt | wc -l",
    "cat a.txt 1>/dev/null; echo ok",
    // control flow
    "for i in 1 2 3; do echo $i; done",
    "i=0; while [ $i -lt 2 ]; do echo $i; i=$((i+1)); done",
    "if true; then echo t; else echo f; fi",
    "if false; then echo t; else echo f; fi",
    "case abc in a*) echo A;; *) echo B;; esac",
    "f(){ echo \"f:$1\"; }; f x",
    "f(){ return 5; }; f; echo $?",
    "unset v; f(){ local v=1; echo $v; }; f; echo \"[$v]\"",
    "set -- a b c; echo $#; echo $1; echo $*",
    "i=1; ((i+=2)); echo $i",
    "i=1; ((i++)); echo $i",
    // misc builtins
    "true; echo $?",
    "false; echo $?",
    ": ; echo colon",
    "printf 'x' > /dev/null; echo redir",
    "yes x | head -n 2",
    "timeout 5 echo quick",
    "printf 'a\\0b' | od -c | head -1",
];

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn fastshell_matches_bash_on_builtin_flag_matrix() {
    let bin = common::oracle_bash();
    if !common::runnable(&bin) {
        eprintln!("bash oracle not found; skipping matrix");
        return;
    }
    let major = common::bash_major(&bin);
    let (allow_always, allow_bash3) = common::load_allowlist();
    let (sdk, sh_dir) = setup();
    let mut failures = Vec::new();

    for cmd in MATRIX {
        if allow_always.iter().any(|a| cmd.contains(a.as_str()))
            || (major < 4 && allow_bash3.iter().any(|a| cmd.contains(a.as_str())))
        {
            continue;
        }
        let _ = sdk.execute("cd /");
        let f = sdk.execute(cmd);
        let (b_raw, b_code) = common::run_oracle(&bin, &sh_dir, cmd);
        let b_out = common::norm(&b_raw);
        let f_out = common::norm(&f.stdout);
        if f_out != b_out || f.exit_code != b_code {
            failures.push(format!(
                "cmd: {cmd}\n  fs : exit={} out={f_out:?}\n  bash: exit={b_code} out={b_out:?}",
                f.exit_code
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "fastshell != bash for {} of {} matrix cell(s):\n\n{}",
        failures.len(),
        MATRIX.len(),
        failures.join("\n\n")
    );
}
