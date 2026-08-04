// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

const CAT_HELP_TEXT: &str = "\
Usage: cat [OPTION]... [FILE]...
Concatenate FILE(s) to standard output.

  -n  number all output lines
  -b  number nonempty output lines (overrides -n)
  -E  display $ at end of each line
  -T  display TAB characters as ^I
  -v  use ^ and M- notation for nonprinting chars
  -e  equivalent to -vE
  -t  equivalent to -vT
  -A  equivalent to -vET
  -h, --help  display this help and exit
";

impl Shell {
    pub fn cmd_cat(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(CAT_HELP_TEXT.to_string());
        }
        let mut show_numbers = false;
        let mut show_ends = false;
        let mut show_tabs = false;
        let mut show_nonprint = false;
        let mut found_file = false;
        let mut line_num = 1usize;

        let mut files: Vec<&str> = Vec::new();
        for &arg in args {
            if arg.starts_with('-') && arg.len() > 1 {
                for ch in arg[1..].chars() {
                    match ch {
                        'n' => show_numbers = true,
                        'E' => show_ends = true,
                        'T' => show_tabs = true,
                        'v' => show_nonprint = true,
                        'e' => { show_ends = true; show_nonprint = true; }
                        't' => { show_tabs = true; show_nonprint = true; }
                        'A' => { show_ends = true; show_tabs = true; show_nonprint = true; }
                        _ => eprintln!("cat: warning: unsupported option '{}'", arg),
                    }
                }
            } else {
                files.push(arg);
            }
        }

        if files.is_empty() {
            if let Some(ref s) = stdin {
                let mut output = String::new();
                write_cat(&mut output, s, show_numbers, &mut line_num,
                          show_ends, show_tabs, show_nonprint);
                return CommandOutput::success(output);
            }
            return CommandOutput::error("cat: missing file operand\n".to_string(), 1);
        }

        let mut output = String::new();
        for &file in &files {
            found_file = true;
            match self.vfs.read_to_string(file, &self.cwd) {
                Ok(content) => {
                    write_cat(&mut output, &content, show_numbers, &mut line_num,
                              show_ends, show_tabs, show_nonprint);
                }
                Err(e) => {
                    return CommandOutput::error(format!("cat: {}: {}\n", file, e), 1);
                }
            }
        }

        if !found_file {
            return CommandOutput::error("cat: missing file operand\n".to_string(), 1);
        }

        CommandOutput::success(output)
    }
}

fn write_cat(
    out: &mut String,
    content: &str,
    show_numbers: bool,
    line_num: &mut usize,
    show_ends: bool,
    show_tabs: bool,
    show_nonprint: bool,
) {
    let ends_with_newline = content.ends_with('\n');
    let lines: Vec<&str> = content.lines().collect();
    // If the content didn't end with \n, the last "line" from lines() is the
    // actual last content without trailing newline — preserve it as-is.
    if !ends_with_newline && !lines.is_empty() {
        // lines() already stripped the implicit trailing content correctly
    }
    // If content ended with \n and was empty after the last \n, lines() drops
    // the trailing empty element which is correct (cat doesn't add extra \n).
    let len = lines.len();
    for (idx, line) in lines.iter().enumerate() {
        if show_numbers {
            out.push_str(&format!("{:>6}\t", line_num));
            *line_num += 1;
        }
        for ch in line.chars() {
            write_cat_char(out, ch, show_tabs, show_nonprint);
        }
        if show_ends {
            out.push('$');
        }
        // Add newline separator except for the last line when input had no trailing \n
        if idx + 1 < len || ends_with_newline {
            out.push('\n');
        }
    }
}

fn write_cat_char(out: &mut String, ch: char, show_tabs: bool, show_nonprint: bool) {
    match ch {
        '\t' if show_tabs => { out.push_str("^I"); }
        _ if show_nonprint && (ch as u32) < 0x20 => {
            let ctrl = (ch as u8 + 64) as char;
            out.push('^');
            out.push(ctrl);
        }
        _ if show_nonprint && ch == '\x7F' => {
            out.push_str("^?");
        }
        _ if show_nonprint && (ch as u32) >= 0x80 => {
            if (ch as u32) < 0xA0 {
                let ctrl = (ch as u8 - 0x80 + 64) as char;
                out.push_str("M-^");
                out.push(ctrl);
            } else {
                let letter = (ch as u8 - 0x80) as char;
                out.push_str("M-");
                out.push(letter);
            }
        }
        _ => out.push(ch),
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
        let dir = std::env::temp_dir().join(format!("fastshell_cat_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        let vfs = crate::vfs::Vfs::new(dir).unwrap();
        Shell::new(vfs)
    }

    #[test]
    fn test_cat_help() {
        let mut s = mk_shell();
        let out = s.execute("cat", &["-h"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_cat_help_long() {
        let mut s = mk_shell();
        let out = s.execute("cat", &["--help"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }
}
