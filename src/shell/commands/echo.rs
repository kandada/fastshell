// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

const ECHO_HELP_TEXT: &str = "\
Usage: echo [OPTION]... [STRING]...
Display a line of text.

  -n         do not output the trailing newline
  -e         enable interpretation of backslash escapes
  -E         disable interpretation of backslash escapes (default)
  -h, --help  display this help and exit
";

impl Shell {
    pub fn cmd_echo(&self, args: &[&str]) -> CommandOutput {
        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(ECHO_HELP_TEXT.to_string());
        }
        let mut no_newline = false;
        let mut enable_escapes = false;
        let mut start = 0;

        while start < args.len() {
            match args[start] {
                "-n" => {
                    no_newline = true;
                    start += 1;
                }
                "-e" => {
                    enable_escapes = true;
                    start += 1;
                }
                "-E" => {
                    enable_escapes = false;
                    start += 1;
                }
                a if a.starts_with('-') => {
                    eprintln!("echo: warning: unsupported option '{}'", a);
                    start += 1;
                }
                _ => break,
            }
        }

        let output = args[start..].join(" ");
        let output = if enable_escapes {
            unescape_echo(&output)
        } else {
            output
        };

        if no_newline {
            CommandOutput::success(output)
        } else {
            CommandOutput::success(output + "\n")
        }
    }
}

fn unescape_echo(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            i += 1;
            match chars[i] {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                '\\' => out.push('\\'),
                'a' => out.push('\x07'),
                'b' => out.push('\x08'),
                'f' => out.push('\x0C'),
                'v' => out.push('\x0B'),
                'e' => out.push('\x1B'),
                'x' => {
                    if i + 2 < chars.len() {
                        let hex: String = [chars[i + 1], chars[i + 2]].iter().collect();
                        if let Ok(b) = u8::from_str_radix(&hex, 16) {
                            out.push(b as char);
                            i += 2;
                        } else {
                            out.push_str(&format!("\\x{}", hex));
                            i += 2;
                        }
                    } else {
                        out.push_str("\\x");
                    }
                }
                c => {
                    out.push('\\');
                    out.push(c);
                }
            }
        } else {
            out.push(chars[i]);
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::Shell;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn mk_shell() -> Shell {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("fastshell_echo_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        let vfs = crate::vfs::Vfs::new(dir).unwrap();
        Shell::new(vfs)
    }

    #[test]
    fn test_echo_help() {
        let mut s = mk_shell();
        let out = s.execute("echo", &["-h"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_echo_help_long() {
        let mut s = mk_shell();
        let out = s.execute("echo", &["--help"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_echo_hex_escape() {
        let mut s = mk_shell();
        let out = s.execute("echo", &["-e", "\\x41"], None);
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout.trim(), "A");
    }
}
