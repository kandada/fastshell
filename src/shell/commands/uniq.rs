// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

const UNIQ_HELP_TEXT: &str = "\
uniq: report or omit repeated lines
Usage: uniq [OPTIONS] [FILE]
Options:
  -c          Prefix lines by occurrence count
  -d          Only print duplicate lines
  -u          Only print unique lines
  -i          Ignore case when comparing
  -f N        Skip first N fields
  -h, --help  Show this help message
";

impl Shell {
    pub fn cmd_uniq(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(UNIQ_HELP_TEXT.to_string());
        }
        let mut files = Vec::new();
        let mut count = false;
        let mut dup_only = false;
        let mut uniq_only = false;
        let mut ignore_case = false;
        let mut skip_fields: usize = 0;
        let mut skip_chars: usize = 0;
        let mut check_chars: Option<usize> = None;

        let mut i = 0;
        while i < args.len() {
            let arg = args[i];
            if arg.starts_with("--") {
                match arg {
                    "--count" => count = true,
                    "--repeated" => dup_only = true,
                    "--unique" => uniq_only = true,
                    "--ignore-case" => ignore_case = true,
                    _ => crate::warn!("uniq: warning: unsupported option '{}'", arg),
                }
            } else if arg.starts_with('-') && arg.len() > 1 {
                let chars: Vec<char> = arg.chars().skip(1).collect();
                let mut j = 0;
                while j < chars.len() {
                    match chars[j] {
                        'c' => count = true,
                        'd' => dup_only = true,
                        'u' => uniq_only = true,
                        'i' => ignore_case = true,
                        'f' | 's' | 'w' => {
                            let rest: String = chars[j + 1..].iter().collect();
                            let val = if !rest.is_empty() {
                                rest
                            } else {
                                i += 1;
                                if i < args.len() {
                                    args[i].to_string()
                                } else {
                                    String::new()
                                }
                            };
                            match chars[j] {
                                'f' => skip_fields = val.parse().unwrap_or(0),
                                's' => skip_chars = val.parse().unwrap_or(0),
                                'w' => check_chars = val.parse().ok(),
                                _ => {}
                            }
                            j = chars.len();
                            continue;
                        }
                        _ => crate::warn!("uniq: warning: unsupported option '-{}'", chars[j]),
                    }
                    j += 1;
                }
            } else {
                files.push(arg.to_string());
            }
            i += 1;
        }

        if files.is_empty() {
            match stdin {
                Some(input) => {
                    return uniq_process(
                        input,
                        count,
                        dup_only,
                        uniq_only,
                        ignore_case,
                        skip_fields,
                        skip_chars,
                        check_chars,
                    );
                }
                None => return CommandOutput::error("uniq: missing file operand\n".to_string(), 1),
            }
        }

        let mut output = String::new();
        for file in &files {
            let content = match self.vfs.read_to_string(file, &self.cwd) {
                Ok(c) => c,
                Err(e) => return CommandOutput::error(format!("uniq: {}: {}\n", file, e), 1),
            };
            match uniq_process(
                &content,
                count,
                dup_only,
                uniq_only,
                ignore_case,
                skip_fields,
                skip_chars,
                check_chars,
            ) {
                CommandOutput {
                    stdout,
                    exit_code: 0,
                    ..
                } => output.push_str(&stdout),
                err => return err,
            }
        }

        CommandOutput::success(output)
    }
}

fn uniq_process(
    input: &str,
    count: bool,
    dup_only: bool,
    uniq_only: bool,
    ignore_case: bool,
    skip_fields: usize,
    skip_chars: usize,
    check_chars: Option<usize>,
) -> CommandOutput {
    let lines: Vec<&str> = input.lines().collect();
    let lines: Vec<(&str, String)> = if skip_fields > 0 || skip_chars > 0 || check_chars.is_some() || ignore_case {
        lines
            .into_iter()
            .map(|l| {
                let cmp_part = if skip_fields > 0 {
                    let parts: Vec<&str> = l.split_whitespace().collect();
                    if parts.len() > skip_fields {
                        parts[skip_fields..].join(" ")
                    } else {
                        String::new()
                    }
                } else {
                    l.to_string()
                };
                // Skip leading chars.
                let after_skip_chars = if skip_chars > 0 {
                    cmp_part.chars().skip(skip_chars).collect::<String>()
                } else {
                    cmp_part
                };
                // Limit to first N chars.
                let limited = match check_chars {
                    Some(n) => after_skip_chars.chars().take(n).collect::<String>(),
                    None => after_skip_chars,
                };
                let key = if ignore_case {
                    limited.to_lowercase()
                } else {
                    limited
                };
                (l, key)
            })
            .collect()
    } else {
        lines.into_iter().map(|l| (l, l.to_string())).collect()
    };

    let mut output = String::new();
    let mut i = 0;
    while i < lines.len() {
        let mut cnt = 1usize;
        while i + cnt < lines.len() && lines[i + cnt].1 == lines[i].1 {
            cnt += 1;
        }
        let should_print = if dup_only {
            cnt > 1
        } else if uniq_only {
            cnt == 1
        } else {
            true
        };
        if should_print {
            if count {
                output.push_str(&format!("{:>7} {}\n", cnt, lines[i].0));
            } else {
                output.push_str(lines[i].0);
                output.push('\n');
            }
        }
        i += cnt;
    }

    CommandOutput::success(output)
}

#[cfg(test)]
mod tests {
    use crate::shell::Shell;
    use crate::vfs::Vfs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn mk_shell() -> Shell {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("fastshell_uniq_test_{}_{}", std::process::id(), n));
        let _ = std::fs::remove_dir_all(&dir);
        Shell::new(Vfs::new(dir).unwrap())
    }

    #[test]
    fn test_uniq_basic() {
        let shell = mk_shell();
        shell.vfs.write("/f.txt", "", "a\na\nb\nb\nb\nc\n").unwrap();
        let out = shell.cmd_uniq(&["/f.txt"], None);
        assert_eq!(out.stdout.trim().lines().count(), 3);
    }

    #[test]
    fn test_uniq_count() {
        let shell = mk_shell();
        shell.vfs.write("/f.txt", "", "a\na\nb\nb\nb\nc\n").unwrap();
        let out = shell.cmd_uniq(&["-c", "/f.txt"], None);
        assert!(out.stdout.contains("2 a"));
        assert!(out.stdout.contains("3 b"));
    }

    #[test]
    fn test_uniq_duplicate_only() {
        let shell = mk_shell();
        shell.vfs.write("/f.txt", "", "a\na\nb\nb\nb\nc\n").unwrap();
        let out = shell.cmd_uniq(&["-d", "/f.txt"], None);
        let lines: Vec<&str> = out.stdout.trim().lines().collect();
        assert_eq!(lines, vec!["a", "b"]);
    }

    #[test]
    fn test_uniq_unique_only() {
        let shell = mk_shell();
        shell.vfs.write("/f.txt", "", "a\na\nb\nb\nb\nc\n").unwrap();
        let out = shell.cmd_uniq(&["-u", "/f.txt"], None);
        let lines: Vec<&str> = out.stdout.trim().lines().collect();
        assert_eq!(lines, vec!["c"]);
    }

    #[test]
    fn test_uniq_ignore_case() {
        let shell = mk_shell();
        shell
            .vfs
            .write("/f.txt", "", "Hello\nhello\nHELLO\nworld\n")
            .unwrap();
        let out = shell.cmd_uniq(&["-i", "/f.txt"], None);
        assert_eq!(out.stdout.trim().lines().count(), 2);
    }

    #[test]
    fn test_uniq_skip_fields() {
        let shell = mk_shell();
        shell
            .vfs
            .write("/f.txt", "", "1 a\n2 a\n3 b\n4 b\n")
            .unwrap();
        let out = shell.cmd_uniq(&["-f", "1", "/f.txt"], None);
        let lines: Vec<&str> = out.stdout.trim().lines().collect();
        assert_eq!(lines, vec!["1 a", "3 b"]);
    }

    #[test]
    fn test_uniq_stdin() {
        let shell = mk_shell();
        let out = shell.cmd_uniq(&["-c"], Some("a\na\nb\n"));
        assert!(out.stdout.contains("2 a"));
        assert!(out.stdout.contains("1 b"));
    }

    #[test]
    fn test_uniq_missing_file() {
        let shell = mk_shell();
        let out = shell.cmd_uniq(&[], None);
        assert_ne!(out.exit_code, 0);
    }

    #[test]
    fn test_uniq_help() {
        let mut shell = mk_shell();
        let out = shell.execute("uniq", &["-h"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_uniq_help_long() {
        let mut shell = mk_shell();
        let out = shell.execute("uniq", &["--help"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_uniq_skip_chars() {
        let shell = mk_shell();
        // Skip the leading "1 "/"2 " so "a"/"a" are considered equal.
        let out = shell.cmd_uniq(&["-s", "2"], Some("1 a\n2 a\n3 b\n"));
        let lines: Vec<&str> = out.stdout.trim().lines().collect();
        assert_eq!(lines, vec!["1 a", "3 b"], "skip-chars should group 1 a / 2 a");
    }

    #[test]
    fn test_uniq_check_chars() {
        let shell = mk_shell();
        // Compare only the first 2 chars: "abc" vs "abd" share "ab".
        let out = shell.cmd_uniq(&["-w", "2"], Some("abc\nabd\nxyz\n"));
        let lines: Vec<&str> = out.stdout.trim().lines().collect();
        assert_eq!(lines, vec!["abc", "xyz"], "check-chars should group abc/abd");
    }
}
