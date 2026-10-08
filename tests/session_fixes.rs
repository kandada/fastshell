// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Regression tests for issues found in the Android fastshell capability
//! session (`session_1789961870_e78fe2ac`).

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn sdk() -> Fastshell {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_session_fixes_{}_{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Fastshell::new();
    s.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: true,
        allow_subprocess: true,
        command_timeout_ms: 5_000,
        ..Default::default()
    })
    .unwrap();
    s
}

fn run(s: &Fastshell, cmd: &str) -> (String, String, i32) {
    let r = s.execute(cmd);
    (r.stdout, r.stderr, r.exit_code)
}

fn out(s: &Fastshell, cmd: &str) -> String {
    let (o, e, rc) = run(s, cmd);
    assert_eq!(rc, 0, "cmd={cmd:?} err={e}");
    o
}

#[test]
fn tail_shorthand_prints_last_n() {
    let s = sdk();
    s.execute("printf 'a\\nb\\nc\\n' > f.txt");
    assert_eq!(run(&s, "tail -1 f.txt").0, "c\n");
    assert_eq!(run(&s, "tail -2 f.txt").0, "b\nc\n");
    assert_eq!(run(&s, "tail +2 f.txt").0, "b\nc\n");
    assert_eq!(run(&s, "tail -n 1 f.txt").0, "c\n");
    assert_eq!(run(&s, "head -1 f.txt").0, "a\n");
}

#[test]
fn gzip_stdout_redirect_roundtrips_binary() {
    let s = sdk();
    s.execute("printf 'a\\nb\\nc\\n' > f.txt");
    let (out, err, rc) = run(&s, "gzip -c f.txt > g.gz && gunzip -c g.gz");
    assert_eq!(rc, 0, "err={err}");
    assert_eq!(out, "a\nb\nc\n");
}

#[test]
fn tar_dash_c_extracts_into_directory() {
    let s = sdk();
    s.execute("mkdir -p d1 d2 && echo hi > d1/x");
    let (out, err, rc) = run(&s, "tar czf t.tgz d1 && tar xzf t.tgz -C d2 && ls d2");
    assert_eq!(rc, 0, "err={err}");
    assert_eq!(out, "d1\n");
}

#[test]
fn unzip_dash_d_extracts_into_directory() {
    let s = sdk();
    s.execute("echo hi > f.txt");
    let (out, err, rc) = run(
        &s,
        "zip -q z.zip f.txt && unzip -o -q z.zip -d out && ls out",
    );
    assert_eq!(rc, 0, "err={err}");
    assert!(out.contains("f.txt"), "out={out}");
}

#[test]
fn grep_double_dash_treats_next_token_as_pattern() {
    let s = sdk();
    s.execute("printf '%s\\n' --help normal > f.txt");
    // `--help` after `--` is a PATTERN, not the help flag.
    let (out, _err, rc) = run(&s, "grep -- \"--help\" f.txt");
    assert_eq!(rc, 0);
    assert_eq!(out.trim(), "--help");
    // `--help` as an option still shows help.
    let (help, _e, _r) = run(&s, "grep --help");
    assert!(help.contains("Usage"), "help={help}");
}

#[test]
fn bc_supports_scale_and_flags() {
    let s = sdk();
    assert_eq!(run(&s, "echo 'scale=2; 10/3' | bc").0, "3.33\n");
    assert_eq!(run(&s, "echo '3+4' | bc").0, "7\n");
    // `-l` must not be treated as an expression.
    let (out, err, rc) = run(&s, "echo '10/3' | bc -l");
    assert_eq!(rc, 0, "err={err}");
    assert!(out.starts_with("3.3333"), "out={out}");
}

#[test]
fn printf_percent_q_quotes() {
    let s = sdk();
    assert_eq!(run(&s, "printf '%q\\n' 'a b'").0, "'a b'\n");
    assert_eq!(run(&s, "printf '%q\\n' plain").0, "plain\n");
    assert_eq!(run(&s, "printf '%q\\n' \"it's\"").0, "'it'\\''s'\n");
}

#[test]
fn python_version_and_help() {
    let s = sdk();
    let (out, err, rc) = run(&s, "python3 --version");
    assert_eq!(rc, 0, "err={err}");
    assert!(out.to_lowercase().contains("python"), "out={out}");
    let (help, _e, _r) = run(&s, "python3 --help");
    assert!(help.contains("usage"), "help={help}");
}

#[test]
fn dollar_bang_expands_to_empty() {
    let s = sdk();
    // No asynchronous job control: `$!` must not leak as a literal.
    assert_eq!(run(&s, "echo \"pid=[$!]\"").0, "pid=[]\n");
}

#[test]
fn timeout_runs_builtins_in_process() {
    let s = sdk();
    // `wget`/`sleep` are builtins (no external binary) — must not "fail to spawn".
    assert_eq!(run(&s, "timeout 2 sleep 1 && echo ok").0, "ok\n");
    assert_eq!(run(&s, "timeout 2 echo quick").0, "quick\n");
}

#[test]
fn set_errexit_stops_on_failure() {
    let s = sdk();
    let (out, _err, rc) = run(&s, "set -e; false; echo should-not-print");
    assert_eq!(rc, 1);
    assert!(!out.contains("should-not-print"), "out={out}");
    let (out2, _e, rc2) = run(&s, "set +e; false; echo after");
    assert_eq!(rc2, 0);
    assert!(out2.contains("after"));
}

#[test]
fn camera_help_has_no_side_effect() {
    let s = sdk();
    let (out, _err, rc) = run(&s, "camera --help");
    assert_eq!(rc, 0);
    assert!(out.contains("usage: camera"), "out={out}");
    // Must NOT create a file literally named "--help".
    let (ls, _e, _r) = run(&s, "ls");
    assert!(!ls.contains("--help"), "ls={ls}");
}

#[test]
fn python_resolves_virtual_projects_path() {
    let s = sdk();
    s.execute("mkdir -p projects/test1");
    // Python must resolve the shell's virtual root exactly like the shell.
    let (out, err, rc) = run(
        &s,
        "python3 -c \"open('/projects/test1/f.txt','w').write('hi')\"",
    );
    assert_eq!(rc, 0, "err={err}");
    assert_eq!(run(&s, "cat /projects/test1/f.txt").0, "hi");
    let (lst, err2, rc2) = run(
        &s,
        "python3 -c \"import os; print(sorted(os.listdir('/projects/test1')))\"",
    );
    assert_eq!(rc2, 0, "err={err2}");
    assert_eq!(lst.trim(), "['f.txt']");
    // makedirs on a virtual path
    let (_o, err3, rc3) = run(
        &s,
        "python3 -c \"import os; os.makedirs('/projects/test1/sub', exist_ok=True)\"",
    );
    assert_eq!(rc3, 0, "err={err3}");
    assert!(run(&s, "ls /projects/test1").0.contains("sub"));
}

#[test]
fn python_repeated_calls_stay_sandboxed() {
    // Regression: the embedded interpreter is long-lived, so a stale sandbox
    // patch used to double-prefix the root on later calls.
    let s = sdk();
    s.execute("mkdir -p projects/test1");
    for i in 0..5 {
        let (out, err, rc) = run(&s, "python3 -c \"import os; print('cwd', os.getcwd())\"");
        assert_eq!(rc, 0, "call {i} failed: {err}");
        assert!(out.contains("cwd "), "out={out}");
        let (_o, err2, rc2) = run(
            &s,
            &format!("python3 -c \"open('/projects/test1/f{i}.txt','w').write('v{i}')\""),
        );
        assert_eq!(rc2, 0, "call {i} write failed: {err2}");
    }
    let ls = run(&s, "ls /projects/test1").0;
    for i in 0..5 {
        assert!(ls.contains(&format!("f{i}.txt")), "ls={ls}");
    }
}

#[test]
fn python_sandbox_survives_multiple_sessions() {
    // The embedded interpreter is process-global; separate Fastshell instances
    // (tasks) must not leak sandbox patches into one another.
    for _ in 0..3 {
        let s = sdk();
        s.execute("mkdir -p projects/test1");
        let (_, err, rc) = run(
            &s,
            "python3 -c \"import os; os.makedirs('/projects/test1/d', exist_ok=True); open('/projects/test1/d/x','w').write('1')\"",
        );
        assert_eq!(rc, 0, "err={err}");
        assert!(run(&s, "ls /projects/test1/d").0.contains("x"));
    }
}

#[test]
fn python_stdin_via_heredoc_and_pipe() {
    let s = sdk();
    // heredoc program
    assert_eq!(run(&s, "python3 << 'EOF'\nprint(1)\nEOF").0, "1\n");
    // heredoc + output redirect (regression: used to fall back to the REPL)
    assert_eq!(
        run(&s, "python3 << 'EOF' > out.txt\nprint(2)\nEOF\ncat out.txt").0,
        "2\n"
    );
    // pipe into bare python (regression: real python reads the program from stdin)
    assert_eq!(run(&s, "echo 'print(3)' | python3").0, "3\n");
    // pipe into `python3 -`
    assert_eq!(run(&s, "echo 'print(4)' | python3 -").0, "4\n");
    // -c with a redirect still works
    assert_eq!(
        run(&s, "python3 -c \"print(5)\" > c.txt && cat c.txt").0,
        "5\n"
    );
}

#[test]
fn dangerous_commands_are_sandboxed() {
    let s = sdk();
    // chroot / mknod must NOT call the privileged syscalls (Android SIGSYS → crash).
    let (out, err, _rc) = run(&s, "chroot / true");
    assert!(!err.contains("SIGSYS"), "err={err}");
    let (_o, err2, rc2) = run(&s, "mknod x c 1 2");
    assert_eq!(rc2, 1, "mknod must be refused, err={err2}");
    assert!(err2.contains("not permitted"), "err={err2}");
    // renice/kill must refuse to touch this process.
    let (_o, err3, rc3) = run(&s, "renice 5 -p $$");
    assert_eq!(rc3, 1, "err={err3}");
    assert!(err3.contains("refusing"), "err={err3}");
    let (_o, err4, rc4) = run(&s, "kill -9 $$");
    assert_eq!(rc4, 1, "err={err4}");
    assert!(err4.contains("refusing"), "err={err4}");
    // groups must not overflow (getgroups(-1)).
    let (_o, err5, rc5) = run(&s, "groups");
    assert_eq!(rc5, 0, "err={err5}");
}

#[test]
fn timeout_bounds_in_process_builtins() {
    let s = sdk();
    let start = std::time::Instant::now();
    let (_o, err, rc) = run(&s, "timeout 1 sleep 5");
    assert_eq!(rc, 124, "err={err}");
    assert!(
        start.elapsed() < std::time::Duration::from_secs(3),
        "timeout did not bound the builtin: {:?}",
        start.elapsed()
    );
    // A fast builtin still works.
    assert_eq!(run(&s, "timeout 2 echo hi").0, "hi\n");
}

#[test]
fn case_word_is_expanded_and_unquoted() {
    let s = sdk();
    assert_eq!(
        run(&s, "V=abc; case $V in abc) echo M;; *) echo D;; esac").0,
        "M\n"
    );
    assert_eq!(
        run(&s, "V=abc; case \"$V\" in abc) echo M;; *) echo D;; esac").0,
        "M\n"
    );
    assert_eq!(
        run(&s, "V=apple; case $V in a*) echo G;; *) echo D;; esac").0,
        "G\n"
    );
    assert_eq!(run(&s, "V=x; case $V in a|b|x) echo A;; esac").0, "A\n");
}

#[test]
fn subshell_isolates_variables() {
    let s = sdk();
    assert_eq!(run(&s, "unset Z; (Z=sub2); echo [$Z]").0, "[]\n");
    assert_eq!(run(&s, "A=1; (A=9; B=2); echo $A [$B]").0, "1 []\n");
}

#[test]
fn binary_roundtrips_are_byte_exact() {
    let s = sdk();
    s.execute("python3 -c \"open('bin.dat','wb').write(bytes(range(256)))\"");
    assert_eq!(run(&s, "wc -c < bin.dat").0.trim(), "256");
    assert_eq!(run(&s, "cat bin.dat | wc -c").0.trim(), "256");
    for cmd in [
        "base64 bin.dat > b64; base64 -d b64 > o; cmp bin.dat o && echo SAME",
        "gzip -c bin.dat > g; gunzip -c g > o; cmp bin.dat o && echo SAME",
        "bzip2 -c bin.dat > b; bunzip2 -c b > o; cmp bin.dat o && echo SAME",
        "xz -c bin.dat > x; unxz -c x > o; cmp bin.dat o && echo SAME",
        "xxd -p bin.dat | xxd -r -p > o; cmp bin.dat o && echo SAME",
        "cat bin.dat | base64 | base64 -d > o; cmp bin.dat o && echo SAME",
    ] {
        assert_eq!(run(&s, cmd).0.trim(), "SAME", "cmd: {cmd}");
    }
}

#[test]
fn unzip_list_test_stdout() {
    let s = sdk();
    s.execute("printf 'hello' > f.txt; zip -q z.zip f.txt");
    assert!(run(&s, "unzip -l z.zip").0.contains("f.txt"));
    assert!(run(&s, "unzip -t z.zip").0.contains("No errors"));
    assert_eq!(run(&s, "unzip -p z.zip").0, "hello");
    assert!(run(&s, "mkdir -p u; unzip -o -q z.zip -d u; ls u")
        .0
        .contains("f.txt"));
}

#[test]
fn shell_options_do_not_leak_across_calls() {
    let s = sdk();
    let a = s.execute_in_with_timeout("/", "set -e; false; echo A", 5000);
    assert_eq!(a.exit_code, 1);
    assert!(!a.stdout.contains("A"));
    // errexit must NOT persist into the next isolated call.
    let b = s.execute_in_with_timeout("/", "echo B; false; echo C", 5000);
    assert_eq!(b.exit_code, 0);
    assert!(b.stdout.contains("C"), "out={}", b.stdout);
}

#[test]
fn param_gap_fixes() {
    let s = sdk();
    assert_eq!(run(&s, "printf 'a\\x41b\\n'").0, "aAb\n");
    s.execute("printf 'abcdef' > f.txt");
    assert_eq!(run(&s, "cut -b 1-2 f.txt").0, "ab\n");
    assert_eq!(run(&s, "basename -a /a/b.txt /c/d.txt").0, "b.txt\nd.txt\n");
    assert_eq!(run(&s, "seq -f 'x%03g' 1 3").0, "x001\nx002\nx003\n");
    assert_eq!(run(&s, "head -1 f.txt").0, "abcdef\n");
}

#[test]
fn python_c_sandbox_enforced() {
    // Regression: `python3 -c` must export the sandbox root, else the embedded
    // RustPython engine disables its file sandbox and can read host files.
    let s = sdk();
    assert_eq!(
        run(
            &s,
            "python3 -c \"import os;print(os.path.exists('/etc/passwd'))\""
        )
        .0
        .trim(),
        "False"
    );
    let (_o, err, _rc) = run(&s, "python3 -c \"print(open('/etc/hosts').read()[:1])\"");
    assert!(!err.is_empty(), "host file should not be readable");
}

#[test]
fn python_pipe_is_not_swallowed() {
    let s = sdk();
    // `python3 -c ... | cmd` must actually pipe (was: NameError 'cat').
    assert_eq!(run(&s, "python3 -c \"print(42)\" | cat").0.trim(), "42");
    assert_eq!(run(&s, "python3 -c \"print(1)\" | tr 1 9").0.trim(), "9");
    // `python3 script.py | cmd` — the `| cmd` must NOT become sys.argv.
    s.execute("printf 'import sys\\nprint(\",\".join(sys.argv[1:]))\\n' > argv.py");
    assert_eq!(run(&s, "python3 argv.py | cat").0.trim(), "");
    // python on the right still works
    assert_eq!(run(&s, "echo x | python3 -c \"print(7)\"").0.trim(), "7");
}

#[test]
fn jq_assignment_and_del() {
    let s = sdk();
    assert_eq!(
        run(&s, "echo '{\"x\":5}' | jq '.x |= .+1'").0.trim(),
        "{\"x\":6}"
    );
    assert_eq!(
        run(&s, "echo '{\"x\":5}' | jq '.x += 1'").0.trim(),
        "{\"x\":6}"
    );
    assert_eq!(
        run(&s, "echo '{\"x\":5,\"y\":1}' | jq 'del(.y)'").0.trim(),
        "{\"x\":5}"
    );
    assert_eq!(run(&s, "echo '[1,2,3]' | jq 'del(.[1])'").0.trim(), "[1,3]");
    assert_eq!(
        run(&s, "echo '{\"a\":{\"b\":2}}' | jq '.a.b = 9'").0.trim(),
        "{\"a\":{\"b\":9}}"
    );
}

#[test]
fn case_pattern_quote_removal() {
    let s = sdk();
    assert_eq!(
        run(
            &s,
            "v=\"hello world\"; case \"$v\" in *\"world\"*) echo Y;; *) echo N;; esac"
        )
        .0,
        "Y\n"
    );
}

#[test]
fn expr_comparisons() {
    let s = sdk();
    assert_eq!(run(&s, "expr 3 = 3").0, "1\n");
    assert_eq!(run(&s, "expr 3 != 4").0, "1\n");
    assert_eq!(run(&s, "expr \"a\" = \"a\"").0, "1\n");
    assert_eq!(run(&s, "expr \"a\" = \"b\"").0, "0\n");
    assert_eq!(run(&s, "expr \"a\" = \"b\"").2, 1);
}

#[test]
fn awk_arithmetic_modulo_and_pow() {
    let s = sdk();
    assert_eq!(
        run(&s, "printf '1\\n2\\n3\\n4\\n' | awk '$1%2==0'").0,
        "2\n4\n"
    );
    assert_eq!(
        run(&s, "printf '1\\n2\\n3\\n4\\n' | awk '{print $1 % 2}'").0,
        "1\n0\n1\n0\n"
    );
    assert_eq!(run(&s, "awk 'BEGIN{print 2^10}'").0, "1024\n");
    // non-numeric fields must still be preserved
    assert_eq!(run(&s, "printf 'hello\\n' | awk '{print $1}'").0, "hello\n");
}

#[test]
fn shell_sandbox_blocks_host_paths() {
    let s = sdk();
    for p in ["/etc/passwd", "/etc/hosts", "/etc/shadow"] {
        let (out, err, rc) = run(&s, &format!("cat {p}"));
        assert_eq!(rc, 1, "cat {p} should fail");
        assert!(
            out.is_empty() && err.contains("Not found"),
            "{p}: out={out} err={err}"
        );
    }
}

#[test]
fn gnu_long_options_and_dialects() {
    let s = sdk();
    s.execute("printf 'b 2\\na 3\\nc 1\\n' > f.txt; printf 'Apple\\napple\\n' > h.txt");
    assert_eq!(
        run(&s, "sort --key=2 --numeric-sort f.txt").0,
        "c 1\nb 2\na 3\n"
    );
    assert_eq!(
        run(&s, "grep --ignore-case --line-number apple h.txt").0,
        "1:Apple\n2:apple\n"
    );
    assert_eq!(
        run(&s, "grep --fixed-strings --quiet apple h.txt; echo rc=$?").0,
        "rc=0\n"
    );
    // busybox checksum shorthands
    assert_eq!(
        run(&s, "echo -n abc | md5").0,
        "900150983cd24fb0d6963f7d28e17f72  -\n"
    );
    // head -N shorthand
    assert_eq!(
        run(&s, "printf 'a\\nb\\nc\\n' > n.txt; head -1 n.txt").0,
        "a\n"
    );
}

#[test]
fn more_binary_roundtrips() {
    let s = sdk();
    s.execute("python3 -c \"open('bin.dat','wb').write(bytes(range(256)))\"");
    // compress -> pipe -> decompress (byte counts must match)
    for cmd in [
        "gzip -c bin.dat | gunzip -c | wc -c",
        "bzip2 -c bin.dat | bunzip2 -c | wc -c",
        "xz -c bin.dat | unxz -c | wc -c",
    ] {
        assert_eq!(run(&s, cmd).0.trim(), "256", "cmd: {cmd}");
    }
    // tar round-trip
    assert_eq!(
        run(&s, "mkdir -p td && cp bin.dat td/ && tar cf t.tar td && mkdir -p t2 && tar xf t.tar -C t2 && cmp bin.dat t2/td/bin.dat && echo SAME").0.trim(),
        "SAME"
    );
    // head -c byte-exact
    assert_eq!(run(&s, "head -c 4 bin.dat | wc -c").0.trim(), "4");
}

#[test]
fn python_sqlite3_module_available() {
    // RustPython has no `_sqlite3` on mobile; the injected pure-Python shim +
    // native rusqlite bridge must provide a working DB-API subset.
    let s = sdk();
    let code = "import sqlite3\nc=sqlite3.connect(':memory:')\nc.execute('create table t(id integer,name text)')\nc.executemany('insert into t values(?,?)',[(1,'a'),(2,'b')])\nc.commit()\nprint(c.execute('select count(*) from t').fetchone()[0])\nprint(c.execute('select name from t where id=?',(2,)).fetchone()[0])";
    let q = format!("'{}'", code.replace('\'', "'\\''"));
    let (out, err, rc) = run(&s, &format!("python3 -c {q}"));
    assert_eq!(rc, 0, "err={err}");
    assert_eq!(out, "2\nb\n");
    // file-backed DB persists
    s.execute("python3 -c \"import sqlite3; c=sqlite3.connect('app.db'); c.execute('create table u(x int)'); c.execute('insert into u values(7)'); c.commit(); c.close()\"");
    assert_eq!(
        run(&s, "python3 -c \"import sqlite3; print(sqlite3.connect('app.db').execute('select x from u').fetchone()[0])\"").0.trim(),
        "7"
    );
}

#[test]
fn command_v_and_type_report_builtins() {
    let s = sdk();
    assert_eq!(run(&s, "command -v ls").0.trim(), "ls");
    assert!(run(&s, "type ls").0.contains("built-in"));
}

#[test]
fn nested_double_quotes_inside_command_substitution() {
    // `;` inside a nested-quoted `$(...)` must not split the command.
    let s = sdk();
    assert_eq!(run(&s, "echo \"[$(echo \"a;b\")]\"").0, "[a;b]\n");
    assert_eq!(run(&s, "echo \"[$(echo a; echo b)]\"").0, "[a\nb]\n");
}

#[test]
fn expansion_preserves_quotes_in_values() {
    let s = sdk();
    assert_eq!(run(&s, "V=$(printf '\"q\"'); echo \"[$V]\"").0, "[\"q\"]\n");
    assert_eq!(run(&s, "V='\"q\"'; echo \"[$V]\"").0, "[\"q\"]\n");
    assert_eq!(run(&s, "printf '%s|' \"$(printf '\"q\"')\"").0, "\"q\"|");
    // variable content is data, not re-expanded
    assert_eq!(run(&s, "V='$HOME'; echo \"[$V]\"").0, "[$HOME]\n");
    // whitespace still word-splits when unquoted
    assert_eq!(
        run(&s, "V='a b'; for w in $V; do echo \"<$w>\"; done").0,
        "<a>\n<b>\n"
    );
}

#[test]
fn brace_group_and_subshell_can_be_piped() {
    let s = sdk();
    assert_eq!(run(&s, "{ echo a; echo b; } | tr a-z A-Z").0, "A\nB\n");
    assert_eq!(run(&s, "(echo a; echo b) | tr a-z A-Z").0, "A\nB\n");
}

#[test]
fn python_stage_in_middle_of_pipeline() {
    let s = sdk();
    let (out, err, rc) = run(
        &s,
        "printf 'x\\ny\\n' | python3 -c 'import sys; sys.stdout.write(sys.stdin.read().upper())' | tr a-z A-Z",
    );
    assert_eq!(rc, 0, "err={err}");
    assert_eq!(out, "X\nY\n");
    assert_eq!(
        run(&s, "printf 'ab\\n' | python3 -c 'print(1+1)' | wc -l")
            .0
            .trim(),
        "1"
    );
}

#[test]
fn sqlite3_memory_db_does_not_create_file() {
    let s = sdk();
    let (out, err, rc) = run(&s, "sqlite3 :memory: 'select 1+1'");
    assert_eq!(rc, 0, "err={err}");
    assert_eq!(out.trim(), "2");
    assert_ne!(run(&s, "ls :memory:").2, 0);
}

#[test]
fn xargs_defaults_to_echo() {
    let s = sdk();
    assert_eq!(run(&s, "printf 'a\\nb\\n' | xargs").0.trim(), "a b");
}

#[test]
fn file_reports_directory() {
    let s = sdk();
    s.execute("mkdir -p d");
    assert_eq!(run(&s, "file d").0.trim(), "d: directory");
}

#[test]
fn which_reports_runtime_builtins() {
    let s = sdk();
    assert!(run(&s, "which render").0.contains("render"));
    assert_eq!(run(&s, "command -v render").0.trim(), "render");
}

#[test]
fn id_honors_u_and_g_flags() {
    let s = sdk();
    assert!(run(&s, "id -u").0.trim().parse::<u32>().is_ok());
    assert!(run(&s, "id -g").0.trim().parse::<u32>().is_ok());
}

#[test]
fn sh_and_bash_builtin_wrapper() {
    let s = sdk();
    assert_eq!(out(&s, "sh -c 'echo hi'"), "hi\n");
    assert_eq!(out(&s, "bash -c 'echo bash-ok'"), "bash-ok\n");
    // positional params: $0=name, $1=a, $2=b
    assert_eq!(out(&s, "sh -c 'echo $1-$2' name a b"), "a-b\n");
    // script file
    s.execute("printf 'echo script-ok\\nX=5\\necho X=$X\\n' > s.sh");
    assert_eq!(out(&s, "sh s.sh"), "script-ok\nX=5\n");
    // sh as a pipeline producer and consumer
    assert_eq!(out(&s, "sh -c 'echo a; echo b' | tr a-z A-Z"), "A\nB\n");
    assert_eq!(out(&s, "echo piped | sh -c 'cat'"), "piped\n");
    assert_eq!(out(&s, "printf 'echo from-pipe\\n' | sh"), "from-pipe\n");
    assert!(out(&s, "which sh").contains("sh"));
    assert_eq!(out(&s, "command -v bash").trim(), "bash");
}

#[test]
fn zip_recursive_directory() {
    let s = sdk();
    s.execute("mkdir -p d/sub && echo a > d/f1.txt && echo b > d/sub/f2.txt");
    let (o, e, rc) = run(&s, "zip -q -r d.zip d && unzip -l d.zip");
    assert_eq!(rc, 0, "err={e}");
    assert!(o.contains("d/f1.txt"), "o={o:?}");
    assert!(o.contains("d/sub/f2.txt"), "o={o:?}");
    // extract into a destination
    let (o2, e2, rc2) = run(&s, "mkdir -p x && unzip -o -q d.zip -d x && find x -type f");
    assert_eq!(rc2, 0, "err={e2}");
    assert!(o2.contains("x/d/sub/f2.txt"), "o2={o2:?}");
    // -x exclusion
    let (o3, _, _) = run(&s, "zip -q -r noex.zip d -x 'd/sub/*' && unzip -l noex.zip");
    assert!(
        o3.contains("d/f1.txt") && !o3.contains("f2.txt"),
        "o3={o3:?}"
    );
}

#[test]
fn dns_lookup_builtins_resolve_localhost() {
    let s = sdk();
    assert!(out(&s, "nslookup localhost").contains("127.0.0.1"));
    assert!(out(&s, "dig localhost").contains("127.0.0.1"));
}

#[test]
fn ping_resolves_hostnames() {
    let s = sdk();
    let (o, e, _rc) = run(&s, "ping -c 1 -W 1 localhost");
    let combined = format!("{o}{e}");
    assert!(
        !combined.contains("cannot resolve"),
        "ping failed to resolve localhost: {combined:?}"
    );
}

#[test]
fn wc_multiple_flags_select_counters() {
    let s = sdk();
    s.execute("printf 'hello world\\nfoo\\n' > m.txt");
    // -c -w → bytes + words (previously both cancelled out → filename only)
    let cw = out(&s, "wc -c -w m.txt");
    assert_eq!(cw.split_whitespace().count(), 3, "cw={cw:?}");
    let lw = out(&s, "wc -l -w m.txt");
    assert_eq!(lw.split_whitespace().count(), 3, "lw={lw:?}");
    let all = out(&s, "wc -c -w -l m.txt");
    assert_eq!(all.split_whitespace().count(), 4, "all={all:?}");
    let combined = out(&s, "wc -lw m.txt");
    assert_eq!(
        combined.split_whitespace().count(),
        3,
        "combined={combined:?}"
    );
    assert_eq!(out(&s, "wc m.txt").split_whitespace().count(), 4);
}

#[test]
fn xz_dash_d_decompresses() {
    let s = sdk();
    s.execute("echo PAYLOAD > f.txt && xz -k f.txt");
    assert!(out(&s, "ls").contains("f.txt.xz"));
    let (_o, e, rc) = run(&s, "xz -d f.txt.xz");
    assert_eq!(rc, 0, "err={e}");
    assert_eq!(out(&s, "cat f.txt"), "PAYLOAD\n");
    assert!(!out(&s, "ls").contains("f.txt.xz.xz"), "created .xz.xz");
    // -dc writes to stdout
    s.execute("echo PAYLOAD2 > g.txt && xz -k g.txt");
    assert_eq!(out(&s, "xz -dc g.txt.xz"), "PAYLOAD2\n");
}

#[test]
fn ln_force_onto_symlink_does_not_destroy_source() {
    let s = sdk();
    s.execute("mkdir -p lt && echo real > lt/src.txt && ln -s src.txt lt/dst.txt");
    let (o, e, rc) = run(
        &s,
        "cd lt && ln -sf src.txt dst.txt && cat src.txt && readlink dst.txt",
    );
    assert_eq!(rc, 0, "err={e}");
    assert!(o.contains("real"), "source destroyed: {o:?}");
    assert!(o.contains("src.txt"), "dst link wrong: {o:?}");
    // src must remain a regular file (not a self-referential symlink)
    assert_eq!(out(&s, "test -f src.txt && echo file").trim(), "file");
}

#[test]
fn tr_squeeze_uses_translated_set() {
    let s = sdk();
    assert_eq!(out(&s, "printf 'a   b\\n' | tr -s ' ' '_'"), "a_b\n");
    assert_eq!(out(&s, "printf 'aaab\\n' | tr -s 'a'"), "ab\n");
}

#[test]
fn gzip_stdin_stdout_streaming() {
    let s = sdk();
    assert_eq!(out(&s, "printf 'x\\n' | gzip | gunzip"), "x\n");
    let n = out(&s, "printf 'x\\n' | gzip | wc -c");
    assert!(n.trim().parse::<usize>().unwrap() > 0, "n={n:?}");
}

#[test]
fn expr_substr_index_length_match() {
    let s = sdk();
    assert_eq!(out(&s, "expr substr abcde 2 3"), "bcd\n");
    assert_eq!(out(&s, "expr index abcde cd"), "3\n");
    assert_eq!(out(&s, "expr length abcde"), "5\n");
    assert_eq!(out(&s, "expr match abcde 'ab.'"), "3\n");
}

#[test]
fn getconf_and_getent_implemented() {
    let s = sdk();
    assert_eq!(out(&s, "getconf PATH_MAX").trim(), "1024");
    assert!(!out(&s, "getconf _NPROCESSORS_ONLN").trim().is_empty());
    assert!(out(&s, "getent hosts localhost").contains("127.0.0.1"));
    assert!(out(&s, "command -v getconf").contains("getconf"));
}

#[test]
fn unzip_junk_paths_flattens() {
    let s = sdk();
    s.execute("mkdir -p d/sub && echo A > d/sub/a.txt && zip -q -r z.zip d");
    let (o, e, rc) = run(&s, "mkdir -p o && unzip -q -j z.zip -d o && find o -type f");
    assert_eq!(rc, 0, "err={e}");
    assert!(o.contains("o/a.txt"), "o={o:?}");
    assert!(!o.contains("o/d/sub"), "not flattened: {o:?}");
}

#[test]
fn od_dash_c_prints_characters() {
    let s = sdk();
    let o = out(&s, "printf 'AB\\n' | od -c");
    assert!(
        o.contains("A") && o.contains("B") && o.contains("\\n"),
        "o={o:?}"
    );
    // -An suppresses the address column (and trailing total).
    let a = out(&s, "printf 'AB\\n' | od -c -An");
    assert!(!a.contains("0000000"), "a={a:?}");
    let named = out(&s, "printf 'AB\\n' | od -a");
    assert!(named.contains("nl"), "named={named:?}");
}

#[test]
fn ls_dash_d_missing_path_errors() {
    let s = sdk();
    assert_eq!(run(&s, "ls -d definitely_missing_xyz").2, 1);
    assert_eq!(run(&s, "ls definitely_missing_xyz").2, 1);
    s.execute("mkdir -p d");
    assert_eq!(out(&s, "ls -d d").trim(), "d");
}

#[test]
fn tar_exclude_pattern() {
    let s = sdk();
    s.execute("mkdir -p arc && echo a > arc/keep.txt && echo b > arc/skip.log");
    let o = out(&s, "tar cf x.tar --exclude='*.log' arc && tar tf x.tar");
    assert!(o.contains("keep.txt") && !o.contains("skip.log"), "o={o:?}");
    let o2 = out(&s, "tar cf y.tar --exclude=skip.log arc && tar tf y.tar");
    assert!(!o2.contains("skip.log"), "o2={o2:?}");
}

#[test]
fn tar_append_and_verbose() {
    let s = sdk();
    s.execute("mkdir -p a && echo 1 > a/f1 && tar cf t.tar a && echo 2 > a/f2");
    let o = out(&s, "tar rf t.tar a/f2 && tar tf t.tar");
    assert!(o.contains("a/f1") && o.contains("a/f2"), "o={o:?}");
    // -v lists entries
    let v = out(&s, "tar cvf v.tar a");
    assert!(v.contains("a/f1"), "v={v:?}");
    let x = out(&s, "tar xvf v.tar");
    assert!(x.contains("a/f1"), "x={x:?}");
}

#[test]
fn set_e_does_not_leak_from_subshell() {
    let s = sdk();
    let o = out(&s, "( set -e; echo a; false; echo b ); echo after rc=$?");
    assert_eq!(o, "a\nafter rc=1\n");
    // A plain failing subshell must not abort the parent.
    let o2 = out(&s, "( false; echo b ); echo after");
    assert_eq!(o2, "b\nafter\n");
}

#[test]
fn quoted_redirect_operators_are_literal() {
    let s = sdk();
    assert_eq!(out(&s, "echo '>'"), ">\n");
    assert_eq!(out(&s, "echo '<'"), "<\n");
    assert_eq!(out(&s, "echo a \\> b"), "a > b\n");
    assert_eq!(out(&s, "echo 'a>b'"), "a>b\n");
    // real redirect still works
    assert_eq!(out(&s, "printf 'x\\n' > o.txt && cat o.txt"), "x\n");
}

#[test]
fn redirect_to_stderr_and_case_var_and_negation() {
    let s = sdk();
    let (o, e, _) = run(&s, "echo hi >&2");
    assert_eq!(o, "");
    assert_eq!(e.trim(), "hi");
    assert_eq!(
        out(&s, "V=o; case foo in *$V*) echo M;; *) echo N;; esac"),
        "M\n"
    );
    let (_o2, _e2, rc) = run(&s, "! true");
    assert_eq!(rc, 1);
    assert_eq!(run(&s, "! false").2, 0);
}

#[test]
fn test_dash_L_and_ls_symlink_marker() {
    let s = sdk();
    s.execute("echo hi > tgt.txt && ln -s tgt.txt sym.txt");
    assert_eq!(out(&s, "test -L sym.txt && echo yes").trim(), "yes");
    assert_eq!(out(&s, "test -h sym.txt && echo yes").trim(), "yes");
    let l = out(&s, "ls -l sym.txt");
    assert!(l.starts_with('l'), "l={l:?}");
    assert!(l.contains("sym.txt -> tgt.txt"), "l={l:?}");
    assert!(out(&s, "ls -l tgt.txt").starts_with('-'));
}

#[test]
fn find_mindepth_filters_by_depth() {
    let s = sdk();
    s.execute("mkdir -p a/b/c");
    let o = out(&s, "find . -mindepth 2 -type d | sort");
    assert!(o.contains("./a/b"), "o={o:?}");
    assert!(
        !o.contains("./a\n") && !o.lines().any(|l| l == "."),
        "o={o:?}"
    );
    let o1 = out(&s, "find . -mindepth 1 | sort");
    assert!(!o1.lines().any(|l| l == "."), "o1={o1:?}");
}

#[test]
fn split_byte_mode() {
    let s = sdk();
    s.execute("printf 'abcdefgh' > b.bin");
    let o = out(&s, "split -b 3 b.bin bp_ && wc -c bp_aa bp_ab");
    assert!(o.contains("3 bp_aa"), "o={o:?}");
    assert!(o.contains("3 bp_ab"), "o={o:?}");
}

#[test]
fn awk_printf_and_escapes() {
    let s = sdk();
    assert_eq!(
        out(
            &s,
            "awk 'BEGIN{printf \"%02d|%5.2f|%s|\\n\", 3, 3.14159, \"hi\"}'"
        ),
        "03| 3.14|hi|\n"
    );
    assert_eq!(
        out(&s, "printf 'a b\\n' | awk '{printf \"%s-%s\\n\", $1, $2}'"),
        "a-b\n"
    );
}

#[test]
fn env_lists_environment() {
    let s = sdk();
    assert!(!out(&s, "env").trim().is_empty());
    assert!(out(&s, "printenv").contains("PATH") || !out(&s, "printenv").trim().is_empty());
}

#[test]
fn nohup_runs_in_process() {
    let s = sdk();
    assert_eq!(out(&s, "nohup echo hi"), "hi\n");
    assert_eq!(out(&s, "nohup python3 -c 'print(42)'"), "42\n");
}

#[cfg(feature = "git")]
#[test]
fn git_status_uno_skips_untracked() {
    let s = sdk();
    s.execute(
        "git init -q && git config user.email a@b && git config user.name a \
         && echo a > t.txt && git add t.txt && git commit -m init",
    );
    s.execute("echo b > u.txt && mkdir -p ud && echo c > ud/x");
    let full = out(&s, "git status --porcelain");
    assert!(full.contains("?? u.txt"), "full={full:?}");
    assert!(full.contains("?? ud/x"), "full={full:?}");
    // `-uno` must skip the whole untracked scan.
    assert!(!out(&s, "git status --porcelain -uno").contains("??"));
    assert!(!out(&s, "git status --porcelain --untracked-files=no").contains("??"));
    // Tracked changes still reported with `-uno`.
    s.execute("echo changed >> t.txt");
    assert!(out(&s, "git status --porcelain -uno").contains("t.txt"));
}

#[test]
fn grep_basic_mode_is_bre() {
    let s = sdk();
    s.execute("printf 'a+b\\nc\\n' > f.txt");
    // `+` is literal in basic (BRE) mode — must not be a regex error.
    assert_eq!(run(&s, "grep '+++' f.txt").2, 1);
    assert_eq!(out(&s, "grep 'a+b' f.txt"), "a+b\n");
    // `\+` is the BRE one-or-more quantifier.
    assert_eq!(out(&s, "printf 'ab\\n' | grep 'a\\+b'"), "ab\n");
    // `-E` restores extended semantics (`a+b` = one-or-more `a` then `b`).
    assert_eq!(run(&s, "printf 'a+b\\n' | grep -E 'a+b'").2, 1);
}

#[test]
fn du_file_argument_prints_size() {
    let s = sdk();
    s.execute("printf 'abcdef' > f.txt");
    assert_eq!(out(&s, "du f.txt").trim(), "6\tf.txt");
    assert_eq!(out(&s, "du -a f.txt").trim(), "6\tf.txt");
}

#[test]
fn od_attached_type_flag() {
    let s = sdk();
    s.execute("printf 'abc' > b.dat");
    assert_eq!(out(&s, "od -An -tx1 b.dat").trim(), "61 62 63");
    assert_eq!(out(&s, "od -An -t x1 b.dat").trim(), "61 62 63");
}

#[test]
fn case_inside_command_substitution() {
    let s = sdk();
    assert_eq!(
        out(
            &s,
            "v=2; echo \"[$(case $v in 1) echo one;; 2) echo two;; esac)]\""
        ),
        "[two]\n"
    );
    assert_eq!(out(&s, "echo \"$(case x in x) echo yes;; esac)\""), "yes\n");
}

#[test]
fn long_command_is_killed_by_timeout() {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_timeout_{}_{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Fastshell::new();
    s.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        allow_subprocess: false,
        command_timeout_ms: 1_000,
        ..Default::default()
    })
    .unwrap();
    let start = std::time::Instant::now();
    let r = s.execute("sleep 30; echo done");
    let elapsed = start.elapsed();
    assert!(
        !r.stdout.contains("done"),
        "should be killed: {:?}",
        r.stdout
    );
    assert!(
        elapsed < std::time::Duration::from_secs(10),
        "timeout not enforced, took {elapsed:?}"
    );
}

#[test]
fn sh_command_redirects_applied() {
    let s = sdk();
    // Redirect on the `sh` command itself must not be treated as a script file.
    let (o, e, rc) = run(&s, "sh -c 'echo out' > o.txt 2>&1; cat o.txt");
    assert_eq!(rc, 0, "err={e}");
    assert_eq!(o.trim(), "out");
    // Bare `sh` with a redirect is an interactive error (rc=2), not a file read.
    assert_eq!(run(&s, "sh 2>&1").2, 2);
}

#[test]
fn split_reads_stdin_with_prefix() {
    let s = sdk();
    // `-` is the stdin operand and the following word is the PREFIX (GNU
    // `split [OPTION]... [FILE [PREFIX]]`). Previously `-` was dropped as an
    // unknown option so `sp` was treated as the input FILE.
    let (o, e, rc) = run(&s, "printf ABCDEFGHI | split -b 3 - sp; ls sp*");
    assert_eq!(rc, 0, "err={e}");
    assert!(o.contains("spaa"), "stdout={o}");
    assert!(o.contains("spac"), "stdout={o}");
    assert_eq!(out(&s, "cat spaa"), "ABC");
    assert_eq!(out(&s, "cat spac"), "GHI");
    // FILE + PREFIX still works.
    let (o, e, rc) = run(&s, "printf ABCDEFGHI > f.bin; split -b 3 f.bin px; ls px*");
    assert_eq!(rc, 0, "err={e}");
    assert!(o.contains("pxaa"), "stdout={o}");
    assert_eq!(out(&s, "cat pxaa"), "ABC");
}

#[test]
fn jq_keys_supports_index() {
    let s = sdk();
    s.execute("printf '{\"b\":1,\"a\":2}' > o.json");
    assert_eq!(out(&s, "cat o.json | jq 'keys[0]'").trim(), "\"a\"");
    assert_eq!(out(&s, "cat o.json | jq 'keys[-1]'").trim(), "\"b\"");
    // `keys` itself still returns the whole array.
    assert!(out(&s, "cat o.json | jq 'keys'").contains("\"a\""));
    // A valid index chain inside a pipe works too.
    assert_eq!(out(&s, "cat o.json | jq 'keys | .[0]'").trim(), "\"a\"");
}

#[cfg(feature = "js-oxc")]
#[test]
fn jscheck_eval_flag_and_clean_error() {
    let s = sdk();
    // `-e CODE` evaluates the inline source instead of treating it as a file.
    let (o, e, rc) = run(&s, "jscheck -e 'const x=1; function f(){return x}'");
    assert_eq!(rc, 0, "stdout={o} err={e}");
    assert!(o.contains("syntax OK"), "stdout={o}");
    // Invalid input → a clean syntax error, never the alarming "panicked".
    let (o, e, rc) = run(&s, "jscheck -e 'var a = ;'");
    assert_eq!(rc, 1, "stdout={o}");
    assert!(e.contains("syntax error"), "stderr={e}");
    assert!(!e.contains("panicked"), "must not say 'panicked': {e}");
}

#[cfg(feature = "git")]
#[test]
fn git_commit_accepts_combined_short_flags() {
    let s = sdk();
    out(&s, "mkdir -p repo && cd repo");
    out(&s, "git init -q");
    out(&s, "git config user.email a@b.c");
    out(&s, "git config user.name tester");
    out(&s, "echo hi > f.txt");
    out(&s, "git add f.txt");
    // `-qm` = `-q -m` (previously "please supply the message (-m)").
    let (o, e, rc) = run(&s, "git commit -qm 'init'");
    assert_eq!(rc, 0, "stdout={o} err={e}");
    assert!(out(&s, "git log --oneline").contains("init"));
    // `-am` stages tracked changes and takes the message.
    out(&s, "echo more >> f.txt");
    let (o, e, rc) = run(&s, "git commit -am 'second'");
    assert_eq!(rc, 0, "stdout={o} err={e}");
    assert!(out(&s, "git log --oneline").contains("second"));
    // Attached message form `-mMSG`.
    out(&s, "echo third >> f.txt");
    let (o, e, rc) = run(&s, "git commit -am 'third'");
    assert_eq!(rc, 0, "stdout={o} err={e}");
}

#[test]
fn quoted_pipe_is_not_a_pipeline() {
    let s = sdk();
    // A `|` inside quotes must not split the command — otherwise a function call
    // with a quoted pipe becomes a pipeline and the function isn't found.
    assert_eq!(out(&s, "f(){ echo \"[$1]\"; }; f 'a|b'"), "[a|b]\n");
    assert_eq!(out(&s, "echo 'a|b'"), "a|b\n");
    s.execute("printf 'a|b\\n' > p.txt");
    assert_eq!(out(&s, "grep 'a|b' p.txt"), "a|b\n");
}

#[test]
fn here_string_forms() {
    let s = sdk();
    assert_eq!(out(&s, "read H <<< word; echo \"[$H]\""), "[word]\n");
    assert_eq!(
        out(&s, "read H <<< \"two words\"; echo \"[$H]\""),
        "[two words]\n"
    );
    // inside a function body
    assert_eq!(
        out(&s, "f(){ read Z <<< abc; echo \"[$Z]\"; }; f"),
        "[abc]\n"
    );
    // inside a command substitution
    assert_eq!(
        out(&s, "V=$(read W <<< xyz; echo \"[$W]\"); echo \"$V\""),
        "[xyz]\n"
    );
}

#[test]
fn read_and_while_read_file() {
    let s = sdk();
    s.execute("printf 'L1\\nL2\\n' > r.txt");
    // `read` consumes only the first line.
    assert_eq!(out(&s, "read Z < r.txt; echo \"[$Z]\""), "[L1]\n");
    // canonical read loop over a file
    s.execute("printf 'a one\\nb two\\n' > g.txt");
    assert_eq!(
        out(&s, "while read k v; do echo \"[$k|$v]\"; done < g.txt"),
        "[a|one]\n[b|two]\n"
    );
    // here-string loop
    assert_eq!(
        out(&s, "while read L; do echo \"<$L>\"; done <<< solo"),
        "<solo>\n"
    );
}

#[test]
fn set_python_argv_and_wc_width() {
    let s = sdk();
    assert_eq!(out(&s, "set -- 'm|n'; echo \"[$1]\""), "[m|n]\n");
    let (o, _e, rc) = run(&s, "python3 -c 'import sys;print(sys.argv)' a1");
    assert_eq!(rc, 0, "out={o}");
    assert!(o.contains("'a1'"), "argv={o}");
    s.execute("printf 'a\\nb\\nc\\n' > w.txt");
    // GNU wc: single input is un-padded.
    assert_eq!(out(&s, "wc -l < w.txt").trim(), "3");
    assert_eq!(out(&s, "wc -l w.txt").trim(), "3 w.txt");
}

#[test]
fn exit_does_not_poison_later_commands() {
    let s = sdk();
    // Regression: a stray `exit`/`break` used to persist and make every later
    // multi-command input run only its first segment.
    assert_eq!(run(&s, "exit 3").2, 3);
    assert_eq!(out(&s, "echo A; echo B; echo C"), "A\nB\nC\n");
    s.execute("break");
    assert_eq!(out(&s, "echo X; echo Y"), "X\nY\n");
}

#[test]
fn sed_script_file() {
    let s = sdk();
    s.execute("printf '2024-01-15\\n' > d.txt");
    s.execute("printf 's/2024/2025/\\n' > s.sed");
    assert_eq!(out(&s, "sed -f s.sed d.txt"), "2025-01-15\n");
}

#[test]
fn ansi_c_quote_is_literal_inside_double_quotes() {
    let s = sdk();
    // bash: inside double quotes `$'` is a literal `$` + quote.
    assert_eq!(out(&s, "echo \"a$'b'c\""), "a$'b'c\n");
    assert_eq!(out(&s, "printf '%s\\n' \"$'x'\""), "$'x'\n");
    // Outside quotes `$'...'` is still ANSI-C quoting.
    assert_eq!(out(&s, "printf '%s' $'a\\tb'"), "a\tb");
}

#[test]
fn which_does_not_report_host_paths() {
    let s = sdk();
    // Unimplemented external commands must not resolve to a host path.
    assert!(!out(&s, "which cksum 2>&1").contains("/usr/bin"));
    assert_eq!(run(&s, "which definitely_not_a_command_xyz").2, 1);
    // Built-ins are reported as such.
    assert!(out(&s, "which ls").contains("built-in"));
}

#[test]
fn cksum_crc32_join_csplit() {
    let s = sdk();
    // POSIX cksum / zlib crc32 of "abc".
    assert_eq!(out(&s, "printf abc | cksum").trim(), "1219131554 3");
    assert_eq!(out(&s, "printf abc | crc32").trim(), "352441c2");
    // join on the first field.
    s.execute("printf 'a 1\\nb 2\\n' > j1");
    s.execute("printf 'a x\\nb y\\n' > j2");
    assert_eq!(out(&s, "join j1 j2"), "a 1 x\nb 2 y\n");
    // csplit by line number.
    s.execute("printf '1\\n2\\n3\\n4\\n' > c.txt");
    let _ = out(&s, "csplit c.txt 3");
    assert_eq!(out(&s, "cat xx00"), "1\n2\n");
    assert_eq!(out(&s, "cat xx01"), "3\n4\n");
}

#[test]
fn awk_length_ternary_builtins() {
    let s = sdk();
    assert_eq!(out(&s, "echo hello | awk '{print length}'"), "5\n");
    assert_eq!(
        out(&s, "echo 5 | awk '{print ($1>3)?\"big\":\"small\"}'"),
        "big\n"
    );
    assert_eq!(out(&s, "echo 3.7 | awk '{print int($1)}'"), "3\n");
    assert_eq!(out(&s, "echo 144 | awk '{print sqrt($1)}'"), "12\n");
    assert_eq!(
        out(&s, "echo hi | awk '{print sprintf(\"[%s]\", $1)}'"),
        "[hi]\n"
    );
    assert_eq!(out(&s, "echo 7 | awk '{print int($1/2)}'"), "3\n");
}

#[test]
fn sed_append_insert_change() {
    let s = sdk();
    s.execute("printf 'a1\\nb2\\nc3\\n' > s.txt");
    assert_eq!(out(&s, "sed '1a X' s.txt"), "a1\nX\nb2\nc3\n");
    assert_eq!(out(&s, "sed '1i X' s.txt"), "X\na1\nb2\nc3\n");
    assert_eq!(out(&s, "sed '2c X' s.txt"), "a1\nX\nc3\n");
    assert_eq!(out(&s, "sed '/b/ c X' s.txt"), "a1\nX\nc3\n");
}

#[test]
fn shuf_range_bc_math_join_o_mktemp() {
    let s = sdk();
    assert_eq!(out(&s, "shuf -i 1-5 -n 2 | wc -l").trim(), "2");
    assert!(
        out(&s, "echo 'sqrt(144)' | bc -l")
            .trim_start()
            .starts_with("12"),
        "bc -l sqrt"
    );
    s.execute("printf '1 a\\n2 b\\n' > j1");
    s.execute("printf '1 x\\n2 y\\n' > j2");
    assert_eq!(out(&s, "join -o 0,1.2,2.2 j1 j2"), "1 a x\n2 b y\n");
    let m = out(&s, "mktemp");
    assert!(m.trim().starts_with('/'), "mktemp={m}");
    let d = out(&s, "mktemp -d");
    assert!(d.trim().starts_with('/'), "mktemp -d={d}");
}

#[cfg(feature = "git")]
#[test]
fn git_init_subdir() {
    let s = sdk();
    out(&s, "git init sub_repo 2>&1 | head -1");
    assert_eq!(run(&s, "ls sub_repo/.git >/dev/null 2>&1").2, 0);
}

#[test]
fn shell_syntax_arith_group_case() {
    let s = sdk();
    assert_eq!(out(&s, "i=5; ((i++)); echo $i"), "6\n");
    assert_eq!(run(&s, "(( 0 )); echo $?").0.trim(), "1");
    assert_eq!(out(&s, "{ echo a; echo b; }"), "a\nb\n");
    assert_eq!(out(&s, "x=1; { x=2; }; echo $x"), "2\n");
    assert_eq!(
        out(&s, "v=$(case x in x) echo yes;; esac); echo $v"),
        "yes\n"
    );
}

#[test]
fn ifs_word_splitting() {
    let s = sdk();
    assert_eq!(
        out(&s, "IFS=,; v=a,b,c; for p in $v; do echo \"<$p>\"; done"),
        "<a>\n<b>\n<c>\n"
    );
    assert_eq!(
        out(&s, "IFS=,; read a b <<< 'x,y'; echo \"$a-$b\""),
        "x-y\n"
    );
    assert_eq!(out(&s, "IFS=','; v='a b'; set -- $v; echo $#"), "1\n");
    assert_eq!(
        out(
            &s,
            "unset IFS; v='a b'; for p in $v; do echo \"<$p>\"; done"
        ),
        "<a>\n<b>\n"
    );
}

#[test]
fn printf_v_getopts_ln_hardlink() {
    let s = sdk();
    assert_eq!(out(&s, "printf -v foo '%s-%s' a b; echo \"$foo\""), "a-b\n");
    assert_eq!(out(&s, "printf -v n '%03d' 7; echo $n"), "007\n");
    assert_eq!(
        out(
            &s,
            "while getopts 'ab:' o -a -b val; do echo \"$o=$OPTARG\"; done"
        ),
        "a=\nb=val\n"
    );
    let _ = out(&s, "echo hi > hsrc; ln hsrc hdst");
    assert_eq!(out(&s, "cat hdst"), "hi\n");
}

#[test]
fn tar_unzip_directory_option() {
    let s = sdk();
    s.execute("mkdir -p tdir && echo a > tdir/1.txt && tar cf t.tar tdir");
    out(&s, "mkdir -p outd");
    let (o, e, rc) = run(&s, "tar -C outd -xf t.tar");
    assert_eq!(rc, 0, "out={o} err={e}");
    assert_eq!(out(&s, "cat outd/tdir/1.txt"), "a\n");
    out(&s, "mkdir -p oute");
    assert_eq!(run(&s, "tar --directory=oute -xf t.tar").2, 0);
    assert_eq!(out(&s, "cat oute/tdir/1.txt"), "a\n");
    s.execute("echo z > z.txt && zip -q z.zip z.txt && rm z.txt");
    out(&s, "mkdir -p uz");
    assert_eq!(run(&s, "unzip -o -d uz z.zip").2, 0);
    assert_eq!(out(&s, "cat uz/z.txt"), "z\n");
}

#[test]
fn external_commands_disabled_do_not_spawn() {
    // allow_subprocess=false (mobile): builtins must never spawn a process
    // (Android seccomp → SIGSYS would kill the app). The command is refused.
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_noexec_{}_{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Fastshell::new();
    s.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        allow_subprocess: false,
        ..Default::default()
    })
    .unwrap();

    for cmd in ["nice echo SPWNHI", "watch echo SPWNHI"] {
        let (o, e, _rc) = run(&s, cmd);
        assert!(
            !o.contains("SPWNHI") && !e.contains("SPWNHI"),
            "cmd={cmd:?} unexpectedly ran externally: out={o:?} err={e:?}"
        );
    }
    let (_o, _e, rc) = run(&s, "command -v no_such_cmd_xyz");
    assert_ne!(rc, 0);

    // Builtins still work; `timeout`/`xargs` run in-process (no spawn).
    assert_eq!(out(&s, "echo hi"), "hi\n");
    assert_eq!(out(&s, "timeout 5 echo hi"), "hi\n");
    assert_eq!(out(&s, "printf 'a\\nb\\n' | xargs").trim(), "a b");
}

#[test]
fn arith_assign_in_expansion() {
    let s = sdk();
    // `$((N=9))` applies the assignment (like `((N=9))`), not just evaluates.
    assert_eq!(out(&s, "N=0; echo $((N=9)); echo N=$N"), "9\nN=9\n");
    assert_eq!(out(&s, "S=1; echo $((S+=4)); echo S=$S"), "5\nS=5\n");
}

#[test]
fn sed_transliterate_and_tar_strip() {
    let s = sdk();
    assert_eq!(out(&s, "echo abcdef | sed y/abc/xyz/"), "xyzdef\n");
    s.execute("cd /; mkdir -p pkg/sub && echo hi > pkg/sub/f.txt && tar cf p.tar pkg");
    out(&s, "mkdir -p estrip");
    assert_eq!(run(&s, "tar xf p.tar -C estrip --strip-components=1").2, 0);
    assert_eq!(out(&s, "cat estrip/sub/f.txt"), "hi\n");
}

#[test]
fn arith_literals_and_pow() {
    let s = sdk();
    assert_eq!(out(&s, "echo $((0x10))"), "16\n");
    assert_eq!(out(&s, "echo $((16#ff))"), "255\n");
    assert_eq!(out(&s, "echo $((2#101))"), "5\n");
    assert_eq!(out(&s, "echo $((10#08))"), "8\n");
    assert_eq!(out(&s, "echo $((010))"), "8\n");
    assert_eq!(out(&s, "x=0x1f; echo $((x))"), "31\n");
    assert_eq!(out(&s, "echo $((2**10))"), "1024\n");
    assert_eq!(out(&s, "echo $((2**3**2))"), "512\n");
    assert_eq!(out(&s, "echo $((-2**2))"), "4\n");
    assert_eq!(out(&s, "echo $((-2**3))"), "-8\n");
    assert_eq!(out(&s, "echo $((2*-3))"), "-6\n");
}

#[test]
fn param_expansion_extras() {
    let s = sdk();
    assert_eq!(out(&s, "y=foo; p=y; echo ${!p}"), "foo\n");
    assert_eq!(out(&s, "unset u; echo ${u-default}"), "default\n");
    assert_eq!(out(&s, "u=; echo [${u-default}]"), "[]\n");
    assert_eq!(
        out(&s, "unset u; echo ${u=assigned}; echo $u"),
        "assigned\nassigned\n"
    );
    assert_eq!(out(&s, "u=abc; echo [${u+set}]"), "[set]\n");
    assert_eq!(out(&s, "x=abcabc; echo ${x/#a/X}"), "Xbcabc\n");
    assert_eq!(out(&s, "x=abcabc; echo ${x/%c/X}"), "abcabX\n");
    assert_eq!(out(&s, "h=a/b/c; echo ${h//\\//-}"), "a-b-c\n");
    assert_eq!(out(&s, "printf '%b\\n' 'a\\tb'"), "a\tb\n");
    assert_eq!(out(&s, "unset nope; echo rc=$?"), "rc=0\n");
}

#[test]
fn shell_builtins_extras() {
    let s = sdk();
    assert_eq!(
        out(&s, "set -- a b c; for x; do echo $x; done"),
        "a\nb\nc\n"
    );
    assert_eq!(out(&s, "let x=3+4; echo $x"), "7\n");
    assert_eq!(out(&s, "let y=0; echo $?"), "1\n");
    assert_eq!(out(&s, "let y=5; echo $?"), "0\n");
    assert_eq!(out(&s, "[ 1 = 1 -a 2 = 2 ]; echo $?"), "0\n");
    assert_eq!(out(&s, "[ 1 = 2 -o 2 = 2 ]; echo $?"), "0\n");
    assert_eq!(out(&s, "[ 1 = 2 -a 2 = 2 ]; echo $?"), "1\n");
    assert_eq!(
        out(&s, "[ \\( 1 = 1 -o 2 = 3 \\) -a 4 = 4 ]; echo $?"),
        "0\n"
    );
    assert_eq!(out(&s, "read -r a b <<< 'x y'; echo \"$a|$b\""), "x|y\n");
}

#[test]
fn sed_subst_print_flag() {
    let s = sdk();
    assert_eq!(out(&s, "echo hello | sed -n 's/l/L/p'"), "heLlo\n");
    assert_eq!(out(&s, "echo hello | sed -n 's/z/Z/p'"), "");
    assert_eq!(out(&s, "printf 'foo\\nbar\\n' | sed -n 's/o/O/p'"), "fOo\n");
    assert_eq!(out(&s, "echo aaa | sed 's/a/b/gp'"), "bbb\nbbb\n");
}
