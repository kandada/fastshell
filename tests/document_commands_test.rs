// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use fastshell::Config;
use fastshell::Fastshell;
use std::sync::atomic::{AtomicUsize, Ordering};

mod common;

static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn setup() -> Fastshell {
    let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fastshell_doc_test_{}_{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    let mut sdk = Fastshell::new();
    let config = Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        allow_subprocess: false,
        network_ask_permission: false,
        ..Default::default()
    };
    sdk.init(config).expect("init");
    sdk
}

fn execute(sdk: &Fastshell, cmd: &str) -> String {
    sdk.execute(cmd).stdout
}

fn execute_stderr(sdk: &Fastshell, cmd: &str) -> String {
    sdk.execute(cmd).stderr
}

fn exit_code(sdk: &Fastshell, cmd: &str) -> i32 {
    sdk.execute(cmd).exit_code
}

fn write_str(sdk: &Fastshell, name: &str, content: &str) {
    sdk.write_file(name, content).expect("write_file");
}

// ========== file command tests ==========

#[test]
fn file_help_flag() {
    let sdk = setup();
    let out = execute(&sdk, "file -h");
    assert!(out.contains("Usage: file"), "got: {}", out);
}

#[test]
fn file_brief_mode() {
    let sdk = setup();
    write_str(&sdk, "a.txt", "hello world\nmore text\n");
    let out = execute(&sdk, "file -b a.txt");
    assert!(
        !out.contains("a.txt:"),
        "brief should suppress filename: {}",
        out
    );
    assert!(out.contains("ASCII text"), "got: {}", out);
}

#[test]
fn file_mime_mode() {
    let sdk = setup();
    write_str(&sdk, "a.txt", "hello world\nmore text\n");
    let out = execute(&sdk, "file -i a.txt");
    assert!(out.contains("text/plain"), "got: {}", out);
}

#[test]
fn file_mime_long_flag() {
    let sdk = setup();
    write_str(&sdk, "a.txt", "hello world\nmore text\n");
    let out = execute(&sdk, "file --mime a.txt");
    assert!(out.contains("text/plain"), "got: {}", out);
}

#[test]
fn file_mime_pdf() {
    let sdk = setup();
    // PDF header + binary padding
    let _ = execute(&sdk, "printf '%s' '%PDF-1.4' > doc.pdf");
    let _ = execute(&sdk, "printf '%02048d' 0 >> doc.pdf");
    let out = execute(&sdk, "file -i doc.pdf");
    assert!(out.contains("application/pdf"), "got: {}", out);
}

#[test]
fn file_detects_json() {
    let sdk = setup();
    write_str(&sdk, "data.json", "{\"key\": \"value\"}\n");
    let out = execute(&sdk, "file data.json");
    assert!(out.contains("JSON text"), "got: {}", out);
}

#[test]
fn file_detects_html() {
    let sdk = setup();
    write_str(&sdk, "page.html", "<html><body>Hello</body></html>\n");
    let out = execute(&sdk, "file page.html");
    assert!(out.contains("HTML/XML text"), "got: {}", out);
}

// ========== strings command tests ==========

#[test]
fn strings_help_flag() {
    let sdk = setup();
    let out = execute(&sdk, "strings -h");
    assert!(out.contains("Usage: strings"), "got: {}", out);
}

#[test]
fn strings_help_long() {
    let sdk = setup();
    let out = execute(&sdk, "strings --help");
    assert!(out.contains("Usage: strings"), "got: {}", out);
}

#[test]
fn strings_default_minlen() {
    let sdk = setup();
    write_str(&sdk, "test.txt", "ab\nhelloworld\ncd\ntest123\n");
    let out = execute(&sdk, "strings test.txt");
    assert!(out.contains("helloworld"), "got: {}", out);
    assert!(out.contains("test123"), "got: {}", out);
}

#[test]
fn strings_custom_minlen() {
    let sdk = setup();
    write_str(&sdk, "test.txt", "abcdef\nhi\n");
    let out = execute(&sdk, "strings -n 2 test.txt");
    assert!(
        out.contains("hi"),
        "minlen 2 should include 2-char strings: {}",
        out
    );
}

#[test]
fn strings_filename_flag() {
    let sdk = setup();
    write_str(&sdk, "one.bin", "hello123\n");
    write_str(&sdk, "two.bin", "world456\n");
    let out = execute(&sdk, "strings -f one.bin two.bin");
    assert!(!out.is_empty(), "got: {}", out);
}

#[test]
fn strings_radix_decimal() {
    let sdk = setup();
    write_str(&sdk, "test.txt", "hello\n");
    let out = execute(&sdk, "strings --radix d test.txt");
    assert!(!out.is_empty(), "got: {}", out);
}

// ========== pdftotext command tests ==========

#[test]
fn pdftotext_help_flag() {
    let sdk = setup();
    let out = execute(&sdk, "pdftotext -h");
    assert!(out.contains("Usage: pdftotext"), "got: {}", out);
}

#[test]
fn pdftotext_help_long() {
    let sdk = setup();
    let out = execute(&sdk, "pdftotext --help");
    assert!(out.contains("Usage: pdftotext"), "got: {}", out);
}

#[test]
fn pdftotext_missing_file_errors() {
    let sdk = setup();
    let code = exit_code(&sdk, "pdftotext no_such_file.pdf");
    assert_eq!(code, 1, "pdftotext with missing file should exit 1");
}

#[test]
fn pdftotext_extracts_text_fallback() {
    let sdk = setup();
    write_str(
        &sdk,
        "doc.pdf",
        "%PDF-1.4\nHello World\nThis is readable text.\n%%EOF\n",
    );
    let out = execute(&sdk, "pdftotext doc.pdf");
    assert!(
        out.contains("Hello World") || out.contains("readable text"),
        "got: {}",
        out
    );
}

#[test]
fn pdftotext_with_page_range_flags() {
    let sdk = setup();
    write_str(&sdk, "doc.pdf", "%PDF-1.4\nHello World\n%%EOF\n");
    let out = execute(&sdk, "pdftotext -f 1 -l 2 doc.pdf");
    assert!(!out.is_empty(), "got: {}", out);
}

// ========== doctotext command tests ==========

#[test]
fn doctotext_help_flag() {
    let sdk = setup();
    let out = execute(&sdk, "doctotext -h");
    assert!(out.contains("Usage: doctotext"), "got: {}", out);
}

#[test]
fn doctotext_missing_file_errors() {
    let sdk = setup();
    let code = exit_code(&sdk, "doctotext no_such.docx");
    assert_ne!(code, 0, "should error on missing file");
}

#[test]
fn doctotext_non_docx_errors() {
    let sdk = setup();
    write_str(&sdk, "test.docx", "just plain text");
    let code = exit_code(&sdk, "doctotext test.docx");
    assert_ne!(
        code, 0,
        "should error on non-zip file, got exit code {}",
        code
    );
}

// ========== epubtext command tests ==========

#[test]
fn epubtext_help_flag() {
    let sdk = setup();
    let out = execute(&sdk, "epubtext -h");
    assert!(out.contains("Usage: epubtext"), "got: {}", out);
}

#[test]
fn epubtext_missing_file_errors() {
    let sdk = setup();
    let code = exit_code(&sdk, "epubtext no_such.epub");
    assert_ne!(code, 0, "should error on missing file");
}

#[test]
fn epubtext_non_epub_errors() {
    let sdk = setup();
    write_str(&sdk, "test.epub", "not an epub");
    let code = exit_code(&sdk, "epubtext test.epub");
    assert_ne!(code, 0, "should error on non-zip file");
}

// ========== pip-install command tests ==========

#[test]
fn pip_install_help_flag() {
    let sdk = setup();
    let out = execute(&sdk, "pip-install -h");
    assert!(
        out.contains("Usage:") && out.contains("pip-install"),
        "got: {}",
        out
    );
}

#[test]
fn pip_install_missing_package_errors() {
    let sdk = setup();
    let code = exit_code(&sdk, "pip-install");
    assert_ne!(code, 0, "should error on missing package name");
}

// ========== which command ==========

#[test]
fn which_recognizes_new_commands() {
    let sdk = setup();
    for cmd in &["pdftotext", "pip-install", "doctotext", "epubtext"] {
        let out = execute(&sdk, &format!("which {}", cmd));
        assert!(
            out.contains("built-in fastshell command"),
            "which {}: {}",
            cmd,
            out
        );
    }
}
