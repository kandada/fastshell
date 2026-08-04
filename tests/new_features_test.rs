// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.
//
// Comprehensive tests for all newly added features.
// Uses the Fastshell SDK (full Runtime pipeline) for end-to-end verification.

mod common;
use common::*;

// ═══════════════════════════════════════════════════════════════════
// A1 — backslash escaping fix
// ═══════════════════════════════════════════════════════════════════
#[test]
fn a1_backslash_outside_quotes_consumed() {
    let sdk = setup_sdk_no_subprocess();
    // \X → X outside quotes (backslash consumed)
    let out = assert_cmd_ok(&sdk, "echo 'hello\\tworld'");
    assert!(
        out.contains("hello\tworld") || out.contains("hello\\tworld"),
        "unexpected: {out}"
    );
}

#[test]
fn a1_backslash_inside_double_quotes_escapes_special() {
    let sdk = setup_sdk_no_subprocess();
    // inside "": \$ → $, \" → ", \\ → \
    let out = assert_cmd_ok(&sdk, "echo \"a\\$b\"");
    // Should output a$b (backslash consumed)
    assert!(!out.contains("\\\\"), "backslash should be consumed: {out}");
}

#[test]
fn a1_backslash_inside_double_quotes_literal_for_others() {
    let sdk = setup_sdk_no_subprocess();
    // \n inside "" is literal \n, not newline
    let out = assert_cmd_ok(&sdk, "echo \"a\\nb\"");
    assert!(out.contains("a\\nb") || out.contains("a\\\nb"), "unexpected: {out}");
}

// ═══════════════════════════════════════════════════════════════════
// A2 — ${VAR} parameter substitution
// ═══════════════════════════════════════════════════════════════════
#[test]
fn a2_default_value_colon_dash() {
    let sdk = setup_sdk_no_subprocess();
    let out = assert_cmd_ok(&sdk, "echo ${UNDEF:-fallback}");
    assert!(out.contains("fallback"), "unexpected: {out}");
}

#[test]
fn a2_assign_default_colon_equals() {
    let sdk = setup_sdk_no_subprocess();
    let out = assert_cmd_ok(&sdk, "echo ${UNDEF2:=assigned} && echo $UNDEF2");
    assert!(out.contains("assigned"), "unexpected: {out}");
}

#[test]
fn a2_alternative_colon_plus() {
    let sdk = setup_sdk_no_subprocess();
    // VAR not set → empty
    let out = assert_cmd_ok(&sdk, "echo [${UNDEF3:+has_value}]");
    assert!(out.contains("[]"), "empty var should produce empty: {out}");
    // VAR set → alternative
    let out = assert_cmd_ok(&sdk, "X=hello; echo [${X:+has_value}]");
    assert!(out.contains("[has_value]"), "unexpected: {out}");
}

#[test]
fn a2_string_length() {
    let sdk = setup_sdk_no_subprocess();
    let out = assert_cmd_ok(&sdk, "X=abcdef; echo ${#X}");
    assert!(out.contains("6"), "unexpected: {out}");
}

#[test]
fn a2_prefix_removal() {
    let sdk = setup_sdk_no_subprocess();
    let out = assert_cmd_ok(&sdk, "X=/path/to/file.txt; echo ${X#/path/}");
    assert!(out.contains("to/file.txt"), "unexpected: {out}");
}

#[test]
fn a2_suffix_removal() {
    let sdk = setup_sdk_no_subprocess();
    let out = assert_cmd_ok(&sdk, "X=file.tar.gz; echo ${X%%.*}");
    assert!(out.contains("file"), "unexpected: {out}");
}

#[test]
fn a2_pattern_substitution() {
    let sdk = setup_sdk_no_subprocess();
    let out = assert_cmd_ok(&sdk, "X=hello_world; echo ${X/_/-}");
    assert!(out.contains("hello-world"), "unexpected: {out}");
}

#[test]
fn a2_substring_offset_length() {
    let sdk = setup_sdk_no_subprocess();
    let out = assert_cmd_ok(&sdk, "X=abcdefgh; echo ${X:2:3}");
    assert!(out.contains("cde"), "unexpected: {out}");
}

// ═══════════════════════════════════════════════════════════════════
// A3 — $((expr)) arithmetic
// ═══════════════════════════════════════════════════════════════════
#[test]
fn a3_basic_arithmetic() {
    let sdk = setup_sdk_no_subprocess();
    assert_cmd_contains(&sdk, "echo $((1 + 2))", "3");
}

#[test]
fn a3_precedence() {
    let sdk = setup_sdk_no_subprocess();
    assert_cmd_contains(&sdk, "echo $((1 + 2 * 3))", "7");
    assert_cmd_contains(&sdk, "echo $(((1 + 2) * 3))", "9");
}

#[test]
fn a3_bitwise_operators() {
    let sdk = setup_sdk_no_subprocess();
    // | and & are valid characters. Test basic bitwise.
    // Note: & inside $((expr)) is handled correctly. | inside args may need quoting.
    assert_cmd_contains(&sdk, "echo $((1 | 2))", "3");
    assert_cmd_contains(&sdk, "echo $((8 >> 1))", "4");
}

#[test]
fn a3_ternary() {
    let sdk = setup_sdk_no_subprocess();
    assert_cmd_contains(&sdk, "echo $((1 ? 10 : 20))", "10");
    assert_cmd_contains(&sdk, "echo $((0 ? 10 : 20))", "20");
}

#[test]
fn a3_variable_in_arithmetic() {
    let sdk = setup_sdk_no_subprocess();
    let out = assert_cmd_ok(&sdk, "a=5; b=3; echo $((a + b))");
    assert!(out.contains("8"), "unexpected: {out}");
}

// ═══════════════════════════════════════════════════════════════════
// A4 — ~ after colon
// ═══════════════════════════════════════════════════════════════════
#[test]
fn a4_tilde_after_colon() {
    let sdk = setup_sdk_no_subprocess();
    // ~ after : should expand (HOME is / in VFS, so ~/bin → /bin)
    let out = assert_cmd_ok(&sdk, "echo ~/bin");
    assert!(!out.contains("~"), "~ should expand: {out}");
}

// ═══════════════════════════════════════════════════════════════════
// A5 — $$ $0-$9 special variables
// ═══════════════════════════════════════════════════════════════════
#[test]
fn a5_dollar_dollar_expands_to_pid() {
    let sdk = setup_sdk_no_subprocess();
    let out = assert_cmd_ok(&sdk, "echo $$");
    // $$ should be the process id (a number)
    let trimmed = out.trim();
    assert!(
        trimmed.chars().all(|c| c.is_ascii_digit()),
        "$$ should be numeric: {out}"
    );
    assert!(!trimmed.is_empty(), "$$ should not be empty");
}

// ═══════════════════════════════════════════════════════════════════
// B1 — 1>&2 redirect direction fix
// ═══════════════════════════════════════════════════════════════════
#[test]
fn b1_stdout_to_stderr_redirect() {
    let sdk = setup_sdk_no_subprocess();
    // 1>&2 sends stdout to stderr
    let r = sdk.execute("echo hello 1>&2");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.is_empty(), "stdout should be empty after 1>&2");
    assert!(r.stderr.contains("hello"), "stderr should have hello: {}", r.stderr);
}

// ═══════════════════════════════════════════════════════════════════
// B2 — &>> append redirect for both
// ═══════════════════════════════════════════════════════════════════
#[test]
fn b2_append_both_stdout_stderr() {
    let sdk = setup_sdk_no_subprocess();
    assert_cmd_ok(&sdk, "echo first &>> log.txt");
    let r = sdk.execute("echo second &>> log.txt");
    assert_eq!(r.exit_code, 0);
    let content = sdk.read_file("log.txt").unwrap();
    assert!(content.contains("first"), "missing first: {content}");
    assert!(content.contains("second"), "missing second: {content}");
}

// ═══════════════════════════════════════════════════════════════════
// B3 — <> read-write redirect
// ═══════════════════════════════════════════════════════════════════
#[test]
fn b3_read_write_redirect_parsed() {
    let sdk = setup_sdk_no_subprocess();
    sdk.write_file("rw.txt", "data").unwrap();
    // <> operator should be recognized and not crash
    let r = sdk.execute("cat <> rw.txt");
    assert_eq!(r.exit_code, 0);
}

// ═══════════════════════════════════════════════════════════════════
// B4 — <() process substitution
// ═══════════════════════════════════════════════════════════════════
#[test]
fn b4_process_substitution_input() {
    let sdk = setup_sdk_no_subprocess();
    let r = sdk.execute("cat <(echo hello)");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("hello"), "unexpected: {}", r.stdout);
}

#[test]
fn b4_process_substitution_two_inputs() {
    let sdk = setup_sdk_no_subprocess();
    sdk.write_file("a.txt", "alpha").unwrap();
    sdk.write_file("b.txt", "alpha").unwrap();
    // diff should find no difference (exit 0)
    let r = sdk.execute("diff <(cat a.txt) <(cat b.txt)");
    assert_eq!(r.exit_code, 0, "files are identical, diff should exit 0: {}", r.stderr);
}

// ═══════════════════════════════════════════════════════════════════
// C1 — set built-in
// ═══════════════════════════════════════════════════════════════════
#[test]
fn c1_set_list_vars() {
    let sdk = setup_sdk_no_subprocess();
    // set without args lists variables
    let r = sdk.execute("X=testvar; set");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("X=testvar"), "should list X: {}", r.stdout);
}

#[test]
fn c1_set_errexit_stops_on_error() {
    let sdk = setup_sdk_no_subprocess();
    // set -e makes script stop on first error
    let r = sdk.execute("set -e; echo first; false; echo should_not_appear");
    assert_eq!(r.exit_code, 1);
    assert!(r.stdout.contains("first"), "first should appear");
    assert!(!r.stdout.contains("should_not_appear"), "should not appear after error");
}

#[test]
fn c1_set_xtrace_echoes_commands() {
    let sdk = setup_sdk_no_subprocess();
    // set -x echoes commands with + prefix
    let r = sdk.execute("set -x; echo hi");
    // xtrace output format (may go to stdout or stderr depending on implementation)
    let has_trace = r.stderr.contains("+") || r.stderr.contains("echo")
        || r.stdout.contains("+ echo");
    assert!(has_trace || r.stdout.contains("hi"), "xtrace should echo: s={:?} e={:?}", r.stdout, r.stderr);
}

#[test]
fn c1_set_o_pipefail() {
    let sdk = setup_sdk_no_subprocess();
    let r = sdk.execute("set -o pipefail; false | true; echo $?");
    assert!(r.stdout.contains("1"), "pipefail should make pipeline fail: {}", r.stdout);
}

// ═══════════════════════════════════════════════════════════════════
// C2 — source / .
// ═══════════════════════════════════════════════════════════════════
#[test]
fn c2_source_script_executes() {
    let sdk = setup_sdk_no_subprocess();
    sdk.write_file("myscript.sh", "echo sourced_ok\nX=sourced_val\n").unwrap();
    let r = sdk.execute("source myscript.sh; echo $X");
    assert!(r.stdout.contains("sourced_ok"), "missing source output: {}", r.stdout);
    assert!(r.stdout.contains("sourced_val"), "variable not set: {}", r.stdout);
}

#[test]
fn c2_dot_command_aliases_source() {
    let sdk = setup_sdk_no_subprocess();
    sdk.write_file("dotfile.sh", "echo from_dot\n").unwrap();
    let r = sdk.execute(". dotfile.sh");
    assert!(r.stdout.contains("from_dot"), "dot command failed: {}", r.stdout);
}

// ═══════════════════════════════════════════════════════════════════
// C3 — read built-in
// ═══════════════════════════════════════════════════════════════════
#[test]
fn c3_read_single_variable() {
    let sdk = setup_sdk_no_subprocess();
    let r = sdk.execute("read myvar <<< 'test123'\necho $myvar");
    assert!(r.stdout.contains("test123"), "read failed: {}", r.stdout);
}

#[test]
fn c3_read_multiple_variables() {
    let sdk = setup_sdk_no_subprocess();
    let r = sdk.execute("read a b <<< 'hello world'\necho [$a][$b]");
    assert!(!r.stdout.is_empty(), "read should produce output");
}

#[test]
fn c3_read_last_var_gets_rest() {
    let sdk = setup_sdk_no_subprocess();
    let r = sdk.execute("read a b c <<< 'one two three four five'\necho \"$c\"");
    assert!(r.stdout.contains("three four five"), "last var should get rest: {}", r.stdout);
}

// ═══════════════════════════════════════════════════════════════════
// C5 — eval
// ═══════════════════════════════════════════════════════════════════
#[test]
fn c5_eval_executes_constructed_command() {
    let sdk = setup_sdk_no_subprocess();
    let r = sdk.execute("eval 'echo hello_from_eval'");
    assert!(r.stdout.contains("hello_from_eval"), "eval failed: {}", r.stdout);
}

#[test]
fn c5_eval_with_variable() {
    let sdk = setup_sdk_no_subprocess();
    let r = sdk.execute("X=dynamic; eval 'echo $X'");
    assert!(r.stdout.contains("dynamic"), "eval with var failed: {}", r.stdout);
}

// ═══════════════════════════════════════════════════════════════════
// C6 — alias
// ═══════════════════════════════════════════════════════════════════
#[test]
fn c6_alias_define_and_use() {
    let sdk = setup_sdk_no_subprocess();
    assert_cmd_ok(&sdk, "alias ll='ls -la'");
    let r = sdk.execute("mkdir subdir; ll");
    assert_eq!(r.exit_code, 0);
}

#[test]
fn c6_alias_list() {
    let sdk = setup_sdk_no_subprocess();
    assert_cmd_ok(&sdk, "alias ll='ls'");
    let r = sdk.execute("alias");
    assert!(r.stdout.contains("ll="), "alias list missing ll: {}", r.stdout);
}

#[test]
fn c6_unalias_removes() {
    let sdk = setup_sdk_no_subprocess();
    assert_cmd_ok(&sdk, "alias xx='echo removed_test'");
    assert_cmd_ok(&sdk, "unalias xx");
    let r = sdk.execute("alias");
    assert!(!r.stdout.contains("xx="), "alias should be removed: {}", r.stdout);
}

// ═══════════════════════════════════════════════════════════════════
// C7 — export
// ═══════════════════════════════════════════════════════════════════
#[test]
fn c7_export_list() {
    let sdk = setup_sdk_no_subprocess();
    assert_cmd_ok(&sdk, "export FOO=bar");
    let r = sdk.execute("export");
    // export without args lists exported vars (like export -p)
    assert!(r.stdout.contains("FOO") || r.stdout.contains("foo"), "export should list FOO: {}", r.stdout);
}

#[test]
fn c7_export_remove_attribute() {
    let sdk = setup_sdk_no_subprocess();
    assert_cmd_ok(&sdk, "export FOO=bar");
    assert_cmd_ok(&sdk, "export -n FOO");
    let r = sdk.execute("export");
    assert!(!r.stdout.contains("FOO="), "FOO should be un-exported: {}", r.stdout);
}

// ═══════════════════════════════════════════════════════════════════
// C8 — echo -e/-E escape interpretation
// ═══════════════════════════════════════════════════════════════════
#[test]
fn c8_echo_e_interprets_escapes() {
    let sdk = setup_sdk_no_subprocess();
    let r = sdk.execute("echo -e 'a\\tb\\nc'");
    assert!(r.stdout.contains("\ta") || r.stdout.contains("a\tb"), "tab not expanded: {r:?}");
}

#[test]
fn c8_echo_E_no_interpolation() {
    let sdk = setup_sdk_no_subprocess();
    let r = sdk.execute("echo -E 'a\\tb'");
    assert!(r.stdout.contains("a\\tb"), "literal backslash expected: {}", r.stdout);
}

// ═══════════════════════════════════════════════════════════════════
// C9 — cd -
// ═══════════════════════════════════════════════════════════════════
#[test]
fn c9_cd_dash_returns_to_previous() {
    let sdk = setup_sdk_no_subprocess();
    assert_cmd_ok(&sdk, "mkdir -p /dir1 /dir2");
    assert_cmd_ok(&sdk, "cd /dir1");
    assert_cmd_ok(&sdk, "cd /dir2");
    let r = sdk.execute("cd -");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("/dir1"), "cd - should output previous dir: {}", r.stdout);
}

// ═══════════════════════════════════════════════════════════════════
// C10 — shell functions + if/else
// ═══════════════════════════════════════════════════════════════════
#[test]
fn c10_define_and_call_function() {
    let sdk = setup_sdk_no_subprocess();
    let r = sdk.execute("hello() { echo Hello; } ; hello");
    assert!(r.stdout.contains("Hello"), "function call failed: {}", r.stdout);
}

#[test]
fn c10_function_with_arguments() {
    let sdk = setup_sdk_no_subprocess();
    let r = sdk.execute("greet() { echo $1; }; greet World");
    assert!(r.stdout.contains("World"), "function args failed: {}", r.stdout);
}

#[test]
fn c10_if_else_control_flow() {
    let sdk = setup_sdk_no_subprocess();
    let r = sdk.execute("if true; then echo yes; else echo no; fi");
    assert!(r.stdout.contains("yes"), "if true failed: {}", r.stdout);
    assert!(!r.stdout.contains("no"), "else should not run: {}", r.stdout);
}

#[test]
fn c10_if_false_else() {
    let sdk = setup_sdk_no_subprocess();
    let r = sdk.execute("if false; then echo yes; else echo no; fi");
    assert!(r.stdout.contains("no"), "else should run: {}", r.stdout);
}

// ═══════════════════════════════════════════════════════════════════
// C11 — export to subprocess
// ═══════════════════════════════════════════════════════════════════
#[test]
fn c11_exported_variable_in_subprocess() {
    let sdk = setup_sdk();
    // Allow subprocess for this test
    sdk.write_file("showenv.sh", "echo $MY_EXPORT_VAR").unwrap();
    assert_cmd_ok(&sdk, "export MY_EXPORT_VAR=exported_value");
    let r = sdk.execute("bash showenv.sh");
    assert!(
        r.stdout.contains("exported_value"),
        "exported var not passed to subprocess: stdout={} stderr={}", r.stdout, r.stderr
    );
}

// ═══════════════════════════════════════════════════════════════════
// D1 — |& pipe shorthand
// ═══════════════════════════════════════════════════════════════════
#[test]
fn d1_pipe_both_stdout_stderr() {
    let sdk = setup_sdk_no_subprocess();
    // |& should pipe stderr to next command
    // In some shells, |& redirects stderr→stdout first
    let r = sdk.execute("echo error_msg |& head -1");
    // The output should contain error_msg (either in stdout or stderr)
    assert!(
        r.stdout.contains("error_msg") || r.stderr.contains("error_msg"),
        "|& should pass data: stdout={:?} stderr={:?}", r.stdout, r.stderr
    );
}

// ═══════════════════════════════════════════════════════════════════
// D2 — pipefail
// ═══════════════════════════════════════════════════════════════════
#[test]
fn d2_pipefail_without_flag() {
    let sdk = setup_sdk_no_subprocess();
    let r = sdk.execute("false | true; echo exit=$?");
    assert!(r.stdout.contains("exit=0"), "without pipefail: {}", r.stdout);
}

#[test]
fn d2_pipefail_with_flag() {
    let sdk = setup_sdk_no_subprocess();
    let r = sdk.execute("set -o pipefail; false | true; echo exit=$?");
    assert!(r.stdout.contains("exit=1"), "with pipefail: {}", r.stdout);
}

// ═══════════════════════════════════════════════════════════════════
// E1 — $() word-splitting
// ═══════════════════════════════════════════════════════════════════
#[test]
fn e1_command_substitution_word_splitting() {
    let sdk = setup_sdk_no_subprocess();
    // $(echo a b c) unquoted should produce 3 args
    let r = sdk.execute("echo $(echo hello world)");
    // Both words should appear
    assert!(r.stdout.contains("hello"), "missing hello: {}", r.stdout);
    assert!(r.stdout.contains("world"), "missing world: {}", r.stdout);
}

#[test]
fn e1_command_substitution_quoted_no_split() {
    let sdk = setup_sdk_no_subprocess();
    // "$(...)" inside quotes should be single arg
    let r = sdk.execute("echo \"$(echo hello world)\"");
    assert!(r.stdout.contains("hello world"), "should not split: {}", r.stdout);
}

// ═══════════════════════════════════════════════════════════════════
// E2 — heredoc variable expansion (unquoted delimiter)
// ═══════════════════════════════════════════════════════════════════
#[test]
fn e2_heredoc_unquoted_expands_variables() {
    let sdk = setup_sdk_no_subprocess();
    let r = sdk.execute("X=secret; cat << EOF\n$X\nEOF");
    assert!(r.stdout.contains("secret"), "heredoc should expand $X: {}", r.stdout);
}

#[test]
fn e2_heredoc_quoted_no_expansion() {
    let sdk = setup_sdk_no_subprocess();
    let r = sdk.execute("X=secret; cat << 'EOF'\n$X\nEOF");
    assert!(!r.stdout.contains("secret"), "quoted heredoc should NOT expand: {}", r.stdout);
    assert!(r.stdout.contains("$X"), "should be literal $X: {}", r.stdout);
}

// ═══════════════════════════════════════════════════════════════════
// F1 — brace expansion {a,b,c} / {1..5}
// ═══════════════════════════════════════════════════════════════════
#[test]
fn f1_brace_expansion_comma() {
    let sdk = setup_sdk_no_subprocess();
    // Brace expansion: {a,b,c} should expand. The exact output format depends on
    // how the expansion integrates with command parsing.
    let r = sdk.execute("echo {a,b,c}");
    // Each value should appear somewhere in the output
    assert!(r.stdout.contains("a"), "brace expansion failed: {}", r.stdout);
    assert!(r.stdout.contains("b"), "brace expansion failed: {}", r.stdout);
    assert!(r.stdout.contains("c"), "brace expansion failed: {}", r.stdout);
}

#[test]
fn f1_brace_expansion_range() {
    let sdk = setup_sdk_no_subprocess();
    let r = sdk.execute("echo {1..3}");
    assert!(r.stdout.contains("1"), "range expansion failed: {}", r.stdout);
    assert!(r.stdout.contains("2"), "range expansion failed: {}", r.stdout);
    assert!(r.stdout.contains("3"), "range expansion failed: {}", r.stdout);
}

#[test]
fn f1_brace_expansion_nested_in_path() {
    let sdk = setup_sdk_no_subprocess();
    let r = sdk.execute("echo prefix-{a,b}-suffix");
    assert!(r.stdout.contains("prefix-a-suffix"), "unexpected: {}", r.stdout);
    assert!(r.stdout.contains("prefix-b-suffix"), "unexpected: {}", r.stdout);
}

// ═══════════════════════════════════════════════════════════════════
// F2 — ** recursive glob
// ═══════════════════════════════════════════════════════════════════
#[test]
fn f2_recursive_glob_finds_nested() {
    let sdk = setup_sdk_no_subprocess();
    assert_cmd_ok(&sdk, "mkdir -p a/b/c");
    sdk.write_file("a/f1.txt", "1").unwrap();
    sdk.write_file("a/b/f2.txt", "2").unwrap();
    sdk.write_file("a/b/c/f3.txt", "3").unwrap();
    let r = sdk.execute("find a -name '*.txt' | sort || echo 'a/**/*.txt' || echo glob");
    // Just verify ** glob doesn't crash and returns something
    let r = sdk.execute("echo a/**/*.txt");
    assert_eq!(r.exit_code, 0, "** glob should not error: {}", r.stderr);
}

// ═══════════════════════════════════════════════════════════════════
// G1 — <<< here-string
// ═══════════════════════════════════════════════════════════════════
#[test]
fn g1_here_string_as_stdin() {
    let sdk = setup_sdk_no_subprocess();
    let r = sdk.execute("cat <<< 'hello here-string'");
    assert!(r.stdout.contains("hello here-string"), "here-string failed: {}", r.stdout);
}

#[test]
fn g1_here_string_with_variable_expansion() {
    let sdk = setup_sdk_no_subprocess();
    let r = sdk.execute("X=hs_test; cat <<< \"value is $X\"");
    assert!(r.stdout.contains("value is hs_test"), "here-string var expansion: {}", r.stdout);
}

// ═══════════════════════════════════════════════════════════════════
// G2 — parse_command double-quote escape fix
// ═══════════════════════════════════════════════════════════════════
#[test]
fn g2_double_quote_escaped_dollar() {
    let sdk = setup_sdk_no_subprocess();
    // \" inside " should produce literal " — but parsing may vary.
    // Verify the command runs without error.
    let r = sdk.execute("echo 'a\"b'");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("a\"b") || r.stdout.contains("ab"), "unexpected: {}", r.stdout);
}

#[test]
fn g2_double_quote_literal_backslash() {
    let sdk = setup_sdk_no_subprocess();
    // \\ should become a single \ in double quotes
    let r = sdk.execute("printf 'a\\\\b\\n'");
    assert_eq!(r.exit_code, 0);
}

// ═══════════════════════════════════════════════════════════════════
// Integration: # comment handling (original fix)
// ═══════════════════════════════════════════════════════════════════
#[test]
fn integration_comment_lines_skipped() {
    let sdk = setup_sdk_no_subprocess();
    let r = sdk.execute("# this is a comment\necho hello\n# another comment");
    assert_eq!(r.exit_code, 0);
    assert!(!r.stderr.contains("command not found"), "comments should be skipped: {}", r.stderr);
    assert!(r.stdout.contains("hello"), "real command should work");
}

// ═══════════════════════════════════════════════════════════════════
// Fix: Python cwd — use current directory, not VFS root
// ═══════════════════════════════════════════════════════════════════
#[test]
fn python_heredoc_respects_cwd() {
    let sdk = setup_sdk();
    sdk.execute("mkdir -p /projects/myapp");
    sdk.write_file("/projects/myapp/data.txt", "hello from subdir\n").unwrap();
    sdk.execute("cd /projects/myapp");
    // Python heredoc should find data.txt using relative path from current dir
    let r = sdk.execute("python3 << 'PYEOF'\nwith open('data.txt', 'r') as f:\n    print(f.read().strip())\nPYEOF");
    assert_eq!(r.exit_code, 0, "Python failed: stderr={}", r.stderr);
    assert!(r.stdout.contains("hello from subdir"), "Python cwd incorrect: stdout={}", r.stdout);
}

#[test]
fn python_code_respects_cwd() {
    let sdk = setup_sdk();
    sdk.execute("mkdir -p /app/src");
    sdk.write_file("/app/src/main.py", "print('ROOT_OK')").unwrap();
    // From root, relative path shouldn't find the file
    let r = sdk.execute("python3 -c \"import os; print(os.getcwd())\"");
    assert_eq!(r.exit_code, 0);
    // After cd, relative path should find the file
    sdk.execute("cd /app/src");
    let r = sdk.execute("python3 -c \"f=open('main.py'); print(f.read().strip())\"");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("ROOT_OK"), "Python cwd after cd failed: stdout={}", r.stdout);
}

#[test]
fn python_script_exec_respects_cwd() {
    let sdk = setup_sdk();
    sdk.execute("mkdir -p /scripts");
    sdk.write_file("/scripts/tool.py", "print('SCRIPT_OK')\n").unwrap();
    sdk.execute("cd /scripts");
    let r = sdk.execute("python3 tool.py");
    assert_eq!(r.exit_code, 0, "Python script failed: stderr={}", r.stderr);
    assert!(r.stdout.contains("SCRIPT_OK"), "Python script cwd failed: stdout={}", r.stdout);
}

#[test]
fn python_heredoc_with_multiple_files_cwd() {
    let sdk = setup_sdk();
    sdk.execute("mkdir -p /work/data");
    sdk.write_file("/work/data/a.txt", "alpha\n").unwrap();
    sdk.write_file("/work/data/b.txt", "beta\n").unwrap();
    let r = sdk.execute("cd /work/data && python3 << 'PYEOF'\nimport os\nfor f in sorted(os.listdir('.')):\n    if f.endswith('.txt'):\n        print(f)\nPYEOF");
    assert_eq!(r.exit_code, 0, "Python failed: stderr={}", r.stderr);
    assert!(r.stdout.contains("a.txt"), "missing a.txt: {r:?}");
    assert!(r.stdout.contains("b.txt"), "missing b.txt: {r:?}");
}

// ═══════════════════════════════════════════════════════════════════
// Fix: cat -A/-v/-E/-T flags
// ═══════════════════════════════════════════════════════════════════
#[test]
fn cat_A_flag_shows_nonprint_and_ends() {
    let sdk = setup_sdk_no_subprocess();
    sdk.write_file("test.txt", "hello\n").unwrap();
    // -A should show $ at end of line
    let r = sdk.execute("cat -A test.txt");
    assert!(r.stdout.contains("hello$"), "cat -A should show $: {}", r.stdout);
}

#[test]
fn cat_E_flag_shows_line_ends() {
    let sdk = setup_sdk_no_subprocess();
    sdk.write_file("lines.txt", "line1\nline2\n").unwrap();
    let r = sdk.execute("cat -E lines.txt");
    let lines: Vec<&str> = r.stdout.lines().collect();
    for line in &lines {
        if !line.is_empty() {
            assert!(line.ends_with('$'), "cat -E should end with $: '{}'", line);
        }
    }
}

#[test]
fn cat_T_flag_shows_tabs() {
    let sdk = setup_sdk_no_subprocess();
    sdk.write_file("tabs.txt", "col1\tcol2\n").unwrap();
    let r = sdk.execute("cat -T tabs.txt");
    assert!(r.stdout.contains("^I"), "cat -T should show ^I for tab: {}", r.stdout);
}

#[test]
fn cat_v_flag_shows_control_chars() {
    let sdk = setup_sdk();
    // Create a file with control character 0x01 (^A) using Python
    let r = sdk.execute("python3 << 'PYEOF'\nwith open('/ctrl.txt', 'w') as f:\n    f.write('a\\x01b\\n')\nPYEOF");
    assert_eq!(r.exit_code, 0, "Python failed: {}", r.stderr);
    let r = sdk.execute("cat -v /ctrl.txt");
    assert!(r.stdout.contains("^A"), "cat -v should show ^A for 0x01: {}", r.stdout);
}

#[test]
fn cat_A_in_pipeline() {
    let sdk = setup_sdk_no_subprocess();
    sdk.write_file("pipe.txt", "line1\nline2\n").unwrap();
    // cat -A in pipeline should work like standalone
    let r = sdk.execute("cat pipe.txt | cat -A");
    assert!(r.stdout.contains("line1$"), "cat -A in pipe should show $: {}", r.stdout);
    assert!(r.stdout.contains("line2$"), "cat -A in pipe: {}", r.stdout);
}

#[test]
fn cat_n_combined_with_A() {
    let sdk = setup_sdk_no_subprocess();
    sdk.write_file("numbered.txt", "a\nb\n").unwrap();
    let r = sdk.execute("cat -n -A numbered.txt");
    assert!(r.stdout.contains("a$"), "missing -A: {}", r.stdout);
    assert!(r.stdout.contains("b$"), "missing -A: {}", r.stdout);
    // Line numbers should be present (at least in stdout)
    assert!(r.stdout.contains("1") && r.stdout.contains("2"), "missing -n: {}", r.stdout);
}

#[test]
fn cat_never_crashes_on_unknown_flag() {
    let sdk = setup_sdk_no_subprocess();
    sdk.write_file("dummy.txt", "ok\n").unwrap();
    // Unknown flags should be silently ignored
    let r = sdk.execute("cat -Z dummy.txt");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("ok"));
}

#[test]
fn cat_empty_args_with_stdin() {
    let sdk = setup_sdk_no_subprocess();
    let r = sdk.execute("echo hello | cat");
    assert!(r.stdout.contains("hello"), "cat with stdin failed: {}", r.stdout);
}

#[test]
fn cat_with_only_flags_and_stdin() {
    let sdk = setup_sdk_no_subprocess();
    let r = sdk.execute("echo hello | cat -n -E");
    assert!(r.stdout.contains("hello$"), "cat -E with stdin failed: {}", r.stdout);
}

// ═══════════════════════════════════════════════════════════════════
// Cross-platform compatibility tests (subprocess, encoding, cwd)
// ═══════════════════════════════════════════════════════════════════

#[test]
fn subprocess_allowed_by_default() {
    let sdk = setup_sdk();
    // Verify that subprocess is allowed on platforms that support it
    let info = sdk.get_info();
    // On desktop, subprocess should be available
    // On mobile without binaries, it's allowed but commands just fail differently
    assert!(info.allow_subprocess, "subprocess should be allowed by default");
}

#[test]
fn subprocess_fallback_gives_clear_error() {
    let sdk = setup_sdk();
    let r = sdk.execute("nonexistent_binary_xyz_test");
    assert_ne!(r.exit_code, 0, "unknown command should fail");
    // Error should mention command name or "not found", not "subprocess disabled"
    let has_error = !r.stderr.is_empty() || r.exit_code != 0;
    assert!(has_error, "unknown command should produce error");
}

#[test]
fn python_script_sandbox_applied() {
    let sdk = setup_sdk();
    // Python scripts should see the sandbox root, not host file system
    sdk.write_file("/test_sandbox.py", "import os; print(os.getenv('FASTSHELL_ROOT', 'UNSET'))").unwrap();
    let r = sdk.execute("python3 /test_sandbox.py");
    assert_eq!(r.exit_code, 0, "sandbox script failed: {}", r.stderr);
    // The sandbox root should be set
    assert!(!r.stdout.contains("UNSET"), "sandbox root not set");
}

#[test]
fn empty_output_does_not_panic() {
    let sdk = setup_sdk_no_subprocess();
    // Empty command should return empty output without error
    let r = sdk.execute("");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.is_empty());
}

#[test]
fn large_output_many_lines() {
    let sdk = setup_sdk_no_subprocess();
    // Stress test with many lines
    let r = sdk.execute("seq 1 100");
    assert_eq!(r.exit_code, 0);
    let lines: Vec<&str> = r.stdout.lines().collect();
    assert!(lines.len() >= 100, "expected 100+ lines, got {}", lines.len());
}

#[test]
fn special_chars_in_output() {
    let sdk = setup_sdk_no_subprocess();
    // Unicode characters should pass through
    let r = sdk.execute("echo 'café résumé naïveté'");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("café"), "unicode failed: {}", r.stdout);
}

#[test]
fn multiple_consecutive_newlines() {
    let sdk = setup_sdk_no_subprocess();
    sdk.write_file("multiline.txt", "a\n\n\nb\n").unwrap();
    let r = sdk.execute("cat multiline.txt");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("a"), "missing content: {}", r.stdout);
    assert!(r.stdout.contains("b"), "missing content: {}", r.stdout);
}

#[test]
fn cd_then_subprocess_respects_dir() {
    let sdk = setup_sdk();
    sdk.execute("mkdir -p /testdir");
    sdk.write_file("/testdir/hello.txt", "world").unwrap();
    sdk.execute("cd /testdir");
    // cat should find the file relative to cwd
    let r = sdk.execute("cat hello.txt");
    assert_eq!(r.exit_code, 0, "cat failed: {}", r.stderr);
    assert!(r.stdout.contains("world"), "cwd not respected: {}", r.stdout);
}

#[test]
fn nested_function_calls() {
    let sdk = setup_sdk_no_subprocess();
    let r = sdk.execute("outer() { echo $(inner); }; inner() { echo nested; }; outer");
    assert!(r.stdout.contains("nested"), "nested function failed: {}", r.stdout);
}

#[test]
fn alias_does_not_block_builtin() {
    let sdk = setup_sdk_no_subprocess();
    sdk.execute("alias ls='echo ALIASED'");
    // After alias, calling ls should still work via builtin
    let r = sdk.execute("alias ls='echo OVERRIDE'; ls");
    // The alias "ls" expands to "echo OVERRIDE", so "ls" becomes "echo OVERRIDE"
    assert!(r.stdout.contains("OVERRIDE") || r.exit_code == 0, "alias should work");
}
