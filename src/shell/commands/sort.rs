use crate::shell::{Shell, CommandOutput};

impl Shell {
    pub fn cmd_sort(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut files = Vec::new();
        let mut numeric = false;
        let mut reverse = false;
        let mut unique = false;

        for arg in args {
            match *arg {
                "-n" => numeric = true,
                "-r" => reverse = true,
                "-u" => unique = true,
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
        }

        let mut all_lines = Vec::new();

        if files.is_empty() {
            match stdin {
                Some(s) => {
                    for line in s.lines() {
                        all_lines.push(line.to_string());
                    }
                }
                None => return CommandOutput::error("sort: missing file operand\n".to_string(), 1),
            }
        } else {
            for file in &files {
                match self.vfs.read_to_string(file, &self.cwd) {
                    Ok(content) => {
                        for line in content.lines() {
                            all_lines.push(line.to_string());
                        }
                    }
                    Err(e) => return CommandOutput::error(format!("sort: {}: {}\n", file, e), 1),
                }
            }
        }

        if numeric {
            all_lines.sort_by(|a, b| {
                let na = a.parse::<f64>().unwrap_or(f64::NAN);
                let nb = b.parse::<f64>().unwrap_or(f64::NAN);
                na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal)
            });
        } else {
            all_lines.sort();
        }

        if reverse {
            all_lines.reverse();
        }

        let mut output = String::new();
        let mut prev: Option<&str> = None;
        for line in &all_lines {
            if unique {
                if prev == Some(line.as_str()) {
                    continue;
                }
                prev = Some(line.as_str());
            }
            output.push_str(line);
            output.push('\n');
        }

        CommandOutput::success(output)
    }
}
