// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

impl Shell {
    pub fn cmd_head(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut lines_count: i64 = 10;
        let mut files = Vec::new();

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-n" => {
                    if i + 1 < args.len() {
                        lines_count = args[i + 1].parse().unwrap_or(10);
                        i += 1;
                    }
                }
                arg if arg.starts_with("-n") && arg.len() > 2 => {
                    lines_count = arg[2..].parse().unwrap_or(10);
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
            i += 1;
        }

        if files.is_empty() {
            match stdin {
                Some(input) => {
                    let lines: Vec<&str> = input.lines().collect();
                    let mut output = String::new();
                    if lines_count > 0 {
                        let take = (lines_count as usize).min(lines.len());
                        for &line in &lines[..take] {
                            output.push_str(line);
                            output.push('\n');
                        }
                    } else if lines_count < 0 {
                        let skip = (-lines_count) as usize;
                        let take = if skip < lines.len() {
                            lines.len() - skip
                        } else {
                            0
                        };
                        for &line in &lines[..take] {
                            output.push_str(line);
                            output.push('\n');
                        }
                    }
                    return CommandOutput::success(output);
                }
                None => return CommandOutput::error("head: missing file operand\n".to_string(), 1),
            }
        }

        let mut output = String::new();
        for file in &files {
            if files.len() > 1 {
                output.push_str(&format!("==> {} <==\n", file));
            }
            match self.vfs.read_to_string(file, &self.cwd) {
                Ok(content) => {
                    let lines: Vec<&str> = content.lines().collect();
                    if lines_count > 0 {
                        let take = (lines_count as usize).min(lines.len());
                        for &line in &lines[..take] {
                            output.push_str(line);
                            output.push('\n');
                        }
                    } else if lines_count < 0 {
                        let skip = (-lines_count) as usize;
                        let take = if skip < lines.len() {
                            lines.len() - skip
                        } else {
                            0
                        };
                        for &line in &lines[..take] {
                            output.push_str(line);
                            output.push('\n');
                        }
                    }
                }
                Err(e) => {
                    return CommandOutput::error(format!("head: {}: {}\n", file, e), 1);
                }
            }
        }
        CommandOutput::success(output)
    }
}
