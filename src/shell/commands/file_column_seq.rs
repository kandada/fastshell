// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};
use std::io::Cursor;

const FILE_HELP_TEXT: &str = "\
Usage: file [OPTION]... [FILE]...
Determine file type.

  -b            brief mode (no filename prefix)
  -i, --mime    output MIME type strings
  -h, --help    display this help and exit
";

const ZIP_INSPECT_MAX_SIZE: usize = 50 * 1024 * 1024; // 50 MB

impl Shell {
    pub fn cmd_file(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut brief = false;
        let mut mime = false;
        let mut files: Vec<&str> = Vec::new();

        for &arg in args {
            match arg {
                "-h" | "--help" => return CommandOutput::success(FILE_HELP_TEXT.to_string()),
                "-b" => brief = true,
                "-i" | "--mime" => mime = true,
                a if a.starts_with('-') => {
                    crate::warn!("file: warning: unsupported option '{}'", a);
                }
                _ => files.push(arg),
            }
        }

        let mut output = String::new();
        if files.is_empty() {
            if let Some(input) = stdin {
                let bytes = input.as_bytes();
                let ftype = detect_type(bytes, mime);
                if brief || mime {
                    output.push_str(&format!("{}\n", ftype));
                } else {
                    output.push_str(&format!("(stdin): {}\n", ftype));
                }
            }
        } else {
            for file in &files {
                match self.vfs.read(file, &self.cwd) {
                    Ok(data) => {
                        let ftype = detect_type(&data, mime);
                        if mime || brief {
                            output.push_str(&format!("{}\n", ftype));
                        } else {
                            output.push_str(&format!("{}: {}\n", file, ftype));
                        }
                    }
                    Err(e) => {
                        output.push_str(&format!("{}: {}\n", file, e));
                    }
                }
            }
        }

        CommandOutput::success(output)
    }

    pub fn cmd_column(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut delimiter = ' ';
        let mut files = Vec::new();

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-t" => {}
                "-s" => {
                    if i + 1 < args.len() {
                        delimiter = args[i + 1].chars().next().unwrap_or(' ');
                        i += 1;
                    }
                }
                arg if arg.starts_with("-s") && arg.len() > 2 => {
                    delimiter = arg[2..].chars().next().unwrap_or(' ');
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
            i += 1;
        }

        let input = if files.is_empty() {
            match stdin {
                Some(s) => s.to_string(),
                None => return CommandOutput::error("column: missing input\n".to_string(), 1),
            }
        } else {
            let mut content = String::new();
            for file in &files {
                match self.vfs.read_to_string(file, &self.cwd) {
                    Ok(c) => content.push_str(&c),
                    Err(e) => return CommandOutput::error(format!("column: {}: {}\n", file, e), 1),
                }
            }
            content
        };

        let lines: Vec<Vec<&str>> = input
            .lines()
            .map(|line| line.split(delimiter).collect())
            .collect();

        if lines.is_empty() {
            return CommandOutput::success(String::new());
        }

        let max_cols = lines.iter().map(|r| r.len()).max().unwrap_or(0);
        let mut widths = vec![0usize; max_cols];
        for row in &lines {
            for (j, col) in row.iter().enumerate() {
                widths[j] = widths[j].max(col.len());
            }
        }

        let mut output = String::new();
        for row in &lines {
            let mut parts = Vec::new();
            for (j, col) in row.iter().enumerate() {
                if j < widths.len() - 1 {
                    parts.push(format!("{:<width$}", col, width = widths[j] + 2));
                } else {
                    parts.push(col.to_string());
                }
            }
            output.push_str(&parts.join(""));
            output.push('\n');
        }

        CommandOutput::success(output)
    }

    pub fn cmd_seq(&self, args: &[&str]) -> CommandOutput {
        let mut equal_width = false;
        let mut separator: Option<String> = None;
        let mut format: Option<String> = None;
        let mut nums: Vec<&str> = Vec::new();

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-w" | "--equal-width" => equal_width = true,
                "-s" | "--separator" => {
                    if i + 1 < args.len() {
                        separator = Some(args[i + 1].to_string());
                        i += 1;
                    }
                }
                "-f" | "--format" => {
                    if i + 1 < args.len() {
                        format = Some(args[i + 1].to_string());
                        i += 1;
                    }
                }
                a if a.starts_with("-s") && a.len() > 2 => {
                    separator = Some(a[2..].to_string());
                }
                a if a.starts_with("-f") && a.len() > 2 => {
                    format = Some(a[2..].to_string());
                }
                a if a.starts_with('-') && a.len() > 1 => {
                    // A leading '-' followed by a number is an operand, not an
                    // option: `seq 5 -1 1` counts down.
                    if a[1..].parse::<f64>().is_ok() {
                        nums.push(a);
                    } else {
                        crate::warn!("seq: warning: unsupported option '{}'", a);
                    }
                }
                _ => nums.push(args[i]),
            }
            i += 1;
        }

        if nums.is_empty() {
            return CommandOutput::error("seq: missing operand\n".to_string(), 1);
        }

        let (first, step, last) = match nums.len() {
            1 => {
                let last: f64 = nums[0].parse().unwrap_or(1.0);
                (1.0, 1.0, last)
            }
            2 => {
                let first: f64 = nums[0].parse().unwrap_or(1.0);
                let last: f64 = nums[1].parse().unwrap_or(1.0);
                (first, 1.0, last)
            }
            _ => {
                let first: f64 = nums[0].parse().unwrap_or(1.0);
                let step: f64 = nums[1].parse().unwrap_or(1.0);
                let last: f64 = nums[2].parse().unwrap_or(1.0);
                (first, step, last)
            }
        };

        // Collect all values first (needed for -w width and -s separator).
        let mut values: Vec<String> = Vec::new();
        let mut val = first;
        if step > 0.0 {
            while val <= last + 1e-10 {
                values.push(format_seq_value(val));
                val += step;
            }
        } else if step < 0.0 {
            while val >= last - 1e-10 {
                values.push(format_seq_value(val));
                val += step;
            }
        }

        // Apply format if given.
        if let Some(ref fmt) = format {
            values = values
                .iter()
                .map(|v| seq_format(v, fmt))
                .collect();
        }

        // Equal width: left-pad with the width of the widest value.
        if equal_width {
            let width = values.iter().map(|v| v.chars().count()).max().unwrap_or(0);
            values = values
                .iter()
                .map(|v| {
                    let pad = width.saturating_sub(v.chars().count());
                    let mut out = String::new();
                    for _ in 0..pad {
                        out.push('0');
                    }
                    out.push_str(v);
                    out
                })
                .collect();
        }

        let sep = separator.unwrap_or_else(|| "\n".to_string());
        let mut output = values.join(&sep);
        output.push('\n');
        CommandOutput::success(output)
    }
}

fn format_seq_value(val: f64) -> String {
    if (val - val.round()).abs() < 1e-10 {
        format!("{}", val as i64)
    } else {
        format!("{}", val)
    }
}

/// Apply a `seq -f` printf-style format to a value string. Supports the
/// common `%g`/`%e`/`%f` conversions plus a literal prefix/suffix.
fn seq_format(value: &str, fmt: &str) -> String {
    if fmt.contains('%') {
        // Try to parse the numeric value back for float formatting.
        if let Ok(f) = value.parse::<f64>() {
            // Minimal printf: replace %g/%e/%f/%d with the value.
            let mut out = fmt.to_string();
            for (spec, rendered) in [
                ("%g", format!("{}", f)),
                ("%f", format!("{}", f)),
                ("%e", format!("{:e}", f)),
                ("%d", format!("{}", f as i64)),
            ] {
                out = out.replace(spec, &rendered);
            }
            if out != *fmt {
                return out;
            }
        }
    }
    // No format spec: `seq -f 'prefix'` prints the literal + value (GNU quirk).
    format!("{}{}", fmt, value)
}

fn detect_type(data: &[u8], mime: bool) -> String {
    if data.is_empty() {
        return if mime { "inode/x-empty; charset=binary".to_string() } else { "empty".to_string() };
    }

    // Check magic bytes first (binary formats always take precedence)
    let magic_result = match_magic(data);

    // If magic matched a known binary format, return it (with mime override)
    if magic_result != "data" {
        return if mime {
            ftype_to_mime(&magic_result)
        } else {
            magic_result
        };
    }

    // Only classify as text if no known binary format matched
    let text_chars = data
        .iter()
        .filter(|&&b| b >= 0x20 || b == b'\n' || b == b'\r' || b == b'\t')
        .count();
    if text_chars as f64 / data.len() as f64 > 0.95 {
        if data.starts_with(b"{") || data.starts_with(b"[") {
            return if mime { "application/json; charset=utf-8".to_string() } else { "JSON text".to_string() };
        }
        if data.starts_with(b"<") {
            if data.starts_with(b"<?xml")
                || data.starts_with(b"<!DOCTYPE")
                || data.starts_with(b"<html")
            {
                return if mime { "text/html; charset=utf-8".to_string() } else { "HTML/XML text".to_string() };
            }
        }
        if data.iter().any(|&b| b == b';') && data.starts_with(b"#") {
            return if mime { "text/x-script; charset=utf-8".to_string() } else { "script text".to_string() };
        }
        return if mime { "text/plain; charset=utf-8".to_string() } else { "ASCII text".to_string() };
    }

    if mime { ftype_to_mime("data") } else { "data".to_string() }
}

fn match_magic(data: &[u8]) -> String {
    let end = std::cmp::min(16, data.len());
    let magic = &data[..end];

    match magic {
        [0x89, b'P', b'N', b'G', ..] => return "PNG image".to_string(),
        [0xFF, 0xD8, 0xFF, ..] => return "JPEG image".to_string(),
        [b'G', b'I', b'F', b'8', ..] => return "GIF image".to_string(),
        [0x1F, 0x8B, ..] => return "gzip compressed".to_string(),
        [0x1F, 0x9D, ..] => return "compress'd data".to_string(),
        [b'B', b'Z', b'h', ..] => return "bzip2 compressed".to_string(),
        [0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00, ..] => return "XZ compressed".to_string(),
        [b'P', b'K', 0x03, 0x04, ..] | [b'P', b'K', 0x05, 0x06, ..] => {
            if data.len() <= ZIP_INSPECT_MAX_SIZE {
                if let Some(container) = detect_zip_container(data) {
                    return container;
                }
            }
            return "Zip archive".to_string();
        }
        [0x75, 0x73, 0x74, 0x61, 0x72, ..] => return "tar archive (POSIX)".to_string(),
        [0x7F, b'E', b'L', b'F', ..] => return "ELF binary".to_string(),
        [0xCF, 0xFA, 0xED, 0xFE, ..] | [0xFE, 0xED, 0xFA, 0xCF, ..] => {
            return "Mach-O binary".to_string()
        }
        [0xCA, 0xFE, 0xBA, 0xBE, ..] => return "Mach-O fat binary".to_string(),
        [b'S', b'Q', b'L', b'i', b't', b'e', ..] => return "SQLite database".to_string(),
        [0x25, 0x50, 0x44, 0x46, ..] => return extract_pdf_version(data),
        [b'R', b'a', b'r', b'!', ..] => return "RAR archive".to_string(),
        [0x00, 0x00, 0x01, 0xBA, ..] | [0x00, 0x00, 0x01, 0xB3, ..] => {
            return "MPEG video".to_string()
        }
        // HEIC/HEIF detection — ftyp box with heic/heix/hevc/heif/mif1 brand
        [0x00, 0x00, 0x00, _, b'f', b't', b'y', b'p', ..] if data.len() >= 16 => {
            let brand = &data[8..12];
            match brand {
                b"heic" | b"heix" | b"hevc" | b"heim" | b"heis" | b"heif" | b"mif1" | b"msf1" => {
                    return "HEIC image".to_string();
                }
                _ => {}
            }
        }
        // WEBP — RIFF....WEBP
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => {
            return "Web/P image".to_string();
        }
        // MOBI / AZW — PalmDB header with MOBI type
        [b'B', b'O', b'O', b'K', b'M', b'O', b'B', b'I', ..] => {
            return "Mobipocket e-book".to_string();
        }
        _ => {}
    }

    "data".to_string()
}

fn extract_pdf_version(data: &[u8]) -> String {
    let header = std::str::from_utf8(&data[..std::cmp::min(16, data.len())]).unwrap_or("");
    if let Some(version_line) = header.lines().next() {
        if let Some(ver) = version_line.strip_prefix("%PDF-") {
            let ver = ver.trim();
            if !ver.is_empty() {
                return format!("PDF document, version {}", ver);
            }
        }
    }
    "PDF document".to_string()
}

fn detect_zip_container(data: &[u8]) -> Option<String> {
    let cursor = Cursor::new(data);
    let mut archive = zip::ZipArchive::new(cursor).ok()?;

    let mut has_word_doc = false;
    let mut has_epub_container = false;
    let mut has_content_xml = false;
    let mut has_manifest = false;
    let mut has_android_manifest = false;
    let mut has_ppt_presentation = false;
    let mut has_xl_workbook = false;

    for i in 0..archive.len() {
        let entry = match archive.by_index(i) {
            Ok(e) => e,
            Err(_) => continue,
        };
        match entry.name() {
            "word/document.xml" => has_word_doc = true,
            "META-INF/container.xml" => has_epub_container = true,
            "content.xml" => has_content_xml = true,
            "META-INF/manifest.xml" => has_manifest = true,
            "AndroidManifest.xml" => has_android_manifest = true,
            "ppt/presentation.xml" => has_ppt_presentation = true,
            "xl/workbook.xml" => has_xl_workbook = true,
            _ => {}
        }
    }

    if has_content_xml && has_manifest {
        return Some("OpenDocument Text".to_string());
    }
    if has_word_doc {
        return Some("Microsoft Word 2007+".to_string());
    }
    if has_epub_container {
        return Some("EPUB document".to_string());
    }
    if has_android_manifest {
        return Some("Android package".to_string());
    }
    if has_ppt_presentation {
        return Some("Microsoft PowerPoint 2007+".to_string());
    }
    if has_xl_workbook {
        return Some("Microsoft Excel 2007+".to_string());
    }

    None
}

fn ftype_to_mime(ftype: &str) -> String {
    if ftype.starts_with("PDF document") {
        "application/pdf; charset=binary".to_string()
    } else if ftype == "PNG image" {
        "image/png; charset=binary".to_string()
    } else if ftype == "JPEG image" {
        "image/jpeg; charset=binary".to_string()
    } else if ftype == "GIF image" {
        "image/gif; charset=binary".to_string()
    } else if ftype == "HEIC image" {
        "image/heic; charset=binary".to_string()
    } else if ftype == "Web/P image" {
        "image/webp; charset=binary".to_string()
    } else if ftype == "gzip compressed" {
        "application/gzip; charset=binary".to_string()
    } else if ftype == "bzip2 compressed" {
        "application/x-bzip2; charset=binary".to_string()
    } else if ftype == "XZ compressed" {
        "application/x-xz; charset=binary".to_string()
    } else if ftype == "Zip archive" {
        "application/zip; charset=binary".to_string()
    } else if ftype == "Microsoft Word 2007+" {
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document; charset=binary".to_string()
    } else if ftype == "Microsoft Excel 2007+" {
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet; charset=binary".to_string()
    } else if ftype == "Microsoft PowerPoint 2007+" {
        "application/vnd.openxmlformats-officedocument.presentationml.presentation; charset=binary".to_string()
    } else if ftype == "OpenDocument Text" {
        "application/vnd.oasis.opendocument.text; charset=binary".to_string()
    } else if ftype == "EPUB document" {
        "application/epub+zip; charset=binary".to_string()
    } else if ftype == "Android package" {
        "application/vnd.android.package-archive; charset=binary".to_string()
    } else if ftype == "Mobipocket e-book" {
        "application/x-mobipocket-ebook; charset=binary".to_string()
    } else if ftype == "SQLite database" {
        "application/vnd.sqlite3; charset=binary".to_string()
    } else if ftype == "JSON text" {
        "application/json; charset=utf-8".to_string()
    } else if ftype == "HTML/XML text" {
        "text/html; charset=utf-8".to_string()
    } else if ftype.starts_with("script text") || ftype == "ASCII text" {
        "text/plain; charset=utf-8".to_string()
    } else {
        format!("application/octet-stream; charset=binary")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- detect_type tests ---

    #[test]
    fn test_detect_empty() {
        assert_eq!(detect_type(b"", false), "empty");
        assert_eq!(detect_type(b"", true), "inode/x-empty; charset=binary");
    }

    #[test]
    fn test_detect_ascii_text() {
        let data = b"Hello, world!\nThis is a test.\n";
        assert_eq!(detect_type(data, false), "ASCII text");
        assert_eq!(detect_type(data, true), "text/plain; charset=utf-8");
    }

    #[test]
    fn test_detect_json() {
        let data = b"{\"key\": \"value\"}\n";
        assert_eq!(detect_type(data, false), "JSON text");
    }

    #[test]
    fn test_detect_pdf_v1() {
        let mut data = b"%PDF-1.4\n".to_vec();
        data.extend(vec![0u8; 256]);
        assert_eq!(detect_type(&data, false), "PDF document, version 1.4");
    }

    #[test]
    fn test_detect_pdf_no_version() {
        let mut data = b"%PDF-\n".to_vec();
        data.extend(vec![0u8; 256]);
        assert_eq!(detect_type(&data, false), "PDF document");
    }

    #[test]
    fn test_detect_png() {
        let data = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";
        assert_eq!(detect_type(data, false), "PNG image");
    }

    #[test]
    fn test_detect_jpeg() {
        let data = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46];
        assert_eq!(detect_type(&data, false), "JPEG image");
    }

    #[test]
    fn test_detect_gif() {
        let data = b"GIF89a";
        assert_eq!(detect_type(data, false), "GIF image");
    }

    #[test]
    fn test_detect_heic() {
        let mut data = vec![0x00, 0x00, 0x00, 0x18];
        data.extend(b"ftypheic");
        data.extend(vec![0u8; 16]);
        assert_eq!(detect_type(&data, false), "HEIC image");
    }

    #[test]
    fn test_detect_heif() {
        let mut data = vec![0x00, 0x00, 0x00, 0x18];
        data.extend(b"ftypmif1");
        data.extend(vec![0u8; 16]);
        assert_eq!(detect_type(&data, false), "HEIC image");
    }

    #[test]
    fn test_detect_webp() {
        let mut data = b"RIFF".to_vec();
        data.extend(vec![0x00, 0x00, 0x00, 0x00]);
        data.extend(b"WEBP");
        assert_eq!(detect_type(&data, false), "Web/P image");
    }

    #[test]
    fn test_detect_mobi() {
        let mut data = b"BOOKMOBI".to_vec();
        data.extend(vec![0u8; 256]);
        assert_eq!(detect_type(&data, false), "Mobipocket e-book");
    }

    #[test]
    fn test_detect_zip() {
        let data = [0x50, 0x4B, 0x03, 0x04, 0x00, 0x00, 0x00, 0x00];
        assert_eq!(detect_type(&data, false), "Zip archive");
    }

    #[test]
    fn test_mime_pdf() {
        let mut data = b"%PDF-1.4\n".to_vec();
        data.extend(vec![0u8; 256]);
        assert_eq!(detect_type(&data, true), "application/pdf; charset=binary");
    }

    #[test]
    fn test_mime_png() {
        let data = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";
        assert_eq!(detect_type(&data[..], true), "image/png; charset=binary");
    }

    // --- seq tests ---

    #[test]
    fn test_seq_basic() {
        let mut s = crate::shell::Shell::new(crate::vfs::Vfs::new(
            std::env::temp_dir().join(format!("seq_test_{}", std::process::id())),
        ).unwrap());
        let out = s.execute("seq", &["3"], None);
        assert_eq!(out.stdout, "1\n2\n3\n");
    }

    #[test]
    fn test_seq_equal_width() {
        let mut s = crate::shell::Shell::new(crate::vfs::Vfs::new(
            std::env::temp_dir().join(format!("seq_w_test_{}", std::process::id())),
        ).unwrap());
        let out = s.execute("seq", &["-w", "8", "10"], None);
        assert_eq!(out.stdout, "08\n09\n10\n");
    }

    #[test]
    fn test_seq_separator() {
        let mut s = crate::shell::Shell::new(crate::vfs::Vfs::new(
            std::env::temp_dir().join(format!("seq_s_test_{}", std::process::id())),
        ).unwrap());
        let out = s.execute("seq", &["-s", ",", "1", "3"], None);
        assert_eq!(out.stdout, "1,2,3\n");
    }
}
