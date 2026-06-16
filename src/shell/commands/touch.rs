use crate::shell::{Shell, CommandOutput};
use std::fs;

impl Shell {
    pub fn cmd_touch(&self, args: &[&str]) -> CommandOutput {
        for arg in args {
            if arg.starts_with('-') {
                continue;
            }
            let target = match self.vfs.resolve(arg, &self.cwd) {
                Ok(p) => p,
                Err(e) => return CommandOutput::error(format!("touch: {}: {}\n", arg, e), 1),
            };

            if target.exists() {
                let now = filetime::FileTime::now();
                if let Err(e) = filetime::set_file_mtime(&target, now) {
                    return CommandOutput::error(format!("touch: {}: {}\n", arg, e), 1);
                }
            } else {
                match fs::File::create(&target) {
                    Ok(_) => {}
                    Err(e) => {
                        return CommandOutput::error(format!("touch: {}: {}\n", arg, e), 1);
                    }
                }
            }
        }
        CommandOutput::success(String::new())
    }
}
