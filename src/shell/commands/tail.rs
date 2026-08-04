// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};
use std::time::{Duration, Instant};

impl Shell {
    pub fn cmd_tail(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut lines_count: i64 = 10;
        let mut from_start = false;
        let mut follow = false;
        let mut files = Vec::new();

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-n" => {
                    if i + 1 < args.len() {
                        let val = args[i + 1];
                        if let Some(v) = val.strip_prefix('+') {
                            from_start = true;
                            lines_count = v.parse().unwrap_or(10);
                        } else {
                            let n: i64 = val.parse().unwrap_or(10);
                            lines_count = n;
                        }
                        i += 1;
                    }
                }
                "-f" | "--follow" => follow = true,
                arg if arg.starts_with("-n") && arg.len() > 2 => {
                    let val = &arg[2..];
                    if let Some(v) = val.strip_prefix('+') {
                        from_start = true;
                        lines_count = v.parse().unwrap_or(10);
                    } else {
                        lines_count = val.parse().unwrap_or(10);
                    }
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
            i += 1;
        }

        let count = if lines_count < 0 {
            10usize
        } else {
            lines_count as usize
        };

        if files.is_empty() {
            match stdin {
                Some(input) => {
                    let lines: Vec<&str> = input.lines().collect();
                    let output = if from_start {
                        let start = (count.saturating_sub(1)).min(lines.len());
                        lines[start..].join("\n")
                    } else {
                        let start = if lines.len() > count {
                            lines.len() - count
                        } else {
                            0
                        };
                        lines[start..].join("\n")
                    };
                    let mut result = String::new();
                    if !output.is_empty() {
                        result.push_str(&output);
                    }
                    if output.ends_with('\n') || output.is_empty() {
                        return CommandOutput::success(result);
                    }
                    return CommandOutput::success(result);
                }
                None => return CommandOutput::error("tail: missing file operand\n".to_string(), 1),
            }
        }

        // If follow mode is off, read once and return.
        if !follow {
            return self.tail_read_files(&files, count, from_start);
        }

        // ── follow mode ──────────────────────────────────────────────
        let file = &files[0];
        let mut output = String::new();

        // Read initial content.
        match self.vfs.read_to_string(file, &self.cwd) {
            Ok(content) => {
                let lines: Vec<&str> = content.lines().collect();
                let start = if from_start {
                    (count.saturating_sub(1)).min(lines.len())
                } else {
                    if lines.len() > count { lines.len() - count } else { 0 }
                };
                for &line in &lines[start..] {
                    output.push_str(line);
                    output.push('\n');
                }
            }
            Err(e) => {
                return CommandOutput::error(format!("tail: {}: {}\n", file, e), 1);
            }
        }

        let mut last_size = output.len();
        let max_duration = Duration::from_secs(60);

        self.tail_follow_loop(&mut output, &mut last_size, file, max_duration);

        CommandOutput::success(output)
    }

    fn tail_follow_loop(
        &self,
        output: &mut String,
        last_size: &mut usize,
        file: &str,
        max_duration: Duration,
    ) {
        let poll_interval = Duration::from_millis(200);
        let deadline = Instant::now() + max_duration;

        while Instant::now() < deadline {
            std::thread::sleep(poll_interval);

            match self.vfs.read_to_string(file, &self.cwd) {
                Ok(content) => {
                    let lines: Vec<&str> = content.lines().collect();
                    let mut current = String::new();
                    for &line in &lines {
                        current.push_str(line);
                        current.push('\n');
                    }
                    if current.len() > *last_size {
                        let new_content = &current[*last_size..];
                        output.push_str(new_content);
                        *last_size = current.len();
                    }
                }
                Err(_) => {
                    continue;
                }
            }
        }
    }

    fn tail_read_files(&self, files: &[String], count: usize, from_start: bool) -> CommandOutput {
        let mut output = String::new();
        for file in files {
            if files.len() > 1 {
                output.push_str(&format!("==> {} <==\n", file));
            }
            match self.vfs.read_to_string(file, &self.cwd) {
                Ok(content) => {
                    let lines: Vec<&str> = content.lines().collect();
                    if from_start {
                        let start = (count.saturating_sub(1)).min(lines.len());
                        for &line in &lines[start..] {
                            output.push_str(line);
                            output.push('\n');
                        }
                    } else {
                        let start = if lines.len() > count {
                            lines.len() - count
                        } else {
                            0
                        };
                        for &line in &lines[start..] {
                            output.push_str(line);
                            output.push('\n');
                        }
                    }
                }
                Err(e) => {
                    return CommandOutput::error(format!("tail: {}: {}\n", file, e), 1);
                }
            }
        }
        CommandOutput::success(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::Shell;
    use crate::vfs::Vfs;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn setup_vfs() -> Vfs {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir()
            .join(format!("fastshell_tail_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        Vfs::new(dir).unwrap()
    }

    fn mk_shell() -> Shell {
        Shell::new(setup_vfs())
    }

    #[test]
    fn test_tail_basic() {
        let mut shell = mk_shell();
        let out = shell.execute("echo", &["line1\nline2\nline3"], None);
        assert_eq!(out.exit_code, 0);
        let out = shell.execute("tail", &["-n", "1"], Some("a\nb\nc"));
        assert_eq!(out.stdout.trim(), "c");
    }

    #[test]
    fn test_tail_file() {
        let mut shell = mk_shell();
        shell.cmd_touch(&["test.txt"]);
        shell
            .vfs
            .write("test.txt", "/", "one\ntwo\nthree\nfour\nfive\n")
            .unwrap();
        let out = shell.cmd_tail(&["-n", "2", "test.txt"], None);
        assert_eq!(out.stdout, "four\nfive\n");
    }

    #[test]
    fn test_tail_follow_detects_new_lines() {
        let mut shell = mk_shell();
        shell.cmd_touch(&["follow.txt"]);
        shell
            .vfs
            .write("follow.txt", "/", "line1\nline2\n")
            .unwrap();

        let mut output = String::new();
        let mut last_size = 0;

        let vfs = shell.vfs.clone();

        // Write more data in background while follow loop runs
        let vfs2 = vfs;
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(400));
            vfs2
                .write("follow.txt", "/", "line1\nline2\nline3\n")
                .unwrap();
        });

        shell.tail_follow_loop(
            &mut output,
            &mut last_size,
            "follow.txt",
            Duration::from_secs(2),
        );
        assert!(
            output.contains("line3"),
            "expected new line in output: {output}"
        );
    }

    #[test]
    fn test_tail_from_start() {
        let mut shell = mk_shell();
        shell.cmd_touch(&["test.txt"]);
        shell
            .vfs
            .write("test.txt", "/", "one\ntwo\nthree\nfour\nfive\n")
            .unwrap();
        let out = shell.cmd_tail(&["-n", "+3", "test.txt"], None);
        assert_eq!(out.stdout, "three\nfour\nfive\n");
    }
}
