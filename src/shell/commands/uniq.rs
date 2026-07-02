// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

impl Shell {
    pub fn cmd_uniq(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut files = Vec::new();
        let mut count = false;
        let mut dup_only = false;
        let mut uniq_only = false;
        let mut ignore_case = false;
        let mut skip_fields: usize = 0;

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-c" => count = true,
                "-d" => dup_only = true,
                "-u" => uniq_only = true,
                "-i" => ignore_case = true,
                "-f" => {
                    if i + 1 < args.len() {
                        skip_fields = args[i + 1].parse().unwrap_or(0);
                        i += 1;
                    }
                }
                arg if arg.starts_with("-f") && arg.len() > 2 => {
                    skip_fields = arg[2..].parse().unwrap_or(0);
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
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
) -> CommandOutput {
    let lines: Vec<&str> = input.lines().collect();
    let lines: Vec<(&str, String)> = if skip_fields > 0 || ignore_case {
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
                let key = if ignore_case {
                    cmp_part.to_lowercase()
                } else {
                    cmp_part
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
}
