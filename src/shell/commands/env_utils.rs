// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

impl Shell {
    pub fn cmd_env(&self, args: &[&str]) -> CommandOutput {
        let mut output = String::new();
        for (key, value) in std::env::vars() {
            output.push_str(&format!("{}={}\n", key, value));
        }
        let _ = args;
        CommandOutput::success(output)
    }

    pub fn cmd_printenv(&self, args: &[&str]) -> CommandOutput {
        if args.is_empty() {
            return self.cmd_env(args);
        }
        let mut output = String::new();
        for arg in args {
            if !arg.starts_with('-') {
                match std::env::var(arg) {
                    Ok(val) => {
                        output.push_str(&val);
                        output.push('\n');
                    }
                    Err(_) => {}
                }
            }
        }
        let is_empty = output.is_empty();
        CommandOutput {
            stdout: output,
            stderr: String::new(),
            exit_code: if is_empty { 1 } else { 0 },
        }
    }

    pub fn cmd_printf(&self, args: &[&str], _stdin: Option<&str>) -> CommandOutput {
        if args.is_empty() {
            return CommandOutput::error("printf: missing format\n".to_string(), 1);
        }

        let format = args[0];
        let data_args: Vec<&str> = args[1..].to_vec();
        let output = simple_printf(format, &data_args);
        CommandOutput::success(output)
    }

    pub fn cmd_basename(&self, args: &[&str]) -> CommandOutput {
        let files: Vec<&str> = args
            .iter()
            .filter(|a| !a.starts_with('-'))
            .copied()
            .collect();
        if files.is_empty() {
            return CommandOutput::error("basename: missing operand\n".to_string(), 1);
        }

        let path = files[0];
        let suffix = if files.len() > 1 {
            Some(files[1])
        } else {
            None
        };

        let path = std::path::Path::new(path);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        let result = match suffix {
            Some(s) if name.ends_with(s) => name[..name.len() - s.len()].to_string(),
            _ => name,
        };

        CommandOutput::success(result + "\n")
    }

    pub fn cmd_dirname(&self, args: &[&str]) -> CommandOutput {
        let files: Vec<&str> = args
            .iter()
            .filter(|a| !a.starts_with('-'))
            .copied()
            .collect();
        if files.is_empty() {
            return CommandOutput::error("dirname: missing operand\n".to_string(), 1);
        }

        let path = std::path::Path::new(files[0]);
        let parent = path
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| ".".to_string());

        CommandOutput::success(parent + "\n")
    }

    pub fn cmd_realpath(&self, args: &[&str]) -> CommandOutput {
        let files: Vec<&str> = args
            .iter()
            .filter(|a| !a.starts_with('-'))
            .copied()
            .collect();
        if files.is_empty() {
            return CommandOutput::error("realpath: missing operand\n".to_string(), 1);
        }

        let mut output = String::new();
        for file in &files {
            match self.vfs.resolve(file, &self.cwd) {
                Ok(resolved) => match std::fs::canonicalize(&resolved) {
                    Ok(canon) => output.push_str(&format!("{}\n", canon.display())),
                    Err(e) => output.push_str(&format!("realpath: {}: {}\n", file, e)),
                },
                Err(e) => output.push_str(&format!("realpath: {}: {}\n", file, e)),
            }
        }

        CommandOutput::success(output)
    }
}

fn simple_printf(format: &str, args: &[&str]) -> String {
    let mut result = String::new();
    let mut arg_idx = 0;
    let chars: Vec<char> = format.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            match chars[i + 1] {
                'n' => result.push('\n'),
                't' => result.push('\t'),
                '\\' => result.push('\\'),
                'r' => result.push('\r'),
                '0' => {}
                c => {
                    result.push('\\');
                    result.push(c);
                }
            }
            i += 2;
        } else if chars[i] == '%' && i + 1 < chars.len() {
            match chars[i + 1] {
                '%' => {
                    result.push('%');
                    i += 2;
                }
                's' => {
                    if arg_idx < args.len() {
                        result.push_str(args[arg_idx]);
                        arg_idx += 1;
                    }
                    i += 2;
                }
                'd' | 'i' => {
                    if arg_idx < args.len() {
                        let val: i64 = args[arg_idx].parse().unwrap_or(0);
                        result.push_str(&val.to_string());
                        arg_idx += 1;
                    }
                    i += 2;
                }
                'f' => {
                    if arg_idx < args.len() {
                        let val: f64 = args[arg_idx].parse().unwrap_or(0.0);
                        result.push_str(&format!("{:.6}", val));
                        arg_idx += 1;
                    }
                    i += 2;
                }
                'x' => {
                    if arg_idx < args.len() {
                        let val: u64 = u64::from_str_radix(args[arg_idx], 16).unwrap_or(0);
                        result.push_str(&format!("{:x}", val));
                        arg_idx += 1;
                    }
                    i += 2;
                }
                'o' => {
                    if arg_idx < args.len() {
                        let val: u64 = u64::from_str_radix(args[arg_idx], 8).unwrap_or(0);
                        result.push_str(&format!("{:o}", val));
                        arg_idx += 1;
                    }
                    i += 2;
                }
                'u' => {
                    if arg_idx < args.len() {
                        let val: u64 = args[arg_idx].parse().unwrap_or(0);
                        result.push_str(&val.to_string());
                        arg_idx += 1;
                    }
                    i += 2;
                }
                _ => {
                    result.push(chars[i]);
                    i += 1;
                }
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    result
}
