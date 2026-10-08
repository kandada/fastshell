// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

const WC_HELP_TEXT: &str = "\
Usage: wc [OPTION]... [FILE]...
Print newline, word, and byte counts for each FILE.

  -l  print the newline counts
  -w  print the word counts
  -c  print the byte counts
  -m  print the character counts
  -L  print the maximum display width
  -h, --help  display this help and exit
";

impl Shell {
    pub fn cmd_wc(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(WC_HELP_TEXT.to_string());
        }
        // Counter flags SELECT which columns to print; they must not cancel
        // each other (`wc -c -w` must show bytes AND words). Track the requested
        // set, then fall back to the default three only when no flag was given.
        let mut want_lines = false;
        let mut want_words = false;
        let mut want_bytes = false;
        let mut want_chars = false;
        let mut want_max_width = false;
        let mut any_flag = false;
        let mut files = Vec::new();

        for arg in args {
            if arg.starts_with("--") {
                match *arg {
                    "--lines" => {
                        want_lines = true;
                        any_flag = true;
                    }
                    "--words" => {
                        want_words = true;
                        any_flag = true;
                    }
                    "--bytes" => {
                        want_bytes = true;
                        any_flag = true;
                    }
                    "--chars" => {
                        want_chars = true;
                        any_flag = true;
                    }
                    "--max-line-length" => {
                        want_max_width = true;
                        any_flag = true;
                    }
                    _ => crate::warn!("wc: warning: unsupported option '{}'", arg),
                }
            } else if arg.starts_with('-') && arg.len() > 1 {
                for ch in arg.chars().skip(1) {
                    match ch {
                        'l' => {
                            want_lines = true;
                            any_flag = true;
                        }
                        'w' => {
                            want_words = true;
                            any_flag = true;
                        }
                        'c' => {
                            want_bytes = true;
                            any_flag = true;
                        }
                        'm' => {
                            want_chars = true;
                            any_flag = true;
                        }
                        'L' => {
                            want_max_width = true;
                            any_flag = true;
                        }
                        _ => crate::warn!("wc: warning: unsupported option '-{}'", ch),
                    }
                }
            } else {
                files.push(arg.to_string());
            }
        }

        let (show_lines, show_words, show_bytes, show_max_width, bytes_mode) = if any_flag {
            (
                want_lines,
                want_words,
                want_bytes || want_chars,
                want_max_width,
                !want_chars,
            )
        } else {
            (true, true, true, false, true)
        };

        // GNU `wc`: each numeric field is right-aligned to the width of the
        // largest displayed count (min 1). A single input (stdin/one file) is
        // therefore un-padded (`wc -l < f` → `3`), not a fixed width of 7.
        let mut rows: Vec<(Vec<usize>, Option<String>)> = Vec::new();
        let mut totals: Vec<usize> = Vec::new();

        if files.is_empty() {
            let input = match stdin {
                Some(s) => s,
                None => return CommandOutput::error("wc: missing file operand\n".to_string(), 1),
            };
            // Prefer the byte-accurate stdin (`cat bin | wc -c`).
            let raw = self
                .take_binary_in()
                .unwrap_or_else(|| input.as_bytes().to_vec());
            let l = input.lines().count();
            let w = input.split_whitespace().count();
            let bc = if bytes_mode {
                raw.len()
            } else {
                input.chars().count()
            };
            let maxw = input.lines().map(|x| x.chars().count()).max().unwrap_or(0);
            let mut vals = Vec::new();
            if show_lines {
                vals.push(l);
            }
            if show_words {
                vals.push(w);
            }
            if show_bytes {
                vals.push(bc);
            }
            if show_max_width {
                vals.push(maxw);
            }
            rows.push((vals, None));
        } else {
            let mut t = [0usize; 4];
            for file in &files {
                match self.vfs.read(file, &self.cwd) {
                    Ok(data) => {
                        let content = String::from_utf8_lossy(&data);
                        let l = content.lines().count();
                        let w = content.split_whitespace().count();
                        let bc = if bytes_mode {
                            data.len()
                        } else {
                            content.chars().count()
                        };
                        let maxw = content
                            .lines()
                            .map(|x| x.chars().count())
                            .max()
                            .unwrap_or(0);
                        t[0] += l;
                        t[1] += w;
                        t[2] += bc;
                        t[3] = t[3].max(maxw);
                        let mut vals = Vec::new();
                        if show_lines {
                            vals.push(l);
                        }
                        if show_words {
                            vals.push(w);
                        }
                        if show_bytes {
                            vals.push(bc);
                        }
                        if show_max_width {
                            vals.push(maxw);
                        }
                        rows.push((vals, Some(file.clone())));
                    }
                    Err(e) => {
                        return CommandOutput::error(format!("wc: {}: {}\n", file, e), 1);
                    }
                }
            }
            if show_lines {
                totals.push(t[0]);
            }
            if show_words {
                totals.push(t[1]);
            }
            if show_bytes {
                totals.push(t[2]);
            }
            if show_max_width {
                totals.push(t[3]);
            }
        }

        let width = rows
            .iter()
            .flat_map(|(v, _)| v.iter())
            .chain(totals.iter())
            .map(|v| v.to_string().len())
            .max()
            .unwrap_or(1)
            .max(1);

        let fmt_row = |vals: &[usize], name: Option<&str>| {
            let mut s = vals
                .iter()
                .map(|v| format!("{:>w$}", v, w = width))
                .collect::<Vec<_>>()
                .join(" ");
            if let Some(n) = name {
                if !s.is_empty() {
                    s.push(' ');
                }
                s.push_str(n);
            }
            s
        };

        let mut output = String::new();
        for (vals, name) in &rows {
            output.push_str(&fmt_row(vals, name.as_deref()));
            output.push('\n');
        }
        if files.len() > 1 {
            output.push_str(&fmt_row(&totals, Some("total")));
            output.push('\n');
        }

        CommandOutput::success(output)
    }
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
            std::env::temp_dir().join(format!("fastshell_wc_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        let vfs = crate::vfs::Vfs::new(dir).unwrap();
        Shell::new(vfs)
    }

    #[test]
    fn test_wc_help() {
        let mut s = mk_shell();
        let out = s.execute("wc", &["-h"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_wc_help_long() {
        let mut s = mk_shell();
        let out = s.execute("wc", &["--help"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }
}
