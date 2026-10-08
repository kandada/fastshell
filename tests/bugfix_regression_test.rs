// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Regressions for the shell-parser/binary bugs found in real device sessions.

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn setup() -> (Fastshell, std::path::PathBuf) {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_bug_{}_{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dir);
    let mut sdk = Fastshell::new();
    sdk.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: false,
        allow_subprocess: true,
        network_ask_permission: false,
        command_timeout_ms: 30_000,
        ..Default::default()
    })
    .unwrap();
    (sdk, dir)
}

#[test]
fn colon_builtin_noop_and_truncate() {
    let (s, _) = setup();
    // `:` is a POSIX no-op builtin; `: > file` truncates.
    s.write_file("f.txt", "hello\n").unwrap();
    let r = s.execute(": > f.txt; wc -c < f.txt");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.trim().starts_with('0'), "stdout={}", r.stdout);
    // Bare `:` must succeed silently.
    let r = s.execute(": ; echo ok");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("ok"), "stdout={}", r.stdout);
    assert!(r.stderr.is_empty(), "stderr={}", r.stderr);
}

#[test]
fn find_prune_expression() {
    let (s, _) = setup();
    let r = s.execute("mkdir -p keep skip; touch keep/a.md skip/b.md");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    let r = s.execute("find . -path ./skip -prune -o -name '*.md' -print");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("./keep/a.md"), "stdout={}", r.stdout);
    assert!(!r.stdout.contains("skip/b.md"), "stdout={}", r.stdout);
    assert!(!r.stderr.contains("unsupported"), "stderr={}", r.stderr);
}

#[test]
fn for_loop_command_substitution() {
    let (s, _) = setup();
    let r = s.execute("for x in $(echo a b c); do echo \"<$x>\"; done");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("<a>"), "stdout={}", r.stdout);
    assert!(r.stdout.contains("<b>"), "stdout={}", r.stdout);
    assert!(r.stdout.contains("<c>"), "stdout={}", r.stdout);
}

#[test]
fn command_substitution_with_quoted_parens() {
    let (s, _) = setup();
    s.write_file("f.txt", "alpha\nbeta\ngamma\n").unwrap();
    // A `)` inside the quoted regex must not terminate `$(...)` early.
    let r = s.execute("for w in $(grep -oE 'a(l)pha|beta' f.txt); do echo \"[$w]\"; done");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("[alpha]"), "stdout={}", r.stdout);
    assert!(r.stdout.contains("[beta]"), "stdout={}", r.stdout);
}

#[test]
fn pipeline_after_escaped_quote_pattern() {
    let (s, _) = setup();
    // `'...'\''...'` idiom must not leave the parser stuck "inside a quote",
    // so the following `| cat` still splits and echo does NOT receive it.
    let r = s.execute(r#"echo 'a'\''b' | cat"#);
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert_eq!(r.stdout.trim(), "a'b", "stdout={:?}", r.stdout);
    // And a 3-stage pipeline after the idiom. The pattern embeds a literal
    // quote: `'x'\''|y'` → regex `x'|y`, matching both input lines.
    let r = s.execute(r#"printf "x'\ny\n" | grep -E 'x'\''|y' | wc -l"#);
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert_eq!(
        r.stdout.trim(),
        "2",
        "stdout={:?} stderr={}",
        r.stdout,
        r.stderr
    );
}

#[test]
fn head_reads_binary_bytes() {
    let (s, dir) = setup();
    // JPEG magic + arbitrary non-UTF-8 bytes.
    let bytes: Vec<u8> = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x80, 0xFE];
    std::fs::write(dir.join("m.jpg"), &bytes).unwrap();
    // `head` must not error on binary content.
    let r = s.execute("head -c 2 m.jpg > /dev/null; echo ok");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(!r.stderr.contains("valid UTF-8"), "stderr={}", r.stderr);
    assert!(r.stdout.contains("ok"), "stdout={}", r.stdout);
    // Byte-exact inspection goes through xxd/od (they read raw bytes).
    let r = s.execute("xxd m.jpg | head -n 1");
    assert!(
        r.stdout.to_lowercase().contains("ffd8"),
        "stdout={}",
        r.stdout
    );
}

#[test]
fn cat_tolerates_binary() {
    let (s, dir) = setup();
    let bytes: Vec<u8> = vec![0xFF, 0xD8, 0x00, 0x80, 0xFE, b'h', b'i'];
    std::fs::write(dir.join("m.bin"), &bytes).unwrap();
    let r = s.execute("cat m.bin > /dev/null; echo ok");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("ok"), "stdout={}", r.stdout);
    assert!(!r.stderr.contains("valid UTF-8"), "stderr={}", r.stderr);
}

#[test]
fn grep_count_prints_once() {
    let (s, _) = setup();
    s.write_file("f.txt", "a\nb\nb\n").unwrap();
    let r = s.execute("grep -c b f.txt");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    // Exactly one count line (regression: it used to print twice).
    assert_eq!(r.stdout, "2\n", "stdout={:?}", r.stdout);
}

#[test]
fn sed_backreferences() {
    let (s, _) = setup();
    let r = s.execute(r#"echo abc | sed -E 's/(a)(b)/\2\1/'"#);
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert_eq!(r.stdout.trim(), "bac", "stdout={:?}", r.stdout);
}

#[test]
fn glob_trailing_slash_preserved() {
    let (s, _) = setup();
    s.execute("mkdir -p sub");
    let r = s.execute("echo */");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert_eq!(r.stdout.trim(), "sub/", "stdout={:?}", r.stdout);
}

#[test]
fn column_output_separator() {
    let (s, _) = setup();
    // GNU `column -o <sep>`: replace the default 2-space gap.
    let r = s.execute(r#"printf 'a bb\nccc d\n' | column -t -o ' | '"#);
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains(" | "), "stdout={:?}", r.stdout);
    // `-s, -t`: split fields on comma, then align.
    let r = s.execute(r#"printf 'a,bb\nccc,d\n' | column -s, -t"#);
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(
        r.stdout.contains("bb") && r.stdout.contains("ccc"),
        "stdout={:?}",
        r.stdout
    );
}

#[test]
fn arithmetic_comparisons() {
    let (s, _) = setup();
    for (expr, want) in [
        ("$((2>=3))", "0"),
        ("$((3>=3))", "1"),
        ("$((2<3))", "1"),
        ("$((2<=2))", "1"),
        ("$((2==2))", "1"),
        ("$((2!=2))", "0"),
    ] {
        let r = s.execute(&format!("echo {expr}"));
        assert_eq!(r.stdout.trim(), want, "echo {expr}");
    }
}

#[test]
fn ln_symbolic_and_readlink_keep_relative_target() {
    let (s, _) = setup();
    s.write_file("target", "x").unwrap();
    let r = s.execute("ln -s target link; readlink link");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert_eq!(r.stdout.trim(), "target", "stdout={:?}", r.stdout);
}

#[test]
fn tar_dashless_and_roundtrip() {
    let (s, _) = setup();
    s.write_file("tf", "x").unwrap();
    let r = s.execute("tar cf tf.tar tf; tar tf tf.tar");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("tf"), "stdout={:?}", r.stdout);
}

#[test]
fn gzip_file_roundtrip() {
    let (s, _) = setup();
    let r = s.execute("printf 'hello\\n' > gf; gzip gf; gunzip gf.gz; cat gf");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert_eq!(r.stdout.trim(), "hello", "stdout={:?}", r.stdout);
}

#[test]
fn awk_last_field_and_accumulate() {
    let (s, _) = setup();
    let r = s.execute("echo 'a b c' | awk '{print $NF}'");
    assert_eq!(r.stdout.trim(), "c", "stdout={:?}", r.stdout);
    let r = s.execute("printf '1\\n2\\n3\\n' | awk '{s+=$1} END{print s}'");
    assert_eq!(r.stdout.trim(), "6", "stdout={:?}", r.stdout);
}

#[test]
fn printf_emits_nul_and_xargs_zero() {
    let (s, _) = setup();
    // `\0` must survive the pipe; `xargs -0` splits on NUL.
    let r = s.execute(r#"printf 'a\0b\0' | xargs -0 echo"#);
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert_eq!(r.stdout.trim_end(), "a b", "stdout={:?}", r.stdout);
}

#[test]
fn xargs_quote_grouping() {
    let (s, _) = setup();
    let r = s.execute(r#"printf "'a b'\nc\n" | xargs -n1 echo"#);
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert_eq!(r.stdout, "a b\nc\n", "stdout={:?}", r.stdout);
}

#[test]
fn xargs_max_lines() {
    let (s, _) = setup();
    let r = s.execute(r#"printf 'a b\nc\nd\n' | xargs -L1 echo"#);
    assert_eq!(r.stdout, "a b\nc\nd\n", "stdout={:?}", r.stdout);
    let r = s.execute(r#"printf 'a b\nc\nd\n' | xargs -L2 echo"#);
    assert_eq!(r.stdout, "a b c\nd\n", "stdout={:?}", r.stdout);
}

#[test]
fn tac_no_leading_blank_line() {
    let (s, _) = setup();
    let r = s.execute(r#"printf 'a\nb\n' | tac"#);
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert_eq!(r.stdout, "b\na\n", "stdout={:?}", r.stdout);
    // Input without a trailing newline keeps it that way.
    let r = s.execute(r#"printf 'a\nb' | tac"#);
    assert_eq!(r.stdout, "b\na", "stdout={:?}", r.stdout);
}

#[test]
fn break_and_continue_take_effect() {
    let (s, _) = setup();
    let r = s.execute("for i in 1 2 3; do if [ $i -eq 2 ]; then break; fi; echo $i; done");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert_eq!(r.stdout.trim(), "1", "stdout={}", r.stdout);

    let r = s.execute("for i in 1 2 3; do if [ $i -eq 2 ]; then continue; fi; echo $i; done");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert_eq!(
        r.stdout.trim().replace('\n', " "),
        "1 3",
        "stdout={}",
        r.stdout
    );
}

#[test]
fn command_substitution_keeps_internal_newlines() {
    let (s, _) = setup();
    let r = s.execute("x=$(printf 'a\\nb\\n'); printf '%s' \"$x\" | wc -l");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.trim().starts_with('2'), "stdout={:?}", r.stdout);
}

#[test]
fn arrays_assign_and_expand() {
    let (s, _) = setup();
    let r = s.execute("a=(x y z); echo \"${a[1]}\"; echo \"${#a[@]}\"; echo \"${a[@]}\"");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert_eq!(r.stdout, "y\n3\nx y z\n", "stdout={:?}", r.stdout);
    let r = s.execute("a=(x y z); for e in \"${a[@]}\"; do echo \"<$e>\"; done");
    assert_eq!(r.stdout, "<x>\n<y>\n<z>\n", "stdout={:?}", r.stdout);
}

#[test]
fn double_bracket_conditions() {
    let (s, _) = setup();
    s.write_file("f.txt", "hi\n").unwrap();
    for (cmd, want) in [
        ("[[ -f f.txt ]] && echo y || echo n", "y"),
        ("[[ a == a ]] && echo y || echo n", "y"),
        ("[[ abc =~ ^a.c$ ]] && echo y || echo n", "y"),
        ("[[ \"a b\" == *\" \"* ]] && echo y || echo n", "y"),
        ("[[ 3 -gt 2 ]] && echo y || echo n", "y"),
        ("[[ ! -f nope ]] && echo y || echo n", "y"),
    ] {
        let r = s.execute(cmd);
        assert_eq!(r.stdout.trim(), want, "cmd={cmd} stdout={:?}", r.stdout);
    }
}

#[test]
fn arithmetic_command_has_side_effects() {
    let (s, _) = setup();
    let r = s.execute("i=5; ((i++)); echo $i; ((i+=3)); echo $i");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert_eq!(r.stdout, "6\n9\n", "stdout={:?}", r.stdout);
}

#[test]
fn ansi_c_quoting_and_positional_params() {
    let (s, _) = setup();
    let r = s.execute("printf '%s' $'a\\tb' | od -An -c");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    // `od -An -c` renders the tab as `\t` (GNU `od -c` behavior).
    assert!(r.stdout.contains("\\t"), "stdout={:?}", r.stdout);

    let r = s.execute("set -- a b c; echo $#; echo $1; echo $*");
    assert_eq!(r.stdout, "3\na\na b c\n", "stdout={:?}", r.stdout);

    let r = s.execute("set -- a b c; for x in \"$@\"; do echo \"<$x>\"; done");
    assert_eq!(r.stdout, "<a>\n<b>\n<c>\n", "stdout={:?}", r.stdout);

    let r = s.execute("f(){ local v=1; echo $v; }; f; echo \"[$v]\"");
    assert_eq!(r.stdout, "1\n[]\n", "stdout={:?}", r.stdout);
}

#[test]
fn return_and_exit_codes() {
    let (s, _) = setup();
    let r = s.execute("f(){ return 3; }; f; echo $?");
    assert_eq!(r.stdout.trim(), "3", "stdout={:?}", r.stdout);
    let r = s.execute("(exit 3); echo $?");
    assert_eq!(r.stdout.trim(), "3", "stdout={:?}", r.stdout);
}

#[test]
fn parameter_case_and_substr() {
    let (s, _) = setup();
    assert_eq!(s.execute("v=abc; echo \"${v^^}\"").stdout.trim(), "ABC");
    assert_eq!(s.execute("v=ABC; echo \"${v,,}\"").stdout.trim(), "abc");
    assert_eq!(s.execute("v=abcdef; echo \"${v: -2}\"").stdout.trim(), "ef");
}

#[test]
fn ls_missing_path_goes_to_stderr() {
    let (s, _) = setup();
    // `2>/dev/null` suppresses the error; the compound's status comes from echo.
    let r = s.execute("ls nope 2>/dev/null; echo X");
    assert_eq!(r.stdout.trim(), "X", "stdout={:?}", r.stdout);
    // The command itself must report failure and keep stdout clean.
    let r = s.execute("ls nope 2>/dev/null");
    assert_eq!(r.stdout, "", "stdout={:?}", r.stdout);
    assert_ne!(
        r.exit_code, 0,
        "ls exit={} stderr={:?}",
        r.exit_code, r.stderr
    );
    let r = s.execute("ls nope");
    assert_eq!(r.stdout, "", "stdout={:?}", r.stdout);
    assert!(!r.stderr.is_empty(), "stderr should carry the error");
    assert_ne!(r.exit_code, 0, "bare ls exit={}", r.exit_code);
}

#[test]
fn awk_control_flow() {
    let (s, _) = setup();
    s.write_file("nums.txt", "10\n2\n33\n").unwrap();
    let cases = [
        ("awk 'BEGIN{for(i=1;i<=3;i++)print i}'", "1\n2\n3\n"),
        ("awk 'BEGIN{i=0; while(i<3){print i; i++}}'", "0\n1\n2\n"),
        ("awk 'BEGIN{s=0; for(i=1;i<=5;i++) s+=i; print s}'", "15\n"),
        (
            "awk 'BEGIN{for(i=1;i<=5;i++){if(i==3) continue; print i}}'",
            "1\n2\n4\n5\n",
        ),
        (
            "awk 'BEGIN{for(i=1;i<=5;i++){if(i==4) break; print i}}'",
            "1\n2\n3\n",
        ),
        (
            "awk 'BEGIN{if(2>1) print \"yes\"; else print \"no\"}'",
            "yes\n",
        ),
        ("awk '{if($1>5) print $1}' nums.txt", "10\n33\n"),
    ];
    for (cmd, want) in cases {
        let r = s.execute(cmd);
        assert_eq!(r.stdout, want, "cmd={cmd} stderr={}", r.stderr);
    }
}

#[test]
fn jq_arithmetic_and_comparison() {
    let (s, _) = setup();
    for (cmd, want) in [
        ("jq -n '1+1'", "2"),
        ("jq -n '2*3'", "6"),
        ("jq -n '10/4'", "2.5"),
        ("jq -n '1+2*3'", "7"),
        ("echo '{\"a\":3}' | jq '.a + 1'", "4"),
        ("echo '{\"a\":3}' | jq '.a > 1'", "true"),
        ("echo '\"x\"' | jq '. + \"y\"'", "\"xy\""),
    ] {
        let r = s.execute(cmd);
        assert_eq!(r.stdout.trim(), want, "cmd={cmd} stderr={}", r.stderr);
    }
}

#[test]
fn pipe_into_brace_read_group() {
    let (s, _) = setup();
    let r = s.execute("echo one two three | { read x y z; echo \"$z\"; }");
    assert_eq!(r.stdout.trim(), "three", "stderr={}", r.stderr);
    let r = s.execute("printf 'a b\\n' | { read x y; echo \"$y-$x\"; }");
    assert_eq!(r.stdout.trim(), "b-a", "stderr={}", r.stderr);
}

#[test]
fn engine_fixes_batch() {
    let (s, _) = setup();
    // cp -r into itself must error, not recurse forever (stack overflow).
    let r = s.execute("mkdir d1; cp -r d1 d1/sub");
    assert_ne!(r.exit_code, 0, "cp -r into itself must fail");
    // killall with no program name must not signal every process.
    let r = s.execute("killall");
    assert_ne!(r.exit_code, 0);
    let r = s.execute("killall --list");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("SIGTERM"));
    // tail from a pipe keeps the trailing newline.
    let r = s.execute("printf 'a\\nb\\n' | tail -n 1");
    assert_eq!(r.stdout, "b\n");
    // nested if.
    let r = s.execute("if true; then if false; then echo a; else echo b; fi; fi");
    assert_eq!(r.stdout.trim(), "b");
    // case with `;;` branches (second branch must run).
    let r = s.execute("case x in a) echo A;; x) echo X;; esac");
    assert_eq!(r.stdout.trim(), "X");
    // $(( )) inside a subshell must not split the `;`.
    let r = s.execute("( v=$((2+2)); echo $v )");
    assert_eq!(r.stdout.trim(), "4");
    // `for` leaves the loop variable set to its last value (bash semantics).
    let r = s.execute("for i in 1 2 3; do :; done; echo $i");
    assert_eq!(r.stdout.trim(), "3");
}
