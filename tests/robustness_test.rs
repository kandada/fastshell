// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Robustness: malformed / adversarial shell input must never panic or hang.
//! The parser (segments, quotes, blocks, substitutions, redirects) is the most
//! panic-prone surface; this sweeps a corpus of broken constructs.

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn setup() -> Fastshell {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_robust_{}_{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dir);
    let mut sdk = Fastshell::new();
    sdk.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: false,
        allow_subprocess: true,
        network_ask_permission: false,
        command_timeout_ms: 5_000,
        ..Default::default()
    })
    .unwrap();
    sdk
}

const MALFORMED: &[&str] = &[
    // empty / whitespace
    "",
    "   ",
    "\n",
    "\t",
    // block constructs, incomplete
    "for",
    "for x in",
    "for x in a; do",
    "for x in a; do echo",
    "for x in a; do echo x",
    "if",
    "if true",
    "if true; then",
    "if true; then echo",
    "elif",
    "else",
    "fi",
    "while",
    "while true; do",
    "until",
    "done",
    "case",
    "case x",
    "case x in",
    "case x in a)",
    "esac",
    // quotes, unterminated
    "echo \"unterminated",
    "echo 'unterminated",
    "echo \"a\"'b",
    "echo `unterminated",
    "echo $(",
    "echo $(",
    "echo ${",
    "echo ${x",
    // substitutions / arithmetic
    "echo $((1+",
    "echo $(((",
    "echo ))",
    "echo $(())",
    "echo ${x:-",
    "echo ${x%%",
    // pipes / redirects
    "|",
    "||",
    "&&",
    ";",
    "echo |",
    "| cat",
    "echo >",
    "echo >>",
    "echo 2>&",
    "echo > >",
    "cat <",
    "<<",
    "echo | | cat",
    // braces / globs
    "echo {a,",
    "echo {a,b",
    "echo }",
    "echo {",
    "echo {,}",
    "echo [a-",
    // misc commands with bad args
    "sed 's/",
    "sed -e",
    "grep -E '(",
    "grep -",
    "cut -",
    "sort -",
    "head -n",
    "tail -n",
    "printf",
    "printf %",
    "test",
    "[",
    "[ ]",
    "[ 1 -eq ]",
    // control flow edges
    "exit",
    "return",
    "break",
    "continue",
    "cd",
    "cd /nonexistent/xyz && echo ok",
    // nested / deep
    "for a in 1; do for b in 2; do echo $a$b; done; done",
    "echo $(echo $(echo $(echo deep)))",
    "x=$((1+2)); echo $x",
    // arithmetic edge cases (division/modulo by zero must not panic)
    "echo $((1/0))",
    "echo $((1%0))",
    "echo $((0/0))",
    "echo $((-1/0))",
    "echo $((2**0))",
    "echo $((  ))",
    // parameter-expansion edge cases
    "x=abc; echo ${x:1}",
    "x=abc; echo ${x:1:2}",
    "x=abc; echo ${x//a/X}",
    "echo ${x:-d}",
    "echo ${x:=d}",
    "echo ${x%%b*}",
    // format / range edge cases
    "printf '%d' abc",
    "printf '%*d' -5 1",
    "printf '%s'",
    "echo {a..e}",
    "echo {1..0}",
    "echo {5..1}",
    // file tools with edge args
    "head -c -1 a.txt",
    "tail -c 0 a.txt",
    "sed 's//x/' a.txt",
    "grep '' a.txt",
    "cut -d'' -f1 a.txt",
];

#[test]
fn malformed_inputs_do_not_panic() {
    let s = setup();
    for cmd in MALFORMED {
        // Must return (Ok or Err), never panic or hang (timeout guarded by init).
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.execute(cmd)));
        assert!(r.is_ok(), "panic on input: {cmd:?}");
    }
}
