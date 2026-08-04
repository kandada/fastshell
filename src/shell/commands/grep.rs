// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};
use regex::Regex;

const GREP_HELP_TEXT: &str = "\
Usage: grep [OPTION]... PATTERN [FILE]...
       rg [OPTION]... PATTERN [PATH]...

Pattern matching:
  -F, --fixed-strings     Interpret pattern as fixed string
  -e, --regexp PATTERN    Use PATTERN as the pattern
  -w, --word-regexp       Match whole words only

Matching control:
  -i                       Ignore case
  -v                       Invert match (select non-matching lines)
  -c                       Count matching lines
  -o, --only-matching      Print only matched parts
  -l, --files-with-matches Print only file names
  -n                       Show line numbers
  -r, -R                   Recursive search
  -N, --no-line-number     Suppress line numbers (rg)

rg extras: -g (glob), -t (type), -T (type-not), -A/-B/-C (context),
            -m (max-count), --max-depth, -S (smart-case), --count

  -h, --help               Show this help\n";

impl Shell {
    /// ripgrep-style entry point mapped onto the built-in grep engine:
    /// line numbers on by default, searches `.` recursively when no path is
    /// given, and directory arguments are searched recursively automatically.
    pub fn cmd_rg(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(GREP_HELP_TEXT.to_string());
        }
        let mut flags: Vec<String> = Vec::new();
        let mut positional: Vec<String> = Vec::new();
        let mut no_line_number = false;
        let mut count_or_list = false;

        let mut i = 0;
        while i < args.len() {
            let a = args[i];
            match a {
                "-N" | "--no-line-number" => no_line_number = true,
                "-e" | "--regexp" => {
                    if i + 1 < args.len() {
                        positional.insert(0, args[i + 1].to_string());
                        i += 1;
                    }
                }
                // Value-taking rg options we don't model — consume the value.
                "-g" | "--glob" | "-t" | "--type" | "-T" | "--type-not" | "-A" | "-B" | "-C"
                | "-m" | "--max-count" | "--max-depth" => {
                    i += 1;
                }
                "--fixed-strings" => flags.push("-F".to_string()),
                "--ignore-case" | "-S" | "--smart-case" => flags.push("-i".to_string()),
                "--files-with-matches" => {
                    flags.push("-l".to_string());
                    count_or_list = true;
                }
                "--word-regexp" => flags.push("-w".to_string()),
                "--invert-match" => flags.push("-v".to_string()),
                "--count" => {
                    flags.push("-c".to_string());
                    count_or_list = true;
                }
                a if a.starts_with("--") => {
                    eprintln!("grep: warning: unsupported option '{}'", a);
                } // --no-heading, --color=..., etc.
                a if a.starts_with('-') && a.len() > 1 => {
                    if a.contains('c') || a.contains('l') {
                        count_or_list = true;
                    }
                    flags.push(a.to_string());
                }
                _ => positional.push(a.to_string()),
            }
            i += 1;
        }

        // rg prints line numbers by default (when not counting/listing).
        if !no_line_number && !count_or_list {
            flags.push("-n".to_string());
        }

        // Paths: everything after the pattern. Default to a recursive search
        // of the current directory when reading from files (not stdin).
        let has_paths = positional.len() > 1;
        if !has_paths && stdin.is_none() {
            flags.push("-r".to_string());
            positional.push(".".to_string());
        } else if has_paths {
            // A directory argument implies recursion (rg behavior).
            let any_dir = positional[1..].iter().any(|p| {
                self.vfs
                    .resolve(p, &self.cwd)
                    .map(|r| r.is_dir())
                    .unwrap_or(false)
            });
            if any_dir {
                flags.push("-r".to_string());
            }
        }

        let mut full: Vec<&str> = flags.iter().map(|s| s.as_str()).collect();
        full.extend(positional.iter().map(|s| s.as_str()));
        self.cmd_grep(&full, stdin)
    }

    pub fn cmd_grep(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        if args.contains(&"-h") || args.contains(&"--help") {
            return CommandOutput::success(GREP_HELP_TEXT.to_string());
        }
        let mut pattern: Option<String> = None;
        let mut files = Vec::new();
        let mut ignore_case = false;
        let mut invert = false;
        let mut count_only = false;
        let mut show_line_number = false;
        let mut fixed_strings = false;
        let mut recursive = false;
        let mut files_with_matches = false;
        let mut only_matching = false;
        let mut word_regexp = false;

        for arg in args {
            match *arg {
                "-i" => ignore_case = true,
                "-v" => invert = true,
                "-c" => count_only = true,
                "-n" => show_line_number = true,
                "-F" | "--fixed-strings" => fixed_strings = true,
                "-r" | "-R" => recursive = true,
                "-l" | "--files-with-matches" => files_with_matches = true,
                "-o" | "--only-matching" => only_matching = true,
                "-w" | "--word-regexp" => word_regexp = true,
                "--" => {}, // end of options
                a if a.starts_with('-') && a.len() > 2 => {
                    // Support combined short flags: -ril, -rn, -vc, -ic, etc.
                    for ch in a.chars().skip(1) {
                        match ch {
                            'i' => ignore_case = true,
                            'v' => invert = true,
                            'c' => count_only = true,
                            'n' => show_line_number = true,
                            'F' => fixed_strings = true,
                            'r' | 'R' => recursive = true,
                            'l' => files_with_matches = true,
                            'o' => only_matching = true,
                            'w' => word_regexp = true,
                            _ => eprintln!("grep: warning: unsupported option '-{}'", ch),
                        }
                    }
                }
                _ if arg.starts_with('-') => {
                    eprintln!("grep: warning: unsupported option '{}'", arg);
                } // unknown single-char flags
                _ if pattern.is_none() => pattern = Some(arg.to_string()),
                _ => files.push(arg.to_string()),
            }
        }

        let pattern = match pattern {
            Some(p) => p,
            None => return CommandOutput::error("grep: missing pattern\n".to_string(), 2),
        };

        // Build pattern, optionally wrapping in word boundaries
        let effective_pattern = if word_regexp {
            format!(r"\b{}\b", if fixed_strings { regex::escape(&pattern) } else { pattern.clone() })
        } else if fixed_strings {
            regex::escape(&pattern)
        } else {
            pattern.to_string()
        };

        let regex = match build_regex(&effective_pattern, ignore_case) {
            Ok(r) => Some(r),
            Err(e) => {
                return CommandOutput::error(format!("grep: {e}"), 2);
            }
        };

        let matcher: Box<dyn Fn(&str) -> Option<Vec<String>>> = match regex {
            Some(re) => {
                if only_matching {
                    Box::new(move |line: &str| {
                        let found: Vec<String> = re.find_iter(line)
                            .map(|m| m.as_str().to_string())
                            .collect();
                        if found.is_empty() { None } else { Some(found) }
                    })
                } else {
                    Box::new(move |line: &str| {
                        if re.is_match(line) { Some(vec![line.to_string()]) } else { None }
                    })
                }
            }
            None => {
                let p = if word_regexp { pattern.clone() } else { pattern.clone() };
                if ignore_case {
                    let pl = p.to_lowercase();
                    if only_matching {
                        Box::new(move |line: &str| {
                            let lower = line.to_lowercase();
                            if lower.contains(&pl) {
                                Some(vec![find_substring_match(line, &p, true)])
                            } else {
                                None
                            }
                        })
                    } else {
                        Box::new(move |line: &str| {
                            if line.to_lowercase().contains(&pl) {
                                Some(vec![line.to_string()])
                            } else {
                                None
                            }
                        })
                    }
                } else {
                    if only_matching {
                        Box::new(move |line: &str| {
                            if line.contains(&p) {
                                Some(vec![p.clone()])
                            } else {
                                None
                            }
                        })
                    } else {
                        Box::new(move |line: &str| {
                            if line.contains(&p) {
                                Some(vec![line.to_string()])
                            } else {
                                None
                            }
                        })
                    }
                }
            }
        };

        let mut output = String::new();
        let mut stderr = String::new();
        let mut file_errors = 0usize;
        let mut total_matches = 0usize;

        // Expand recursive directories into flat file list
        let mut all_files: Vec<String> = Vec::new();
        for file in &files {
            let path = self.vfs.resolve(file, &self.cwd);
            match path {
                Ok(p) if p.is_dir() && recursive => {
                    self.collect_files_recursive(file, &mut all_files, &mut stderr);
                }
                _ => {
                    all_files.push(file.to_string());
                }
            }
        }

        if all_files.is_empty() && files.is_empty() {
            let input = match stdin {
                Some(s) => s.to_string(),
                None => return CommandOutput::error("grep: missing file operand\n".to_string(), 2),
            };
            let count = grep_lines(
                &input, &matcher, invert, count_only, show_line_number,
                files_with_matches, only_matching, None, &mut output,
            );
            total_matches = count;
            if count_only {
                output.push_str(&format!("{}\n", count));
            }
        } else {
            let multi_file = all_files.len() > 1;
            for file in &all_files {
                match self.vfs.read_to_string(file, &self.cwd) {
                    Ok(content) => {
                        let count = grep_lines(
                            &content, &matcher, invert, count_only, show_line_number,
                            files_with_matches, only_matching,
                            Some((file, multi_file)), &mut output,
                        );
                        if count > 0 {
                            if files_with_matches {
                                output.push_str(file);
                                output.push('\n');
                            }
                            total_matches += 1; // count files, not lines, for -l
                        } else if !files_with_matches {
                            total_matches += 0; // for non-l mode, total is line count
                        }
                        if !files_with_matches {
                            total_matches += count;
                        }
                    }
                    Err(e) => {
                        stderr.push_str(&format!("grep: {}: {}\n", file, e));
                        file_errors += 1;
                    }
                }
            }
        }

        let exit_code = if file_errors > 0 {
            2
        } else if total_matches == 0 {
            1
        } else {
            0
        };

        CommandOutput {
            stdout: output,
            stderr,
            exit_code,
        }
    }

    /// Recursively collect regular files under `dir_path` into `out`.
    fn collect_files_recursive(&self, dir_path: &str, out: &mut Vec<String>, stderr: &mut String) {
        match self.vfs.list_dir(dir_path, &self.cwd) {
            Ok(entries) => {
                for entry in &entries {
                    let full = if dir_path.ends_with('/') {
                        format!("{}{}", dir_path, entry.name)
                    } else {
                        format!("{}/{}", dir_path, entry.name)
                    };
                    if entry.is_dir {
                        self.collect_files_recursive(&full, out, stderr);
                    } else {
                        out.push(full);
                    }
                }
            }
            Err(e) => {
                stderr.push_str(&format!("grep: {}: {}\n", dir_path, e));
            }
        }
    }
}

fn build_regex(pattern: &str, ignore_case: bool) -> Result<Regex, regex::Error> {
    if ignore_case {
        Regex::new(&format!("(?i){}", pattern))
    } else {
        Regex::new(pattern)
    }
}

fn find_substring_match(line: &str, pattern: &str, ignore_case: bool) -> String {
    if ignore_case {
        let lower = line.to_lowercase();
        let pl = pattern.to_lowercase();
        if let Some(pos) = lower.find(&pl) {
            line[pos..pos + pattern.len()].to_string()
        } else {
            pattern.to_string()
        }
    } else {
        if let Some(pos) = line.find(pattern) {
            line[pos..pos + pattern.len()].to_string()
        } else {
            pattern.to_string()
        }
    }
}

fn grep_lines(
    content: &str,
    matcher: &dyn Fn(&str) -> Option<Vec<String>>,
    invert: bool,
    count_only: bool,
    show_line_number: bool,
    files_with_matches: bool,
    only_matching: bool,
    file_label: Option<(&str, bool)>, // (filename, multi_file)
    output: &mut String,
) -> usize {
    if files_with_matches {
        // Just check if any line matches
        for line in content.lines() {
            let matches = matcher(line);
            let has_match = matches.is_some();
            if (invert && !has_match) || (!invert && has_match) {
                return 1;
            }
        }
        return 0;
    }

    let (fname, multi) = file_label.unwrap_or(("", false));
    let mut count = 0usize;

    for (line_num, line) in content.lines().enumerate() {
        let match_result = matcher(line);
        let has_match = match_result.is_some();
        let show = if invert { !has_match } else { has_match };

        if show {
            count += 1;
            if !count_only {
                let prefix = if multi {
                    format!("{}:", fname)
                } else {
                    String::new()
                };

                if only_matching {
                    if let Some(matches) = match_result {
                        for m in matches {
                            if !prefix.is_empty() {
                                output.push_str(&prefix);
                            }
                            if show_line_number {
                                output.push_str(&format!("{}:", line_num + 1));
                            }
                            output.push_str(&m);
                            output.push('\n');
                        }
                    }
                } else {
                    if !prefix.is_empty() {
                        output.push_str(&prefix);
                    }
                    if show_line_number {
                        output.push_str(&format!("{}:", line_num + 1));
                    }
                    output.push_str(line);
                    output.push('\n');
                }
            }
        }
    }

    if count_only {
        let prefix = if multi {
            format!("{}:", fname)
        } else {
            String::new()
        };
        output.push_str(&format!("{}{}\n", prefix, count));
    }

    count
}

#[cfg(test)]
mod tests {
    use crate::shell::Shell;
    use crate::vfs::Vfs;

    fn mk_shell() -> Shell {
        use std::fs;
        let dir = std::env::temp_dir().join(format!("fastshell_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let vfs = Vfs::new(dir).unwrap();
        Shell::new(vfs)
    }

    #[test]
    fn test_grep_help() {
        let mut shell = mk_shell();
        let out = shell.execute("grep", &["-h"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_grep_help_long() {
        let mut shell = mk_shell();
        let out = shell.execute("grep", &["--help"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_rg_help() {
        let mut shell = mk_shell();
        let out = shell.execute("rg", &["-h"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }

    #[test]
    fn test_rg_help_long() {
        let mut shell = mk_shell();
        let out = shell.execute("rg", &["--help"], None);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.is_empty());
    }
}
