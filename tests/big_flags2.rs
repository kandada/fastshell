// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Final coverage batch: shell builtins, parser/executor edges, curl/pip/awk/jq
//! extras.

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::sync::atomic::{AtomicUsize, Ordering};

static SEQ: AtomicUsize = AtomicUsize::new(0);

fn fresh() -> Fastshell {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_big2_{}_{}", std::process::id(), n));
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
    s.write_file("nums.txt", "10\n2\n33\n").unwrap();
    s.write_file("csv.txt", "a,1\nb,2\n").unwrap();
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
fn shell_builtins() {
    run_all(&[
        "set -u; echo ok",
        "set +u; echo ok",
        "set -o pipefail; echo ok",
        "set +o pipefail; echo ok",
        "set -x; echo hi; set +x",
        "unset NOPE; echo ok",
        "declare x=1; echo $x",
        "declare -a arr=(1 2); echo ${arr[0]}",
        "declare -i n=2; echo $n",
        "readonly r=1; echo $r",
        "f(){ local v=1; echo $v; }; f",
        "eval 'echo evaled'",
        "echo 'echo sourced' > s.sh; source s.sh",
        "echo 'echo dotted' > d.sh; . d.sh",
        "type echo",
        "type -t echo",
        "command -v echo",
        "command echo hi",
        "which echo",
        "help echo",
        "hash",
        "history",
        "times",
        "umask",
        "umask 022",
        "ulimit",
        "let x=1+1; echo $x",
        "shift 0; echo ok",
        "set -- a b c; shift 2; echo $1",
        "set -e; false || true; set +e; echo ok",
        "printf '%s\\n' a b c",
        "read -r line < a.txt; echo $line",
        "exec echo execed",
        "trap 'echo bye' EXIT",
        "export A=1 B=2; echo $A$B",
        "unset A B; echo done",
    ]);
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn parser_and_executor_edges() {
    run_all(&[
        // malformed / edge syntax must not panic
        "if true; then echo a; fi",
        "if false; then echo a; else echo b; fi",
        "if false; then echo a; elif true; then echo b; fi",
        "for i in 1 2; do echo $i; done",
        "while false; do echo x; done; echo done",
        "until true; do echo x; done; echo done",
        "case x in x) echo m;; *) echo n;; esac",
        "case x in a|x) echo alt;; esac",
        "{ echo a; echo b; }",
        "( echo a; echo b )",
        "f(){ return 3; }; f; echo $?",
        "f(){ echo $1 $2; }; f a b",
        "echo $((1+2))",
        "echo $(( (1+2) * 3 ))",
        "i=1; ((i++)); echo $i",
        "a=(x y z); echo ${a[1]} ${#a[@]}",
        "v=abc; echo ${v:1:2} ${v^^} ${v%c}",
        "echo ${NOPE:-def}",
        "x=$(echo nested); echo $x",
        "echo `echo backtick`",
        "echo $'a\\tb'",
        "echo \"a b\" 'c d'",
        "printf 'x\\n' | cat | cat",
        "echo one | { read v; echo \"$v\"; }",
        "true && echo t || echo f",
        "false && echo t || echo f",
        "echo a; echo b; echo c",
        "if [ -f a.txt ]; then echo y; fi",
        "if [ 1 -eq 1 ] && [ 2 -gt 1 ]; then echo y; fi",
        "[[ a == a ]] && echo y",
        "[[ abc =~ ^a ]] && echo y",
        "for f in *.txt; do echo $f; done | sort",
        "echo {a,b}{1,2}",
        "echo $((0x10))",
        "cat < a.txt | wc -l",
        "echo hi > o.txt; cat o.txt; rm o.txt",
        "echo hi >> o.txt; cat o.txt",
        "echo x 2>/dev/null; echo y",
        "true | false; echo $?",
        "! true; echo $?",
    ]);
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn curl_more_flags() {
    run_all(&[
        "curl --help",
        "curl -V",
        "curl",
        "curl -s http://127.0.0.1:1/",
        "curl -s -O http://127.0.0.1:1/",
        "curl -s -x http://127.0.0.1:1 http://127.0.0.1:1/",
        "curl -s --connect-timeout 1 http://127.0.0.1:1/",
        "curl -s --retry 1 --retry-delay 0 http://127.0.0.1:1/",
        "curl -s --data-binary 'x' http://127.0.0.1:1/",
        "curl -s -F 'f=@a.txt' http://127.0.0.1:1/",
        "curl -s -D - http://127.0.0.1:1/",
        "curl -s -r 0-1 http://127.0.0.1:1/",
        "curl -s --limit-rate 1000 http://127.0.0.1:1/",
        "curl -s -4 http://127.0.0.1:1/",
        "curl -s -6 http://127.0.0.1:1/",
        "curl -s --compressed http://127.0.0.1:1/",
        "curl -s -j http://127.0.0.1:1/",
        "curl -s -R http://127.0.0.1:1/",
        "curl -s -z '2020-01-01' http://127.0.0.1:1/",
        "curl -s --cacert /nope http://127.0.0.1:1/",
        "curl -s --interface lo http://127.0.0.1:1/",
        "curl -s -w '\\n%{http_code} %{time_total}\\n' http://127.0.0.1:1/",
        "curl -s -o /dev/null -w '%{size_download}' http://127.0.0.1:1/",
    ]);
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn pip_and_misc_more() {
    run_all(&[
        "pip --help",
        "pip list",
        "pip freeze",
        "pip show nope",
        "pip uninstall nope",
        "pip install",
        "pip install --help",
        "pip-install --list",
        "pip-install nope-pkg-xyz",
        "file a.txt",
        "file --help",
        "basename -s .txt a.txt",
        "dirname a.txt",
        "realpath a.txt",
        "readlink a.txt",
        "which -a echo",
        "env",
        "printenv",
        "printenv HOME",
        "true",
        "false",
        ":",
        "sleep 0",
        "date +%Y",
        "date -u",
        "uname -a",
        "id",
        "whoami",
        "hostname",
        "nproc",
        "tty",
    ]);
}

#[test]
#[ignore = "heavy: run explicitly or in CI (cargo test -- --ignored)"]
fn awk_jq_sed_more() {
    run_all(&[
        "awk 'BEGIN{print sprintf(\"%05.2f\", 3.1)}'",
        "awk 'BEGIN{printf \"%x %o %c\\n\", 255, 8, 65}'",
        "awk 'BEGIN{print (1<2) ? \"a\" : \"b\"}'",
        "awk 'BEGIN{n=0; for(i=1;i<=3;i++) n+=i; print n}'",
        "awk '{next} END{print \"end\"}' a.txt",
        "awk 'BEGIN{exit 0}'",
        "awk 'BEGIN{arr[1]=\"x\"; print arr[1]}'",
        "awk 'BEGIN{print length()}'",
        "awk '{print substr($0,1,3)}' a.txt",
        "awk '{print toupper($0)}' a.txt",
        "awk -F, 'BEGIN{OFS=\"-\"} {print $1,$2}' csv.txt",
        "jq -n '[] | length'",
        "jq -n '{} | length'",
        "jq -n '\"abc\" | length'",
        "jq -n '1 | tostring'",
        "jq -n '\"2\" | tonumber'",
        "jq -n 'true | not'",
        "jq -n '[1,2] | .[]'",
        "jq -n '{a:1,b:2} | .b'",
        "jq -n '[1,2,3] | .[1:]'",
        "jq -n 'null // 1'",
        "jq -n '1,2,3'",
        "jq -n '{a:{b:1}} | .a.b'",
        "jq . a.txt",
        "jq '.[0]' nums.txt",
        "sed -n '1,2p' a.txt",
        "sed 's/a/X/g' a.txt",
        "sed -e 's/a/1/' -e 's/b/2/' a.txt",
        "sed -i 's/a/X/' a.txt; cat a.txt",
        "sed '2d' a.txt",
        "sed '$d' a.txt",
        "sed -n '/beta/p' a.txt",
        "sed 's/\\(a\\)/\\1\\1/' a.txt",
        "sed 'y/abc/xyz/' a.txt",
        "sed '1i\\inserted' a.txt",
        "sed '1a\\appended' a.txt",
        "sed '2q' a.txt",
        "sed -E 's/(a)(b)/\\2\\1/' a.txt",
    ]);
}
