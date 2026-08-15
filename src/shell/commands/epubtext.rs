// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};
use std::io::Cursor;

const EPUBTEXT_HELP_TEXT: &str = "\
Usage: epubtext [OPTION]... [EPUB-FILE]
Extract text from an EPUB e-book file.

  -h, --help   display this help and exit
";

const EPUBTEXT_MAX_SIZE: usize = 50 * 1024 * 1024; // 50 MB

impl Shell {
    pub fn cmd_epubtext(&self, args: &[&str]) -> CommandOutput {
        let mut files = Vec::new();

        for arg in args {
            match *arg {
                "-h" | "--help" => {
                    return CommandOutput::success(EPUBTEXT_HELP_TEXT.to_string());
                }
                a if !a.starts_with('-') => files.push(a.to_string()),
                _ => {}
            }
        }

        if files.is_empty() {
            return CommandOutput::error("epubtext: missing file operand\n".to_string(), 1);
        }

        let mut output = String::new();
        for file in &files {
            let data = match self.vfs.read(file, &self.cwd) {
                Ok(d) => d,
                Err(e) => {
                    return CommandOutput::error(
                        format!("epubtext: {}: {}\n", file, e),
                        1,
                    )
                }
            };

            if data.len() > EPUBTEXT_MAX_SIZE {
                return CommandOutput::error(
                    format!("epubtext: {}: file too large (max {} MB)\n", file, EPUBTEXT_MAX_SIZE / 1024 / 1024),
                    1,
                );
            }

            match extract_epub_text(&data) {
                Ok(text) => output.push_str(&text),
                Err(e) => {
                    return CommandOutput::error(
                        format!("epubtext: {}: {}\n", file, e),
                        1,
                    )
                }
            }
        }

        CommandOutput::success(output)
    }
}

fn extract_epub_text(data: &[u8]) -> Result<String, String> {
    let cursor = Cursor::new(data);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| format!("not a valid ZIP/EPUB file: {}", e))?;

    // Find all XHTML/HTML content files, exclude navigation/toc files
    let mut html_files = Vec::new();
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| format!("error reading archive: {}", e))?;
        let name = entry.name().to_lowercase();
        if (name.ends_with(".html") || name.ends_with(".xhtml") || name.ends_with(".htm"))
            && !name.contains("nav")
            && !name.contains("toc")
        {
            html_files.push(i);
        }
    }

    if html_files.is_empty() {
        return Err("no HTML content files found in EPUB".to_string());
    }

    let mut output = String::new();
    for idx in html_files {
        let entry = archive
            .by_index(idx)
            .map_err(|e| format!("error reading archive: {}", e))?;
        let mut buf = Vec::new();
        let mut reader = entry;
        std::io::Read::read_to_end(&mut reader, &mut buf)
            .map_err(|e| format!("error reading content: {}", e))?;

        let html = String::from_utf8_lossy(&buf);
        let text = strip_html_tags(&html);
        if !text.trim().is_empty() {
            output.push_str(text.trim());
            output.push_str("\n\n");
        }
    }

    Ok(output)
}

/// Simple HTML tag stripper — removes all <tags>, decodes basic entities.
fn strip_html_tags(html: &str) -> String {
    let mut output = String::new();
    let mut in_tag = false;
    let mut in_script = false;
    let mut in_style = false;
    let chars: Vec<char> = html.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '<' {
            // Check for comments
            if i + 4 <= chars.len()
                && chars[i..i + 4].iter().collect::<String>() == "<!--"
            {
                // Skip until -->
                while i < chars.len() {
                    if i + 3 <= chars.len()
                        && chars[i..i + 3].iter().collect::<String>() == "-->"
                    {
                        i += 3;
                        break;
                    }
                    i += 1;
                }
                continue;
            }

            in_tag = true;
            // Check for script/style blocks
            let remaining: String = chars[i + 1..].iter().take(8).collect();
            let remaining = remaining.to_lowercase();
            if remaining.starts_with("script") {
                in_script = true;
            } else if remaining.starts_with("style") {
                in_style = true;
            } else if remaining.starts_with("/script") {
                in_script = false;
            } else if remaining.starts_with("/style") {
                in_style = false;
            }
        } else if chars[i] == '>' {
            in_tag = false;
        } else if !in_tag && !in_script && !in_style {
            output.push(chars[i]);
        }
        i += 1;
    }

    // Decode common HTML entities
    let text = output
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");

    // Collapse multiple whitespace
    let mut result = String::new();
    let mut prev_space = false;
    for ch in text.chars() {
        if ch == '\n' {
            if !prev_space {
                result.push('\n');
            }
            prev_space = true;
        } else if ch.is_whitespace() {
            if !prev_space {
                result.push(' ');
            }
            prev_space = true;
        } else {
            result.push(ch);
            prev_space = false;
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn mk_shell() -> Shell {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "fastshell_epubtext_test_{}_{}",
            std::process::id(),
            n
        ));
        let _ = fs::remove_dir_all(&dir);
        let vfs = crate::vfs::Vfs::new(dir).unwrap();
        Shell::new(vfs)
    }

    fn make_minimal_epub() -> Vec<u8> {
        let chapter_html = b"<!DOCTYPE html>
<html><head><title>Chapter 1</title></head>
<body>
  <h1>Chapter One</h1>
  <p>Hello World. This is a <em>test</em> document.</p>
  <p>Second paragraph with <a href=\"http://example.com\">a link</a>.</p>
</body></html>";

        let container_xml = b"<?xml version=\"1.0\"?>
<container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\">
  <rootfiles>
    <rootfile full-path=\"content.opf\" media-type=\"application/oebps-package+xml\"/>
  </rootfiles>
</container>";

        let cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let options =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        writer.start_file("mimetype", zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored)).unwrap();
        writer.write_all(b"application/epub+zip").unwrap();
        writer.start_file("META-INF/container.xml", options).unwrap();
        writer.write_all(container_xml).unwrap();
        writer.start_file("chapter1.xhtml", options).unwrap();
        writer.write_all(chapter_html).unwrap();

        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn test_epubtext_help() {
        let mut s = mk_shell();
        let out = s.execute("epubtext", &["-h"], None);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("Usage: epubtext"));
    }

    #[test]
    fn test_epubtext_extract() {
        let mut s = mk_shell();
        let data = make_minimal_epub();
        let file = "test.epub";
        s.vfs.write_bytes(file, &s.cwd, &data).unwrap();
        let out = s.execute("epubtext", &[file], None);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("Hello World"));
        assert!(out.stdout.contains("test"));
        assert!(out.stdout.contains("Second paragraph"));
        assert!(!out.stdout.contains("<p>"));
    }

    #[test]
    fn test_strip_basic_html() {
        let html = "<p>Hello <b>World</b></p>";
        let result = strip_html_tags(html);
        assert_eq!(result.trim(), "Hello World");
    }

    #[test]
    fn test_strip_html_with_comment() {
        let html = "<!-- comment --><p>Hello</p><!-- another -->";
        let result = strip_html_tags(html);
        assert_eq!(result.trim(), "Hello");
    }
}
