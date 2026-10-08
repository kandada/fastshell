// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Regression tests for the robustness pass: comment stripping, grep attached
//! context flags, glob expansion, binary-safe curl downloads and the
//! single-segment fast paths that guard them.

use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn setup() -> Fastshell {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_regr_{}_{}", std::process::id(), n));
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

// ── Comment stripping ────────────────────────────────────────────────────

#[test]
fn comments_full_line_with_apostrophe() {
    let sdk = setup();
    let r = sdk.execute("#!/bin/sh\n# don't break\nls > /dev/null\necho done");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("done"), "stdout={}", r.stdout);
    assert!(
        !r.stderr.contains("command not found"),
        "stderr={}",
        r.stderr
    );
}

#[test]
fn comment_after_command_same_line() {
    let sdk = setup();
    let r = sdk.execute("echo hi # it's a comment\necho bye");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("hi"), "stdout={}", r.stdout);
    assert!(r.stdout.contains("bye"), "stdout={}", r.stdout);
    assert!(!r.stdout.contains("comment"), "stdout={}", r.stdout);
}

#[test]
fn hash_inside_word_is_literal() {
    let sdk = setup();
    let r = sdk.execute("echo a#b");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("a#b"), "stdout={}", r.stdout);
}

#[test]
fn hash_inside_single_quotes_is_literal() {
    let sdk = setup();
    let r = sdk.execute("echo '# not a comment'");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("# not a comment"), "stdout={}", r.stdout);
}

#[test]
fn hash_operators_not_treated_as_comments() {
    let sdk = setup();
    // `${#var}` string-length operator.
    let r = sdk.execute("x=abc\necho ${#x}");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert_eq!(r.stdout.trim(), "3", "stdout={}", r.stdout);
    // `$#` positional-parameter count.
    let r = sdk.execute("echo $#");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    // Escaped hash stays literal.
    let r = sdk.execute("echo \\#tag");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("#tag"), "stdout={}", r.stdout);
}

#[test]
fn comment_inside_for_loop_body() {
    let sdk = setup();
    let r = sdk.execute("for i in 1 2; do\n  # it's a comment in a loop\n  echo item$i\ndone");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(
        r.stdout.contains("item1") && r.stdout.contains("item2"),
        "stdout={}",
        r.stdout
    );
}

#[test]
fn comment_after_and_and() {
    let sdk = setup();
    let r = sdk.execute("true && # comment\necho ok");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("ok"), "stdout={}", r.stdout);
}

#[test]
fn comment_in_heredoc_body_is_literal() {
    let sdk = setup();
    let r = sdk.execute("cat <<EOF\n# not stripped\nEOF");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("# not stripped"), "stdout={}", r.stdout);
}

// ── grep attached context flags ──────────────────────────────────────────

#[test]
fn grep_attached_context_values() {
    let sdk = setup();
    sdk.write_file("g.txt", "1a\n2b\n3c\n4d\n5e\n").unwrap();
    let r = sdk.execute("grep -A1 3 g.txt");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(
        r.stdout.contains("3c") && r.stdout.contains("4d"),
        "stdout={}",
        r.stdout
    );
    assert!(!r.stderr.contains("unsupported"), "stderr={}", r.stderr);

    let r = sdk.execute("grep -B1 3 g.txt");
    assert!(
        r.stdout.contains("2b") && r.stdout.contains("3c"),
        "stdout={}",
        r.stdout
    );

    let r = sdk.execute("grep -C1 3 g.txt");
    assert!(
        r.stdout.contains("2b") && r.stdout.contains("3c") && r.stdout.contains("4d"),
        "stdout={}",
        r.stdout
    );

    let r = sdk.execute("grep -m1 -n . g.txt");
    assert_eq!(r.stdout.trim(), "1:1a", "stdout={}", r.stdout);
}

#[test]
fn grep_combined_flags_still_work() {
    let sdk = setup();
    sdk.write_file("g.txt", "Apple\napple\nbanana\n").unwrap();
    let r = sdk.execute("grep -in apple g.txt");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("1:Apple"), "stdout={}", r.stdout);
    assert!(r.stdout.contains("2:apple"), "stdout={}", r.stdout);
}

// ── glob expansion ───────────────────────────────────────────────────────

#[test]
fn glob_question_and_class() {
    let sdk = setup();
    sdk.write_file("a1.txt", "x").unwrap();
    sdk.write_file("a2.txt", "y").unwrap();
    sdk.write_file("ab.txt", "z").unwrap();
    // `?` matches exactly one char, so a?.txt matches all three.
    let r = sdk.execute("ls a?.txt");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(
        r.stdout.contains("a1.txt") && r.stdout.contains("a2.txt") && r.stdout.contains("ab.txt"),
        "stdout={}",
        r.stdout
    );
    // A character class restricts the match.
    let r = sdk.execute("ls a[12].txt");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(
        r.stdout.contains("a1.txt") && r.stdout.contains("a2.txt"),
        "stdout={}",
        r.stdout
    );
    assert!(!r.stdout.contains("ab.txt"), "stdout={}", r.stdout);
}

// ── js render help ───────────────────────────────────────────────────────

#[test]
fn render_help_without_plugin() {
    let sdk = setup();
    let r = sdk.execute("render -h");
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("Usage: render"), "stdout={}", r.stdout);
}

// ── curl header/body handling after the binary-safe refactor ─────────────

/// Serve exactly one HTTP response with the given body and close.
fn serve_once(body: Vec<u8>) -> u16 {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let mut resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .into_bytes();
            resp.extend_from_slice(&body);
            let _ = stream.write_all(&resp);
            let _ = stream.flush();
        }
    });
    port
}

#[test]
fn curl_include_headers_stdout_and_file() {
    let sdk = setup();
    let port = serve_once(b"hello-body".to_vec());
    let r = sdk.execute(&format!("curl -s -i http://127.0.0.1:{port}/t"));
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("HTTP/1.1 200 OK"), "stdout={}", r.stdout);
    assert!(r.stdout.contains("hello-body"), "stdout={}", r.stdout);

    let port = serve_once(b"file-body".to_vec());
    let r = sdk.execute(&format!("curl -s -i -o out.txt http://127.0.0.1:{port}/t"));
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    let written = sdk.read_file("out.txt").unwrap();
    assert!(written.contains("HTTP/1.1 200 OK"), "file={written}");
    assert!(written.contains("file-body"), "file={written}");
}

#[test]
fn curl_write_out_format_goes_to_stdout() {
    let sdk = setup();
    let port = serve_once(b"x".to_vec());
    let r = sdk.execute(&format!(
        "curl -s -o out.txt -w '%{{http_code}}' http://127.0.0.1:{port}/t"
    ));
    assert_eq!(r.exit_code, 0, "stderr={}", r.stderr);
    assert!(r.stdout.contains("200"), "stdout={}", r.stdout);
    assert_eq!(sdk.read_file("out.txt").unwrap(), "x");
}
