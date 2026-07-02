// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

impl Shell {
    pub fn cmd_truncate(&self, args: &[&str]) -> CommandOutput {
        let mut size: Option<u64> = None;
        let mut files = Vec::new();

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-s" | "--size" => {
                    if i + 1 < args.len() {
                        size = parse_truncate_size(args[i + 1]);
                        i += 1;
                    }
                }
                arg if arg.starts_with("-s") && arg.len() > 2 => {
                    size = parse_truncate_size(&arg[2..]);
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
            i += 1;
        }

        if files.is_empty() {
            return CommandOutput::error("truncate: missing file operand\n".to_string(), 1);
        }

        let target_size = size.unwrap_or(0);

        for file in &files {
            let resolved = match self.vfs.resolve(file, &self.cwd) {
                Ok(p) => p,
                Err(e) => return CommandOutput::error(format!("truncate: {}: {}\n", file, e), 1),
            };

            let f = match std::fs::OpenOptions::new().write(true).open(&resolved) {
                Ok(f) => f,
                Err(e) => return CommandOutput::error(format!("truncate: {}: {}\n", file, e), 1),
            };

            if let Err(e) = f.set_len(target_size) {
                return CommandOutput::error(format!("truncate: {}: {}\n", file, e), 1);
            }
        }

        CommandOutput::success(String::new())
    }

    pub fn cmd_cmp(&self, args: &[&str]) -> CommandOutput {
        let mut silent = false;
        let mut files = Vec::new();

        for arg in args {
            match *arg {
                "-s" | "--silent" | "--quiet" => silent = true,
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
        }

        if files.len() < 2 {
            return CommandOutput::error("cmp: missing file operand\n".to_string(), 1);
        }

        let data1 = match self.vfs.read(&files[0], &self.cwd) {
            Ok(d) => d,
            Err(e) => return CommandOutput::error(format!("cmp: {}: {}\n", files[0], e), 1),
        };
        let data2 = match self.vfs.read(&files[1], &self.cwd) {
            Ok(d) => d,
            Err(e) => return CommandOutput::error(format!("cmp: {}: {}\n", files[1], e), 1),
        };

        let len = data1.len().min(data2.len());
        for k in 0..len {
            if data1[k] != data2[k] {
                let line = (k / 16) + 1;
                let byte = (k % 16) + 1;
                if !silent {
                    return CommandOutput {
                        stdout: format!(
                            "{} {} differ: byte {}, line {}\n",
                            files[0], files[1], byte, line
                        ),
                        stderr: String::new(),
                        exit_code: 1,
                    };
                }
                return CommandOutput {
                    stdout: String::new(),
                    stderr: String::new(),
                    exit_code: 1,
                };
            }
        }

        if data1.len() != data2.len() {
            let shorter = if data1.len() < data2.len() {
                &files[0]
            } else {
                &files[1]
            };
            if !silent {
                return CommandOutput {
                    stdout: format!("cmp: EOF on {} after byte {}\n", shorter, len + 1),
                    stderr: String::new(),
                    exit_code: 1,
                };
            }
            return CommandOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 1,
            };
        }

        CommandOutput::success(String::new())
    }

    pub fn cmd_strings(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut min_len = 4usize;
        let mut files = Vec::new();

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-n" => {
                    if i + 1 < args.len() {
                        min_len = args[i + 1].parse().unwrap_or(4);
                        i += 1;
                    }
                }
                arg if arg.starts_with("-n") && arg.len() > 2 => {
                    min_len = arg[2..].parse().unwrap_or(4);
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
            i += 1;
        }

        let data = if files.is_empty() {
            match stdin {
                Some(s) => s.as_bytes().to_vec(),
                None => return CommandOutput::error("strings: missing input\n".to_string(), 1),
            }
        } else {
            let mut all = Vec::new();
            for file in &files {
                match self.vfs.read(file, &self.cwd) {
                    Ok(d) => all.extend_from_slice(&d),
                    Err(e) => {
                        return CommandOutput::error(format!("strings: {}: {}\n", file, e), 1)
                    }
                }
            }
            all
        };

        let mut output = String::new();
        let mut current = String::new();
        for &byte in &data {
            if byte >= 0x20 && byte < 0x7f {
                current.push(byte as char);
            } else {
                if current.len() >= min_len {
                    output.push_str(&current);
                    output.push('\n');
                }
                current.clear();
            }
        }
        if current.len() >= min_len {
            output.push_str(&current);
            output.push('\n');
        }

        CommandOutput::success(output)
    }

    pub fn cmd_fold(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut width = 80usize;
        let mut files = Vec::new();

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-w" | "--width" => {
                    if i + 1 < args.len() {
                        width = args[i + 1].parse().unwrap_or(80);
                        i += 1;
                    }
                }
                arg if arg.starts_with("-w") && arg.len() > 2 => {
                    width = arg[2..].parse().unwrap_or(80);
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
            i += 1;
        }

        let content = if files.is_empty() {
            match stdin {
                Some(s) => s.to_string(),
                None => return CommandOutput::error("fold: missing input\n".to_string(), 1),
            }
        } else {
            let mut all = String::new();
            for file in &files {
                match self.vfs.read_to_string(file, &self.cwd) {
                    Ok(c) => all.push_str(&c),
                    Err(e) => return CommandOutput::error(format!("fold: {}: {}\n", file, e), 1),
                }
            }
            all
        };

        let mut output = String::new();
        for line in content.lines() {
            let mut pos = 0;
            let chars: Vec<char> = line.chars().collect();
            while pos < chars.len() {
                let end = (pos + width).min(chars.len());
                output.extend(&chars[pos..end]);
                output.push('\n');
                pos = end;
            }
        }

        CommandOutput::success(output)
    }

    pub fn cmd_expand(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut tab_size = 8usize;
        let mut files = Vec::new();

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-t" | "--tabs" => {
                    if i + 1 < args.len() {
                        tab_size = args[i + 1].parse().unwrap_or(8);
                        i += 1;
                    }
                }
                arg if arg.starts_with("-t") && arg.len() > 2 => {
                    tab_size = arg[2..].parse().unwrap_or(8);
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
            i += 1;
        }

        let content = match get_file_content(self, &files, stdin, "expand") {
            Ok(c) => c,
            Err(e) => return e,
        };

        let mut output = String::new();
        for line in content.lines() {
            let mut col = 0;
            for ch in line.chars() {
                if ch == '\t' {
                    let spaces = tab_size - (col % tab_size);
                    output.push_str(&" ".repeat(spaces));
                    col += spaces;
                } else {
                    output.push(ch);
                    col += 1;
                }
            }
            output.push('\n');
        }

        CommandOutput::success(output)
    }

    pub fn cmd_unexpand(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut tab_size = 8usize;
        let mut all = false;
        let mut files = Vec::new();

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-t" | "--tabs" => {
                    if i + 1 < args.len() {
                        tab_size = args[i + 1].parse().unwrap_or(8);
                        i += 1;
                    }
                }
                "-a" | "--all" => all = true,
                arg if arg.starts_with("-t") && arg.len() > 2 => {
                    tab_size = arg[2..].parse().unwrap_or(8);
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
            i += 1;
        }

        let content = match get_file_content(self, &files, stdin, "unexpand") {
            Ok(c) => c,
            Err(e) => return e,
        };

        let mut output = String::new();
        for line in content.lines() {
            let chars: Vec<char> = line.chars().collect();
            let mut result = String::new();
            let mut col = 0;
            let mut space_run = 0;

            for &ch in &chars {
                if ch == ' ' {
                    space_run += 1;
                } else {
                    result.push_str(&" ".repeat(space_run));
                    result.push(ch);
                    col += space_run + 1;
                    space_run = 0;
                }

                if space_run > 0 && ((col + space_run) % tab_size == 0) {
                    if all || space_run >= 2 {
                        result.push('\t');
                        col += space_run;
                        space_run = 0;
                    }
                }
            }
            result.push_str(&" ".repeat(space_run));
            result.push('\n');
            output.push_str(&result);
        }

        CommandOutput::success(output)
    }

    pub fn cmd_yes(&self, args: &[&str]) -> CommandOutput {
        let msg = if args.is_empty() {
            "y".to_string()
        } else {
            args.join(" ")
        };

        let mut output = String::new();
        for _ in 0..10000 {
            output.push_str(&msg);
            output.push('\n');
        }

        CommandOutput::success(output)
    }
}

fn get_file_content(
    shell: &Shell,
    files: &[String],
    stdin: Option<&str>,
    cmd: &str,
) -> Result<String, CommandOutput> {
    if files.is_empty() {
        match stdin {
            Some(s) => Ok(s.to_string()),
            None => Err(CommandOutput::error(format!("{}: missing input\n", cmd), 1)),
        }
    } else {
        let mut all = String::new();
        for file in files {
            match shell.vfs.read_to_string(file, &shell.cwd) {
                Ok(c) => all.push_str(&c),
                Err(e) => {
                    return Err(CommandOutput::error(
                        format!("{}: {}: {}\n", cmd, file, e),
                        1,
                    ))
                }
            }
        }
        Ok(all)
    }
}

fn parse_truncate_size(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some(_rest) = s.strip_prefix('+') {
        None // extend mode not supported
    } else if let Some(_rest) = s.strip_prefix('-') {
        None // shrink mode
    } else if let Some(rest) = s.strip_suffix('K') {
        rest.parse::<u64>().ok().map(|n| n * 1024)
    } else if let Some(rest) = s.strip_suffix('M') {
        rest.parse::<u64>().ok().map(|n| n * 1024 * 1024)
    } else if let Some(rest) = s.strip_suffix('G') {
        rest.parse::<u64>().ok().map(|n| n * 1024 * 1024 * 1024)
    } else {
        s.parse().ok()
    }
}
