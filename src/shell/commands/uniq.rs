use crate::shell::{Shell, CommandOutput};

impl Shell {
    pub fn cmd_uniq(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut files = Vec::new();
        let mut count = false;

        for arg in args {
            match *arg {
                "-c" => count = true,
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
        }

        if files.is_empty() {
            match stdin {
                Some(input) => {
                    let lines: Vec<&str> = input.lines().collect();
                    let mut output = String::new();
                    let mut i = 0;
                    while i < lines.len() {
                        let mut cnt = 1usize;
                        while i + cnt < lines.len() && lines[i + cnt] == lines[i] {
                            cnt += 1;
                        }
                        if count {
                            output.push_str(&format!("{:>7} {}\n", cnt, lines[i]));
                        } else {
                            output.push_str(lines[i]);
                            output.push('\n');
                        }
                        i += cnt;
                    }
                    return CommandOutput::success(output);
                }
                None => return CommandOutput::error("uniq: missing file operand\n".to_string(), 1),
            }
        }

        let mut output = String::new();
        for file in &files {
            let content = match self.vfs.read_to_string(file, &self.cwd) {
                Ok(c) => c,
                Err(e) => return CommandOutput::error(format!("uniq: {}: {}\n", file, e), 1),
            };

            let lines: Vec<&str> = content.lines().collect();
            let mut i = 0;
            while i < lines.len() {
                let mut cnt = 1usize;
                while i + cnt < lines.len() && lines[i + cnt] == lines[i] {
                    cnt += 1;
                }
                if count {
                    output.push_str(&format!("{:>7} {}\n", cnt, lines[i]));
                } else {
                    output.push_str(lines[i]);
                    output.push('\n');
                }
                i += cnt;
            }
        }

        CommandOutput::success(output)
    }
}
