use crate::shell::{Shell, CommandOutput};

impl Shell {
    pub fn cmd_tail(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut lines_count: i64 = 10;
        let mut from_start = false;
        let mut files = Vec::new();

        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-n" => {
                    if i + 1 < args.len() {
                        let val = args[i + 1];
                        if let Some(v) = val.strip_prefix('+') {
                            from_start = true;
                            lines_count = v.parse().unwrap_or(10);
                        } else {
                            let n: i64 = val.parse().unwrap_or(10);
                            lines_count = n;
                        }
                        i += 1;
                    }
                }
                arg if arg.starts_with("-n") && arg.len() > 2 => {
                    let val = &arg[2..];
                    if let Some(v) = val.strip_prefix('+') {
                        from_start = true;
                        lines_count = v.parse().unwrap_or(10);
                    } else {
                        lines_count = val.parse().unwrap_or(10);
                    }
                }
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
            i += 1;
        }

        let count = if lines_count < 0 { 10usize } else { lines_count as usize };

        if files.is_empty() {
            match stdin {
                Some(input) => {
                    let lines: Vec<&str> = input.lines().collect();
                    let output = if from_start {
                        let start = (count.saturating_sub(1)).min(lines.len());
                        lines[start..].join("\n")
                    } else {
                        let start = if lines.len() > count { lines.len() - count } else { 0 };
                        lines[start..].join("\n")
                    };
                    let mut result = String::new();
                    if !output.is_empty() {
                        result.push_str(&output);
                    }
                    if output.ends_with('\n') || output.is_empty() {
                        return CommandOutput::success(result);
                    }
                    return CommandOutput::success(result);
                }
                None => return CommandOutput::error("tail: missing file operand\n".to_string(), 1),
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
                    if from_start {
                        let start = (count.saturating_sub(1)).min(lines.len());
                        for &line in &lines[start..] {
                            output.push_str(line);
                            output.push('\n');
                        }
                    } else {
                        let start = if lines.len() > count { lines.len() - count } else { 0 };
                        for &line in &lines[start..] {
                            output.push_str(line);
                            output.push('\n');
                        }
                    }
                }
                Err(e) => {
                    return CommandOutput::error(format!("tail: {}: {}\n", file, e), 1);
                }
            }
        }
        CommandOutput::success(output)
    }
}
