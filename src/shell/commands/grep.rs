// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};
use regex::Regex;

impl Shell {
    pub fn cmd_grep(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut pattern: Option<String> = None;
        let mut files = Vec::new();
        let mut ignore_case = false;
        let mut invert = false;
        let mut count_only = false;
        let mut show_line_number = false;
        let mut fixed_strings = false;

        for arg in args {
            match *arg {
                "-i" => ignore_case = true,
                "-v" => invert = true,
                "-c" => count_only = true,
                "-n" => show_line_number = true,
                "-F" | "--fixed-strings" => fixed_strings = true,
                _ if arg.starts_with('-') => {}
                _ if pattern.is_none() => pattern = Some(arg.to_string()),
                _ => files.push(arg.to_string()),
            }
        }

        let pattern = match pattern {
            Some(p) => p,
            None => return CommandOutput::error("grep: missing pattern\n".to_string(), 2),
        };

        let regex = if fixed_strings {
            let escaped = regex::escape(&pattern);
            build_regex(&escaped, ignore_case).ok()
        } else {
            build_regex(&pattern, ignore_case).ok()
        };

        let matcher: Box<dyn Fn(&str) -> bool> = match regex {
            Some(re) => Box::new(move |line: &str| re.is_match(line)),
            None => {
                let p = pattern.clone();
                if ignore_case {
                    let pl = p.to_lowercase();
                    Box::new(move |line: &str| line.to_lowercase().contains(&pl))
                } else {
                    Box::new(move |line: &str| line.contains(&p))
                }
            }
        };

        let mut output = String::new();
        let mut stderr = String::new();
        let mut file_errors = 0usize;
        let mut total_matches = 0usize;

        if files.is_empty() {
            let input = match stdin {
                Some(s) => s.to_string(),
                None => return CommandOutput::error("grep: missing file operand\n".to_string(), 2),
            };
            let mut count = 0usize;
            for (line_num, line) in input.lines().enumerate() {
                let matches = matcher(line);
                let show = if invert { !matches } else { matches };
                if show {
                    count += 1;
                    if !count_only {
                        if show_line_number {
                            output.push_str(&format!("{}:", line_num + 1));
                        }
                        output.push_str(line);
                        output.push('\n');
                    }
                }
            }
            total_matches = count;
            if count_only {
                output.push_str(&format!("{}\n", count));
            }
        } else {
            let multi_file = files.len() > 1;
            for file in &files {
                match self.vfs.read_to_string(file, &self.cwd) {
                    Ok(content) => {
                        let mut count = 0usize;
                        for (line_num, line) in content.lines().enumerate() {
                            let matches = matcher(line);
                            let show = if invert { !matches } else { matches };
                            if show {
                                count += 1;
                                if !count_only {
                                    if multi_file {
                                        output.push_str(&format!("{}:", file));
                                    }
                                    if show_line_number {
                                        output.push_str(&format!("{}:", line_num + 1));
                                    }
                                    output.push_str(line);
                                    output.push('\n');
                                }
                            }
                        }
                        if count_only {
                            if multi_file {
                                output.push_str(&format!("{}:", file));
                            }
                            output.push_str(&format!("{}\n", count));
                        }
                        total_matches += count;
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
}

fn build_regex(pattern: &str, ignore_case: bool) -> Result<Regex, regex::Error> {
    if ignore_case {
        Regex::new(&format!("(?i){}", pattern))
    } else {
        Regex::new(pattern)
    }
}
