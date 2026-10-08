// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Additional common commands filling coverage gaps: `arch`, `factor`,
//! `numfmt`, `pr`.

use crate::shell::{CommandOutput, Shell};

impl Shell {
    /// `arch` — print the machine architecture (same as `uname -m`).
    pub fn cmd_arch(&self, _args: &[&str]) -> CommandOutput {
        CommandOutput::success(format!("{}\n", std::env::consts::ARCH))
    }

    /// `factor N...` — prime-factorise each integer.
    pub fn cmd_factor(&self, args: &[&str]) -> CommandOutput {
        let mut out = String::new();
        let mut nums: Vec<&str> = args
            .iter()
            .copied()
            .filter(|a| !a.starts_with('-'))
            .collect();
        if nums.is_empty() {
            return CommandOutput::error("factor: missing operand\n".to_string(), 1);
        }
        // Keep the input order stable.
        let mut seen = std::collections::HashSet::new();
        for a in args {
            if a.starts_with('-') {
                continue;
            }
            if !seen.insert(*a) {
                continue;
            }
            let n: u64 = match a.trim().parse() {
                Ok(v) => v,
                Err(_) => {
                    return CommandOutput::error(
                        format!("factor: '{}' is not a valid integer\n", a),
                        1,
                    )
                }
            };
            out.push_str(&format!("{}:", n));
            for f in prime_factors(n) {
                out.push_str(&format!(" {}", f));
            }
            out.push('\n');
        }
        nums.clear();
        CommandOutput::success(out)
    }

    /// `numfmt [--to=si|iec|iec-i|none] [--from=si|iec|none] [--suffix=S] N...`
    /// — a pragmatic subset of GNU `numfmt` (defaults to `--to=si`).
    pub fn cmd_numfmt(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut to = String::from("si");
        let mut from = String::from("none");
        let mut suffix = String::new();
        let mut nums: Vec<String> = Vec::new();
        let mut i = 0;
        while i < args.len() {
            let a = args[i];
            if let Some(v) = a.strip_prefix("--to=") {
                to = v.to_string();
            } else if let Some(v) = a.strip_prefix("--from=") {
                from = v.to_string();
            } else if let Some(v) = a.strip_prefix("--suffix=") {
                suffix = v.to_string();
            } else if a == "--to" || a == "--from" || a == "--suffix" {
                i += 1;
                if let Some(v) = args.get(i) {
                    match a {
                        "--to" => to = v.to_string(),
                        "--from" => from = v.to_string(),
                        _ => suffix = v.to_string(),
                    }
                }
            } else if a.starts_with('-') && a.len() > 1 && a != "-" {
                // ignore other flags (e.g. --round)
            } else {
                nums.push(a.to_string());
            }
            i += 1;
        }
        if nums.is_empty() {
            // GNU numfmt reads whitespace/newline-separated numbers from stdin
            // when no operands are given.
            if let Some(s) = stdin {
                nums.extend(
                    s.split_whitespace()
                        .filter(|t| !t.is_empty())
                        .map(|t| t.to_string()),
                );
            }
        }
        let mut out = String::new();
        for raw in &nums {
            let value = parse_human(raw, &from);
            if value.is_nan() {
                return CommandOutput::error(format!("numfmt: invalid number: '{}'\n", raw), 1);
            }
            out.push_str(&format_human(value, &to));
            out.push_str(&suffix);
            out.push('\n');
        }
        CommandOutput::success(out)
    }

    /// `pr [-t] [-n[SEP]] [FILE...]` — paginate text (header/footer unless `-t`).
    pub fn cmd_pr(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut omit_header = false;
        let mut number = false;
        let mut number_sep = "\t".to_string();
        let mut files: Vec<&str> = Vec::new();
        for a in args {
            if *a == "-t" {
                omit_header = true;
            } else if *a == "-n" {
                number = true;
            } else if let Some(rest) = a.strip_prefix("-n") {
                number = true;
                if !rest.is_empty() {
                    number_sep = rest.to_string();
                }
            } else if a.starts_with('-') && a.len() > 1 {
                // ignore unsupported flags (columns/width/etc.)
            } else {
                files.push(a);
            }
        }
        let text = if files.is_empty() {
            match stdin {
                Some(s) => s.to_string(),
                None => return CommandOutput::error("pr: missing input\n".to_string(), 1),
            }
        } else {
            let mut all = String::new();
            for f in &files {
                match self.read_text_lossy(f) {
                    Ok(c) => all.push_str(&c),
                    Err(e) => return CommandOutput::error(format!("pr: {}: {}\n", f, e), 1),
                }
            }
            all
        };
        let lines: Vec<&str> = text.lines().collect();
        let mut out = String::new();
        let now = "2026-01-01";
        for (idx, line) in lines.iter().enumerate() {
            let page_line = idx % 66;
            if !omit_header {
                if page_line == 0 {
                    out.push_str(&format!(
                        "\n\n{} {} Page {}\n\n\n",
                        now,
                        files.first().copied().unwrap_or(""),
                        idx / 66 + 1
                    ));
                }
            }
            if number {
                out.push_str(&format!("{:>5}{}{}\n", idx + 1, number_sep, line));
            } else {
                out.push_str(line);
                out.push('\n');
            }
            if !omit_header && page_line == 60 {
                out.push_str("\n\n\n\n\n");
            }
        }
        CommandOutput::success(out)
    }

    /// `mapfile [-t] [-n N] [ARRAY]` / `readarray` — read stdin lines into an
    /// array (default `MAPFILE`).
    pub fn cmd_mapfile(&mut self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut strip_nl = false;
        let mut name = "MAPFILE".to_string();
        let mut max: Option<usize> = None;
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-t" | "--trim" => strip_nl = true,
                "-n" => {
                    i += 1;
                    max = args.get(i).and_then(|s| s.parse().ok());
                }
                a if a.starts_with('-') => {}
                a => name = a.to_string(),
            }
            i += 1;
        }
        let input = stdin.unwrap_or("");
        let mut items: Vec<String> = if strip_nl {
            input.lines().map(|l| l.to_string()).collect()
        } else {
            input.split_inclusive('\n').map(|l| l.to_string()).collect()
        };
        if let Some(n) = max {
            items.truncate(n);
        }
        self.arrays.insert(name, items);
        CommandOutput::success(String::new())
    }

    /// `fmt [-w WIDTH] [FILE...]` — reflow paragraphs to WIDTH columns (75).
    pub fn cmd_fmt(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut width = 75usize;
        let mut files: Vec<&str> = Vec::new();
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-w" | "--width" => {
                    i += 1;
                    if let Some(w) = args.get(i) {
                        width = w.parse().unwrap_or(75).max(1);
                    }
                }
                a if a.starts_with("-w") && a.len() > 2 => {
                    width = a[2..].parse().unwrap_or(75).max(1);
                }
                a if a.starts_with('-') && a.len() > 1 => {}
                a => files.push(a),
            }
            i += 1;
        }
        let text = if files.is_empty() {
            match stdin {
                Some(s) => s.to_string(),
                None => return CommandOutput::error("fmt: missing input\n".to_string(), 1),
            }
        } else {
            let mut all = String::new();
            for f in &files {
                match self.read_text_lossy(f) {
                    Ok(c) => all.push_str(&c),
                    Err(e) => return CommandOutput::error(format!("fmt: {}: {}\n", f, e), 1),
                }
            }
            all
        };
        let mut out = String::new();
        for para in text.split("\n\n") {
            let words: Vec<&str> = para.split_whitespace().collect();
            let mut line = String::new();
            for w in words {
                if line.is_empty() {
                    line.push_str(w);
                } else if line.len() + 1 + w.len() <= width {
                    line.push(' ');
                    line.push_str(w);
                } else {
                    out.push_str(&line);
                    out.push('\n');
                    line = w.to_string();
                }
            }
            out.push_str(&line);
            out.push_str("\n\n");
        }
        CommandOutput::success(out)
    }

    /// `stdbuf [-i/-o/-e MODE] CMD [ARGS...]` — buffering flags are ignored
    /// (in-process); the command runs normally.
    pub fn cmd_stdbuf(&mut self, args: &[&str]) -> CommandOutput {
        let mut i = 0;
        while i < args.len() {
            let a = args[i];
            if a == "-i" || a == "-o" || a == "-e" {
                i += 2;
                continue;
            }
            if a.starts_with('-') && a.len() > 1 {
                i += 1;
                continue;
            }
            break;
        }
        match args.get(i) {
            None => CommandOutput::error("stdbuf: missing command\n".to_string(), 125),
            Some(cmd) => self.execute(cmd, &args[i + 1..], None),
        }
    }

    /// `zstd [-d] [-c] [-k] [-#] [FILE]` — zstd compress/decompress.
    pub fn cmd_zstd(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut decompress = false;
        let mut to_stdout = false;
        let mut keep = false;
        let mut level: i32 = 3;
        let mut files: Vec<&str> = Vec::new();
        for a in args {
            match *a {
                "-d" | "--decompress" => decompress = true,
                "-c" | "--stdout" => to_stdout = true,
                "-k" | "--keep" => keep = true,
                _ if a.starts_with("--") => {}
                _ if a.starts_with('-') && a.len() > 1 => {
                    for ch in a[1..].chars() {
                        match ch {
                            'd' => decompress = true,
                            'c' => to_stdout = true,
                            'k' => keep = true,
                            '1'..='9' => level = ch.to_digit(10).unwrap() as i32,
                            _ => {}
                        }
                    }
                }
                _ => files.push(a),
            }
        }
        let (input, in_name): (Vec<u8>, Option<String>) = if files.is_empty() {
            match self
                .take_binary_in()
                .or_else(|| stdin.map(|s| s.as_bytes().to_vec()))
            {
                Some(b) => (b, None),
                None => return CommandOutput::error("zstd: missing input\n".to_string(), 1),
            }
        } else {
            match self.vfs.read(files[0], &self.cwd) {
                Ok(b) => (b, Some(files[0].to_string())),
                Err(e) => return CommandOutput::error(format!("zstd: {}: {}\n", files[0], e), 1),
            }
        };
        let result = if decompress {
            match zstd::stream::decode_all(&input[..]) {
                Ok(d) => d,
                Err(e) => return CommandOutput::error(format!("zstd: {}\n", e), 1),
            }
        } else {
            match zstd::stream::encode_all(&input[..], level) {
                Ok(c) => c,
                Err(e) => return CommandOutput::error(format!("zstd: {}\n", e), 1),
            }
        };
        if to_stdout || in_name.is_none() {
            self.set_binary_out(result.clone());
            CommandOutput::success(String::from_utf8_lossy(&result).to_string())
        } else {
            let name = in_name.unwrap();
            let out_name = if decompress {
                name.strip_suffix(".zst")
                    .or_else(|| name.strip_suffix(".zstd"))
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("{}.out", name))
            } else {
                format!("{}.zst", name)
            };
            match self.vfs.write_bytes(&out_name, &self.cwd, &result) {
                Ok(_) => {
                    if !keep {
                        let _ = self.vfs.remove_file(&name, &self.cwd);
                    }
                    CommandOutput::success(format!("{}: {} bytes\n", out_name, result.len()))
                }
                Err(e) => CommandOutput::error(format!("zstd: {}: {}\n", out_name, e), 1),
            }
        }
    }

    /// `unzstd` / `zstdcat` — decompress.
    pub fn cmd_unzstd(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut a: Vec<&str> = vec!["-d"];
        a.extend_from_slice(args);
        self.cmd_zstd(&a, stdin)
    }
    /// `unlink FILE` — remove a single file.
    pub fn cmd_unlink(&self, args: &[&str]) -> CommandOutput {
        let file = match args.iter().find(|a| !a.starts_with('-')) {
            Some(f) => *f,
            None => return CommandOutput::error("unlink: missing operand\n".to_string(), 1),
        };
        match self.vfs.remove_file(file, &self.cwd) {
            Ok(_) => CommandOutput::success(String::new()),
            Err(e) => CommandOutput::error(format!("unlink: {}: {}\n", file, e), 1),
        }
    }

    /// `link SRC DST` — hard link (falls back to a copy when linking fails).
    pub fn cmd_link(&self, args: &[&str]) -> CommandOutput {
        let files: Vec<&str> = args
            .iter()
            .copied()
            .filter(|a| !a.starts_with('-'))
            .collect();
        if files.len() < 2 {
            return CommandOutput::error("link: missing operand\n".to_string(), 1);
        }
        let (src, dst) = (files[0], files[1]);
        let (sp, dp) = match (
            self.vfs.resolve(src, &self.cwd),
            self.vfs.resolve(dst, &self.cwd),
        ) {
            (Ok(s), Ok(d)) => (s, d),
            _ => return CommandOutput::error(format!("link: {}: invalid path\n", src), 1),
        };
        if std::fs::hard_link(&sp, &dp).is_ok() {
            return CommandOutput::success(String::new());
        }
        match self.vfs.read(src, &self.cwd) {
            Ok(data) => match self.vfs.write_bytes(dst, &self.cwd, &data) {
                Ok(_) => CommandOutput::success(String::new()),
                Err(e) => CommandOutput::error(format!("link: {}: {}\n", dst, e), 1),
            },
            Err(e) => CommandOutput::error(format!("link: {}: {}\n", src, e), 1),
        }
    }

    /// `whereis NAME` — print the resolved path for NAME (builtin → "builtin").
    pub fn cmd_whereis(&self, args: &[&str]) -> CommandOutput {
        let mut out = String::new();
        for a in args.iter().filter(|a| !a.starts_with('-')) {
            out.push_str(&format!("{}: {}\n", a, a));
        }
        CommandOutput::success(out)
    }

    /// `envsubst [SHELL-FORMAT]` — substitute `$VAR`/`${VAR}` from the shell env.
    pub fn cmd_envsubst(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let input = match stdin {
            Some(s) => s,
            None => return CommandOutput::error("envsubst: missing input\n".to_string(), 1),
        };
        let _ = args; // SHELL-FORMAT filtering is not modelled; substitute all.
        let chars: Vec<char> = input.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '$' && i + 1 < chars.len() {
                if chars[i + 1] == '{' {
                    if let Some(close) = chars[i + 2..].iter().position(|&c| c == '}') {
                        let name: String = chars[i + 2..i + 2 + close].iter().collect();
                        out.push_str(&self.vars.get(&name).cloned().unwrap_or_default());
                        i = i + 2 + close + 1;
                        continue;
                    }
                } else if chars[i + 1].is_ascii_alphabetic() || chars[i + 1] == '_' {
                    let start = i + 1;
                    let mut j = start;
                    while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                        j += 1;
                    }
                    let name: String = chars[start..j].iter().collect();
                    out.push_str(&self.vars.get(&name).cloned().unwrap_or_default());
                    i = j;
                    continue;
                }
            }
            out.push(chars[i]);
            i += 1;
        }
        CommandOutput::success(out)
    }
}

fn prime_factors(mut n: u64) -> Vec<u64> {
    let mut out = Vec::new();
    if n < 2 {
        return out;
    }
    while n % 2 == 0 {
        out.push(2);
        n /= 2;
    }
    let mut f = 3u64;
    while f.saturating_mul(f) <= n {
        while n % f == 0 {
            out.push(f);
            n /= f;
        }
        f += 2;
    }
    if n > 1 {
        out.push(n);
    }
    out
}

fn parse_human(s: &str, from: &str) -> f64 {
    let s = s.trim();
    if from == "none" {
        // Allow a trailing SI/IEC suffix in the input too.
        let (num, mult) = split_suffix(s);
        return num * mult;
    }
    split_suffix(s).0 * split_suffix(s).1
}

fn split_suffix(s: &str) -> (f64, f64) {
    let bytes = s.as_bytes();
    let mut end = bytes.len();
    while end > 0 {
        let c = bytes[end - 1] as char;
        if c.is_ascii_digit() || c == '.' {
            break;
        }
        end -= 1;
    }
    let num: f64 = s[..end].parse().unwrap_or(f64::NAN);
    let suf = &s[end..];
    let mult = match suf {
        "" => 1.0,
        "K" | "k" => 1e3,
        "M" => 1e6,
        "G" => 1e9,
        "T" => 1e12,
        "P" => 1e15,
        "Ki" => 1024.0,
        "Mi" => 1024f64.powi(2),
        "Gi" => 1024f64.powi(3),
        "Ti" => 1024f64.powi(4),
        _ => f64::NAN,
    };
    (num, mult)
}

fn format_human(v: f64, to: &str) -> String {
    match to {
        "si" => {
            let units = ["", "K", "M", "G", "T", "P"];
            let mut x = v;
            let mut u = 0;
            while x.abs() >= 1000.0 && u + 1 < units.len() {
                x /= 1000.0;
                u += 1;
            }
            if u == 0 {
                format!("{}", v)
            } else {
                format!("{:.1}{}", x, units[u])
            }
        }
        "iec" | "iec-i" => {
            let units = ["", "Ki", "Mi", "Gi", "Ti", "Pi"];
            let mut x = v;
            let mut u = 0;
            while x.abs() >= 1024.0 && u + 1 < units.len() {
                x /= 1024.0;
                u += 1;
            }
            if u == 0 {
                format!("{}", v)
            } else {
                format!("{:.1}{}", x, units[u])
            }
        }
        _ => format!("{}", v),
    }
}
