use crate::shell::{Shell, CommandOutput};

impl Shell {
    pub fn cmd_mkdir(&self, args: &[&str]) -> CommandOutput {
        let mut create_parents = false;
        let mut dirs = Vec::new();

        for arg in args {
            if *arg == "-p" {
                create_parents = true;
            } else if !arg.starts_with('-') {
                dirs.push(arg.to_string());
            }
        }

        if dirs.is_empty() {
            return CommandOutput::error("mkdir: missing operand\n".to_string(), 1);
        }

        for dir in &dirs {
            let result = if create_parents {
                self.vfs.create_dir_all(dir, &self.cwd)
            } else {
                self.vfs.create_dir(dir, &self.cwd)
            };
            if let Err(e) = result {
                return CommandOutput::error(format!("mkdir: {}: {}\n", dir, e), 1);
            }
        }

        CommandOutput::success(String::new())
    }
}
