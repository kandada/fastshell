// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};
use std::time::{Duration, Instant};

const TAIL_HELP_TEXT: &str = "\
Usage: tail [OPTION]... [FILE]...
Print the last 10 lines of each FILE to standard output.

  -n N        print the last N lines (use +N to start at line N)
  -c N        print the last N bytes
  -f          follow: output appended data as the file grows
  -h, --help  display this help and exit
";

impl Shell {
    pub fn cmd_tail(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(TAIL_HELP_TEXT.to_string());
        }
        let mut lines_count: i64 = 10;
        let mut char_count: Option<i64> = None;
        let mut from_start = false;
        let mut follow = false;
        let mut quiet = false;
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
                "-c" => {
                    if i + 1 < args.len() {
                        let val = args[i + 1];
                        if let Some(v) = val.strip_prefix('+') {
                            from_start = true;
                            char_count = Some(v.parse().unwrap_or(0));
                        } else {
                            char_count = Some(val.parse().unwrap_or(0));
                        }
                        i += 1;
                    }
                }
                "-f" | "-F" | "--follow" => follow = true,
                "-q" | "--quiet" | "--silent" => quiet = true,
                "-v" | "--verbose" => quiet = false,
                "-s" | "--sleep-interval" | "--pid" | "--retry" => {
                    if args[i] == "-s" || args[i] == "--sleep-interval" || args[i] == "--pid" {
                        if i + 1 < args.len() {
                            i += 1;
                        }
                    }
                }
                arg if arg.starts_with("-n") && arg.len() > 2 => {
                    let val = &arg[2..];
                    if let Some(v) = val.strip_prefix('+') {
                        from_start = true;
                        lines_count = v.parse().unwrap_or(10);
                    } else {
                        lines_count = val.parse().unwrap_or(10);
                    }
                }
                arg if arg.starts_with("-c") && arg.len() > 2 => {
                    let val = &arg[2..];
                    if let Some(v) = val.strip_prefix('+') {
                        from_start = true;
                        char_count = Some(v.parse().unwrap_or(0));
                    } else {
                        char_count = Some(val.parse().unwrap_or(0));
                    }
                }
                // `tail -N` shorthand for `-n N` (e.g. `-1`, `-20`); `-Nf`/`-Nq`/`-Nv`.
                arg if arg.len() > 1
                    && arg.starts_with('-')
                    && arg.as_bytes()[1].is_ascii_digit() =>
                {
                    let digits: String = arg[1..]
                        .chars()
                        .take_while(|c| c.is_ascii_digit())
                        .collect();
                    lines_count = digits.parse().unwrap_or(10);
                    let rest = &arg[1 + digits.len()..];
                    if rest.contains('f') || rest.contains('F') {
                        follow = true;
                    }
                    if rest.contains('q') {
                        quiet = true;
                    }
                    if rest.contains('v') {
                        quiet = false;
                    }
                }
                // `tail +N` shorthand for `-n +N`.
                arg if arg.len() > 1
                    && arg.starts_with('+')
                    && arg.as_bytes()[1].is_ascii_digit() =>
                {
                    from_start = true;
                    lines_count = arg[1..].parse().unwrap_or(10);
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
            i += 1;
        }

        // Byte mode: `tail -c N` / `-c +N` / `-c -N`.
        if let Some(cn) = char_count {
            return self.tail_bytes(files, stdin, cn, from_start, quiet);
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
                    let start = if from_start {
                        (count.saturating_sub(1)).min(lines.len())
                    } else if lines.len() > count {
                        lines.len() - count
                    } else {
                        0
                    };
                    // Each emitted line is newline-terminated (GNU tail).
                    let mut result = String::new();
                    for line in &lines[start..] {
                        result.push_str(line);
                        result.push('\n');
                    }
                    return CommandOutput::success(result);
                }
                None => return CommandOutput::error("tail: missing file operand\n".to_string(), 1),
            }
        }

        // If follow mode is off, read once and return.
        if !follow {
            return self.tail_read_files(&files, count, from_start, quiet);
        }

        // ── follow mode ──────────────────────────────────────────────
        let file = &files[0];
        let mut output = String::new();

        // Read initial content.
        match self.read_text_lossy(file) {
            Ok(content) => {
                let lines: Vec<&str> = content.lines().collect();
                let start = if from_start {
                    (count.saturating_sub(1)).min(lines.len())
                } else {
                    if lines.len() > count {
                        lines.len() - count
                    } else {
                        0
                    }
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

            match self.read_text_lossy(file) {
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

    /// `tail -c N` / `-c +N` / `-c -N` — byte-oriented tail.
    fn tail_bytes(
        &self,
        files: Vec<String>,
        stdin: Option<&str>,
        count: i64,
        from_start: bool,
        quiet: bool,
    ) -> CommandOutput {
        let slice_bytes = |content: &[u8]| -> String {
            let bytes = content;
            if from_start {
                // `-c +N`: from the (N-1)-th byte (0-based) to the end.
                let start = (count.saturating_sub(1)).max(0) as usize;
                if start >= bytes.len() {
                    String::new()
                } else {
                    String::from_utf8_lossy(&bytes[start..]).to_string()
                }
            } else if count >= 0 {
                // `-c N`: last N bytes.
                let n = count as usize;
                if n >= bytes.len() {
                    String::from_utf8_lossy(bytes).to_string()
                } else {
                    String::from_utf8_lossy(&bytes[bytes.len() - n..]).to_string()
                }
            } else {
                // `-c -N`: all but the last N bytes.
                let n = (-count) as usize;
                if n >= bytes.len() {
                    String::new()
                } else {
                    String::from_utf8_lossy(&bytes[..bytes.len() - n]).to_string()
                }
            }
        };

        if files.is_empty() {
            match stdin {
                Some(input) => return CommandOutput::success(slice_bytes(input.as_bytes())),
                None => return CommandOutput::error("tail: missing file operand\n".to_string(), 1),
            }
        }

        let mut output = String::new();
        for file in &files {
            if files.len() > 1 && !quiet {
                output.push_str(&format!("==> {} <==\n", file));
            }
            match self.read_bytes(file) {
                Ok(content) => output.push_str(&slice_bytes(&content)),
                Err(e) => return CommandOutput::error(format!("tail: {}: {}\n", file, e), 1),
            }
        }
        CommandOutput::success(output)
    }

    fn tail_read_files(
        &self,
        files: &[String],
        count: usize,
        from_start: bool,
        quiet: bool,
    ) -> CommandOutput {
        let mut output = String::new();
        for file in files {
            if files.len() > 1 && !quiet {
                output.push_str(&format!("==> {} <==\n", file));
            }
            match self.read_text_lossy(file) {
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
        let dir =
            std::env::temp_dir().join(format!("fastshell_tail_test_{}_{}", std::process::id(), n));
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
            vfs2.write("follow.txt", "/", "line1\nline2\nline3\n")
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

    #[test]
    fn test_tail_bytes_last_n() {
        let mut shell = mk_shell();
        let out = shell.cmd_tail(&["-c", "4"], Some("hello world"));
        assert_eq!(out.stdout, "orld");
    }

    #[test]
    fn test_tail_bytes_from_start() {
        let mut shell = mk_shell();
        let out = shell.cmd_tail(&["-c", "+7"], Some("hello world"));
        assert_eq!(out.stdout, "world");
    }

    #[test]
    fn test_tail_bytes_negative() {
        let mut shell = mk_shell();
        let out = shell.cmd_tail(&["-c", "-6"], Some("hello world"));
        assert_eq!(out.stdout, "hello");
    }
}
