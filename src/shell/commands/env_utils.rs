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
        // reaching here means bare `env` (print the sandbox environment).
        let mut entries: Vec<(&String, &String)> = self
            .vars
            .iter()
            .filter(|(k, _)| self.exported.contains(*k))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        let mut output = String::new();
        for (k, v) in entries {
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

    pub fn cmd_printf(&self, args: &[&str], _stdin: Option<&str>) -> CommandOutput {
        if args.is_empty() {
            return CommandOutput::error("printf: missing format\n".to_string(), 1);
        }

        let format = args[0];
        let data_args: Vec<&str> = args[1..].to_vec();
        let output = simple_printf(format, &data_args);
        CommandOutput::success(output)
    }

    pub fn cmd_basename(&self, args: &[&str]) -> CommandOutput {
        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(BASENAME_HELP_TEXT.to_string());
        }
        for arg in args {
            if arg.starts_with('-') {
                crate::warn!("basename: warning: unsupported option '{}'", arg);
            }
        }
        let files: Vec<&str> = args
            .iter()
            .filter(|a| !a.starts_with('-'))
            .copied()
            .collect();
        if files.is_empty() {
            return CommandOutput::error("basename: missing operand\n".to_string(), 1);
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
            .unwrap_or_else(|| ".".to_string());

        CommandOutput::success(parent + "\n")
    }

    pub fn cmd_realpath(&self, args: &[&str]) -> CommandOutput {
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
                Ok(resolved) => match std::fs::canonicalize(&resolved) {
                    Ok(canon) => output.push_str(&format!("{}\n", canon.display())),
                    Err(e) => output.push_str(&format!("realpath: {}: {}\n", file, e)),
                },
                Err(e) => output.push_str(&format!("realpath: {}: {}\n", file, e)),
            }
        }

        CommandOutput::success(output)
    }
}

fn simple_printf(format: &str, args: &[&str]) -> String {
    let mut result = String::new();
    let mut arg_idx = 0;
    let chars: Vec<char> = format.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            match chars[i + 1] {
                'n' => result.push('\n'),
                't' => result.push('\t'),
                '\\' => result.push('\\'),
                'r' => result.push('\r'),
                '0' => {}
                c => {
                    result.push('\\');
                    result.push(c);
                }
            }
            i += 2;
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
                result.push('%');
                i += 1;
                continue;
            }
            let spec = chars[j];
            j += 1;

            if spec == '%' {
                result.push('%');
                i = j;
                continue;
            }

            let arg = args.get(arg_idx).copied().unwrap_or("");
            arg_idx += 1;

            let mut rendered = match spec {
                's' => arg.to_string(),
                'd' | 'i' => arg.parse::<i64>().map(|v| v.to_string()).unwrap_or_else(|_| "0".into()),
                'u' => arg.parse::<u64>().map(|v| v.to_string()).unwrap_or_else(|_| "0".into()),
                'f' => {
                    let v = arg.parse::<f64>().unwrap_or(0.0);
                    match precision {
                        Some(p) => format!("{:.*}", p, v),
                        None => format!("{:.6}", v),
                    }
                }
                'x' => arg.parse::<u64>().map(|v| format!("{:x}", v)).unwrap_or_else(|_| "0".into()),
                'o' => arg.parse::<u64>().map(|v| format!("{:o}", v)).unwrap_or_else(|_| "0".into()),
                'c' => arg.chars().next().map(|c| c.to_string()).unwrap_or_default(),
                other => format!("%{}", other),
            };

            // String precision truncates.
            if spec == 's' {
                if let Some(p) = precision {
                    rendered = rendered.chars().take(p).collect();
                }
            }

            // Apply width / alignment / zero-pad.
            let len = rendered.chars().count();
            if width > len {
                let pad = width - len;
                let pad_char = if zero_pad && !left_align && spec != 's' { '0' } else { ' ' };
                let pad_str: String = std::iter::repeat(pad_char).take(pad).collect();
                if left_align {
                    rendered = format!("{}{}", rendered, pad_str);
                } else {
                    rendered = format!("{}{}", pad_str, rendered);
                }
            }
            result.push_str(&rendered);
            i = j;
        } else {
            result.push(chars[i]);
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
        let dir = std::env::temp_dir().join(format!("fastshell_env_test_{}_{}", std::process::id(), n));
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
