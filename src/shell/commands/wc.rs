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
        let mut show_lines = true;
        let mut show_words = true;
        let mut show_bytes = true;
        let mut show_max_width = false;
        let mut bytes_mode = true;
        let mut files = Vec::new();

        for arg in args {
            if arg.starts_with("--") {
                match *arg {
                    "--lines" => {
                        show_words = false;
                        show_bytes = false;
                    }
                    "--words" => {
                        show_lines = false;
                        show_bytes = false;
                    }
                    "--bytes" => {
                        show_lines = false;
                        show_words = false;
                        bytes_mode = true;
                    }
                    "--chars" => {
                        show_lines = false;
                        show_words = false;
                        bytes_mode = false;
                    }
                    "--max-line-length" => {
                        show_lines = false;
                        show_words = false;
                        show_bytes = false;
                        show_max_width = true;
                    }
                    _ => crate::warn!("wc: warning: unsupported option '{}'", arg),
                }
            } else if arg.starts_with('-') && arg.len() > 1 {
                for ch in arg.chars().skip(1) {
                    match ch {
                        'l' => {
                            show_words = false;
                            show_bytes = false;
                        }
                        'w' => {
                            show_lines = false;
                            show_bytes = false;
                        }
                        'c' => {
                            show_lines = false;
                            show_words = false;
                            bytes_mode = true;
                        }
                        'm' => {
                            show_lines = false;
                            show_words = false;
                            bytes_mode = false;
                        }
                        'L' => {
                            show_lines = false;
                            show_words = false;
                            show_bytes = false;
                            show_max_width = true;
                        }
                        _ => crate::warn!("wc: warning: unsupported option '-{}'", ch),
                    }
                }
            } else {
                files.push(arg.to_string());
            }
        }

        if files.is_empty() {
            match stdin {
                Some(input) => {
                    let l = input.lines().count();
                    let w = input.split_whitespace().count();
                    let bc = if bytes_mode {
                        input.as_bytes().len()
                    } else {
                        input.chars().count()
                    };
                    let maxw = input.lines().map(|x| x.chars().count()).max().unwrap_or(0);
                    let mut parts = Vec::new();
                    if show_lines {
                        parts.push(format!("{:>7}", l));
                    }
                    if show_words {
                        parts.push(format!("{:>7}", w));
                    }
                    if show_bytes {
                        parts.push(format!("{:>7}", bc));
                    }
                    if show_max_width {
                        parts.push(format!("{:>7}", maxw));
                    }
                    return CommandOutput::success(parts.join("") + "\n");
                }
                None => return CommandOutput::error("wc: missing file operand\n".to_string(), 1),
            }
        }

        let mut output = String::new();
        let mut total_lines = 0usize;
        let mut total_words = 0usize;
        let mut total_bc = 0usize;
        let mut total_maxw = 0usize;

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
                    let maxw = content.lines().map(|x| x.chars().count()).max().unwrap_or(0);
                    let mut parts = Vec::new();
                    if show_lines {
                        parts.push(format!("{:>7}", l));
                    }
                    if show_words {
                        parts.push(format!("{:>7}", w));
                    }
                    if show_bytes {
                        parts.push(format!("{:>7}", bc));
                    }
                    if show_max_width {
                        parts.push(format!("{:>7}", maxw));
                    }
                    parts.push(file.clone());
                    output.push_str(&parts.join(" "));
                    output.push('\n');
                    total_lines += l;
                    total_words += w;
                    total_bc += bc;
                    total_maxw = total_maxw.max(maxw);
                }
                Err(e) => {
                    return CommandOutput::error(format!("wc: {}: {}\n", file, e), 1);
                }
            }
        }

        if files.len() > 1 {
            let mut parts = Vec::new();
            if show_lines {
                parts.push(format!("{:>7}", total_lines));
            }
            if show_words {
                parts.push(format!("{:>7}", total_words));
            }
            if show_bytes {
                parts.push(format!("{:>7}", total_bc));
            }
            if show_max_width {
                parts.push(format!("{:>7}", total_maxw));
            }
            parts.push("total".to_string());
            output.push_str(&parts.join(" "));
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
        let dir = std::env::temp_dir().join(format!("fastshell_wc_test_{}_{}", std::process::id(), n));
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
