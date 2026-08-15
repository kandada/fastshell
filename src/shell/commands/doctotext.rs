// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};
use std::io::Cursor;

const DOCTOTEXT_HELP_TEXT: &str = "\
Usage: doctotext [OPTION]... [DOCX-FILE]
Extract text from a DOCX (Microsoft Word 2007+) file.

  -h, --help   display this help and exit
";

const DOCTOTEXT_MAX_SIZE: usize = 50 * 1024 * 1024; // 50 MB

impl Shell {
    pub fn cmd_doctotext(&self, args: &[&str]) -> CommandOutput {
        let mut files = Vec::new();

        for arg in args {
            match *arg {
                "-h" | "--help" => {
                    return CommandOutput::success(DOCTOTEXT_HELP_TEXT.to_string());
                }
                a if !a.starts_with('-') => files.push(a.to_string()),
                _ => {}
            }
        }

        if files.is_empty() {
            return CommandOutput::error("doctotext: missing file operand\n".to_string(), 1);
        }

        let mut output = String::new();
        for file in &files {
            let data = match self.vfs.read(file, &self.cwd) {
                Ok(d) => d,
                Err(e) => {
                    return CommandOutput::error(
                        format!("doctotext: {}: {}\n", file, e),
                        1,
                    )
                }
            };

            if data.len() > DOCTOTEXT_MAX_SIZE {
                return CommandOutput::error(
                    format!("doctotext: {}: file too large (max {} MB)\n", file, DOCTOTEXT_MAX_SIZE / 1024 / 1024),
                    1,
                );
            }

            match extract_docx_text(&data) {
                Ok(text) => output.push_str(&text),
                Err(e) => {
                    return CommandOutput::error(
                        format!("doctotext: {}: {}\n", file, e),
                        1,
                    )
                }
            }
        }

        CommandOutput::success(output)
    }
}

fn extract_docx_text(data: &[u8]) -> Result<String, String> {
    let cursor = Cursor::new(data);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| format!("not a valid ZIP/DOCX file: {}", e))?;

    let mut doc_xml = None;
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| format!("error reading archive: {}", e))?;
        if entry.name() == "word/document.xml" {
            let mut buf = Vec::new();
            let mut reader = entry;
            std::io::Read::read_to_end(&mut reader, &mut buf)
                .map_err(|e| format!("error reading document.xml: {}", e))?;
            doc_xml = Some(buf);
            break;
        }
    }

    let doc_xml = doc_xml.ok_or_else(|| "word/document.xml not found in archive".to_string())?;

    let xml_str = String::from_utf8_lossy(&doc_xml);
    extract_from_docx_xml(&xml_str)
}

/// Extract text content from DOCX document.xml.
/// DOCX stores text in <w:t> elements. We extract all <w:t> content and
/// insert newlines for <w:p> (paragraph) boundaries.
fn extract_from_docx_xml(xml: &str) -> Result<String, String> {
    let mut output = String::new();
    let mut in_text = false;
    let mut in_paragraph = false;
    let mut text_content = String::new();
    let chars: Vec<char> = xml.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        // Check for <w:tab/> — tab character (must check before <w:t)
        if i + 8 <= chars.len() && chars[i..i + 8].iter().collect::<String>() == "<w:tab/>" {
            text_content.push('\t');
            i += 8;
            continue;
        }

        // Check for <w:br/> — line break (must check before <w:t)
        if i + 7 <= chars.len() && chars[i..i + 7].iter().collect::<String>() == "<w:br/>" {
            text_content.push('\n');
            i += 7;
            continue;
        }

        // Check for <w:p> or <w:p ...> — paragraph start
        if i + 4 <= chars.len() && chars[i..i + 4].iter().collect::<String>() == "<w:p" {
            in_paragraph = true;
            // If we had pending text from previous paragraph, flush it
            if !text_content.is_empty() {
                output.push_str(text_content.trim());
                output.push('\n');
                text_content.clear();
            }
        }

        // Check for </w:p> — paragraph end
        if i + 6 <= chars.len() && chars[i..i + 6].iter().collect::<String>() == "</w:p>" {
            if in_paragraph {
                if !text_content.is_empty() {
                    output.push_str(text_content.trim());
                    output.push('\n');
                    text_content.clear();
                }
                in_paragraph = false;
            }
        }

        // Check for <w:t> or <w:t ...> — text start
        if i + 4 <= chars.len() && chars[i..i + 4].iter().collect::<String>() == "<w:t" {
            // Skip to end of opening tag
            let mut j = i + 4;
            while j < chars.len() && chars[j] != '>' {
                j += 1;
            }
            if j < chars.len() {
                i = j + 1;
                in_text = true;
                continue;
            }
        }

        // Check for </w:t> — text end
        if i + 6 <= chars.len() && chars[i..i + 6].iter().collect::<String>() == "</w:t>" {
            in_text = false;
            i += 6;
            continue;
        }

        if in_text {
            text_content.push(chars[i]);
        }
        i += 1;
    }

    // Flush any remaining text
    if !text_content.is_empty() {
        output.push_str(text_content.trim());
        output.push('\n');
    }

    Ok(output)
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
            "fastshell_doctotext_test_{}_{}",
            std::process::id(),
            n
        ));
        let _ = fs::remove_dir_all(&dir);
        let vfs = crate::vfs::Vfs::new(dir).unwrap();
        Shell::new(vfs)
    }

    fn make_minimal_docx() -> Vec<u8> {
        let doc_xml = br#"<?xml version="1.0"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>Hello World</w:t></w:r></w:p>
    <w:p><w:r><w:t>This is a test document.</w:t></w:r></w:p>
  </w:body>
</w:document>"#;

        let content_types = br#"<?xml version="1.0"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"#;

        let rels = br#"<?xml version="1.0"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#;

        let cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let options =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        writer.start_file("[Content_Types].xml", options).unwrap();
        writer.write_all(content_types).unwrap();
        writer.start_file("_rels/.rels", options).unwrap();
        writer.write_all(rels).unwrap();
        writer.start_file("word/document.xml", options).unwrap();
        writer.write_all(doc_xml).unwrap();

        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn test_doctotext_help() {
        let mut s = mk_shell();
        let out = s.execute("doctotext", &["-h"], None);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("Usage: doctotext"));
    }

    #[test]
    fn test_doctotext_extract() {
        let mut s = mk_shell();
        let data = make_minimal_docx();
        let file = "test.docx";
        s.vfs.write_bytes(file, &s.cwd, &data).unwrap();
        let out = s.execute("doctotext", &[file], None);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("Hello World"));
        assert!(out.stdout.contains("test document"));
    }

    #[test]
    fn test_extract_from_docx_xml_basic() {
        let xml = r#"<w:document><w:body><w:p><w:r><w:t>Hello</w:t></w:r></w:p></w:body></w:document>"#;
        let result = extract_from_docx_xml(xml).unwrap();
        assert_eq!(result, "Hello\n");
    }

    #[test]
    fn test_extract_from_docx_xml_with_formatting() {
        let xml = r#"<w:document><w:body>
            <w:p><w:r><w:rPr/><w:t>First</w:t></w:r></w:p>
            <w:p><w:r><w:t>Second</w:t><w:tab/><w:t>tabbed</w:t></w:r></w:p>
            <w:p><w:r><w:t>Line1</w:t><w:br/><w:t>Line2</w:t></w:r></w:p>
        </w:body></w:document>"#;
        let result = extract_from_docx_xml(xml).unwrap();
        assert!(result.contains("First"));
        assert!(result.contains("Second\ttabbed"));
        assert!(result.contains("Line1\nLine2"));
    }
}
