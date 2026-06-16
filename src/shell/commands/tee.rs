use crate::shell::{Shell, CommandOutput};

impl Shell {
    pub fn cmd_tee(&self, args: &[&str], stdin: Option<&str>) -> CommandOutput {
        let mut append = false;
        let mut files = Vec::new();

        for arg in args {
            match *arg {
                "-a" => append = true,
                arg if !arg.starts_with('-') => files.push(arg.to_string()),
                _ => {}
            }
        }

        if files.is_empty() {
            return CommandOutput::error("tee: missing file operand\n".to_string(), 1);
        }

        let input = match stdin {
            Some(s) => s.to_string(),
            None => return CommandOutput::error("tee: no input\n".to_string(), 1),
        };

        for file in &files {
            let resolved = match self.vfs.resolve(file, &self.cwd) {
                Ok(r) => r,
                Err(e) => return CommandOutput::error(format!("tee: {}: {}\n", file, e), 1),
            };
            let result: Result<(), String> = if append {
                use std::io::Write;
                std::fs::OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(&resolved)
                    .map_err(|e| e.to_string())
                    .and_then(|mut f| f.write_all(input.as_bytes()).map_err(|e| e.to_string()))
            } else {
                self.vfs.write(file, &self.cwd, &input).map_err(|e| e.to_string())
            };
            if let Err(e) = result {
                return CommandOutput::error(format!("tee: {}: {}\n", file, e), 1);
            }
        }

        CommandOutput::success(input)
    }
}
