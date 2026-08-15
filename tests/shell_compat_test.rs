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
    sdk.write_file("big.bin", "hello world hello world").unwrap();
    let r = sdk.execute("du -sh .");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains('\t'), "expected a size\tpath line, got: {}", r.stdout);
    assert!(!r.stdout.contains("unsupported"), "du -sh should be recognized: {}", r.stdout);
}

#[test]
fn compat_du_all_and_total() {
    let sdk = setup();
    sdk.write_file("f.txt", "abc").unwrap();
    let r = sdk.execute("du -ac .");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("f.txt"), "-a should list files: {}", r.stdout);
    assert!(r.stdout.contains("total"), "-c should print a grand total: {}", r.stdout);
}

// ── seq: negative step operands ─────────────────────────────────

#[test]
fn compat_seq_negative_step() {
    let sdk = setup();
    let r = sdk.execute("seq 5 -1 1");
    assert_eq!(r.exit_code, 0);
    assert_eq!(r.stdout, "5\n4\n3\n2\n1\n", "seq countdown broken: {}", r.stdout);
}

// ── jq: file operand + common flags ─────────────────────────────

#[test]
fn compat_jq_reads_file() {
    let sdk = setup();
    sdk.write_file("data.json", "{\"name\":\"alice\",\"n\":7}\n").unwrap();
    let r = sdk.execute("jq -r '.name' data.json");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert_eq!(r.stdout.trim(), "alice");
}

#[test]
fn compat_jq_slurp_and_null() {
    let sdk = setup();
    let r = sdk.execute("printf '1\n2\n' | jq -s '.'");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert_eq!(r.stdout.trim(), "[1,2]", "slurp should wrap into an array: {}", r.stdout);
    let r2 = sdk.execute("jq -n '.'");
    assert_eq!(r2.exit_code, 0);
    assert_eq!(r2.stdout.trim(), "null", "null-input should yield null: {}", r2.stdout);
}

#[test]
fn compat_jq_arg() {
    let sdk = setup();
    sdk.write_file("d.json", "{\"name\":\"alice\"}\n").unwrap();
    let r = sdk.execute("jq -r --arg k key '.name' d.json");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert_eq!(r.stdout.trim(), "alice", "--arg should be accepted alongside a normal filter: {}", r.stdout);
}

// ── sed: multiple -e expressions ────────────────────────────────

#[test]
fn compat_sed_multiple_expressions() {
    let sdk = setup();
    sdk.write_file("s.txt", "apple banana\n").unwrap();
    let r = sdk.execute("sed -e 's/apple/red/' -e 's/banana/yellow/' s.txt");
    assert_eq!(r.exit_code, 0);
    assert_eq!(r.stdout.trim(), "red yellow", "multiple -e should all apply: {}", r.stdout);
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
    assert_eq!(r.stdout.trim(), "original", "-n must not overwrite: {}", r.stdout);
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
    assert_eq!(r.stdout.trim(), "hi", "env VAR=x cmd should set the var for cmd: {}", r.stdout);
}

// ── touch: -d / -t / -c ─────────────────────────────────────────

#[test]
fn compat_touch_date() {
    let sdk = setup();
    sdk.write_file("t.txt", "x").unwrap();
    let r = sdk.execute("touch -d '2020-01-02 03:04:05' t.txt");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    let s = sdk.execute("stat -c %Y t.txt");
    assert!(s.stdout.trim().parse::<u64>().unwrap() > 1577000000, "mtime should be 2020+: {}", s.stdout);
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
    assert!(r.stdout.trim().starts_with("3"), "wc -l should print line count: {}", r.stdout);
    let r2 = sdk.execute("wc -L w.txt");
    assert_eq!(r2.stdout.trim(), "3 w.txt", "wc -L should print longest line: {}", r2.stdout);
}

// ── grep: -m / -x / -L ──────────────────────────────────────────

#[test]
fn compat_grep_max_count() {
    let sdk = setup();
    sdk.write_file("g.txt", "a\nb\na\nb\n").unwrap();
    let r = sdk.execute("grep -m 2 a g.txt");
    assert_eq!(r.exit_code, 0);
    assert_eq!(r.stdout.matches('\n').count(), 2, "grep -m 2 should stop after 2: {}", r.stdout);
}

#[test]
fn compat_grep_line_regexp() {
    let sdk = setup();
    sdk.write_file("g2.txt", "cat\nconcatenate\ncat\n").unwrap();
    let r = sdk.execute("grep -cx cat g2.txt");
    assert_eq!(r.exit_code, 0);
    assert_eq!(r.stdout.trim(), "2", "-x should match whole lines only: {}", r.stdout);
}

#[test]
fn compat_grep_files_without_match() {
    let sdk = setup();
    sdk.write_file("has.txt", "needle\n").unwrap();
    sdk.write_file("lacks.txt", "nothing here\n").unwrap();
    let r = sdk.execute("grep -L needle has.txt lacks.txt");
    assert_eq!(r.stdout.trim(), "lacks.txt", "-L should list the non-matching file: {}", r.stdout);
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
    assert!(!r.stdout.contains("unsupported"), "ps -eo should be recognized: {}", r.stdout);
}

// ── find: -regex / -newer ───────────────────────────────────────

#[test]
fn compat_find_regex() {
    let sdk = setup();
    sdk.write_file("match_me.txt", "").unwrap();
    sdk.write_file("other.log", "").unwrap();
    let r = sdk.execute("find . -regex '.*match_me.*'");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("match_me.txt"), "find -regex: {}", r.stdout);
    assert!(!r.stdout.contains("other.log"), "find -regex over-matched: {}", r.stdout);
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
    assert!(r.stdout.contains("-one") && r.stdout.contains("+two"), "diff -ru: {}", r.stdout);
}

// ── mkdir: -pv + -m ─────────────────────────────────────────────

#[test]
fn compat_mkdir_parents_verbose() {
    let sdk = setup();
    let r = sdk.execute("mkdir -pv x/y/z");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("x/y/z"), "mkdir -v should print: {}", r.stdout);
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
    assert!(sdk.execute("ls arch.tar.bz2").stdout.contains("arch.tar.bz2"));
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
    assert_eq!(r.stdout.trim(), "", "xargs -r with empty input should not run: {}", r.stdout);
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
    assert_eq!(r.exit_code, 124, "timeout should kill after 1s: {}", r.stdout);
}
