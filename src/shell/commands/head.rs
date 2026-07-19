// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

impl Shell {
    pub fn cmd_head(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut lines_count: Option<i64> = None;
        let mut char_count: Option<i64> = None;
        let mut files = Vec::new();

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-n" => {
                    if i + 1 < args.len() {
                        lines_count = Some(args[i + 1].parse().unwrap_or(10));
                        i += 1;
                    }
                }
                "-c" => {
                    if i + 1 < args.len() {
                        char_count = Some(args[i + 1].parse().unwrap_or(0));
                        i += 1;
                    }
                }
                arg if arg.starts_with("-n") && arg.len() > 2 => {
                    lines_count = Some(arg[2..].parse().unwrap_or(10));
                }
                arg if arg.starts_with("-c") && arg.len() > 2 => {
                    char_count = Some(arg[2..].parse().unwrap_or(0));
                }
                // Dash-number shorthand: `head -2` == `head -n 2`
                arg if arg.starts_with('-')
                    && arg.len() > 1
                    && arg[1..].chars().all(|c| c.is_ascii_digit()) =>
                {
                    lines_count = Some(arg[1..].parse().unwrap_or(10));
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
            i += 1;
        }

        let render = |content: &str| -> String {
            if let Some(count) = char_count {
                if count >= 0 {
                    content.chars().take(count as usize).collect()
                } else {
                    // -c -N: all but the last N chars
                    let total = content.chars().count() as i64;
                    let take = (total + count).max(0) as usize;
                    content.chars().take(take).collect()
                }
            } else {
                let n = lines_count.unwrap_or(10);
                let lines: Vec<&str> = content.lines().collect();
                let take = if n >= 0 {
                    (n as usize).min(lines.len())
                } else {
                    // -n -N: all but the last N lines
                    lines.len().saturating_sub((-n) as usize)
                };
                let mut out = String::new();
                for &line in &lines[..take] {
                    out.push_str(line);
                    out.push('\n');
                }
                out
            }
        };

        if files.is_empty() {
            match stdin {
                Some(input) => return CommandOutput::success(render(input)),
                None => return CommandOutput::error("head: missing file operand\n".to_string(), 1),
            }
        }

        let mut output = String::new();
        for file in &files {
            if files.len() > 1 {
                output.push_str(&format!("==> {} <==\n", file));
            }
            match self.vfs.read_to_string(file, &self.cwd) {
                Ok(content) => output.push_str(&render(&content)),
                Err(e) => {
                    return CommandOutput::error(format!("head: {}: {}\n", file, e), 1);
                }
            }
        }
        CommandOutput::success(output)
    }
}

#[cfg(test)]
mod tests {
    use crate::shell::Shell;
    use crate::vfs::Vfs;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn mk_shell() -> Shell {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("fastshell_head_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        Shell::new(Vfs::new(dir).unwrap())
    }

    #[test]
    fn head_default_10() {
        let shell = mk_shell();
        let input: String = (1..=15).map(|i| format!("l{}\n", i)).collect();
        let out = shell.cmd_head(&[], Some(&input));
        assert_eq!(out.stdout.lines().count(), 10);
    }

    #[test]
    fn head_dash_number() {
        let shell = mk_shell();
        let out = shell.cmd_head(&["-2"], Some("a\nb\nc\n"));
        assert_eq!(out.stdout, "a\nb\n");
    }

    #[test]
    fn head_c_bytes() {
        let shell = mk_shell();
        let out = shell.cmd_head(&["-c", "4"], Some("hello world"));
        assert_eq!(out.stdout, "hell");
    }

    #[test]
    fn head_c_file() {
        let shell = mk_shell();
        shell.vfs.write("f.txt", "/", "abcdef").unwrap();
        let out = shell.cmd_head(&["-c", "3", "f.txt"], None);
        assert_eq!(out.stdout, "abc");
        assert_eq!(out.exit_code, 0);
    }

    #[test]
    fn head_negative_n() {
        let shell = mk_shell();
        let out = shell.cmd_head(&["-n", "-1"], Some("a\nb\nc\n"));
        assert_eq!(out.stdout, "a\nb\n");
    }
}
