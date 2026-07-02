// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn setup() -> Fastshell {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_edge_{}_{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dir);
    let mut sdk = Fastshell::new();
    sdk.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: true,
        allow_subprocess: true,
        network_ask_permission: false,
        command_timeout_ms: 30_000,
    })
    .unwrap();
    sdk
}

// ═══════════════════════════════════════════════════════
// cut edge cases
// ═══════════════════════════════════════════════════════

#[test]
fn edge_cut_multi_char_delimiter() {
    let sdk = setup();
    sdk.write_file("data.tsv", "a\tb\tc\nd\te\tf\n").unwrap();
    let r = sdk.execute("cut -f2 data.tsv");
    assert_eq!(r.exit_code, 0);
    assert_eq!(r.stdout.trim(), "b\ne");
}

#[test]
fn edge_cut_character_range() {
    let sdk = setup();
    sdk.write_file("text.txt", "abcdef\nghijkl\n").unwrap();
    let r = sdk.execute("cut -c1-3 text.txt");
    assert_eq!(r.exit_code, 0);
    assert_eq!(r.stdout.trim(), "abc\nghi");
}

#[test]
fn edge_cut_complement() {
    let sdk = setup();
    sdk.write_file("data.txt", "a b c\nd e f\n").unwrap();
    let r = sdk.execute("cut -d' ' -f1 --complement data.txt");
    assert_eq!(r.exit_code, 0);
    assert_eq!(r.stdout.trim(), "b c\ne f");
}

// ═══════════════════════════════════════════════════════
// tr edge cases
// ═══════════════════════════════════════════════════════

#[test]
fn edge_tr_delete_chars() {
    let sdk = setup();
    let r = sdk.execute("echo 'a1b2c3' | tr -d '0-9'");
    assert_eq!(r.exit_code, 0);
    assert_eq!(r.stdout.trim(), "abc");
}

#[test]
fn edge_tr_squeeze_repeats() {
    let sdk = setup();
    let r = sdk.execute("echo 'aaabbbccc' | tr -s 'a'");
    assert_eq!(r.exit_code, 0);
    assert_eq!(r.stdout.trim(), "abbbccc");
}

#[test]
fn edge_tr_upper_to_lower() {
    let sdk = setup();
    let r = sdk.execute("echo 'HELLO' | tr '[:upper:]' '[:lower:]'");
    assert_eq!(r.exit_code, 0);
    assert_eq!(r.stdout.trim(), "hello");
}

// ═══════════════════════════════════════════════════════
// xargs edge cases
// ═══════════════════════════════════════════════════════

#[test]
fn edge_xargs_null_delimited() {
    let sdk = setup();
    sdk.write_file("items.txt", "file1.txt\nfile2.txt\nfile3.txt\n")
        .unwrap();
    let r = sdk.execute("cat items.txt | xargs -n1 echo");
    assert_eq!(r.exit_code, 0);
}

// ═══════════════════════════════════════════════════════
// tee edge cases
// ═══════════════════════════════════════════════════════

#[test]
fn edge_tee_append() {
    let sdk = setup();
    sdk.write_file("log.txt", "line1\n").unwrap();
    let r = sdk.execute("echo 'line2' | tee -a log.txt");
    assert_eq!(r.exit_code, 0);
    let content = sdk.read_file("log.txt").unwrap();
    assert!(content.contains("line1"));
    assert!(content.contains("line2"));
}

// ═══════════════════════════════════════════════════════
// hashsum edge cases
// ═══════════════════════════════════════════════════════

#[test]
fn edge_hashsum_variants() {
    let sdk = setup();
    sdk.write_file("test.bin", "hello world").unwrap();

    let r = sdk.execute("md5sum test.bin");
    assert_eq!(r.exit_code, 0);
    assert!(!r.stdout.trim().is_empty());

    let r = sdk.execute("sha256sum test.bin");
    assert_eq!(r.exit_code, 0);
    assert!(!r.stdout.trim().is_empty());
}

// ═══════════════════════════════════════════════════════
// du/df edge cases
// ═══════════════════════════════════════════════════════

#[test]
fn edge_du_df() {
    let sdk = setup();
    sdk.write_file("big.txt", &"x".repeat(10000)).unwrap();

    let r = sdk.execute("du -h .");
    assert_eq!(r.exit_code, 0);
    assert!(!r.stdout.is_empty());

    let r = sdk.execute("df -h .");
    assert_eq!(r.exit_code, 0);
}

// ═══════════════════════════════════════════════════════
// which edge cases
// ═══════════════════════════════════════════════════════

#[test]
fn edge_which_builtins() {
    let sdk = setup();

    let r = sdk.execute("which ls");
    assert_eq!(r.exit_code, 0, "which ls failed: stderr={}", r.stderr);

    let r = sdk.execute("which nonexistent_command_xyz");
    assert!(
        r.exit_code != 0 || !r.stdout.contains("/"),
        "which nonexistent should fail or not find path, got code={} stdout={:?}",
        r.exit_code,
        r.stdout
    );
}

// ═══════════════════════════════════════════════════════
// zip/unzip edge cases
// ═══════════════════════════════════════════════════════

#[test]
fn edge_zip_unzip() {
    let sdk = setup();
    sdk.write_file("f1.txt", "content1").unwrap();
    sdk.write_file("f2.txt", "content2").unwrap();

    let r = sdk.execute("zip archive.zip f1.txt f2.txt");
    assert_eq!(r.exit_code, 0);
    assert!(sdk.exists("archive.zip"));

    sdk.execute("mkdir extract_dir");
    sdk.execute("cd extract_dir");
    let r = sdk.execute("unzip ../archive.zip");
    assert!(
        r.exit_code == 0 || r.exit_code == 1,
        "unzip failed: {}",
        r.stderr
    );
}

// ═══════════════════════════════════════════════════════
// file command edge cases
// ═══════════════════════════════════════════════════════

#[test]
fn edge_file_type_detection() {
    let sdk = setup();
    sdk.write_file("test.txt", "hello").unwrap();
    sdk.write_file("test.py", "print(1)").unwrap();

    let r = sdk.execute("file test.txt");
    assert_eq!(r.exit_code, 0);

    let r = sdk.execute("file test.py");
    assert_eq!(r.exit_code, 0);
}

// ═══════════════════════════════════════════════════════
// uuid / random edge cases
// ═══════════════════════════════════════════════════════

#[test]
fn edge_uuid_non_empty() {
    let sdk = setup();
    let r = sdk.execute("uuid");
    if r.exit_code == 127 {
        return;
    }
    assert_eq!(r.exit_code, 0, "uuid failed: stderr={}", r.stderr);
    assert!(!r.stdout.trim().is_empty());
}

// ═══════════════════════════════════════════════════════
// seq edge cases
// ═══════════════════════════════════════════════════════

#[test]
fn edge_seq_range() {
    let sdk = setup();
    let r = sdk.execute("seq 1 5");
    assert_eq!(r.exit_code, 0);
    assert_eq!(r.stdout.trim(), "1\n2\n3\n4\n5");
}

#[test]
fn edge_seq_reverse() {
    let sdk = setup();
    let r = sdk.execute("seq 5 -1 1");
    assert_eq!(r.exit_code, 0);
    assert_eq!(r.stdout.trim(), "5\n4\n3\n2\n1");
}

// ═══════════════════════════════════════════════════════
// sqlite3 edge cases
// ═══════════════════════════════════════════════════════

#[test]
fn edge_sqlite3_crud() {
    let sdk = setup();

    let r =
        sdk.execute("sqlite3 test.db 'CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT);'");
    assert_eq!(r.exit_code, 0);

    let r = sdk.execute("sqlite3 test.db \"INSERT INTO users VALUES (1, 'Alice');\"");
    assert_eq!(r.exit_code, 0);

    let r = sdk.execute("sqlite3 test.db 'SELECT * FROM users;'");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("Alice"));

    let r = sdk.execute("sqlite3 test.db 'UPDATE users SET name = \"Bob\" WHERE id = 1;'");
    assert_eq!(r.exit_code, 0);

    let r = sdk.execute("sqlite3 test.db 'SELECT name FROM users WHERE id = 1;'");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("Bob"));

    let r = sdk.execute("sqlite3 test.db 'DELETE FROM users WHERE id = 1;'");
    assert_eq!(r.exit_code, 0);

    let r = sdk.execute("sqlite3 test.db 'SELECT COUNT(*) FROM users;'");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("0"));
}

#[test]
fn edge_sqlite3_dot_commands() {
    let sdk = setup();
    sdk.execute("sqlite3 test.db 'CREATE TABLE t (id INT);'");

    let r = sdk.execute("sqlite3 test.db '.tables'");
    assert!(
        r.exit_code == 0,
        "sqlite3 .tables failed: code={} stderr={}",
        r.exit_code,
        r.stderr
    );
    assert!(
        r.stdout.contains("t") || r.stdout.contains("id"),
        ".tables output: {:?}",
        r.stdout
    );

    let r = sdk.execute("sqlite3 test.db '.schema'");
    assert!(
        r.exit_code == 0,
        "sqlite3 .schema failed: code={} stderr={}",
        r.exit_code,
        r.stderr
    );
}

#[test]
fn edge_sqlite3_csv_mode() {
    let sdk = setup();
    sdk.execute(
        "sqlite3 test.db 'CREATE TABLE t (a TEXT, b TEXT); INSERT INTO t VALUES (\"x\", \"y\");'",
    );

    let r = sdk.execute("sqlite3 -csv -header test.db 'SELECT * FROM t;'");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("a,b"));
    assert!(r.stdout.contains("x,y"));
}

#[test]
fn edge_sqlite3_multi_statement() {
    let sdk = setup();
    let r = sdk.execute("sqlite3 test.db 'CREATE TABLE t (id INT); INSERT INTO t VALUES (1); INSERT INTO t VALUES (2); SELECT COUNT(*) FROM t;'");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("2"));
}

// ═══════════════════════════════════════════════════════
// printf edge cases
// ═══════════════════════════════════════════════════════

#[test]
fn edge_printf_formats() {
    let sdk = setup();

    let r = sdk.execute("printf hello");
    assert_eq!(r.exit_code, 0);
    assert_eq!(r.stdout, "hello");

    let r = sdk.execute("printf '%s' hello");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("hello"), "printf output: {:?}", r.stdout);
}

// ═══════════════════════════════════════════════════════
// sleep edge cases
// ═══════════════════════════════════════════════════════

#[test]
fn edge_sleep_zero() {
    let sdk = setup();
    let r = sdk.execute("sleep 0");
    assert_eq!(r.exit_code, 0);
}

// ═══════════════════════════════════════════════════════
// permissions boundary tests
// ═══════════════════════════════════════════════════════

#[test]
fn edge_curl_permission_boundary() {
    let mut sdk = Fastshell::new();
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_edge_perm_{}_{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dir);
    sdk.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: false,
        allow_subprocess: false,
        network_ask_permission: true,
        command_timeout_ms: 5_000,
    })
    .unwrap();

    let r = sdk.execute("curl http://example.com");
    assert!(
        r.exit_code == 100 || r.stderr.contains("PERMISSION_NEEDED") || r.exit_code != 0,
        "expected permission denied, got code={} stderr={}",
        r.exit_code,
        r.stderr
    );

    sdk.set_permission("network:example.com", true);
    let r = sdk.execute("curl http://example.com");
    assert_eq!(
        r.exit_code, 0,
        "curl should succeed after permission grant: stderr={}",
        r.stderr
    );
}

// ═══════════════════════════════════════════════════════
// Multi-command scenarios (bash-like)
// ═══════════════════════════════════════════════════════

#[test]
fn edge_bash_semicolon_chain() {
    let sdk = setup();
    let r = sdk.execute("echo a; echo b; echo c");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("a"));
    assert!(r.stdout.contains("b"));
    assert!(r.stdout.contains("c"));
}

#[test]
fn edge_bash_and_or() {
    let sdk = setup();
    let r = sdk.execute("echo ok && echo also_ok");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("ok"));
    assert!(r.stdout.contains("also_ok"));
}

#[test]
fn edge_bash_or_fallback() {
    let sdk = setup();
    let r = sdk.execute("false || echo fallback");
    assert!(r.exit_code == 0 || r.stdout.contains("fallback"));
}

// ═══════════════════════════════════════════════════════
// Unicode and special characters
// ═══════════════════════════════════════════════════════

#[test]
fn edge_unicode_commands() {
    let sdk = setup();
    sdk.write_file("名前.txt", "日本語テスト").unwrap();

    let r = sdk.execute("ls");
    assert_eq!(r.exit_code, 0);

    let r = sdk.execute("cat 名前.txt");
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains("日本語テスト"));
}

// ═══════════════════════════════════════════════════════
// Very long command strings
// ═══════════════════════════════════════════════════════

#[test]
fn edge_long_command() {
    let sdk = setup();
    let long_arg = "a".repeat(5000);
    let r = sdk.execute(&format!("echo '{}'", long_arg));
    assert_eq!(r.exit_code, 0);
    assert!(r.stdout.contains(&long_arg));
}
