// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! End-to-end tests for the Unix-compatibility strengthening pass:
//! combined short flags, previously-missing common options, and warning
//! visibility through `run_shell` stderr.

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn setup() -> Fastshell {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_compat_{}_{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dir);
    let mut sdk = Fastshell::new();
    sdk.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: true,
        allow_subprocess: true,
        network_ask_permission: false,
        command_timeout_ms: 30_000,
        ..Default::default()
    })
    .unwrap();
    sdk
}

// ── du: combined flags -sh, -a, -c ───────────────────────────────

#[test]
fn compat_du_sh_combined() {
    let sdk = setup();
    sdk.write_file("big.bin", "hello world hello world")
        .unwrap();
    let r = sdk.execute("du -sh .");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(
        r.stdout.contains('\t'),
        "expected a size\tpath line, got: {}",
        r.stdout
    );
    assert!(
        !r.stdout.contains("unsupported"),
        "du -sh should be recognized: {}",
        r.stdout
    );
}

#[test]
fn compat_du_all_and_total() {
    let sdk = setup();
    sdk.write_file("f.txt", "abc").unwrap();
    let r = sdk.execute("du -ac .");
    assert_eq!(r.exit_code, 0);
    assert!(
        r.stdout.contains("f.txt"),
        "-a should list files: {}",
        r.stdout
    );
    assert!(
        r.stdout.contains("total"),
        "-c should print a grand total: {}",
        r.stdout
    );
}

// ── seq: negative step operands ─────────────────────────────────

#[test]
fn compat_seq_negative_step() {
    let sdk = setup();
    let r = sdk.execute("seq 5 -1 1");
    assert_eq!(r.exit_code, 0);
    assert_eq!(
        r.stdout, "5\n4\n3\n2\n1\n",
        "seq countdown broken: {}",
        r.stdout
    );
}

// ── jq: file operand + common flags ─────────────────────────────

#[test]
fn compat_jq_reads_file() {
    let sdk = setup();
    sdk.write_file("data.json", "{\"name\":\"alice\",\"n\":7}\n")
        .unwrap();
    let r = sdk.execute("jq -r '.name' data.json");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert_eq!(r.stdout.trim(), "alice");
}

#[test]
fn compat_jq_slurp_and_null() {
    let sdk = setup();
    let r = sdk.execute("printf '1\n2\n' | jq -s '.'");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert_eq!(
        r.stdout.trim(),
        "[1,2]",
        "slurp should wrap into an array: {}",
        r.stdout
    );
    let r2 = sdk.execute("jq -n '.'");
    assert_eq!(r2.exit_code, 0);
    assert_eq!(
        r2.stdout.trim(),
        "null",
        "null-input should yield null: {}",
        r2.stdout
    );
}

#[test]
fn compat_jq_arg() {
    let sdk = setup();
    sdk.write_file("d.json", "{\"name\":\"alice\"}\n").unwrap();
    let r = sdk.execute("jq -r --arg k key '.name' d.json");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert_eq!(
        r.stdout.trim(),
        "alice",
        "--arg should be accepted alongside a normal filter: {}",
        r.stdout
    );
}

// ── sed: multiple -e expressions ────────────────────────────────

#[test]
fn compat_sed_multiple_expressions() {
    let sdk = setup();
    sdk.write_file("s.txt", "apple banana\n").unwrap();
    let r = sdk.execute("sed -e 's/apple/red/' -e 's/banana/yellow/' s.txt");
    assert_eq!(r.exit_code, 0);
    assert_eq!(
        r.stdout.trim(),
        "red yellow",
        "multiple -e should all apply: {}",
        r.stdout
    );
}

// ── cp / mv: combined flags, -n, -t, verbose → stdout ───────────

#[test]
fn compat_cp_recursive_force_combined() {
    let sdk = setup();
    sdk.execute("mkdir -p src/sub");
    sdk.write_file("src/a.txt", "x").unwrap();
    sdk.write_file("src/sub/b.txt", "y").unwrap();
    let r = sdk.execute("cp -rf src dst");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    let check = sdk.execute("cat dst/a.txt");
    assert_eq!(check.stdout.trim(), "x");
}

#[test]
fn compat_cp_no_clobber() {
    let sdk = setup();
    sdk.write_file("orig.txt", "original").unwrap();
    sdk.write_file("new.txt", "new").unwrap();
    sdk.execute("cp -n new.txt orig.txt");
    let r = sdk.execute("cat orig.txt");
    assert_eq!(
        r.stdout.trim(),
        "original",
        "-n must not overwrite: {}",
        r.stdout
    );
}

#[test]
fn compat_mv_target_dir() {
    let sdk = setup();
    sdk.execute("mkdir -p dest");
    sdk.write_file("m.txt", "hello").unwrap();
    let r = sdk.execute("mv -t dest m.txt");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert_eq!(sdk.execute("cat dest/m.txt").stdout.trim(), "hello");
}

// ── env: prefix assignments ─────────────────────────────────────

#[test]
fn compat_env_prefix_runs_command() {
    let sdk = setup();
    let r = sdk.execute("env GREETING=hi printenv GREETING");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert_eq!(
        r.stdout.trim(),
        "hi",
        "env VAR=x cmd should set the var for cmd: {}",
        r.stdout
    );
}

// ── touch: -d / -t / -c ─────────────────────────────────────────

#[test]
fn compat_touch_date() {
    let sdk = setup();
    sdk.write_file("t.txt", "x").unwrap();
    let r = sdk.execute("touch -d '2020-01-02 03:04:05' t.txt");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    let s = sdk.execute("stat -c %Y t.txt");
    assert!(
        s.stdout.trim().parse::<u64>().unwrap() > 1577000000,
        "mtime should be 2020+: {}",
        s.stdout
    );
}

#[test]
fn compat_touch_no_create() {
    let sdk = setup();
    let r = sdk.execute("touch -c nonexistent.txt");
    assert_eq!(r.exit_code, 0);
    let t = sdk.execute("test -e nonexistent.txt");
    assert_eq!(t.exit_code, 1, "-c must not create the file");
}

// ── wc: combined flags + -L ─────────────────────────────────────

#[test]
fn compat_wc_combined_and_longest() {
    let sdk = setup();
    sdk.write_file("w.txt", "a\nbbb\ncc\n").unwrap();
    let r = sdk.execute("wc -l w.txt");
    assert_eq!(r.exit_code, 0);
    assert!(
        r.stdout.trim().starts_with("3"),
        "wc -l should print line count: {}",
        r.stdout
    );
    let r2 = sdk.execute("wc -L w.txt");
    assert_eq!(
        r2.stdout.trim(),
        "3 w.txt",
        "wc -L should print longest line: {}",
        r2.stdout
    );
}

// ── grep: -m / -x / -L ──────────────────────────────────────────

#[test]
fn compat_grep_max_count() {
    let sdk = setup();
    sdk.write_file("g.txt", "a\nb\na\nb\n").unwrap();
    let r = sdk.execute("grep -m 2 a g.txt");
    assert_eq!(r.exit_code, 0);
    assert_eq!(
        r.stdout.matches('\n').count(),
        2,
        "grep -m 2 should stop after 2: {}",
        r.stdout
    );
}

#[test]
fn compat_grep_line_regexp() {
    let sdk = setup();
    sdk.write_file("g2.txt", "cat\nconcatenate\ncat\n").unwrap();
    let r = sdk.execute("grep -cx cat g2.txt");
    assert_eq!(r.exit_code, 0);
    assert_eq!(
        r.stdout.trim(),
        "2",
        "-x should match whole lines only: {}",
        r.stdout
    );
}

#[test]
fn compat_grep_files_without_match() {
    let sdk = setup();
    sdk.write_file("has.txt", "needle\n").unwrap();
    sdk.write_file("lacks.txt", "nothing here\n").unwrap();
    let r = sdk.execute("grep -L needle has.txt lacks.txt");
    assert_eq!(
        r.stdout.trim(),
        "lacks.txt",
        "-L should list the non-matching file: {}",
        r.stdout
    );
}

// ── sort: combined flags + -o ───────────────────────────────────

#[test]
fn compat_sort_numeric_reverse_combined() {
    let sdk = setup();
    let r = sdk.execute("printf '10\\n2\\n1\\n' | sort -nr");
    assert_eq!(r.stdout, "10\n2\n1\n", "sort -nr: {}", r.stdout);
}

#[test]
fn compat_sort_output_file() {
    let sdk = setup();
    sdk.execute("printf 'b\\na\\n' | sort -o out.txt");
    assert_eq!(sdk.execute("cat out.txt").stdout, "a\nb\n");
}

// ── cut: -s + --output-delimiter ────────────────────────────────

#[test]
fn compat_cut_only_delimited_and_output_delim() {
    let sdk = setup();
    let r = sdk.execute("printf 'a:b:c\\nno-delim\\n' | cut -d: -f1,2 -s --output-delimiter=, ");
    assert_eq!(r.stdout, "a,b\n", "cut -s --output-delimiter: {}", r.stdout);
}

// ── tr: combined flags -ds ──────────────────────────────────────

#[test]
fn compat_tr_delete_squeeze_combined() {
    let sdk = setup();
    let r = sdk.execute("printf 'aaabbbccc\\n' | tr -ds 'b' ' '");
    assert_eq!(r.stdout, "aaaccc\n", "tr -ds: {}", r.stdout);
}

// ── uniq: combined flags ────────────────────────────────────────

#[test]
fn compat_uniq_count_combined() {
    let sdk = setup();
    let r = sdk.execute("printf 'a\\na\\nb\\n' | uniq -c");
    assert!(r.stdout.contains("2 a"), "uniq -c: {}", r.stdout);
    assert!(r.stdout.contains("1 b"), "uniq -c: {}", r.stdout);
}

// ── ps: -eo format ──────────────────────────────────────────────

#[test]
fn compat_ps_eo_format() {
    let sdk = setup();
    let r = sdk.execute("ps -eo pid,comm");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(
        !r.stdout.contains("unsupported"),
        "ps -eo should be recognized: {}",
        r.stdout
    );
}

// ── find: -regex / -newer ───────────────────────────────────────

#[test]
fn compat_find_regex() {
    let sdk = setup();
    sdk.write_file("match_me.txt", "").unwrap();
    sdk.write_file("other.log", "").unwrap();
    let r = sdk.execute("find . -regex '.*match_me.*'");
    assert_eq!(r.exit_code, 0);
    assert!(
        r.stdout.contains("match_me.txt"),
        "find -regex: {}",
        r.stdout
    );
    assert!(
        !r.stdout.contains("other.log"),
        "find -regex over-matched: {}",
        r.stdout
    );
}

// ── diff: combined -ru ──────────────────────────────────────────

#[test]
fn compat_diff_recursive_unified_combined() {
    let sdk = setup();
    sdk.execute("mkdir -p a b");
    sdk.write_file("a/x.txt", "one\n").unwrap();
    sdk.write_file("b/x.txt", "two\n").unwrap();
    let r = sdk.execute("diff -ru a b");
    assert_eq!(r.exit_code, 1, "files differ → exit 1: {}", r.stdout);
    assert!(
        r.stdout.contains("-one") && r.stdout.contains("+two"),
        "diff -ru: {}",
        r.stdout
    );
}

// ── mkdir: -pv + -m ─────────────────────────────────────────────

#[test]
fn compat_mkdir_parents_verbose() {
    let sdk = setup();
    let r = sdk.execute("mkdir -pv x/y/z");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(
        r.stdout.contains("x/y/z"),
        "mkdir -v should print: {}",
        r.stdout
    );
    assert!(sdk.execute("test -d x/y/z").exit_code == 0);
}

// ── df: -h ──────────────────────────────────────────────────────

#[test]
fn compat_df_human() {
    let sdk = setup();
    let r = sdk.execute("df -h");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("Filesystem"), "df header: {}", r.stdout);
}

// ── stat: -f ────────────────────────────────────────────────────

#[test]
fn compat_stat_filesystem() {
    let sdk = setup();
    sdk.write_file("st.txt", "x").unwrap();
    let r = sdk.execute("stat -f st.txt");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("Block size"), "stat -f: {}", r.stdout);
}

// ── base64: -i ignore garbage ───────────────────────────────────

#[test]
fn compat_base64_ignore_garbage() {
    let sdk = setup();
    let r = sdk.execute("printf 'aGVsbG8=' | base64 -di");
    assert_eq!(r.stdout, "hello", "base64 -d: {}", r.stdout);
}

// ── date: -R / -I / --date ──────────────────────────────────────

#[test]
fn compat_date_formats() {
    let sdk = setup();
    let iso = sdk.execute("date -I");
    assert!(iso.stdout.contains('-'), "date -I: {}", iso.stdout);
    let rfc = sdk.execute("date -R");
    assert!(rfc.stdout.contains(':'), "date -R: {}", rfc.stdout);
    let d = sdk.execute("date -d '2020-01-02' +%Y-%m-%d");
    assert_eq!(d.stdout.trim(), "2020-01-02", "date -d: {}", d.stdout);
}

// ── tar: -j / -J / -a ───────────────────────────────────────────

#[test]
fn compat_tar_bzip2_and_auto() {
    let sdk = setup();
    sdk.write_file("payload.txt", "content").unwrap();
    let r = sdk.execute("tar -cjf arch.tar.bz2 payload.txt");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(sdk
        .execute("ls arch.tar.bz2")
        .stdout
        .contains("arch.tar.bz2"));
    // -a auto-detects bzip2 by extension on extract.
    sdk.execute("rm payload.txt");
    let x = sdk.execute("tar -xaf arch.tar.bz2");
    assert_eq!(x.exit_code, 0, "stderr={}", x.stderr);
    assert_eq!(sdk.execute("cat payload.txt").stdout.trim(), "content");
}

// ── paste: -s ───────────────────────────────────────────────────

#[test]
fn compat_paste_serial() {
    let sdk = setup();
    let r = sdk.execute("printf 'a\\nb\\n' | paste -sd,");
    assert_eq!(r.stdout, "a,b\n", "paste -sd,: {}", r.stdout);
}

// ── xargs: -r accepted ──────────────────────────────────────────

#[test]
fn compat_xargs_no_run_if_empty() {
    let sdk = setup();
    let r = sdk.execute("printf '' | xargs -r echo");
    assert_eq!(r.exit_code, 0);
    assert_eq!(
        r.stdout.trim(),
        "",
        "xargs -r with empty input should not run: {}",
        r.stdout
    );
}

// ── warning visibility through stderr ───────────────────────────

#[test]
fn compat_unsupported_option_warning_visible() {
    let sdk = setup();
    let r = sdk.execute("wc --totally-bogus-flag somefile 2>/dev/null");
    // The warning is routed into CommandOutput.stderr (before 2>/dev/null drops it).
    let _ = r;
    let r2 = sdk.execute("sort --definitely-not-a-flag");
    assert!(
        r2.stderr.contains("unsupported") || r2.stderr.contains("unknown"),
        "warning must appear in stderr, got: {:?}",
        r2.stderr
    );
}

// ── timeout: options accepted ───────────────────────────────────

#[test]
fn compat_timeout_with_signal_option() {
    let sdk = setup();
    let r = sdk.execute("timeout -s TERM 1 sleep 2");
    assert_eq!(
        r.exit_code, 124,
        "timeout should kill after 1s: {}",
        r.stdout
    );
}

// ── for/while: `do` followed by newline (multi-line POSIX form) ──

#[test]
fn compat_for_loop_multiline() {
    let sdk = setup();
    let r = sdk.execute("for u in a b c; do\n  echo \"item=$u\"\ndone");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("item=a"), "stdout={}", r.stdout);
    assert!(r.stdout.contains("item=b"), "stdout={}", r.stdout);
    assert!(r.stdout.contains("item=c"), "stdout={}", r.stdout);
}

#[test]
fn compat_for_loop_do_on_own_line() {
    let sdk = setup();
    let r = sdk.execute("for u in x y\ndo\n  echo \"v=$u\"\ndone");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("v=x"), "stdout={}", r.stdout);
    assert!(r.stdout.contains("v=y"), "stdout={}", r.stdout);
}

// ── echo: `---` / `--` are literal, not options ─────────────────

#[test]
fn compat_echo_dashes_literal() {
    let sdk = setup();
    assert_eq!(sdk.execute("echo ---").stdout.trim(), "---");
    assert_eq!(sdk.execute("echo -- -n").stdout.trim(), "-n");
    // -n is still honoured when it is the first option.
    assert_eq!(sdk.execute("echo -n hi").stdout, "hi");
}

// ── grep: combined flags incl. -P (perl-regexp) ─────────────────

#[test]
fn compat_grep_combined_op() {
    let sdk = setup();
    sdk.write_file("g.txt", "abc123\ndef\n").unwrap();
    let r = sdk.execute("grep -oP '[0-9]+' g.txt");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("123"), "stdout={}", r.stdout);
    assert!(!r.stderr.contains("unsupported"), "stderr={}", r.stderr);
}

// ── ls -d / type / help ─────────────────────────────────────────

#[test]
fn compat_ls_d_lists_dir_itself() {
    let sdk = setup();
    sdk.execute("mkdir -p d/sub");
    sdk.write_file("d/f.txt", "x").unwrap();
    let r = sdk.execute("ls -d d");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert_eq!(r.stdout.trim(), "d");
    let r = sdk.execute("ls -ld d");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(
        r.stdout.trim_start().starts_with('d'),
        "stdout={}",
        r.stdout
    );
    assert!(!r.stdout.contains("f.txt"), "stdout={}", r.stdout);
}

#[test]
fn compat_type_reports_builtin_and_missing() {
    let sdk = setup();
    let r = sdk.execute("type ls");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("built-in"), "stdout={}", r.stdout);
    let r = sdk.execute("type definitely_not_a_command_xyz");
    assert_ne!(r.exit_code, 0);
    assert!(r.stderr.contains("not found"), "stderr={}", r.stderr);
}

#[test]
fn compat_help_lists_builtins() {
    let sdk = setup();
    let r = sdk.execute("help");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("ls"), "stdout={}", r.stdout);
    let r = sdk.execute("help ls");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(!r.stdout.is_empty());
}

#[test]
fn compat_head_tail_chmod_help() {
    let sdk = setup();
    assert!(sdk.execute("head --help").stdout.contains("Usage: head"));
    assert!(sdk.execute("tail --help").stdout.contains("Usage: tail"));
    assert!(sdk.execute("chmod --help").stdout.contains("Usage: chmod"));
    // help text now matches capability
    assert!(sdk.execute("ls --help").stdout.contains("-d"));
    assert!(sdk.execute("find --help").stdout.contains("-iname"));
}

// ── xargs -I{} replace ──────────────────────────────────────────

#[test]
fn compat_xargs_replace() {
    let sdk = setup();
    sdk.write_file("urls.txt", "a\nb\n").unwrap();
    let r = sdk.execute("cat urls.txt | xargs -n1 -I{} echo got-{}");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("got-a"), "stdout={}", r.stdout);
    assert!(r.stdout.contains("got-b"), "stdout={}", r.stdout);
}

#[test]
fn compat_xargs_replace_standalone() {
    let sdk = setup();
    let r = sdk.execute("printf 'x\\ny\\n' | xargs -I{} echo [{}]");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("[x]"), "stdout={}", r.stdout);
    assert!(r.stdout.contains("[y]"), "stdout={}", r.stdout);
}

#[test]
fn compat_brace_empty_and_single_are_literal() {
    let sdk = setup();
    assert_eq!(sdk.execute("echo {}").stdout.trim(), "{}");
    assert_eq!(sdk.execute("echo {a}").stdout.trim(), "{a}");
    // Real expansion still happens (fastshell expands the whole line).
    let r = sdk.execute("echo {a,b}");
    assert!(
        r.stdout.contains("a") && r.stdout.contains("b"),
        "stdout={}",
        r.stdout
    );
}

// ── pipe into `while read` (common idiom) ───────────────────────

#[test]
fn compat_pipe_into_while_read() {
    let sdk = setup();
    sdk.write_file("urls.txt", "a\nb\nc\n").unwrap();
    let r = sdk.execute("cat urls.txt | while read u; do echo \"got $u\"; done");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("got a"), "stdout={}", r.stdout);
    assert!(r.stdout.contains("got c"), "stdout={}", r.stdout);
}

#[test]
fn compat_pipe_into_while_read_multiline() {
    let sdk = setup();
    sdk.write_file("urls.txt", "1\n2\n").unwrap();
    let r = sdk.execute("cat urls.txt | while read n; do\n  echo \"n=$n\"\ndone");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("n=1"), "stdout={}", r.stdout);
    assert!(r.stdout.contains("n=2"), "stdout={}", r.stdout);
}

#[test]
fn compat_for_with_nested_if() {
    let sdk = setup();
    sdk.write_file("d.txt", "alpha\nbeta\n").unwrap();
    let r = sdk.execute(
        "for x in 1 2; do\n  t=$(cat d.txt | head -1)\n  if [ -n \"$t\" ]; then\n    echo \"got $t\"\n  else\n    echo empty\n  fi\ndone",
    );
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("got alpha"), "stdout={}", r.stdout);
    assert!(!r.stdout.contains("empty"), "stdout={}", r.stdout);
}

#[test]
fn compat_if_nested_else() {
    let sdk = setup();
    let r = sdk.execute(
        "for x in 1 2; do\n  t=$(echo alpha | head -1)\n  if [ -n \"$t\" ]; then\n    if [ \"$t\" != \"0\" ] && [ -n \"$t\" ]; then\n      d=\"ok\"\n    else\n      d=\"?\"\n    fi\n    echo \"x=$x t=$t d=$d\"\n  else\n    echo empty\n  fi\ndone",
    );
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("x=1 t=alpha d=ok"), "stdout={}", r.stdout);
    assert!(r.stdout.contains("x=2 t=alpha d=ok"), "stdout={}", r.stdout);
    assert!(!r.stdout.contains("empty"), "stdout={}", r.stdout);
}

#[test]
fn compat_assignment_preserves_spaces() {
    let sdk = setup();
    let r = sdk.execute("t=$(echo \"hello world\")\necho \"[$t]\"");
    assert!(r.stdout.contains("[hello world]"), "stdout={}", r.stdout);
    // `X=v cmd` (assignment prefix) still runs the command.
    let r = sdk.execute("X=1 echo done");
    assert!(r.stdout.contains("done"), "stdout={}", r.stdout);
}

#[test]
fn compat_comment_lines_ignored() {
    let sdk = setup();
    // full-line comment at start + a comment containing an apostrophe
    let r = sdk.execute("# a comment\n# it's got a quote\nls > /dev/null; echo ok");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("ok"), "stdout={}", r.stdout);
    assert!(
        !r.stderr.contains("command not found"),
        "stderr={}",
        r.stderr
    );
    // a lone comment is a no-op
    let r = sdk.execute("# just a comment");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.is_empty(), "stdout={}", r.stdout);
}

#[test]
fn compat_grep_context_attached() {
    let sdk = setup();
    sdk.write_file("g.txt", "a\nb\nc\nd\n").unwrap();
    let r = sdk.execute("grep -A1 b g.txt");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(
        r.stdout.contains('b') && r.stdout.contains('c'),
        "stdout={}",
        r.stdout
    );
    assert!(!r.stderr.contains("unsupported"), "stderr={}", r.stderr);
    let r = sdk.execute("grep -B1 -A1 c g.txt");
    assert!(!r.stderr.contains("unsupported"), "stderr={}", r.stderr);
}

#[test]
fn compat_glob_expansion() {
    let sdk = setup();
    sdk.write_file("a.jpg", "x").unwrap();
    sdk.write_file("b.jpg", "y").unwrap();
    sdk.write_file("c.txt", "z").unwrap();
    let r = sdk.execute("ls *.jpg");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(
        r.stdout.contains("a.jpg") && r.stdout.contains("b.jpg"),
        "stdout={}",
        r.stdout
    );
    assert!(!r.stdout.contains("c.txt"), "stdout={}", r.stdout);
}

#[test]
fn compat_tmp_persists() {
    let sdk = setup();
    sdk.execute("echo hi > /tmp/fs_t.txt");
    let r = sdk.execute("cat /tmp/fs_t.txt");
    assert!(
        r.stdout.contains("hi"),
        "stdout={} stderr={}",
        r.stdout,
        r.stderr
    );
}

#[test]
fn compat_for_nested_if_date_printf() {
    let sdk = setup();
    sdk.write_file(
        "r.json",
        "{\"ctime\":1788763808,\"title\":\"hello world\"}\n",
    )
    .unwrap();
    let r = sdk.execute(
        "for lid in 1 2; do\n  title=$(grep -o '\"title\":\"[^\"]*\"' r.json | head -1 | sed 's/\"title\":\"//;s/\"$//')\n  ct=$(grep -o '\"ctime\":[0-9]*' r.json | head -1 | sed 's/\"ctime\"://')\n  if [ -n \"$title\" ]; then\n    if [ -n \"$ct\" ] && [ \"$ct\" != \"0\" ]; then\n      dt=$(date -r \"$ct\" \"+%Y-%m-%d\" 2>/dev/null)\n    else\n      dt=\"?\"\n    fi\n    printf \"lid=%-6s [%s] %s\\n\" \"$lid\" \"$dt\" \"${title:0:60}\"\n  else\n    echo \"lid=$lid (empty or fail)\"\n  fi\ndone",
    );
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("hello world"), "stdout={}", r.stdout);
    assert!(!r.stdout.contains("empty or fail"), "stdout={}", r.stdout);
}

#[test]
fn compat_semicolon_inside_cmd_subst_quotes() {
    let sdk = setup();
    let r = sdk.execute("t=$(echo abc | sed 's/a/x/;s/b/y/')\necho \"t=$t\"");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("t=xyc"), "stdout={}", r.stdout);
}

#[test]
fn compat_jq_length_and_interpolation() {
    let sdk = setup();
    sdk.write_file("d.json", "{\"a\":[1,2,3],\"t\":\"hi\"}\n")
        .unwrap();
    let r = sdk.execute("jq '.a | length' d.json");
    assert_eq!(r.stdout.trim(), "3", "stderr={}", r.stderr);
    let r = sdk.execute("jq -r '\"\\(.t)-\\(.a|length)\"' d.json");
    assert_eq!(r.stdout.trim(), "hi-3", "stderr={}", r.stderr);
    let r = sdk.execute("jq 'keys' d.json");
    assert!(r.stdout.contains("\"a\""), "stdout={}", r.stdout);
}

#[test]
fn compat_date_epoch_forms() {
    let sdk = setup();
    let r = sdk.execute("date -r 0 -u +%Y-%m-%d");
    assert_eq!(r.stdout.trim(), "1970-01-01", "stderr={}", r.stderr);
    let r = sdk.execute("date -d @0 -u +%Y-%m-%d");
    assert_eq!(r.stdout.trim(), "1970-01-01", "stderr={}", r.stderr);
}

#[test]
fn compat_curl_binary_download_to_file() {
    use std::io::{Read, Write};
    // Serve a small non-UTF8 payload; `curl -o` must write it byte-for-byte.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 2048];
            let _ = stream.read(&mut buf);
            let body: [u8; 10] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0xFF];
            let mut resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .into_bytes();
            resp.extend_from_slice(&body);
            let _ = stream.write_all(&resp);
            let _ = stream.flush();
        }
    });

    let dir = std::env::temp_dir().join(format!("fs_curl_bin_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let mut sdk = Fastshell::new();
    sdk.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: false,
        allow_subprocess: false,
        network_ask_permission: false,
        command_timeout_ms: 30_000,
        ..Default::default()
    })
    .unwrap();
    let r = sdk.execute(&format!(
        "curl -s -o out.png http://127.0.0.1:{port}/pic.png"
    ));
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    handle.join().unwrap();
    let bytes = fs::read(dir.join("out.png")).unwrap();
    assert_eq!(
        bytes,
        vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0xFF],
        "binary body must not be UTF-8 mangled"
    );
    let _ = fs::remove_dir_all(&dir);
}
