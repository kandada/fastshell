// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

const SED_HELP_TEXT: &str = "\
sed: stream editor
Usage: sed [OPTIONS] 'script' [FILE...]
Options:
  -e SCRIPT   Add script to commands
  -i          Edit files in-place
  -n          Suppress automatic printing
  -ni/-in     Combine -n and -i
  -i.bak      In-place with backup suffix
  -h, --help  Show this help message
";

impl Shell {
    pub fn cmd_sed(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(SED_HELP_TEXT.to_string());
        }
        let mut expression: Option<String> = None;
        let mut files = Vec::new();
        let mut in_place = false;
        let mut quiet = false; // -n: suppress automatic printing

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-e" => {
                    if i + 1 < args.len() {
                        expression = Some(args[i + 1].to_string());
                        i += 1;
                    }
                }
                "-i" => in_place = true,
                "-n" => quiet = true,
                "-ni" | "-in" => {
                    quiet = true;
                    in_place = true;
                }
                arg if arg.starts_with("-i") && arg.len() > 2 => {
                    // -i.bak style backup suffix (suffix ignored, VFS keeps no backups)
                    in_place = true;
                }
                arg if arg.starts_with("-e") => {
                    expression = Some(arg[2..].to_string());
                }
                arg if !arg.starts_with('-') && expression.is_none() => {
                    expression = Some(arg.to_string());
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => eprintln!("sed: warning: unsupported option '{}'", args[i]),
            }
            i += 1;
        }

        let expr = match expression {
            Some(e) => e,
            None => return CommandOutput::error("sed: missing expression\n".to_string(), 1),
        };

        let parsed = parse_sed_command(&expr);

        if files.is_empty() {
            match stdin {
                Some(input) => {
                    let processed = apply_sed_commands(input, &parsed, quiet);
                    return CommandOutput::success(processed);
                }
                None => return CommandOutput::error("sed: missing file operand\n".to_string(), 1),
            }
        }

        let mut output = String::new();
        for file in &files {
            let content = match self.vfs.read_to_string(file, &self.cwd) {
                Ok(c) => c,
                Err(e) => return CommandOutput::error(format!("sed: {}: {}\n", file, e), 1),
            };

            let processed = apply_sed_commands(&content, &parsed, quiet);

            if in_place {
                if let Err(e) = self.vfs.write(file, &self.cwd, &processed) {
                    return CommandOutput::error(format!("sed: {}: {}\n", file, e), 1);
                }
            } else {
                output.push_str(&processed);
            }
        }

        CommandOutput::success(output)
    }
}

/// A single line address: number, `$` (last line) or /pattern/.
#[derive(Debug, Clone)]
enum LineSpec {
    Num(usize),
    Last,
    Pat(String),
}

/// Optional address (single line or inclusive range) attached to a command.
#[derive(Debug, Clone)]
struct Addr {
    start: LineSpec,
    end: Option<LineSpec>,
}

impl Addr {
    /// True when the 1-based `line_no` matches this address.
    fn matches(&self, line_no: usize, total: usize, line: &str) -> bool {
        let point = |spec: &LineSpec| -> Option<usize> {
            match spec {
                LineSpec::Num(n) => Some(*n),
                LineSpec::Last => Some(total),
                LineSpec::Pat(_) => None,
            }
        };
        match (&self.start, &self.end) {
            (LineSpec::Pat(p), None) => match regex::Regex::new(p) {
                Ok(re) => re.is_match(line),
                Err(_) => line.contains(p.as_str()),
            },
            (s, None) => point(s) == Some(line_no),
            (s, Some(e)) => {
                let lo = point(s).unwrap_or(1);
                let hi = point(e).unwrap_or(total);
                line_no >= lo && line_no <= hi
            }
        }
    }
}

enum SedCommand {
    Substitute {
        addr: Option<Addr>,
        pattern: String,
        replacement: String,
        global: bool,
        regex: Option<String>,
    },
    Delete(Option<Addr>),
    Print(Option<Addr>),
}

/// Parses an optional numeric/`$`//pat/ address prefix from `part`.
/// Returns (address, rest_after_address).
fn parse_addr(part: &str) -> (Option<Addr>, &str) {
    let parse_spec = |s: &str| -> Option<(LineSpec, usize)> {
        if s.starts_with('$') {
            return Some((LineSpec::Last, 1));
        }
        if s.starts_with('/') {
            if let Some(close) = s[1..].find('/') {
                return Some((LineSpec::Pat(s[1..1 + close].to_string()), close + 2));
            }
            return None;
        }
        let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            return None;
        }
        let n: usize = digits.parse().ok()?;
        Some((LineSpec::Num(n), digits.len()))
    };

    let (start, used) = match parse_spec(part) {
        Some(x) => x,
        None => return (None, part),
    };
    let rest = &part[used..];
    if let Some(rest2) = rest.strip_prefix(',') {
        if let Some((end, used2)) = parse_spec(rest2) {
            return (
                Some(Addr {
                    start,
                    end: Some(end),
                }),
                &rest2[used2..],
            );
        }
    }
    (Some(Addr { start, end: None }), rest)
}

fn parse_sed_command(expr: &str) -> Vec<SedCommand> {
    let mut commands = Vec::new();
    for part in expr.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        let (addr, rest) = parse_addr(part);
        let rest = rest.trim();

        match rest.chars().next() {
            Some('p') if rest == "p" => {
                commands.push(SedCommand::Print(addr));
                continue;
            }
            Some('d') if rest == "d" => {
                commands.push(SedCommand::Delete(addr));
                continue;
            }
            _ => {}
        }

        // Substitute: s/pat/rep/[flags]
        if let Some(stripped) = rest.strip_prefix('s') {
            let delim = match stripped.chars().next() {
                Some(d) => d,
                None => continue,
            };
            let body = &stripped[delim.len_utf8()..];
            let parts: Vec<&str> = body.splitn(3, delim).collect();
            if parts.len() >= 2 {
                let pattern = parts[0].to_string();
                let replacement = parts[1].to_string();
                let flags = parts.get(2).unwrap_or(&"");
                let global = flags.contains('g');

                let regex = if pattern.contains('*')
                    || pattern.contains('.')
                    || pattern.contains('[')
                    || pattern.contains('(')
                    || pattern.contains('\\')
                    || pattern.contains('^')
                    || pattern.contains('$')
                    || pattern.contains('+')
                    || pattern.contains('?')
                    || pattern.contains('{')
                    || pattern.contains('|')
                {
                    Some(pattern.clone())
                } else {
                    None
                };

                commands.push(SedCommand::Substitute {
                    addr,
                    pattern,
                    replacement,
                    global,
                    regex,
                });
            }
        }
    }
    commands
}

fn apply_sed_commands(content: &str, commands: &[SedCommand], quiet: bool) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    let mut output = String::new();

    for (idx, &line) in lines.iter().enumerate() {
        let line_no = idx + 1;
        let mut keep = true;
        let mut modified = line.to_string();
        let mut extra_prints = 0usize;

        for cmd in commands {
            match cmd {
                SedCommand::Delete(addr) => {
                    let hit = addr
                        .as_ref()
                        .map(|a| a.matches(line_no, total, &modified))
                        .unwrap_or(true);
                    if hit {
                        keep = false;
                        break;
                    }
                }
                SedCommand::Print(addr) => {
                    let hit = addr
                        .as_ref()
                        .map(|a| a.matches(line_no, total, &modified))
                        .unwrap_or(true);
                    if hit {
                        extra_prints += 1;
                    }
                }
                SedCommand::Substitute {
                    addr,
                    ref pattern,
                    ref replacement,
                    global,
                    ref regex,
                } => {
                    let hit = addr
                        .as_ref()
                        .map(|a| a.matches(line_no, total, &modified))
                        .unwrap_or(true);
                    if !hit {
                        continue;
                    }
                    let prev = modified.clone();
                    if let Some(re) = regex {
                        match regex::Regex::new(re) {
                            Ok(re) => {
                                if *global {
                                    modified =
                                        re.replace_all(&modified, replacement.as_str()).to_string();
                                } else {
                                    modified =
                                        re.replace(&modified, replacement.as_str()).to_string();
                                }
                            }
                            Err(_) => {
                                if *global {
                                    modified = prev.replace(pattern.as_str(), replacement.as_str());
                                } else {
                                    modified =
                                        prev.replacen(pattern.as_str(), replacement.as_str(), 1);
                                }
                            }
                        }
                    } else {
                        if *global {
                            modified = prev.replace(pattern.as_str(), replacement.as_str());
                        } else {
                            modified = prev.replacen(pattern.as_str(), replacement.as_str(), 1);
                        }
                    }
                }
            }
        }

        if quiet {
            // -n: only `p` output (printed once per matching p command).
            if keep {
                for _ in 0..extra_prints {
                    output.push_str(&modified);
                    output.push('\n');
                }
            }
        } else if keep {
            output.push_str(&modified);
            output.push('\n');
            // Without -n, `p` duplicates matching lines (GNU sed behavior).
            for _ in 0..extra_prints {
                output.push_str(&modified);
                output.push('\n');
            }
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(content: &str, expr: &str, quiet: bool) -> String {
        apply_sed_commands(content, &parse_sed_command(expr), quiet)
    }

    #[test]
    fn print_range_with_n() {
        let c = "l1\nl2\nl3\nl4\n";
        assert_eq!(run(c, "1,2p", true), "l1\nl2\n");
        assert_eq!(run(c, "2p", true), "l2\n");
        assert_eq!(run(c, "3,$p", true), "l3\nl4\n");
        assert_eq!(run(c, "$p", true), "l4\n");
    }

    #[test]
    fn print_pattern_with_n() {
        let c = "apple\nbanana\ncherry\n";
        assert_eq!(run(c, "/an/p", true), "banana\n");
    }

    #[test]
    fn delete_with_address() {
        let c = "l1\nl2\nl3\n";
        assert_eq!(run(c, "2d", false), "l1\nl3\n");
        assert_eq!(run(c, "1,2d", false), "l3\n");
        assert_eq!(run(c, "/l2/d", false), "l1\nl3\n");
    }

    #[test]
    fn substitute_plain_and_global() {
        assert_eq!(run("aaa\n", "s/a/b/", false), "baa\n");
        assert_eq!(run("aaa\n", "s/a/b/g", false), "bbb\n");
    }

    #[test]
    fn substitute_with_address() {
        let c = "x\nx\nx\n";
        assert_eq!(run(c, "2s/x/y/", false), "x\ny\nx\n");
        assert_eq!(run(c, "2,3s/x/y/", false), "x\ny\ny\n");
    }

    #[test]
    fn without_n_p_duplicates() {
        assert_eq!(run("a\nb\n", "1p", false), "a\na\nb\n");
    }

    fn mk_shell() -> Shell {
        use std::fs;
        use crate::vfs::Vfs;
        let dir = std::env::temp_dir().join(format!("fastshell_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let vfs = Vfs::new(dir).unwrap();
        Shell::new(vfs)
    }

    #[test]
    fn test_sed_help() {
        let mut shell = mk_shell();
        let out = shell.execute("sed", &["-h"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_sed_help_long() {
        let mut shell = mk_shell();
        let out = shell.execute("sed", &["--help"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }
}
