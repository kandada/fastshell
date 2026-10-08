// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

const BASENAME_HELP_TEXT: &str = "\
Usage: basename PATH [SUFFIX]
Strip directory and optionally suffix from filenames.

  SUFFIX    remove trailing SUFFIX from the result
  -h, --help  display this help and exit
";

const DIRNAME_HELP_TEXT: &str = "\
Usage: dirname PATH
Strip last component from file name.

  -h, --help  display this help and exit
";

impl Shell {
    pub fn cmd_env(&self, args: &[&str]) -> CommandOutput {
        // `env VAR=x cmd ...` is handled upstream by `consume_assignments`;
        // reaching here means bare `env` (print the process + exported shell env).
        use std::collections::BTreeMap;
        let mut map: BTreeMap<String, String> = std::env::vars().collect();
        for (k, v) in &self.vars {
            if self.exported.contains(k) {
                map.insert(k.clone(), v.clone());
            }
        }
        let mut output = String::new();
        for (k, v) in map {
            output.push_str(&format!("{}={}\n", k, v));
        }
        let _ = args;
        CommandOutput::success(output)
    }

    pub fn cmd_printenv(&self, args: &[&str]) -> CommandOutput {
        if args.is_empty() {
            return self.cmd_env(args);
        }
        let mut output = String::new();
        for arg in args {
            if !arg.starts_with('-') {
                let val = self
                    .vars
                    .get(*arg)
                    .cloned()
                    .or_else(|| std::env::var(arg).ok());
                if let Some(val) = val {
                    output.push_str(&val);
                    output.push('\n');
                }
            }
        }
        let is_empty = output.is_empty();
        CommandOutput {
            stdout: output,
            stderr: String::new(),
            exit_code: if is_empty { 1 } else { 0 },
        }
    }

    pub fn cmd_printf(&mut self, args: &[&str], _stdin: Option<&str>) -> CommandOutput {
        if args.is_empty() {
            return CommandOutput::error("printf: missing format\n".to_string(), 1);
        }

        // `printf -v VAR fmt args...` assigns the formatted output to VAR
        // instead of writing it to stdout (bash).
        let mut target: Option<String> = None;
        let mut rest: &[&str] = args;
        if args[0] == "-v" {
            if args.len() < 3 {
                return CommandOutput::error(
                    "printf: -v requires a variable name and a format\n".to_string(),
                    1,
                );
            }
            target = Some(args[1].to_string());
            rest = &args[2..];
        } else if let Some(v) = args[0].strip_prefix("-v") {
            if v.is_empty() || args.len() < 2 {
                return CommandOutput::error(
                    "printf: -v requires a variable name and a format\n".to_string(),
                    1,
                );
            }
            target = Some(v.to_string());
            rest = &args[1..];
        }

        let format = rest[0];
        let data_args: Vec<&str> = rest[1..].to_vec();
        let output_bytes = simple_printf_bytes(format, &data_args);
        let output = String::from_utf8_lossy(&output_bytes).into_owned();
        match target {
            Some(name) => {
                self.vars.insert(name, output);
                CommandOutput::success(String::new())
            }
            None => {
                // Byte-accurate stdout so `\xHH`/`\NNN` escapes keep raw bytes.
                if !output_bytes.is_empty() {
                    self.set_binary_out(output_bytes);
                }
                CommandOutput::success(output)
            }
        }
    }

    /// `getopts optstring name [arg...]` — a small POSIX-style option parser.
    /// Sets `name` to the option letter and `OPTARG` to its argument; advances
    /// `OPTIND`. Returns 0 while options remain, 1 at the end.
    pub fn cmd_getopts(&mut self, args: &[&str]) -> CommandOutput {
        if args.len() < 2 {
            return CommandOutput::error(
                "getopts: usage: getopts optstring name [arg...]\n".to_string(),
                2,
            );
        }
        let optstring = args[0];
        let name = args[1].to_string();
        let parms: Vec<String> = if args.len() > 2 {
            args[2..].iter().map(|s| s.to_string()).collect()
        } else {
            self.positional.iter().skip(1).cloned().collect()
        };
        let silent = optstring.starts_with(':');
        let optspec = optstring.trim_start_matches(':');
        let needs_arg = |o: char| -> bool {
            let b: Vec<char> = optspec.chars().collect();
            b.iter()
                .enumerate()
                .any(|(i, c)| *c == o && b.get(i + 1) == Some(&':'))
        };

        let mut optind: usize = self
            .vars
            .get("OPTIND")
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);
        if optind == 0 {
            optind = 1;
        }
        let mut pos: usize = self
            .vars
            .get("__GETOPTS_POS")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let mut token = self
            .vars
            .get("__GETOPTS_TOKEN")
            .cloned()
            .unwrap_or_default();

        if pos == 0 || token.is_empty() {
            let Some(arg) = parms.get(optind - 1).cloned() else {
                self.vars.remove(&name);
                return CommandOutput::error(String::new(), 1);
            };
            if !arg.starts_with('-') || arg == "-" {
                self.vars.remove(&name);
                return CommandOutput::error(String::new(), 1);
            }
            token = arg;
            pos = 1;
        }

        let chars: Vec<char> = token.chars().collect();
        let opt = chars[pos];
        pos += 1;
        let mut optarg = String::new();
        if needs_arg(opt) {
            if pos < chars.len() {
                optarg = chars[pos..].iter().collect();
                pos = chars.len();
            } else if optind < parms.len() {
                optarg = parms[optind].clone();
                optind += 1;
            } else {
                // Missing required argument.
                self.vars
                    .insert("OPTIND".to_string(), (optind + 1).to_string());
                self.vars.remove("__GETOPTS_POS");
                self.vars.remove("__GETOPTS_TOKEN");
                if silent {
                    self.vars.insert(name, ":".to_string());
                    self.vars.insert("OPTARG".to_string(), opt.to_string());
                    return CommandOutput::success(String::new());
                }
                return CommandOutput::error(
                    format!("getopts: option requires an argument -- '{opt}'\n"),
                    2,
                );
            }
        }

        if pos >= chars.len() {
            pos = 0;
            token.clear();
            optind += 1;
        }
        self.vars.insert(name, opt.to_string());
        if !optarg.is_empty() {
            self.vars.insert("OPTARG".to_string(), optarg);
        }
        self.vars
            .insert("__GETOPTS_POS".to_string(), pos.to_string());
        self.vars.insert("__GETOPTS_TOKEN".to_string(), token);
        self.vars.insert("OPTIND".to_string(), optind.to_string());
        CommandOutput::success(String::new())
    }

    pub fn cmd_basename(&self, args: &[&str]) -> CommandOutput {
        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(BASENAME_HELP_TEXT.to_string());
        }
        for arg in args {
            if arg.starts_with('-') && !matches!(*arg, "-a" | "--multiple" | "-s" | "--suffix") {
                crate::warn!("basename: warning: unsupported option '{}'", arg);
            }
        }
        let multiple = args.iter().any(|a| *a == "-a" || *a == "--multiple");
        let files: Vec<&str> = args
            .iter()
            .filter(|a| !a.starts_with('-'))
            .copied()
            .collect();
        if files.is_empty() {
            return CommandOutput::error("basename: missing operand\n".to_string(), 1);
        }
        if multiple {
            let mut out = String::new();
            for f in &files {
                let name = std::path::Path::new(f)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                out.push_str(&name);
                out.push('\n');
            }
            return CommandOutput::success(out);
        }

        let path = files[0];
        let suffix = if files.len() > 1 {
            Some(files[1])
        } else {
            None
        };

        let path = std::path::Path::new(path);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        let result = match suffix {
            Some(s) if name.ends_with(s) => name[..name.len() - s.len()].to_string(),
            _ => name,
        };

        CommandOutput::success(result + "\n")
    }

    pub fn cmd_dirname(&self, args: &[&str]) -> CommandOutput {
        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(DIRNAME_HELP_TEXT.to_string());
        }
        for arg in args {
            if arg.starts_with('-') {
                crate::warn!("dirname: warning: unsupported option '{}'", arg);
            }
        }
        let files: Vec<&str> = args
            .iter()
            .filter(|a| !a.starts_with('-'))
            .copied()
            .collect();
        if files.is_empty() {
            return CommandOutput::error("dirname: missing operand\n".to_string(), 1);
        }

        let path = std::path::Path::new(files[0]);
        let parent = path
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| ".".to_string());

        CommandOutput::success(parent + "\n")
    }

    pub fn cmd_realpath(&self, args: &[&str]) -> CommandOutput {
        // `-m`/`--canonicalize-missing`: don't require the path to exist.
        let allow_missing = args
            .iter()
            .any(|a| *a == "-m" || *a == "--canonicalize-missing");
        let files: Vec<&str> = args
            .iter()
            .filter(|a| !a.starts_with('-'))
            .copied()
            .collect();
        if files.is_empty() {
            return CommandOutput::error("realpath: missing operand\n".to_string(), 1);
        }

        let mut output = String::new();
        for file in &files {
            match self.vfs.resolve(file, &self.cwd) {
                // Report the VFS path, never the host location.
                Ok(resolved) => output.push_str(&format!("{}\n", self.vfs.to_vpath(&resolved))),
                Err(e) => {
                    if allow_missing {
                        // Normalise the requested path lexically (`.`/`..`).
                        let base = if file.starts_with('/') {
                            String::new()
                        } else {
                            format!("/{}", self.cwd.trim_matches('/'))
                        };
                        let mut parts: Vec<&str> =
                            base.split('/').filter(|s| !s.is_empty()).collect();
                        for seg in file.split('/') {
                            match seg {
                                "" | "." => {}
                                ".." => {
                                    parts.pop();
                                }
                                s => parts.push(s),
                            }
                        }
                        output.push_str(&format!("/{}\n", parts.join("/")));
                    } else {
                        output.push_str(&format!("realpath: {}: {}\n", file, e));
                    }
                }
            }
        }

        CommandOutput::success(output)
    }
}

/// POSIX `printf`: the format is reused until all args are consumed (a format
/// with no conversions is emitted once). `printf '%s\n' a b c` → "a\nb\nc".
#[allow(dead_code)]
fn simple_printf(format: &str, args: &[&str]) -> String {
    String::from_utf8_lossy(&simple_printf_bytes(format, args)).into_owned()
}

/// Byte-accurate printf output. Escape sequences such as `\xHH` / `\0NNN`
/// emit raw bytes (not UTF-8-encoded code points) so binary data survives.
fn simple_printf_bytes(format: &str, args: &[&str]) -> Vec<u8> {
    let conversions = count_conversions(format);
    if conversions == 0 || args.is_empty() {
        return simple_printf_once_bytes(format, args);
    }
    let mut result: Vec<u8> = Vec::new();
    let mut consumed = 0usize;
    loop {
        result.extend_from_slice(&simple_printf_once_bytes(format, &args[consumed..]));
        let n = conversions.min(args.len() - consumed);
        consumed += n;
        if consumed >= args.len() {
            break;
        }
    }
    result
}

/// Number of `%` conversion specs (excluding `%%`).
fn count_conversions(format: &str) -> usize {
    let chars: Vec<char> = format.chars().collect();
    let mut count = 0;
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '%' && i + 1 < chars.len() {
            // Skip flags/width/precision to find the spec char.
            let mut j = i + 1;
            while j < chars.len() && matches!(chars[j], '-' | '0' | '+' | ' ' | '.' | '0'..='9') {
                j += 1;
            }
            if j < chars.len() {
                if chars[j] == '%' {
                    i = j + 1;
                    continue;
                }
                count += 1;
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    count
}

/// POSIX shell quoting for `printf %q`: leave safe words bare, single-quote the
/// rest (escaping embedded single quotes as `'\''`).
fn shell_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    let safe = s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "_./:=@%+,-".contains(c));
    if safe {
        return s.to_string();
    }
    let mut out = String::from("'");
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// Expand backslash escape sequences in a `%b` argument (bash `printf %b`).
fn decode_backslash_escapes(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '\\' || i + 1 >= chars.len() {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        match chars[i + 1] {
            'n' => {
                out.push('\n');
                i += 2;
            }
            't' => {
                out.push('\t');
                i += 2;
            }
            'r' => {
                out.push('\r');
                i += 2;
            }
            'a' => {
                out.push('\u{07}');
                i += 2;
            }
            'b' => {
                out.push('\u{08}');
                i += 2;
            }
            'f' => {
                out.push('\u{0C}');
                i += 2;
            }
            'v' => {
                out.push('\u{0B}');
                i += 2;
            }
            '\\' => {
                out.push('\\');
                i += 2;
            }
            'c' => break,
            'x' => {
                let mut k = 0;
                let mut val: u32 = 0;
                while k < 2 && i + 2 + k < chars.len() {
                    if let Some(d) = chars[i + 2 + k].to_digit(16) {
                        val = val * 16 + d;
                        k += 1;
                    } else {
                        break;
                    }
                }
                if k == 0 {
                    out.push('\\');
                    out.push('x');
                    i += 2;
                } else {
                    if let Some(ch) = char::from_u32(val) {
                        out.push(ch);
                    }
                    i += 2 + k;
                }
            }
            '0' => {
                let mut k = 0;
                let mut val: u32 = 0;
                while k < 3 && i + 2 + k < chars.len() {
                    if let Some(d) = chars[i + 2 + k].to_digit(8) {
                        val = val * 8 + d;
                        k += 1;
                    } else {
                        break;
                    }
                }
                out.push(char::from_u32(val).unwrap_or('\0'));
                i += 2 + k;
            }
            other => {
                out.push('\\');
                out.push(other);
                i += 2;
            }
        }
    }
    out
}

fn simple_printf_once_bytes(format: &str, args: &[&str]) -> Vec<u8> {
    let mut result: Vec<u8> = Vec::new();
    let mut arg_idx = 0;
    let chars: Vec<char> = format.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            match chars[i + 1] {
                'n' => {
                    result.push(b'\n');
                    i += 2;
                }
                't' => {
                    result.push(b'\t');
                    i += 2;
                }
                '\\' => {
                    result.push(b'\\');
                    i += 2;
                }
                'r' => {
                    result.push(b'\r');
                    i += 2;
                }
                'x' => {
                    // `\xHH` hex byte (raw byte, not a UTF-8 code point).
                    let mut k = 0;
                    let mut val: u32 = 0;
                    while k < 2 && i + 2 + k < chars.len() {
                        if let Some(d) = chars[i + 2 + k].to_digit(16) {
                            val = val * 16 + d;
                            k += 1;
                        } else {
                            break;
                        }
                    }
                    if k == 0 {
                        result.push(b'\\');
                        result.push(b'x');
                        i += 2;
                    } else {
                        result.push(val as u8);
                        i += 2 + k;
                    }
                }
                '0' => {
                    // `\0` -> NUL; `\0NNN` -> an octal byte (POSIX printf).
                    let mut k = 0;
                    let mut val: u32 = 0;
                    while k < 3 && i + 2 + k < chars.len() {
                        if let Some(d) = chars[i + 2 + k].to_digit(8) {
                            val = val * 8 + d;
                            k += 1;
                        } else {
                            break;
                        }
                    }
                    result.push(val as u8);
                    i += 2 + k;
                }
                '1'..='7' => {
                    // `\NNN` octal byte (1-3 digits, POSIX printf).
                    let mut k = 0;
                    let mut val: u32 = 0;
                    while k < 3 && i + 1 + k < chars.len() {
                        if let Some(d) = chars[i + 1 + k].to_digit(8) {
                            val = val * 8 + d;
                            k += 1;
                        } else {
                            break;
                        }
                    }
                    result.push(val as u8);
                    i += 1 + k;
                }
                c => {
                    result.push(b'\\');
                    let mut buf = [0u8; 4];
                    result.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                    i += 2;
                }
            }
        } else if chars[i] == '%' && i + 1 < chars.len() {
            // Parse `%[flags][width][.precision]spec`.
            let mut j = i + 1;
            let mut left_align = false;
            let mut zero_pad = false;
            while j < chars.len() {
                match chars[j] {
                    '-' => {
                        left_align = true;
                        j += 1;
                    }
                    '0' => {
                        zero_pad = true;
                        j += 1;
                    }
                    '+' | ' ' => {
                        j += 1;
                    }
                    _ => break,
                }
            }
            let mut width = 0usize;
            while j < chars.len() && chars[j].is_ascii_digit() {
                width = width * 10 + chars[j].to_digit(10).unwrap() as usize;
                j += 1;
            }
            let mut precision: Option<usize> = None;
            if j < chars.len() && chars[j] == '.' {
                j += 1;
                let mut p = 0usize;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    p = p * 10 + chars[j].to_digit(10).unwrap() as usize;
                    j += 1;
                }
                precision = Some(p);
            }
            if j >= chars.len() {
                result.push(b'%');
                i += 1;
                continue;
            }
            let spec = chars[j];
            j += 1;

            if spec == '%' {
                result.push(b'%');
                i = j;
                continue;
            }

            let arg = args.get(arg_idx).copied().unwrap_or("");
            arg_idx += 1;

            let mut rendered: Vec<u8> = match spec {
                's' => arg.as_bytes().to_vec(),
                'd' | 'i' => parse_int_base0(arg).to_string().into_bytes(),
                'u' => (parse_int_base0(arg) as u64).to_string().into_bytes(),
                'f' => {
                    let v = arg.parse::<f64>().unwrap_or(0.0);
                    match precision {
                        Some(p) => format!("{:.*}", p, v),
                        None => format!("{:.6}", v),
                    }
                    .into_bytes()
                }
                'x' => format!("{:x}", parse_int_base0(arg) as u64).into_bytes(),
                'o' => format!("{:o}", parse_int_base0(arg) as u64).into_bytes(),
                'c' => arg
                    .chars()
                    .next()
                    .map(|c| c.to_string().into_bytes())
                    .unwrap_or_default(),
                // `%q` - quote for reuse as shell input.
                // `%b` - expand backslash escapes in the argument (like echo -e).
                'b' => decode_backslash_escapes(arg).into_bytes(),
                'q' => shell_quote(arg).into_bytes(),
                other => format!("%{}", other).into_bytes(),
            };

            // String precision truncates (by byte, like GNU printf).
            if spec == 's' || spec == 'b' {
                if let Some(p) = precision {
                    rendered.truncate(p);
                }
            }

            // Apply width / alignment / zero-pad (by byte length).
            let len = rendered.len();
            if width > len {
                let pad = width - len;
                let pad_byte = if zero_pad && !left_align && spec != 's' && spec != 'b' {
                    b'0'
                } else {
                    b' '
                };
                let pad_vec = vec![pad_byte; pad];
                if left_align {
                    rendered.extend_from_slice(&pad_vec);
                } else {
                    let mut new = pad_vec;
                    new.extend_from_slice(&rendered);
                    rendered = new;
                }
            }
            result.extend_from_slice(&rendered);
            i = j;
        } else {
            let mut buf = [0u8; 4];
            result.extend_from_slice(chars[i].encode_utf8(&mut buf).as_bytes());
            i += 1;
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::Shell;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn mk_shell() -> Shell {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("fastshell_env_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        let vfs = crate::vfs::Vfs::new(dir).unwrap();
        Shell::new(vfs)
    }

    #[test]
    fn test_basename_help() {
        let mut s = mk_shell();
        let out = s.execute("basename", &["-h"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_basename_help_long() {
        let mut s = mk_shell();
        let out = s.execute("basename", &["--help"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_dirname_help() {
        let mut s = mk_shell();
        let out = s.execute("dirname", &["-h"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_dirname_help_long() {
        let mut s = mk_shell();
        let out = s.execute("dirname", &["--help"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_printf_width() {
        assert_eq!(super::simple_printf("[%5s]", &["ab"]), "[   ab]");
        assert_eq!(super::simple_printf("[%-5s]", &["ab"]), "[ab   ]");
    }

    #[test]
    fn test_printf_zero_pad() {
        assert_eq!(super::simple_printf("%05d", &["42"]), "00042");
    }

    #[test]
    fn test_printf_precision() {
        assert_eq!(super::simple_printf("%.2f", &["3.14159"]), "3.14");
        assert_eq!(super::simple_printf("%.3s", &["hello"]), "hel");
    }

    #[test]
    fn test_printf_char() {
        assert_eq!(super::simple_printf("%c", &["A"]), "A");
    }
}

/// Parses an integer the way bash `printf` does: base 0 (auto-detect), i.e.
/// `0x1f` → 31, `010` → 8, otherwise decimal.
fn parse_int_base0(s: &str) -> i64 {
    let t = s.trim();
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        return i64::from_str_radix(hex, 16).unwrap_or(0);
    }
    if t.len() > 1 && t.starts_with('0') && t[1..].chars().all(|c| c.is_ascii_digit()) {
        return i64::from_str_radix(&t[1..], 8).unwrap_or(0);
    }
    t.parse::<i64>().unwrap_or(0)
}
