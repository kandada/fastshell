// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

impl Shell {
    pub fn cmd_shuf(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut count: Option<usize> = None;
        let mut echo_mode = false;
        let mut echo_args = Vec::new();
        let mut range: Option<(i64, i64)> = None;

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-n" => {
                    if i + 1 < args.len() {
                        count = args[i + 1].parse().ok();
                        i += 1;
                    }
                }
                "-e" => echo_mode = true,
                "-i" | "--input-range" => {
                    if i + 1 < args.len() {
                        i += 1;
                        if let Some((a, b)) = args[i].split_once('-') {
                            if let (Ok(lo), Ok(hi)) =
                                (a.trim().parse::<i64>(), b.trim().parse::<i64>())
                            {
                                range = Some((lo, hi));
                            }
                        }
                    }
                }
                arg if !arg.starts_with('-') && echo_mode => echo_args.push(arg.to_string()),
                _ => {}
            }
            i += 1;
        }

        let mut items: Vec<String> = if let Some((lo, hi)) = range {
            (lo..=hi).map(|n| n.to_string()).collect()
        } else if echo_mode {
            echo_args
        } else {
            let input = match stdin {
                Some(s) => s.to_string(),
                None => return CommandOutput::error("shuf: missing input\n".to_string(), 1),
            };
            input.lines().map(|l| l.to_string()).collect()
        };

        shuffle(&mut items);

        let take = count.unwrap_or(items.len()).min(items.len());
        let mut output = String::new();
        for item in items.iter().take(take) {
            output.push_str(item);
            output.push('\n');
        }

        CommandOutput::success(output)
    }

    pub fn cmd_uuidgen(&self, _args: &[&str]) -> CommandOutput {
        let id = uuid::Uuid::new_v4().to_string();
        CommandOutput::success(id + "\n")
    }

    pub fn cmd_rev(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let input = if args.iter().any(|a| !a.starts_with('-')) {
            let mut content = String::new();
            for arg in args {
                if !arg.starts_with('-') {
                    match self.read_text_lossy(arg) {
                        Ok(c) => content.push_str(&c),
                        Err(e) => return CommandOutput::error(format!("rev: {}: {}\n", arg, e), 1),
                    }
                }
            }
            content
        } else {
            match stdin {
                Some(s) => s.to_string(),
                None => return CommandOutput::error("rev: missing input\n".to_string(), 1),
            }
        };

        let mut output = String::new();
        for line in input.lines() {
            let reversed: String = line.chars().rev().collect();
            output.push_str(&reversed);
            output.push('\n');
        }

        CommandOutput::success(output)
    }

    pub fn cmd_split(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut lines_per_file = 1000usize;
        let mut by_lines = true;
        // Positional operands follow GNU `split [OPTION]... [FILE [PREFIX]]`;
        // `-` means read stdin. Both a stdin `-` and a following PREFIX must be
        // kept (previously `-` was dropped as an unknown option, so the PREFIX
        // was misinterpreted as the input FILE).
        let mut positional: Vec<String> = Vec::new();

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-l" => {
                    if i + 1 < args.len() {
                        lines_per_file = args[i + 1].parse().unwrap_or(1000);
                        by_lines = true;
                        i += 1;
                    }
                }
                "-b" => {
                    if i + 1 < args.len() {
                        lines_per_file = parse_size(args[i + 1]);
                        by_lines = false;
                        i += 1;
                    }
                }
                arg if arg.starts_with("-l") && arg.len() > 2 => {
                    lines_per_file = arg[2..].parse().unwrap_or(1000);
                    by_lines = true;
                }
                arg if arg.starts_with("-b") && arg.len() > 2 => {
                    lines_per_file = parse_size(&arg[2..]);
                    by_lines = false;
                }
                arg if arg == "-" || !arg.starts_with('-') => positional.push(arg.to_string()),
                _ => {}
            }
            i += 1;
        }

        let prefix = positional
            .get(1)
            .cloned()
            .unwrap_or_else(|| "x".to_string());
        let input = match positional.first().map(|s| s.as_str()) {
            None | Some("-") => match stdin {
                Some(s) => s.to_string(),
                None => return CommandOutput::error("split: missing input\n".to_string(), 1),
            },
            Some(f) => match self.read_text_lossy(f) {
                Ok(c) => c,
                Err(e) => return CommandOutput::error(format!("split: {}: {}\n", f, e), 1),
            },
        };

        let mut output = String::new();
        if by_lines {
            let lines: Vec<&str> = input.lines().collect();
            let mut chunk_idx = 0;
            for chunk in lines.chunks(lines_per_file) {
                let name = format!("{}{}", prefix, suffix(chunk_idx));
                let content = chunk.join("\n") + "\n";
                match self.vfs.write(&name, &self.cwd, &content) {
                    Ok(_) => {}
                    Err(e) => {
                        output.push_str(&format!("split: {}: {}\n", name, e));
                    }
                }
                chunk_idx += 1;
            }
        } else {
            // Byte mode (`split -b N`): chunk the raw bytes.
            let bytes = input.as_bytes();
            let size = lines_per_file.max(1);
            let mut chunk_idx = 0;
            for chunk in bytes.chunks(size) {
                let name = format!("{}{}", prefix, suffix(chunk_idx));
                if let Err(e) = self.vfs.write_bytes(&name, &self.cwd, chunk) {
                    output.push_str(&format!("split: {}: {}\n", name, e));
                }
                chunk_idx += 1;
            }
        }

        if output.is_empty() {
            CommandOutput::success(String::new())
        } else {
            CommandOutput {
                stdout: String::new(),
                stderr: output,
                exit_code: 1,
            }
        }
    }

    pub fn cmd_comm(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let files: Vec<&str> = args
            .iter()
            .filter(|a| !a.starts_with('-'))
            .copied()
            .collect();

        if files.len() < 2 && stdin.is_none() {
            return CommandOutput::error("comm: missing file operands\n".to_string(), 1);
        }

        let lines1 = if files.is_empty() {
            match stdin {
                Some(s) => s.lines().map(|l| l.to_string()).collect(),
                None => vec![],
            }
        } else {
            match self.read_text_lossy(files[0]) {
                Ok(c) => c.lines().map(|l| l.to_string()).collect(),
                Err(e) => return CommandOutput::error(format!("comm: {}: {}\n", files[0], e), 1),
            }
        };

        let lines2 = if files.len() > 1 {
            match self.read_text_lossy(files[1]) {
                Ok(c) => c.lines().map(|l| l.to_string()).collect(),
                Err(e) => return CommandOutput::error(format!("comm: {}: {}\n", files[1], e), 1),
            }
        } else {
            vec![]
        };

        let mut output = String::new();
        let mut i = 0;
        let mut j = 0;

        // `-1` / `-2` / `-3` suppress the respective column (can be combined,
        // e.g. `comm -12`).
        let flags: Vec<&str> = args
            .iter()
            .filter(|a| a.starts_with('-'))
            .copied()
            .collect();
        let suppressed = |col: char| flags.iter().any(|f| f.contains(col));
        let (s1, s2, s3) = (suppressed('1'), suppressed('2'), suppressed('3'));
        // A suppressed column does NOT contribute its tab (bash `comm -12` prints
        // common lines with no leading tab).
        let p2 = if s1 { "" } else { "\t" };
        let p3 = format!(
            "{}{}",
            if s1 { "" } else { "\t" },
            if s2 { "" } else { "\t" }
        );

        while i < lines1.len() || j < lines2.len() {
            if i >= lines1.len() {
                // only in file2 → column 2
                if !s2 {
                    output.push_str(&format!("{}{}\n", p2, lines2[j]));
                }
                j += 1;
            } else if j >= lines2.len() {
                // only in file1 → column 1 (no tab)
                if !s1 {
                    output.push_str(&format!("{}\n", lines1[i]));
                }
                i += 1;
            } else {
                match lines1[i].cmp(&lines2[j]) {
                    std::cmp::Ordering::Less => {
                        if !s1 {
                            output.push_str(&format!("{}\n", lines1[i]));
                        }
                        i += 1;
                    }
                    std::cmp::Ordering::Greater => {
                        if !s2 {
                            output.push_str(&format!("{}{}\n", p2, lines2[j]));
                        }
                        j += 1;
                    }
                    std::cmp::Ordering::Equal => {
                        if !s3 {
                            output.push_str(&format!("{}{}\n", p3, lines1[i]));
                        }
                        i += 1;
                        j += 1;
                    }
                }
            }
        }

        CommandOutput::success(output)
    }

    pub fn cmd_xxd(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut reverse = false;
        let mut plain = false;
        let mut limit: Option<usize> = None;
        let mut seek: usize = 0;
        let mut files = Vec::new();

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-r" => reverse = true,
                "-p" => plain = true,
                "-l" => {
                    if i + 1 < args.len() {
                        limit = args[i + 1].parse().ok();
                        i += 1;
                    }
                }
                "-s" => {
                    if i + 1 < args.len() {
                        seek = parse_offset(args[i + 1]);
                        i += 1;
                    }
                }
                arg if arg.starts_with("-l") && arg.len() > 2 => {
                    limit = arg[2..].parse().ok();
                }
                arg if arg.starts_with("-s") && arg.len() > 2 => {
                    seek = parse_offset(&arg[2..]);
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
            i += 1;
        }

        let data = if files.is_empty() {
            match stdin {
                Some(s) => self
                    .take_binary_in()
                    .unwrap_or_else(|| s.as_bytes().to_vec()),
                None => return CommandOutput::error("xxd: missing input\n".to_string(), 1),
            }
        } else {
            let mut all = Vec::new();
            for file in &files {
                match self.vfs.read(file, &self.cwd) {
                    Ok(d) => all.extend_from_slice(&d),
                    Err(e) => return CommandOutput::error(format!("xxd: {}: {}\n", file, e), 1),
                }
            }
            all
        };

        let data = if seek > 0 {
            if seek >= data.len() {
                &[] as &[u8]
            } else {
                &data[seek..]
            }
        } else {
            &data[..]
        };

        let data = if let Some(lim) = limit {
            if lim < data.len() {
                &data[..lim]
            } else {
                data
            }
        } else {
            data
        };

        if reverse {
            let bytes = xxd_reverse_bytes(data);
            if !bytes.is_empty() {
                self.set_binary_out(bytes.clone());
            }
            return CommandOutput::success(String::from_utf8_lossy(&bytes).to_string());
        }

        if plain {
            return xxd_plain(data);
        }

        let mut output = String::new();
        let base_offset = seek;
        for (idx, chunk) in data.chunks(16).enumerate() {
            let offset = base_offset + idx * 16;
            output.push_str(&format!("{:08x}: ", offset));
            let mut hex_part = String::new();
            let mut ascii_part = String::new();
            for (j, &byte) in chunk.iter().enumerate() {
                hex_part.push_str(&format!("{:02x}", byte));
                if j % 2 == 1 {
                    hex_part.push(' ');
                }
                ascii_part.push(if byte >= 0x20 && byte < 0x7f {
                    byte as char
                } else {
                    '.'
                });
            }
            if chunk.len() < 16 {
                let padding = (16 - chunk.len()) * 2 + (16 - chunk.len()) / 2;
                hex_part.push_str(&" ".repeat(padding));
            }
            output.push_str(&format!("{:<50} {}\n", hex_part, ascii_part));
        }

        CommandOutput::success(output)
    }

    pub fn cmd_expr(&self, args: &[&str]) -> CommandOutput {
        if args.is_empty() {
            return CommandOutput {
                stdout: "0\n".to_string(),
                stderr: String::new(),
                exit_code: 1,
            };
        }

        let expr: Vec<&str> = args.to_vec();
        // `expr STRING : REGEX` — anchored regex match, returns match length.
        if expr.len() == 3 && expr[1] == ":" {
            let anchored = format!("^(?:{})", expr[2]);
            match regex::Regex::new(&anchored) {
                Ok(re) => {
                    return match re.find(expr[0]) {
                        Some(m) => {
                            CommandOutput::success(format!("{}\n", m.as_str().chars().count()))
                        }
                        None => CommandOutput {
                            stdout: "0\n".to_string(),
                            stderr: String::new(),
                            exit_code: 1,
                        },
                    };
                }
                Err(e) => return CommandOutput::error(format!("expr: {}\n", e), 1),
            }
        }
        // `expr match STRING REGEX` — same as `STRING : REGEX`.
        if expr.len() == 3 && expr[0] == "match" {
            let anchored = format!("^(?:{})", expr[2]);
            match regex::Regex::new(&anchored) {
                Ok(re) => {
                    return match re.find(expr[1]) {
                        Some(m) => {
                            CommandOutput::success(format!("{}\n", m.as_str().chars().count()))
                        }
                        None => CommandOutput {
                            stdout: "0\n".to_string(),
                            stderr: String::new(),
                            exit_code: 1,
                        },
                    };
                }
                Err(e) => return CommandOutput::error(format!("expr: {}\n", e), 1),
            }
        }
        // `expr substr STRING POS LENGTH` (1-based, POSIX).
        if expr.len() == 4 && expr[0] == "substr" {
            let chars: Vec<char> = expr[1].chars().collect();
            let pos: i64 = expr[2].parse().unwrap_or(1);
            let len: i64 = expr[3].parse().unwrap_or(0);
            let start = (pos - 1).max(0) as usize;
            let out: String = if start >= chars.len() || len <= 0 {
                String::new()
            } else {
                chars[start..(start + len as usize).min(chars.len())]
                    .iter()
                    .collect()
            };
            return CommandOutput::success(format!("{}\n", out));
        }
        // `expr index STRING CHARS` — 1-based index of the first matching char.
        if expr.len() == 3 && expr[0] == "index" {
            let idx = expr[1]
                .chars()
                .position(|c| expr[2].contains(c))
                .map(|i| i + 1)
                .unwrap_or(0);
            return CommandOutput {
                stdout: format!("{}\n", idx),
                stderr: String::new(),
                exit_code: if idx == 0 { 1 } else { 0 },
            };
        }
        // `expr length STRING`
        if expr.len() == 2 && expr[0] == "length" {
            return CommandOutput::success(format!("{}\n", expr[1].chars().count()));
        }
        // Comparison (GNU expr): numeric when both parse as ints, else string.
        if expr.len() == 3 {
            let (a, op, b) = (expr[0], expr[1], expr[2]);
            let (ai, bi) = (a.parse::<i64>().ok(), b.parse::<i64>().ok());
            let truth = match op {
                "=" | "==" => ai.zip(bi).map(|(x, y)| x == y).or(Some(a == b)),
                "!=" => ai.zip(bi).map(|(x, y)| x != y).or(Some(a != b)),
                "<" => ai.zip(bi).map(|(x, y)| x < y).or(Some(a < b)),
                ">" => ai.zip(bi).map(|(x, y)| x > y).or(Some(a > b)),
                "<=" => ai.zip(bi).map(|(x, y)| x <= y).or(Some(a <= b)),
                ">=" => ai.zip(bi).map(|(x, y)| x >= y).or(Some(a >= b)),
                _ => None,
            };
            if let Some(t) = truth {
                return CommandOutput {
                    stdout: format!("{}\n", if t { 1 } else { 0 }),
                    stderr: String::new(),
                    exit_code: if t { 0 } else { 1 },
                };
            }
        }
        match evaluate_expr(&expr) {
            Ok(val) => CommandOutput::success(format!("{}\n", val)),
            Err(e) => CommandOutput::error(format!("expr: {}\n", e), 1),
        }
    }
}

fn parse_offset(s: &str) -> usize {
    if s.starts_with("0x") || s.starts_with("0X") {
        usize::from_str_radix(&s[2..], 16).unwrap_or(0)
    } else {
        s.parse().unwrap_or(0)
    }
}

fn xxd_reverse_bytes(data: &[u8]) -> Vec<u8> {
    let input = String::from_utf8_lossy(data);
    let mut output = Vec::new();

    for line in input.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Skip the offset prefix if present
        let hex_part = if let Some(colon_pos) = line.find(':') {
            &line[colon_pos + 1..]
        } else {
            line
        };

        let hex_only: String = hex_part.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        for chunk in hex_only.as_bytes().chunks(2) {
            if chunk.len() == 2 {
                let hex_str = std::str::from_utf8(chunk).unwrap_or("00");
                if let Ok(byte) = u8::from_str_radix(hex_str, 16) {
                    output.push(byte);
                }
            }
        }
    }

    output
}

fn xxd_plain(data: &[u8]) -> CommandOutput {
    let mut output = String::new();
    for chunk in data.chunks(30) {
        for &byte in chunk {
            output.push_str(&format!("{:02x}", byte));
        }
        output.push('\n');
    }
    CommandOutput::success(output)
}

fn shuffle<T>(items: &mut Vec<T>) {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let hasher = RandomState::new().build_hasher();
    let mut seed = hasher.finish();
    let len = items.len();
    for i in (1..len).rev() {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let j = (seed as usize) % (i + 1);
        items.swap(i, j);
    }
}

fn suffix(idx: usize) -> String {
    // GNU split's default suffix length is 2 (`xaa`, `xab`, …).
    let letters = b"abcdefghijklmnopqrstuvwxyz";
    let mut n = idx;
    let mut buf = [b'a'; 2];
    for k in (0..2).rev() {
        buf[k] = letters[n % 26];
        n /= 26;
    }
    String::from_utf8_lossy(&buf).to_string()
}

fn parse_size(s: &str) -> usize {
    let s = s.trim();
    if let Some(rest) = s.strip_suffix('K') {
        rest.parse::<f64>().unwrap_or(0.0) as usize * 1024
    } else if let Some(rest) = s.strip_suffix('M') {
        rest.parse::<f64>().unwrap_or(0.0) as usize * 1024 * 1024
    } else if let Some(rest) = s.strip_suffix('G') {
        rest.parse::<f64>().unwrap_or(0.0) as usize * 1024 * 1024 * 1024
    } else {
        s.parse().unwrap_or(0)
    }
}

fn evaluate_expr(args: &[&str]) -> Result<i64, String> {
    if args.len() == 1 {
        return args[0]
            .parse::<i64>()
            .map_err(|_| format!("syntax error: {}", args[0]));
    }
    if args.len() == 3 {
        let left: i64 = args[0]
            .parse()
            .map_err(|_| format!("non-integer argument: {}", args[0]))?;
        let right: i64 = args[2]
            .parse()
            .map_err(|_| format!("non-integer argument: {}", args[2]))?;
        match args[1] {
            "+" => Ok(left + right),
            "-" => Ok(left - right),
            "*" => Ok(left * right),
            "/" => {
                if right == 0 {
                    Err("division by zero".to_string())
                } else {
                    Ok(left / right)
                }
            }
            "%" => {
                if right == 0 {
                    Err("division by zero".to_string())
                } else {
                    Ok(left % right)
                }
            }
            _ => Err(format!("syntax error: {}", args[1])),
        }
    } else if args.len() > 3 {
        let first = evaluate_expr(&args[..3])?;
        let first_str = first.to_string();
        let mut rest: Vec<&str> = vec![&first_str];
        rest.extend_from_slice(&args[3..]);
        evaluate_expr(&rest)
    } else {
        Err("syntax error".to_string())
    }
}
