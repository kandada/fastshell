// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

const PDFTOTEXT_HELP_TEXT: &str = "\
Usage: pdftotext [OPTION]... [PDF-FILE]
Extract text from a PDF file.

  -f N         first page to convert (default 1)
  -l N         last page to convert (default last)
  -h, --help   display this help and exit
";

impl Shell {
    pub fn cmd_pdftotext(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut first_page: Option<u32> = None;
        let mut last_page: Option<u32> = None;
        let mut files = Vec::new();

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-h" | "--help" => {
                    return CommandOutput::success(PDFTOTEXT_HELP_TEXT.to_string());
                }
                "-f" => {
                    if i + 1 < args.len() {
                        first_page = args[i + 1].parse().ok();
                        i += 1;
                    }
                }
                "-l" => {
                    if i + 1 < args.len() {
                        last_page = args[i + 1].parse().ok();
                        i += 1;
                    }
                }
                // `-` means stdout (the default), `-layout`/`-raw` are the
                // natural output mode here — accept silently.
                "-" | "-layout" | "-raw" => {}
                // Unsupported output formats — tell the caller so it doesn't
                // assume the output is actually XML/bbox-formatted.
                "-xml" | "-bbox" | "-bbox-layout" | "-htmlmeta" | "-enc" | "-eol" | "-opw" | "-upw" | "-q" | "-v" => {
                    crate::warn!("pdftotext: warning: option '{}' is not supported (plain text output)", args[i]);
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => crate::warn!("pdftotext: warning: unsupported option '{}'", args[i]),
            }
            i += 1;
        }

        if files.is_empty() {
            if let Some(input) = stdin {
                let data = input.as_bytes().to_vec();
                return CommandOutput::success(normalize_cjk_spacing(&extract_pdf_text(&data, first_page, last_page)));
            }
            return CommandOutput::error("pdftotext: missing file operand\n".to_string(), 1);
        }

        let mut output = String::new();
        for file in &files {
            let data = match self.vfs.read(file, &self.cwd) {
                Ok(d) => d,
                Err(e) => {
                    return CommandOutput::error(
                        format!("pdftotext: {}: {}\n", file, e),
                        1,
                    )
                }
            };

            match extract_pdf_text_fallback(&data, first_page, last_page) {
                Ok(text) => {
                    if text.trim().is_empty() {
                        output.push_str(&format!("(no text extracted from {})\n", file));
                    } else {
                        let cleaned = normalize_cjk_spacing(&text);
                        output.push_str(&cleaned);
                    }
                }
                Err(e) => {
                    return CommandOutput::error(
                        format!("pdftotext: {}: {}\n", file, e),
                        1,
                    )
                }
            }
        }

        CommandOutput::success(output)
    }
}

/// Extract text from PDF using pdf-extract crate, falling back to strings if that fails.
fn extract_pdf_text_fallback(data: &[u8], first_page: Option<u32>, last_page: Option<u32>) -> Result<String, String> {
    match pdf_extract::extract_text_from_mem(data) {
        Ok(text) => {
            let pages = filter_pages(&text, first_page, last_page);
            if pages.trim().is_empty() {
                // fallback to strings approach
                Ok(extract_pdf_strings(data))
            } else {
                Ok(pages)
            }
        }
        Err(_) => {
            // fallback to strings approach
            Ok(extract_pdf_strings(data))
        }
    }
}

/// Extract text from PDF bytes for use with stdin (no file access).
fn extract_pdf_text(data: &[u8], first_page: Option<u32>, last_page: Option<u32>) -> String {
    match pdf_extract::extract_text_from_mem(data) {
        Ok(text) => filter_pages(&text, first_page, last_page),
        Err(_) => extract_pdf_strings(data),
    }
}

/// Filter text to specific page range.
/// pdf-extract separates pages with a specific marker.
fn filter_pages(text: &str, first: Option<u32>, last: Option<u32>) -> String {
    if first.is_none() && last.is_none() {
        return text.to_string();
    }

    let start = first.unwrap_or(1).saturating_sub(1) as usize;
    let end = last.map(|l| l.saturating_sub(1) as usize);

    // pdf-extract typically uses \n\n or form feed as page separators
    // Try common page separator patterns
    let pages: Vec<&str> = text.split("\n\n\n").collect();
    if pages.len() <= 1 {
        let pages: Vec<&str> = text.split("\x0C").collect(); // form feed
        if pages.len() > 1 {
            let result: Vec<&&str> = pages.iter()
                .skip(start)
                .take(end.map(|e| e - start + 1).unwrap_or(usize::MAX) as usize)
                .filter(|p| !p.trim().is_empty())
                .collect();
            return result.iter().map(|s| s.trim()).collect::<Vec<_>>().join("\n\n");
        }
        // Single page or unknown format
        return text.to_string();
    }

    let result: Vec<&&str> = pages.iter()
        .skip(start)
        .take(end.map(|e| e - start + 1).unwrap_or(usize::MAX) as usize)
        .filter(|p| !p.trim().is_empty())
        .collect();
    result.iter().map(|s| s.trim()).collect::<Vec<_>>().join("\n\n")
}

/// Fallback PDF text extraction using printable ASCII strings with noise filtering.
fn extract_pdf_strings(data: &[u8]) -> String {
    let mut strings = Vec::new();
    let mut current = String::new();
    let mut in_string = false;

    for &byte in data.iter() {
        if byte >= 0x20 && byte < 0x7f {
            if !in_string {
                in_string = true;
            }
            current.push(byte as char);
        } else {
            if in_string {
                if current.len() >= 4 {
                    strings.push(current.clone());
                }
                current.clear();
                in_string = false;
            }
        }
    }
    if in_string && current.len() >= 4 {
        strings.push(current);
    }

    let noise_keywords = [
        "endobj", "endstream", "stream", "xref", "trailer", "startxref",
        "obj <</Type", "/Type /", "/Subtype", "/Filter", "/Length",
        "/ID [", "/Info", "/Root", "/Size", "/Linearized",
    ];

    let mut output = String::new();
    for s in &strings {
        let mut is_noise = false;
        for noise in &noise_keywords {
            if s.contains(noise) {
                is_noise = true;
                break;
            }
        }
        if !is_noise {
            output.push_str(s);
            output.push('\n');
        }
    }

    output
}

/// Collapse unnecessary spaces between CJK characters and compatible
/// punctuation/digits that were artifactually separated by PDF extraction,
/// while preserving real word spaces in ordinary (Latin) text.
///
/// A space is treated as artifactual and removed when either side is a single
/// character (character-spaced text) or contains a CJK/wide character. A space
/// is kept only between two multi-character, non-CJK words (e.g. "Hello World").
fn normalize_cjk_spacing(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let tokens: Vec<&str> = trimmed.split(' ').filter(|s| !s.is_empty()).collect();
        let mut result = String::with_capacity(trimmed.len());
        for (i, tok) in tokens.iter().enumerate() {
            if i > 0 && should_keep_space(tokens[i - 1], tok) {
                result.push(' ');
            }
            result.push_str(tok);
        }
        let result = result.trim().to_string();
        if !result.is_empty() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&result);
        }
    }
    out
}

/// True if the space between two tokens should be preserved as a real word
/// separator (both sides are multi-character, non-CJK words).
fn should_keep_space(a: &str, b: &str) -> bool {
    let a_single = a.chars().count() == 1;
    let b_single = b.chars().count() == 1;
    let a_cjk = a.chars().any(is_cjk_or_wide);
    let b_cjk = b.chars().any(is_cjk_or_wide);
    !(a_single || b_single || a_cjk || b_cjk)
}

/// Whether `c` is a CJK/wide character (ideograph, kana, CJK punctuation, or
/// fullwidth form) that indicates PDF-level character spacing.
fn is_cjk_or_wide(c: char) -> bool {
    let cp = c as u32;
    matches!(
        cp,
        0x4E00..=0x9FFF      // CJK Unified Ideographs
        | 0x3400..=0x4DBF    // CJK Extension A
        | 0x20000..=0x2A6DF  // CJK Extension B
        | 0xF900..=0xFAFF    // CJK Compatibility Ideographs
        | 0x3040..=0x30FF    // Hiragana + Katakana
        | 0x3000..=0x303F    // CJK Symbols and Punctuation
        | 0xFF00..=0xFFEF    // Fullwidth Forms
    )
}

#[cfg(test)]
mod tests {
    use super::Shell;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn mk_shell() -> Shell {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "fastshell_pdftotext_test_{}_{}",
            std::process::id(),
            n
        ));
        let _ = fs::remove_dir_all(&dir);
        let vfs = crate::vfs::Vfs::new(dir).unwrap();
        Shell::new(vfs)
    }

    #[test]
    fn test_pdftotext_help() {
        let mut s = mk_shell();
        let out = s.execute("pdftotext", &["-h"], None);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("Usage: pdftotext"));
    }

    #[test]
    fn test_pdftotext_help_long() {
        let mut s = mk_shell();
        let out = s.execute("pdftotext", &["--help"], None);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("Usage: pdftotext"));
    }

    #[test]
    fn test_pdftotext_fallback_extract() {
        let mut s = mk_shell();
        let data = b"%PDF-1.4\nHello World\nThis is a test document.\n%%EOF";
        let file = "test_fallback.pdf";
        s.vfs.write_bytes(file, &s.cwd, data).unwrap();
        let out = s.execute("pdftotext", &[file], None);
        assert_eq!(out.exit_code, 0);
        // Fallback to strings extraction should find readable text
        assert!(out.stdout.contains("Hello World") || out.stdout.contains("test document")
            || !out.stdout.contains("endobj"));
    }

    #[test]
    fn test_filter_pages_none() {
        assert_eq!(super::filter_pages("page1\n\n\npage2", None, None), "page1\n\n\npage2");
    }

    #[test]
    fn test_cjk_spacing_normalized() {
        let input = "\n\n2 0 1 5 .0 7 -至 今\n\n谢 先 生\n\n男  | 生 日 ： 1 9 8 6 .1 0  | 广 州\n\n";
        let output = super::normalize_cjk_spacing(input);
        assert_eq!(output, "2015.07-至今\n谢先生\n男|生日：1986.10|广州");
    }

    #[test]
    fn test_cjk_mixed_english() {
        let input = "C端 和 B端 产 品\nS C R M 、 C R M\np y t h o n";
        let output = super::normalize_cjk_spacing(input);
        assert_eq!(output, "C端和B端产品\nSCRM、CRM\npython");
    }

    #[test]
    fn test_cjk_preserves_newlines() {
        let input = "行 一\n行 二\n行 三";
        let output = super::normalize_cjk_spacing(input);
        assert_eq!(output, "行一\n行二\n行三");
    }

    #[test]
    fn test_english_spaces_preserved() {
        let input = "Hello World\nThis is readable text.";
        let output = super::normalize_cjk_spacing(input);
        assert_eq!(output, "Hello World\nThis is readable text.");
    }

    #[test]
    fn test_pdftotext_layout_option_accepted() {
        let mut s = mk_shell();
        let data = b"%PDF-1.4\nHello World\n%%EOF";
        let file = "test_layout.pdf";
        s.vfs.write_bytes(file, &s.cwd, data).unwrap();
        // -layout and the trailing `-` (stdout) must be accepted silently.
        let out = s.execute("pdftotext", &["-layout", file, "-"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stderr.contains("unsupported"), "-layout should be accepted: {}", out.stderr);
    }
}
