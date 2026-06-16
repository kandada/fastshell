use crate::shell::{Shell, CommandOutput};

impl Shell {
    pub fn cmd_file(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let files: Vec<&str> = args.iter().filter(|a| !a.starts_with('-')).copied().collect();

        let mut output = String::new();
        if files.is_empty() {
            if let Some(input) = stdin {
                let bytes = input.as_bytes();
                let ftype = detect_type(bytes);
                output.push_str(&format!("(stdin): {}\n", ftype));
            }
        } else {
            for file in &files {
                match self.vfs.read(file, &self.cwd) {
                    Ok(data) => {
                        let ftype = detect_type(&data);
                        output.push_str(&format!("{}: {}\n", file, ftype));
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
                "-t" => {},
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

        let lines: Vec<Vec<&str>> = input.lines()
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
        if args.is_empty() {
            return CommandOutput::error("seq: missing operand\n".to_string(), 1);
        }

        let (first, step, last) = match args.len() {
            1 => {
                let last: f64 = args[0].parse().unwrap_or(1.0);
                (1.0, 1.0, last)
            }
            2 => {
                let first: f64 = args[0].parse().unwrap_or(1.0);
                let last: f64 = args[1].parse().unwrap_or(1.0);
                (first, 1.0, last)
            }
            _ => {
                let first: f64 = args[0].parse().unwrap_or(1.0);
                let step: f64 = args[1].parse().unwrap_or(1.0);
                let last: f64 = args[2].parse().unwrap_or(1.0);
                (first, step, last)
            }
        };

        let mut output = String::new();
        let mut val = first;
        if step > 0.0 {
            while val <= last + 1e-10 {
                output.push_str(&format!("{}\n", format_seq_value(val)));
                val += step;
            }
        } else if step < 0.0 {
            while val >= last - 1e-10 {
                output.push_str(&format!("{}\n", format_seq_value(val)));
                val += step;
            }
        }

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

fn detect_type(data: &[u8]) -> String {
    if data.is_empty() {
        return "empty".to_string();
    }

    let text_chars = data.iter().filter(|&&b| b >= 0x20 || b == b'\n' || b == b'\r' || b == b'\t').count();
    if text_chars as f64 / data.len() as f64 > 0.95 {
        if data.starts_with(b"{") || data.starts_with(b"[") {
            return "JSON text".to_string();
        }
        if data.starts_with(b"<") {
            if data.starts_with(b"<?xml") || data.starts_with(b"<!DOCTYPE") || data.starts_with(b"<html") {
                return "HTML/XML text".to_string();
            }
        }
        if data.iter().any(|&b| b == b';') && data.starts_with(b"#") {
            return "script text".to_string();
        }
        return "ASCII text".to_string();
    }

    match &data[..std::cmp::min(8, data.len())] {
        [0x89, b'P', b'N', b'G', ..] => return "PNG image".to_string(),
        [0xFF, 0xD8, 0xFF, ..] => return "JPEG image".to_string(),
        [b'G', b'I', b'F', b'8', ..] => return "GIF image".to_string(),
        [0x1F, 0x8B, ..] => return "gzip compressed".to_string(),
        [0x1F, 0x9D, ..] => return "compress'd data".to_string(),
        [b'B', b'Z', b'h', ..] => return "bzip2 compressed".to_string(),
        [0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00, ..] => return "XZ compressed".to_string(),
        [b'P', b'K', 0x03, 0x04, ..] => return "Zip archive".to_string(),
        [b'P', b'K', 0x05, 0x06, ..] => return "Zip archive (empty)".to_string(),
        [0x75, 0x73, 0x74, 0x61, 0x72, ..] => return "tar archive (POSIX)".to_string(),
        [0x7F, b'E', b'L', b'F', ..] => return "ELF binary".to_string(),
        [0xCF, 0xFA, 0xED, 0xFE, ..] | [0xFE, 0xED, 0xFA, 0xCF, ..] => return "Mach-O binary".to_string(),
        [0xCA, 0xFE, 0xBA, 0xBE, ..] => return "Mach-O fat binary".to_string(),
        [b'S', b'Q', b'L', b'i', b't', b'e', ..] => return "SQLite database".to_string(),
        [0x25, 0x50, 0x44, 0x46, ..] => return "PDF document".to_string(),
        [b'R', b'a', b'r', b'!', ..] => return "RAR archive".to_string(),
        [0x00, 0x00, 0x01, 0xBA, ..] | [0x00, 0x00, 0x01, 0xB3, ..] => return "MPEG video".to_string(),
        _ => {}
    }

    "data".to_string()
}
