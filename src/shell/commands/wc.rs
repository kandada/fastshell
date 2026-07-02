// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

impl Shell {
    pub fn cmd_wc(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut show_lines = true;
        let mut show_words = true;
        let mut show_bytes = true;
        let mut bytes_mode = true;
        let mut files = Vec::new();

        for arg in args {
            match *arg {
                "-l" => {
                    show_words = false;
                    show_bytes = false;
                }
                "-w" => {
                    show_lines = false;
                    show_bytes = false;
                }
                "-c" => {
                    show_lines = false;
                    show_words = false;
                    bytes_mode = true;
                }
                "-m" => {
                    show_lines = false;
                    show_words = false;
                    bytes_mode = false;
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
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
                    return CommandOutput::success(parts.join("") + "\n");
                }
                None => return CommandOutput::error("wc: missing file operand\n".to_string(), 1),
            }
        }

        let mut output = String::new();
        let mut total_lines = 0usize;
        let mut total_words = 0usize;
        let mut total_bc = 0usize;

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
                    parts.push(file.clone());
                    output.push_str(&parts.join(" "));
                    output.push('\n');
                    total_lines += l;
                    total_words += w;
                    total_bc += bc;
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
            parts.push("total".to_string());
            output.push_str(&parts.join(" "));
            output.push('\n');
        }

        CommandOutput::success(output)
    }
}
